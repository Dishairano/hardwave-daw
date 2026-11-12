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

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

/// How the claim word is split: the batch number above, the index of
/// the next node below.
const INDEX_BITS: u32 = 24;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
/// Batch numbers count up to this and start again at zero. 2^40
/// batches is about four months even at a hundred thousand levels a
/// second, so a worker would have to be held up that long for an old
/// batch number to match again.
const EPOCH_MASK: u64 = (1 << (64 - INDEX_BITS)) - 1;
/// The most nodes a level can have and still be shared out: an index
/// below this plus one still fits under the batch number.
const MAX_SHARED_LEVEL: usize = INDEX_MASK as usize;

/// The claim word for node `index` of batch `epoch`.
fn pack(epoch: u64, index: usize) -> u64 {
    (epoch << INDEX_BITS) | index as u64
}

/// One batch of work: the nodes of a level, and the function that runs
/// one node. The pointers are to the publisher's own stack frame, which
/// is only certain to be alive while a node of this batch is claimed
/// and unfinished, so they stay raw pointers and are only followed
/// after a successful [`claim`].
#[derive(Clone, Copy)]
struct Batch {
    /// Which batch this is. A claim only succeeds while the claim word
    /// still carries this number.
    epoch: u64,
    nodes: *const usize,
    count: usize,
    /// The closure, as a bare pointer pair. A trait-object pointer
    /// cannot be stored with a borrowed lifetime, and the borrow is
    /// what the waiting publisher guarantees instead.
    run: *const (),
    run_vtable: *const (),
}

// SAFETY: the pointers are only followed for a node a thread has
// claimed, and the publisher stays inside `run_level` until every
// claimed node has finished.
unsafe impl Send for Batch {}
unsafe impl Sync for Batch {}

