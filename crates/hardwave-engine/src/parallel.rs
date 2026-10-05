//! Running the graph on more than one core.
//!
//! A song where one track costs more than the whole block's budget is
//! one that cannot play, however many cores the machine has, because
//! the graph used to run node after node on the audio thread alone. On
//! a modern machine that leaves seven eighths of the CPU idle while the
//! audio thread runs out of time.
//!
//! What makes this safe is the shape of the graph rather than any lock.
//! Nodes are grouped into levels: a node's level is one past the
//! highest level of anything that feeds it. Two nodes in the same level
//! therefore have no edge between them, so one can never read what the
//! other is writing. A level is handed to the workers, every worker
//! takes nodes from it until there are none left, and the level is done
//! before the next one starts.
//!
//! The pool is off until it is switched on, and the render is the same
//! either way: the tests check that the parallel path and the plain one
//! produce identical samples.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Which level each node belongs to, and the nodes of each level in
/// order. Worked out when the graph is built, not per block.
#[derive(Debug, Default, Clone)]
pub struct Levels {
    /// Node ids grouped by level, lowest first.
    pub levels: Vec<Vec<usize>>,
}

impl Levels {
    /// Group a topological order into levels.
    ///
    /// `order` must be a topological order of the graph; `sources_of`
    /// gives the nodes feeding one node. A node with nothing feeding it
    /// is level zero.
    pub fn build(
        order: &[usize],
        node_count: usize,
        sources_of: impl Fn(usize) -> Vec<usize>,
    ) -> Self {
        let mut level_of = vec![0usize; node_count];
        for &node in order {
            let mut level = 0usize;
            for src in sources_of(node) {
                level = level.max(level_of[src] + 1);
            }
            level_of[node] = level;
        }
        let depth = order.iter().map(|&n| level_of[n] + 1).max().unwrap_or(0);
        let mut levels = vec![Vec::new(); depth];
        // Keep each level in the topological order's own sequence, so a
        // single-threaded run of the levels is the order the graph had.
        for &node in order {
            levels[level_of[node]].push(node);
        }
        Self { levels }
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }
}

/// One batch of work: the nodes of a level, and the function that runs
/// one node. The pointer is to the caller's own stack frame, which
/// outlives the batch because the caller waits for it to finish.
struct Batch {
    nodes: *const usize,
    count: usize,
    /// The closure, as a bare pointer pair. A trait-object pointer
    /// cannot be stored with a borrowed lifetime, and the borrow is
    /// what the waiting publisher guarantees instead.
    run: *const (),
    run_vtable: *const (),
}

// SAFETY: a batch is only ever read while the thread that published it
// is parked inside `run_level`, which does not return until every
// worker has finished with it. The pointers are to that frame.
unsafe impl Send for Batch {}
unsafe impl Sync for Batch {}

struct Shared {
    batch: Mutex<Option<Batch>>,
    /// Woken when a batch is published.
    ready: Condvar,
    /// Index of the next node to take out of the current batch.
    next: AtomicUsize,
    /// How many nodes of the batch are still unfinished.
    remaining: AtomicUsize,
    /// A batch counter, so a worker knows whether it has already run
    /// the batch it is looking at.
    epoch: AtomicUsize,
    shutdown: AtomicBool,
}

/// A handful of threads that stay alive between blocks.
///
/// Threads are not started per block: starting one costs more than the
/// block itself, and the audio thread must not allocate or wait on the
/// scheduler more than it has to.
pub struct WorkerPool {
    shared: Arc<Shared>,
    workers: Vec<std::thread::JoinHandle<()>>,
}

impl WorkerPool {
    /// Start `threads` workers beside the audio thread, which does its
    /// own share of the work as well.
    pub fn new(threads: usize) -> Self {
        let shared = Arc::new(Shared {
            batch: Mutex::new(None),
            ready: Condvar::new(),
            next: AtomicUsize::new(0),
            remaining: AtomicUsize::new(0),
            epoch: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
        });
        let mut workers = Vec::with_capacity(threads);
        for index in 0..threads {
            let shared = Arc::clone(&shared);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("hardwave-audio-worker-{index}"))
                    .spawn(move || worker_loop(shared))
                    .expect("audio worker thread"),
            );
        }
        Self { shared, workers }
    }

    /// How many threads are helping, not counting the audio thread.
    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// Run `run` for every node of the level, across the pool and this
    /// thread, and return once they are all done.
    pub fn run_level(&self, nodes: &[usize], run: &(dyn Fn(usize) + Sync)) {
        if nodes.is_empty() {
            return;
        }
        // One node on its own is not worth waking anybody for.
        if nodes.len() == 1 || self.workers.is_empty() {
            for &node in nodes {
                run(node);
            }
            return;
        }

        self.shared.next.store(0, Ordering::Release);
        self.shared.remaining.store(nodes.len(), Ordering::Release);
        {
            let mut slot = self.shared.batch.lock().expect("batch lock");
            // SAFETY: the two halves of a trait-object pointer are
            // taken apart here and put back together in the worker.
            // Both halves stay valid because this call does not return
            // until every worker has finished with the batch.
            let raw: *const (dyn Fn(usize) + Sync) = run;
            let parts: (*const (), *const ()) = unsafe { std::mem::transmute(raw) };
            *slot = Some(Batch {
                nodes: nodes.as_ptr(),
                count: nodes.len(),
                run: parts.0,
                run_vtable: parts.1,
            });
            self.shared.epoch.fetch_add(1, Ordering::AcqRel);
        }
        self.shared.ready.notify_all();

        // This thread takes work too rather than standing idle.
        take_work(&self.shared, nodes, run);

        // Wait for the stragglers. A spin first, because the usual case
        // is that they are a few microseconds behind.
        let mut spins = 0u32;
        while self.shared.remaining.load(Ordering::Acquire) > 0 {
            if spins < 2_000 {
                std::hint::spin_loop();
                spins += 1;
            } else {
                std::thread::yield_now();
            }
        }

        let mut slot = self.shared.batch.lock().expect("batch lock");
        *slot = None;
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.ready.notify_all();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

/// Take nodes from the current batch until there are none left.
fn take_work(shared: &Shared, nodes: &[usize], run: &dyn Fn(usize)) {
    loop {
        let index = shared.next.fetch_add(1, Ordering::AcqRel);
        if index >= nodes.len() {
            return;
        }
        run(nodes[index]);
        shared.remaining.fetch_sub(1, Ordering::AcqRel);
    }
}

fn worker_loop(shared: Arc<Shared>) {
    let mut seen_epoch = 0usize;
    loop {
        let (nodes, run) = {
            let mut slot = shared.batch.lock().expect("batch lock");
            loop {
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                let epoch = shared.epoch.load(Ordering::Acquire);
                if epoch != seen_epoch {
                    if let Some(batch) = slot.as_ref() {
                        seen_epoch = epoch;
                        // SAFETY: the publisher waits inside `run_level`
                        // until `remaining` reaches zero, so both
                        // pointers are live for as long as this batch is
                        // being worked on.
                        let nodes = unsafe { std::slice::from_raw_parts(batch.nodes, batch.count) };
                        let raw: *const (dyn Fn(usize) + Sync) =
                            unsafe { std::mem::transmute((batch.run, batch.run_vtable)) };
                        let run: &(dyn Fn(usize) + Sync) = unsafe { &*raw };
                        break (nodes, run);
                    }
                }
                slot = shared.ready.wait(slot).expect("batch wait");
            }
        };
        take_work(&shared, nodes, run);
    }
}

/// How many workers to start beside the audio thread.
///
/// One per core minus the audio thread itself, capped: past a handful
/// the waking costs more than the work saved for a block of a few
/// hundred samples.
pub fn default_worker_count() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    cores.saturating_sub(1).min(5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    #[test]
    fn a_chain_is_one_node_per_level() {
        // 0 -> 1 -> 2
        let order = vec![0, 1, 2];
        let levels = Levels::build(&order, 3, |n| if n == 0 { vec![] } else { vec![n - 1] });
        assert_eq!(levels.levels, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn nodes_that_feed_the_same_node_share_a_level() {
        // 0 and 1 both feed 2.
        let order = vec![0, 1, 2];
        let levels = Levels::build(&order, 3, |n| if n == 2 { vec![0, 1] } else { vec![] });
        assert_eq!(levels.levels, vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn a_level_holds_nothing_that_feeds_another_node_in_it() {
        // A wider graph: 0,1 -> 2; 2,3 -> 4.
        let order = vec![0, 1, 3, 2, 4];
        let levels = Levels::build(&order, 5, |n| match n {
            2 => vec![0, 1],
            4 => vec![2, 3],
            _ => vec![],
        });
        // 3 has nothing feeding it, so it sits with 0 and 1.
        assert_eq!(levels.levels[0], vec![0, 1, 3]);
        assert_eq!(levels.levels[1], vec![2]);
        assert_eq!(levels.levels[2], vec![4]);
    }

    #[test]
    fn every_node_of_a_level_runs_exactly_once() {
        let pool = WorkerPool::new(3);
        let counts: Vec<AtomicU32> = (0..64).map(|_| AtomicU32::new(0)).collect();
        let nodes: Vec<usize> = (0..64).collect();
        let run = |node: usize| {
            counts[node].fetch_add(1, Ordering::Relaxed);
        };
        for _ in 0..50 {
            pool.run_level(&nodes, &run);
        }
        for (node, count) in counts.iter().enumerate() {
            assert_eq!(count.load(Ordering::Relaxed), 50, "node {node}");
        }
    }

    #[test]
    fn a_pool_with_no_workers_still_runs_everything() {
        let pool = WorkerPool::new(0);
        let seen: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        let run = |node: usize| seen.lock().unwrap().push(node);
        pool.run_level(&[4, 7, 1], &run);
        assert_eq!(*seen.lock().unwrap(), vec![4, 7, 1]);
    }
}