struct Shared {
    batch: Mutex<Option<Batch>>,
    /// Woken when a batch is published.
    ready: Condvar,
    /// The current batch's number in the top 40 bits and the index of
    /// the next node to take out of it in the low 24. One word, so a
    /// worker still holding an older batch cannot take a node out of
    /// the new one: its claim sees a different number and fails.
    claim: AtomicU64,
    /// How many nodes of the batch are still unfinished. Only a thread
    /// that claimed a node of the current batch takes one off, so the
    /// count cannot be touched by a batch that is already over.
    remaining: AtomicUsize,
    /// Counts the batches published; the batch number is its low 40
    /// bits.
    epoch: AtomicU64,
    shutdown: AtomicBool,
    /// Called by a worker before each claim, so a test can hold it at
    /// the point where the scheduler could take it off the core.
    #[cfg(test)]
    before_claim: Option<Box<dyn Fn() + Send + Sync>>,
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
        Self::start(
            threads,
            #[cfg(test)]
            None,
        )
    }

    fn start(
        threads: usize,
        #[cfg(test)] before_claim: Option<Box<dyn Fn() + Send + Sync>>,
    ) -> Self {
        let shared = Arc::new(Shared {
            batch: Mutex::new(None),
            ready: Condvar::new(),
            claim: AtomicU64::new(0),
            remaining: AtomicUsize::new(0),
            epoch: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            #[cfg(test)]
            before_claim,
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
        // One node on its own is not worth waking anybody for. A level
        // too big for the claim word's index (16 million nodes, never in
        // practice) runs here as well.
        if nodes.len() == 1 || self.workers.is_empty() || nodes.len() > MAX_SHARED_LEVEL {
            for &node in nodes {
                run(node);
            }
            return;
        }

        // SAFETY: the two halves of a trait-object pointer are taken
        // apart here and put back together in `take_work`. Both halves
        // stay valid because this call does not return until every
        // claimed node has finished.
        let raw: *const (dyn Fn(usize) + Sync) = run;
        let parts: (*const (), *const ()) = unsafe { std::mem::transmute(raw) };
        let epoch = self
            .shared
            .epoch
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
            & EPOCH_MASK;
        let batch = Batch {
            epoch,
            nodes: nodes.as_ptr(),
            count: nodes.len(),
            run: parts.0,
            run_vtable: parts.1,
        };
        // The count first, then the claim word: a thread whose claim
        // sees the new number also sees the new count.
        self.shared.remaining.store(nodes.len(), Ordering::Relaxed);
        self.shared.claim.store(pack(epoch, 0), Ordering::Release);
        {
            let mut slot = self.shared.batch.lock().expect("batch lock");
            *slot = Some(batch);
        }
        self.shared.ready.notify_all();

        // This thread takes work too rather than standing idle.
        take_work(&self.shared, &batch, false);

        // Wait for the stragglers. A spin first, because the usual case
        // is that they are a few microseconds behind. A worker that took
        // the batch but has not claimed a node is not waited for: its
        // claim will fail once the batch is over.
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

/// Take the next node of batch `epoch`, or None once it has none left
/// or is no longer the current batch.
///
/// A worker can be taken off the core at any moment, between taking
/// the batch and claiming its first node or between finishing one node
/// and claiming the next. By the time it runs again the publisher may
/// have finished the level, started the next one, or returned and
/// freed the frame the batch points into. A plain counter let such a
/// worker take an index of the new level and run a node of the old one
/// with it: the closure's frame was gone, and the new level lost a node
/// and its unfinished count. Here the batch number travels with the
/// index, so the claim fails instead.
fn claim(shared: &Shared, epoch: u64, count: usize) -> Option<usize> {
    let mut current = shared.claim.load(Ordering::Acquire);
    loop {
        let index = (current & INDEX_MASK) as usize;
        if current >> INDEX_BITS != epoch || index >= count {
            return None;
        }
        // index < count <= MAX_SHARED_LEVEL, so the add cannot reach
        // the batch number's bits.
        match shared.claim.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(index),
            Err(seen) => current = seen,
        }
    }
}

/// Take nodes from the batch until there are none left.
#[cfg_attr(not(test), allow(unused_variables))]
fn take_work(shared: &Shared, batch: &Batch, on_worker: bool) {
    loop {
        #[cfg(test)]
        if on_worker {
            if let Some(hook) = &shared.before_claim {
                hook();
            }
        }
        let Some(index) = claim(shared, batch.epoch, batch.count) else {
            return;
        };
        // SAFETY: this thread holds an unfinished node of the batch, so
        // `remaining` is above zero and the publisher is still inside
        // `run_level`, keeping the node list and the closure alive until
        // the count is taken off below. Nothing of the batch is touched
        // after that.
        unsafe {
            let node = *batch.nodes.add(index);
            let raw: *const (dyn Fn(usize) + Sync) =
                std::mem::transmute((batch.run, batch.run_vtable));
            (*raw)(node);
        }
        shared.remaining.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What the last worker to start got when it asked for real-time
/// priority, for the diagnostics.
pub static WORKER_PRIORITY: hardwave_audio_io::thread_priority::PriorityStatus =
    hardwave_audio_io::thread_priority::PriorityStatus::new();

fn worker_loop(shared: Arc<Shared>) {
    // These threads run plug-ins and effects like the audio thread does,
    // and only the audio thread had denormals flushed: a reverb tail on a
    // worker took the slow path and spiked the load. The workers live as
    // long as the pool, so once is enough.
    hardwave_audio_io::denormals::flush_denormals_to_zero();
    // They also have to keep up with the audio thread they work for: at
    // normal priority any busy program on the machine delays a level,
    // and the audio thread waits for it. Same class as the audio thread.
    WORKER_PRIORITY.set(hardwave_audio_io::thread_priority::raise_current_thread());
    let mut seen_epoch = 0u64;
    loop {
        let batch = {
            let mut slot = shared.batch.lock().expect("batch lock");
            loop {
                if shared.shutdown.load(Ordering::Acquire) {
                    return;
                }
                if let Some(batch) = *slot {
                    if batch.epoch != seen_epoch {
                        seen_epoch = batch.epoch;
                        break batch;
                    }
                }
                slot = shared.ready.wait(slot).expect("batch wait");
            }
        };
        take_work(&shared, &batch, true);
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
    #[cfg(target_arch = "x86_64")]
    fn the_workers_flush_denormals_like_the_audio_thread() {
        let pool = WorkerPool::new(2);
        let on_workers = AtomicUsize::new(0);
        let flushed = AtomicUsize::new(0);
        let nodes: Vec<usize> = (0..16).collect();
        pool.run_level(&nodes, &|_| {
            let name = std::thread::current().name().unwrap_or("").to_string();
            if name.starts_with("hardwave-audio-worker") {
                on_workers.fetch_add(1, Ordering::Relaxed);
                // SAFETY: reads this thread's SSE control register.
                if unsafe { hardwave_audio_io::denormals::read_mxcsr() } & (1 << 15) != 0 {
                    flushed.fetch_add(1, Ordering::Relaxed);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        });
        let ran = on_workers.load(Ordering::Relaxed);
        assert!(ran > 0, "the workers took part");
        assert_eq!(flushed.load(Ordering::Relaxed), ran);
    }

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
    fn the_workers_ask_for_the_audio_thread_s_priority() {
        let _pool = WorkerPool::new(1);
        // Each worker asks as it starts; give it the time to.
        let asked = (0..200).any(|_| {
            if WORKER_PRIORITY.get() != hardwave_audio_io::thread_priority::PriorityOutcome::Pending
            {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            false
        });
        assert!(asked, "no worker asked for real-time priority");
    }

    #[test]
    fn a_pool_with_no_workers_still_runs_everything() {
        let pool = WorkerPool::new(0);
        let seen: Mutex<Vec<usize>> = Mutex::new(Vec::new());
        let run = |node: usize| seen.lock().unwrap().push(node);
        pool.run_level(&[4, 7, 1], &run);
        assert_eq!(*seen.lock().unwrap(), vec![4, 7, 1]);
    }

    /// Holds every worker at its next claim while `hold` is set.
    #[derive(Default)]
    struct Gate {
        hold: bool,
        /// A worker is sitting at the gate.
        waiting: bool,
        /// How many times a worker has gone through it.
        passed: usize,
    }

    fn on_worker() -> bool {
        std::thread::current()
            .name()
            .is_some_and(|name| name.starts_with("hardwave-audio-worker"))
    }

    /// Wait for `done` for up to five seconds.
    fn wait_until(done: impl Fn() -> bool) -> bool {
        let start = std::time::Instant::now();
        while !done() {
            if start.elapsed() > std::time::Duration::from_secs(5) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        true
    }

    #[test]
    fn a_worker_held_up_past_its_level_takes_nothing_from_the_next() {
        // The crash on the test machine: a worker takes level A and is
        // taken off the core before its claim. The audio thread does all
        // of A itself and moves on to level B. When the worker runs
        // again it must not take a node out of B for A's closure.
        let gate = Arc::new((
            Mutex::new(Gate {
                hold: true,
                ..Gate::default()
            }),
            Condvar::new(),
        ));
        let hook_gate = Arc::clone(&gate);
        let pool = WorkerPool::start(
            1,
            Some(Box::new(move || {
                let (lock, cond) = &*hook_gate;
                let mut g = lock.lock().unwrap();
                while g.hold {
                    g.waiting = true;
                    cond.notify_all();
                    g = cond.wait(g).unwrap();
                }
                g.waiting = false;
                g.passed += 1;
                cond.notify_all();
            })),
        );
        let gate_state = |f: &dyn Fn(&Gate) -> bool| f(&gate.0.lock().unwrap());

        let nodes: Vec<usize> = (0..8).collect();
        let a_runs: Vec<AtomicU32> = (0..8).map(|_| AtomicU32::new(0)).collect();
        let b_runs: Vec<AtomicU32> = (0..8).map(|_| AtomicU32::new(0)).collect();
        let a_over = AtomicBool::new(false);
        let late = AtomicU32::new(0);
        let worker_in_b = AtomicU32::new(0);

        let run_a = |node: usize| {
            if a_over.load(Ordering::SeqCst) {
                late.fetch_add(1, Ordering::SeqCst);
            }
            a_runs[node].fetch_add(1, Ordering::SeqCst);
            // Do not let the level end before the worker has taken it
            // and stopped at its claim.
            if node == 0 && !on_worker() {
                assert!(wait_until(|| gate_state(&|g| g.waiting)), "worker joined A");
            }
        };
        pool.run_level(&nodes, &run_a);
        a_over.store(true, Ordering::SeqCst);

        let run_b = |node: usize| {
            b_runs[node].fetch_add(1, Ordering::SeqCst);
            if on_worker() {
                worker_in_b.fetch_add(1, Ordering::SeqCst);
            }
            if node == 0 && !on_worker() {
                // Let the held worker go while B is under way, and keep
                // B open until it has done something with it.
                {
                    let (lock, cond) = &*gate;
                    lock.lock().unwrap().hold = false;
                    cond.notify_all();
                }
                assert!(
                    wait_until(|| gate_state(&|g| g.passed > 0)),
                    "worker resumed"
                );
                assert!(
                    wait_until(|| {
                        late.load(Ordering::SeqCst) > 0 || worker_in_b.load(Ordering::SeqCst) > 0
                    }),
                    "the held worker did nothing once let go"
                );
            }
        };
        pool.run_level(&nodes, &run_b);

        assert_eq!(
            late.load(Ordering::SeqCst),
            0,
            "A's closure ran after A was over"
        );
        for node in 0..8 {
            assert_eq!(
                a_runs[node].load(Ordering::SeqCst),
                1,
                "level A node {node}"
            );
            assert_eq!(
                b_runs[node].load(Ordering::SeqCst),
                1,
                "level B node {node}"
            );
        }
    }

    #[test]
    fn levels_never_overlap_when_workers_are_preempted_at_random() {
        // Workers stop for a moment at random before their claims, as
        // the scheduler does to them, while the audio thread runs level
        // after level. Every node runs once, in its own level only.
        let seed = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
        let pool = WorkerPool::start(
            3,
            Some(Box::new(move || {
                // A shared xorshift is random enough for this.
                let mut x = seed.load(Ordering::Relaxed);
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                seed.store(x, Ordering::Relaxed);
                match x % 16 {
                    0 => std::thread::sleep(std::time::Duration::from_micros(x % 300)),
                    1..=4 => std::thread::yield_now(),
                    _ => {}
                }
            })),
        );
        let current = AtomicUsize::new(0);
        let overlaps = AtomicUsize::new(0);
        let runs: Vec<AtomicU32> = (0..16).map(|_| AtomicU32::new(0)).collect();
        let all: Vec<usize> = (0..16).collect();
        for level in 0..3_000usize {
            let nodes = &all[..2 + level % 15];
            let run = |node: usize| {
                if current.load(Ordering::SeqCst) != level {
                    overlaps.fetch_add(1, Ordering::SeqCst);
                }
                runs[node].fetch_add(1, Ordering::SeqCst);
                if node.is_multiple_of(3) {
                    std::hint::spin_loop();
                }
            };
            pool.run_level(nodes, &run);
            for (node, count) in runs.iter().enumerate() {
                let want = u32::from(node < nodes.len());
                assert_eq!(
                    count.swap(0, Ordering::SeqCst),
                    want,
                    "level {level} node {node}"
                );
            }
            current.store(level + 1, Ordering::SeqCst);
        }
        assert_eq!(
            overlaps.load(Ordering::SeqCst),
            0,
            "a node ran outside its level"
        );
    }

    #[test]
    fn a_claim_from_a_batch_two_to_the_32_back_fails() {
        // With a 32-bit batch number these two were the same batch.
        let pool = WorkerPool::new(0);
        let now = 5 + (1u64 << 32);
        pool.shared.claim.store(pack(now, 0), Ordering::SeqCst);
        assert_eq!(claim(&pool.shared, 5, 10), None);
        assert_eq!(claim(&pool.shared, now, 10), Some(0));
        assert_eq!(claim(&pool.shared, now, 10), Some(1));
    }

    #[test]
    fn the_batch_number_starts_again_without_losing_a_node() {
        let pool = WorkerPool::new(2);
        pool.shared.epoch.store(EPOCH_MASK - 3, Ordering::SeqCst);
        let counts: Vec<AtomicU32> = (0..16).map(|_| AtomicU32::new(0)).collect();
        let nodes: Vec<usize> = (0..16).collect();
        let run = |node: usize| {
            counts[node].fetch_add(1, Ordering::SeqCst);
        };
        for level in 0..10 {
            pool.run_level(&nodes, &run);
            for (node, count) in counts.iter().enumerate() {
                assert_eq!(
                    count.swap(0, Ordering::SeqCst),
                    1,
                    "level {level} node {node}"
                );
            }
        }
    }
}
