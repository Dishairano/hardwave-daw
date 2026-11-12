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
//! Handing a level out takes the audio thread a few atomic operations:
//! no lock and no condition variable. It then works on the level itself,
//! and waits for the rest by spinning for a few microseconds and, if the
//! last node is still running after that, by sleeping until the worker
//! that finishes it wakes it. Idle workers spin for a moment after each
//! level, because the next one usually follows within microseconds, and
//! then sleep on their own thread's wake-up. A level that takes too
//! little work to be worth a wake-up runs on the audio thread alone.
//!
//! The pool is off until it is switched on, and the render is the same
//! either way: the tests check that the parallel path and the plain one
//! produce identical samples.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::Thread;
use std::time::{Duration, Instant};

/// Which level each node belongs to, and the nodes of each level in
/// order. Worked out when the graph is built, not per block.
#[derive(Debug, Default, Clone)]
pub struct Levels {
    /// Node ids grouped by level, lowest first.
    pub levels: Vec<Vec<usize>>,
    /// What each level has been costing, to decide whether it is worth
    /// sharing out. One per level, made with the levels.
    cost: Vec<LevelCost>,
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
        Self {
            levels,
            cost: vec![LevelCost::default(); depth],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }
}

/// A level is shared out once its work, summed over every thread that
/// ran part of it, reaches this. Below it, waking a sleeping worker
/// (tens of microseconds on Windows, more on a busy machine) and
/// waiting for the slowest one costs about as much as it saves.
const SHARE_ABOVE_NS: u64 = 60_000;
/// A shared level goes back to the audio thread alone once its work
/// falls under this. Lower than [`SHARE_ABOVE_NS`] so a level near the
/// line does not switch every block.
const SHARE_BELOW_NS: u64 = 30_000;

/// What one level has cost over the last blocks.
#[derive(Debug, Default, Clone, Copy)]
struct LevelCost {
    /// Nanoseconds of work, smoothed. Rises at once and falls slowly, so
    /// a part of the song that suddenly gets heavy is shared out from
    /// the next block on.
    work_ns: u64,
    measured: bool,
    shared: bool,
}

impl LevelCost {
    /// Whether to share the level out this block. A level that has not
    /// been measured yet is: running a heavy level on one thread is a
    /// late block, and a light one shared costs only a wake-up.
    fn worth_sharing(&self, nodes: usize) -> bool {
        if nodes < 2 {
            return false;
        }
        if !self.measured {
            return true;
        }
        let line = if self.shared {
            SHARE_BELOW_NS
        } else {
            SHARE_ABOVE_NS
        };
        self.work_ns >= line
    }

    fn note(&mut self, ns: u64, shared: bool) {
        self.work_ns = if !self.measured || ns >= self.work_ns {
            ns
        } else {
            self.work_ns - self.work_ns / 4 + ns / 4
        };
        self.measured = true;
        self.shared = shared;
    }
}

/// How the claim word is split: the batch number above, the number of
/// nodes not yet taken out of the batch below.
const INDEX_BITS: u32 = 24;
const INDEX_MASK: u64 = (1 << INDEX_BITS) - 1;
/// Batch numbers count up to this and start again at zero. 2^40
/// batches is about four months even at a hundred thousand levels a
/// second, so a worker would have to be held up that long for an old
/// batch number to match again.
const EPOCH_MASK: u64 = (1 << (64 - INDEX_BITS)) - 1;
/// The most nodes a level can have and still be shared out: the count
/// has to fit under the batch number.
const MAX_SHARED_LEVEL: usize = INDEX_MASK as usize;
/// Set in the unfinished count while the audio thread sleeps waiting
/// for it, so the worker that finishes the last node knows to wake it.
const WAITER: u64 = 1 << 63;
/// Workers have a bit each in the mask of sleeping workers.
const MAX_WORKERS: usize = 64;

/// The claim word for batch `epoch` with `left` nodes still to take.
fn pack(epoch: u64, left: usize) -> u64 {
    (epoch << INDEX_BITS) | left as u64
}

/// How long the threads spin before they sleep.
#[derive(Debug, Clone, Copy)]
struct Spin {
    /// A worker after its last node, waiting for the next level.
    worker: Duration,
    /// The audio thread, waiting for the workers to finish a level.
    publisher: Duration,
}

impl Spin {
    /// Long enough to cover the gap between two levels of a block, short
    /// enough that a pool between blocks costs next to nothing.
    const DEFAULT: Spin = Spin {
        worker: Duration::from_micros(50),
        publisher: Duration::from_micros(20),
    };

    /// Straight to sleep. With more threads than cores, a thread that
    /// spins can only be taking the core from the one whose work it is
    /// waiting for.
    const NONE: Spin = Spin {
        worker: Duration::ZERO,
        publisher: Duration::ZERO,
    };

    fn for_threads(threads: usize) -> Spin {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        if threads + 1 > cores {
            Spin::NONE
        } else {
            Spin::DEFAULT
        }
    }
}

struct Shared {
    /// The current batch's number in the top 40 bits and how many of its
    /// nodes are still to be taken in the low 24. One word, so a worker
    /// still holding an older batch cannot take a node out of the new
    /// one: its claim sees a different number and fails.
    claim: AtomicU64,
    /// How many nodes of the batch are still unfinished, plus [`WAITER`]
    /// while the audio thread sleeps on it. Only a thread that claimed a
    /// node of the current batch takes one off, so the count cannot be
    /// touched by a batch that is already over.
    remaining: AtomicU64,
    /// Counts the batches published; the batch number is its low 40
    /// bits.
    epoch: AtomicU64,
    /// The current batch's node list. Written before its claim word, and
    /// only followed after a successful claim (see [`take_work`]).
    nodes: AtomicPtr<usize>,
    /// The current batch's closure: a pointer to the publisher's own
    /// reference to it, on its stack frame. Same rules as `nodes`.
    run: AtomicPtr<()>,
    /// Nanoseconds the workers spent on the current batch.
    busy_ns: AtomicU64,
    /// Set while a thread is handing out a level.
    publishing: AtomicBool,
    /// A node of the current level panicked on a worker.
    worker_panicked: AtomicBool,
    /// Set when the pool is dropped.
    shutdown: AtomicBool,
    /// One bit per worker that is asleep or about to be.
    sleeping: AtomicU64,
    /// The workers, to wake them. Set before the pool is handed out.
    threads: OnceLock<Box<[Thread]>>,
    spin: Spin,
    stats: Counters,
    #[cfg(test)]
    hooks: TestHooks,
}

#[derive(Default)]
struct Counters {
    shared: AtomicU64,
    alone: AtomicU64,
    waits_slept: AtomicU64,
    wakes: AtomicU64,
}

/// What the pool has done since it started, for the diagnostics.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PoolStats {
    /// Levels handed out to the workers.
    pub levels_shared: u64,
    /// Levels of more than one node run on the audio thread alone,
    /// because they took too little work to be worth sharing.
    pub levels_alone: u64,
    /// Times the audio thread slept waiting for a worker to finish.
    pub waits_slept: u64,
    /// Sleeping workers woken.
    pub worker_wakes: u64,
}

/// Places where a test can hold a worker, to stand in for the scheduler
/// taking it off the core there.
#[cfg(test)]
#[derive(Default)]
struct TestHooks {
    /// Before each claim.
    before_claim: Option<Box<dyn Fn() + Send + Sync>>,
    /// After saying it is going to sleep and before checking one last
    /// time for work and sleeping.
    before_wait: Option<Box<dyn Fn() + Send + Sync>>,
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
        Self::start_with(
            threads,
            Spin::for_threads(threads),
            #[cfg(test)]
            TestHooks::default(),
        )
    }

    #[cfg(test)]
    fn start(threads: usize, hooks: TestHooks) -> Self {
        Self::start_with(threads, Spin::DEFAULT, hooks)
    }

    fn start_with(threads: usize, spin: Spin, #[cfg(test)] hooks: TestHooks) -> Self {
        let threads = threads.min(MAX_WORKERS);
        let shared = Arc::new(Shared {
            claim: AtomicU64::new(0),
            remaining: AtomicU64::new(0),
            epoch: AtomicU64::new(0),
            nodes: AtomicPtr::new(std::ptr::null_mut()),
            run: AtomicPtr::new(std::ptr::null_mut()),
            busy_ns: AtomicU64::new(0),
            publishing: AtomicBool::new(false),
            worker_panicked: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            sleeping: AtomicU64::new(0),
            threads: OnceLock::new(),
            spin,
            stats: Counters::default(),
            #[cfg(test)]
            hooks,
        });
        let mut workers = Vec::with_capacity(threads);
        for index in 0..threads {
            let shared = Arc::clone(&shared);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("hardwave-audio-worker-{index}"))
                    .spawn(move || worker_loop(shared, index))
                    .expect("audio worker thread"),
            );
        }
        // Before anything is published, so a worker passing a wake-up on
        // always finds the others.
        let _ = shared
            .threads
            .set(workers.iter().map(|w| w.thread().clone()).collect());
        Self { shared, workers }
    }

    /// How many threads are helping, not counting the audio thread.
    pub fn threads(&self) -> usize {
        self.workers.len()
    }

    /// What the pool has done since it started.
    pub fn stats(&self) -> PoolStats {
        let s = &self.shared.stats;
        PoolStats {
            levels_shared: s.shared.load(Ordering::Relaxed),
            levels_alone: s.alone.load(Ordering::Relaxed),
            waits_slept: s.waits_slept.load(Ordering::Relaxed),
            worker_wakes: s.wakes.load(Ordering::Relaxed),
        }
    }

    /// Run `run` for every node of the level, across the pool and this
    /// thread, and return once they are all done.
    pub fn run_level(&self, nodes: &[usize], run: &(dyn Fn(usize) + Sync)) {
        self.share_level(nodes, run);
    }

    /// Run every level in turn, each across the pool or, when it has
    /// been taking too little work to be worth waking anybody for, on
    /// this thread alone.
    pub fn run_levels(&self, levels: &mut Levels, run: &(dyn Fn(usize) + Sync)) {
        if self.workers.is_empty() {
            for nodes in &levels.levels {
                run_alone(nodes, run);
            }
            return;
        }
        let Levels { levels, cost } = levels;
        for (index, nodes) in levels.iter().enumerate() {
            let Some(cost) = cost.get_mut(index) else {
                self.share_level(nodes, run);
                continue;
            };
            if cost.worth_sharing(nodes.len()) {
                if let Some(work) = self.share_level(nodes, run) {
                    cost.note(work, true);
                }
            } else {
                let started = Instant::now();
                run_alone(nodes, run);
                cost.note(started.elapsed().as_nanos() as u64, false);
                if nodes.len() > 1 {
                    self.shared.stats.alone.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Share one level out and wait for it. Returns the nanoseconds of
    /// work it took, summed over the threads, or None when it ran here
    /// alone without being shared.
    fn share_level(&self, nodes: &[usize], run: &(dyn Fn(usize) + Sync)) -> Option<u64> {
        // One node on its own is not worth waking anybody for. A level
        // too big for the claim word's count (16 million nodes, never in
        // practice) runs here as well.
        if nodes.len() < 2 || self.workers.is_empty() || nodes.len() > MAX_SHARED_LEVEL {
            run_alone(nodes, run);
            return None;
        }

        // One level at a time. The graphs that use the pool share it,
        // and the pool hands out one batch: a second thread publishing
        // over a level in flight would reset its claim word and count
        // under it, so nodes would be skipped or the wait would never
        // end. Playback is the only caller today; anything else that
        // gets here at the same time runs its level on its own thread.
        if self.shared.publishing.swap(true, Ordering::Acquire) {
            run_alone(nodes, run);
            return None;
        }
        let shared = &*self.shared;

        // The workers reach the closure through a pointer to this
        // reference. Both stay valid because this call does not return
        // until every claimed node has finished.
        let run_ref: &(dyn Fn(usize) + Sync) = run;
        let epoch = shared.epoch.fetch_add(1, Ordering::Relaxed).wrapping_add(1) & EPOCH_MASK;
        shared
            .nodes
            .store(nodes.as_ptr().cast_mut(), Ordering::Relaxed);
        shared.run.store(
            (&run_ref as *const &(dyn Fn(usize) + Sync))
                .cast::<()>()
                .cast_mut(),
            Ordering::Relaxed,
        );
        shared.busy_ns.store(0, Ordering::Relaxed);
        // The count first, then the claim word: a thread whose claim
        // sees the new number also sees the new count and pointers.
        // SeqCst rather than Release, because a worker going to sleep
        // sets its bit and then reads this word, and this thread writes
        // this word and then reads the bits: one of the two has to see
        // the other (see `worker_loop`).
        shared
            .remaining
            .store(nodes.len() as u64, Ordering::Relaxed);
        shared
            .claim
            .store(pack(epoch, nodes.len()), Ordering::SeqCst);
        // One wake-up at most from here; a worker that joins passes it
        // on to the next while there is work left.
        wake_one(shared);
        shared.stats.shared.fetch_add(1, Ordering::Relaxed);

        // This thread takes work too rather than standing idle.
        let here = take_work(shared, epoch, false);
        wait_for_rest(shared);
        let work = here.busy_ns + shared.busy_ns.load(Ordering::Relaxed);

        let panicked_on_worker = shared.worker_panicked.swap(false, Ordering::Relaxed);
        shared.publishing.store(false, Ordering::Release);
        // A node that panicked goes on up from here, once the level is
        // over and nothing runs on this frame any more, as it would have
        // with no helper threads. Before, a panic here left the workers
        // running on a frame that was being unwound, and a panic on a
        // worker never took its node off the count, so this thread
        // waited for it forever.
        if let Some(payload) = here.panicked {
            std::panic::resume_unwind(payload);
        }
        if panicked_on_worker {
            panic!("a mixer node panicked on an audio worker thread");
        }
        Some(work)
    }
}

fn run_alone(nodes: &[usize], run: &(dyn Fn(usize) + Sync)) {
    for &node in nodes {
        run(node);
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        // A worker checks the flag after saying it is going to sleep, and
        // a wake-up sent before it sleeps is kept for it, so every worker
        // either sees the flag or is woken past its sleep.
        self.shared.shutdown.store(true, Ordering::SeqCst);
        for worker in &self.workers {
            worker.thread().unpark();
        }
        // The workers are let go, not joined. Each holds only its own
        // handle on the shared state and stops at its next wake, and a
        // join would make whichever thread drops the last handle on the
        // pool wait for all of them. That can be the audio thread: with
        // no graph builder thread it frees the graph it replaces itself.
        self.workers.clear();
    }
}

/// Take a node of batch `epoch`, or None once it has none left or is no
/// longer the current batch.
///
/// A worker can be taken off the core at any moment, between seeing the
/// batch and claiming its first node or between finishing one node and
/// claiming the next. By the time it runs again the publisher may have
/// finished the level, started the next one, or returned and freed the
/// frame the batch points into. A plain counter let such a worker take
/// an index of the new level and run a node of the old one with it: the
/// closure's frame was gone, and the new level lost a node and its
/// unfinished count. Here the batch number travels with the count, so
/// the claim fails instead.
fn claim(shared: &Shared, epoch: u64) -> Option<usize> {
    let mut current = shared.claim.load(Ordering::Acquire);
    loop {
        let left = (current & INDEX_MASK) as usize;
        if current >> INDEX_BITS != epoch || left == 0 {
            return None;
        }
        // left > 0, so taking one off cannot reach the batch number.
        match shared.claim.compare_exchange_weak(
            current,
            current - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(left - 1),
            Err(seen) => current = seen,
        }
    }
}

/// Take one node off the unfinished count, and wake the audio thread if
/// it was the last and the audio thread is asleep waiting for it.
fn finish_one(shared: &Shared) {
    if shared.remaining.fetch_sub(1, Ordering::AcqRel) == WAITER | 1 {
        // SAFETY: the key is the address of a field of `Shared`, which
        // this thread holds; parking_lot only compares it.
        unsafe {
            parking_lot_core::unpark_one(remaining_key(shared), |_| {
                parking_lot_core::DEFAULT_UNPARK_TOKEN
            });
        }
    }
}

fn remaining_key(shared: &Shared) -> usize {
    &shared.remaining as *const AtomicU64 as usize
}

/// What a thread did with a batch.
struct Took {
    /// The first panic of a node it ran.
    panicked: Option<Box<dyn std::any::Any + Send>>,
    /// From its first claim to its last node's end.
    busy_ns: u64,
}

/// Take nodes from the batch until there are none left.
///
/// A node that panics still counts as finished, so nobody waits for it
/// forever. The first panic is handed back; on a worker it is also
/// noted for the publisher, which raises it once the level is over.
fn take_work(shared: &Shared, epoch: u64, on_worker: bool) -> Took {
    // Read before the first claim and only followed after one succeeds.
    // A claim on `epoch` that succeeds means the batch still had nodes
    // to take, so its publisher had not moved on to write the next
    // batch's pointers: these are the ones it wrote for `epoch`.
    let nodes = shared.nodes.load(Ordering::Relaxed).cast_const();
    let run = shared
        .run
        .load(Ordering::Relaxed)
        .cast_const()
        .cast::<&(dyn Fn(usize) + Sync)>();
    let mut panicked = None;
    // Set once a node is claimed. The node last run is only counted as
    // finished after the next claim has been tried, so the batch (and
    // the frame its pointers point into) stays alive while this thread
    // is still on it, and the time spent is added before it ends.
    let mut started: Option<Instant> = None;
    loop {
        #[cfg(test)]
        if on_worker {
            if let Some(hook) = &shared.hooks.before_claim {
                hook();
            }
        }
        let Some(index) = claim(shared, epoch) else {
            let busy_ns = started.map_or(0, |s| s.elapsed().as_nanos() as u64);
            if started.is_some() {
                if on_worker {
                    shared.busy_ns.fetch_add(busy_ns, Ordering::Relaxed);
                }
                finish_one(shared);
            }
            return Took { panicked, busy_ns };
        };
        if started.is_some() {
            finish_one(shared);
        } else {
            started = Some(Instant::now());
        }
        // SAFETY: this thread holds an unfinished node of the batch, so
        // `remaining` is above zero and the publisher is still inside
        // `share_level`, keeping the node list and the closure alive.
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            let node = *nodes.add(index);
            (*run)(node);
        }));
        if let Err(payload) = ran {
            // Seen by the publisher through the count.
            if on_worker {
                shared.worker_panicked.store(true, Ordering::Relaxed);
            }
            if panicked.is_none() {
                panicked = Some(payload);
            }
        }
    }
}

/// Wait until every node of the batch has finished: spin a little, then
/// sleep until the worker that finishes the last node wakes this thread.
fn wait_for_rest(shared: &Shared) {
    let started = Instant::now();
    let mut spins = 0u32;
    loop {
        if shared.remaining.load(Ordering::Acquire) == 0 {
            return;
        }
        std::hint::spin_loop();
        spins = spins.wrapping_add(1);
        if spins.is_multiple_of(64) && started.elapsed() >= shared.spin.publisher {
            break;
        }
    }
    // Say this thread is sleeping on the count. A worker taking the last
    // node off after this sees the flag and wakes it; one that took it
    // off before leaves the count at zero, seen here.
    if shared.remaining.fetch_or(WAITER, Ordering::AcqRel) == 0 {
        shared.remaining.store(0, Ordering::Relaxed);
        return;
    }
    shared.stats.waits_slept.fetch_add(1, Ordering::Relaxed);
    let key = remaining_key(shared);
    while shared.remaining.load(Ordering::Acquire) != WAITER {
        // SAFETY: the key is the address of a field of `Shared`; the
        // closures do not park or unpark. The check runs under
        // parking_lot's queue lock, which the waking worker takes too, so
        // its wake-up cannot fall between the check and the sleep.
        unsafe {
            parking_lot_core::park(
                key,
                || shared.remaining.load(Ordering::Acquire) != WAITER,
                || {},
                |_, _| {},
                parking_lot_core::DEFAULT_PARK_TOKEN,
                None,
            );
        }
    }
    shared.remaining.store(0, Ordering::Relaxed);
}

/// Wake one sleeping worker, if there is one.
fn wake_one(shared: &Shared) {
    let mut mask = shared.sleeping.load(Ordering::SeqCst);
    while mask != 0 {
        let index = mask.trailing_zeros() as usize;
        let bit = 1u64 << index;
        match shared.sleeping.compare_exchange_weak(
            mask,
            mask & !bit,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => {
                if let Some(thread) = shared.threads.get().and_then(|t| t.get(index)) {
                    thread.unpark();
                    shared.stats.wakes.fetch_add(1, Ordering::Relaxed);
                }
                return;
            }
            Err(seen) => mask = seen,
        }
    }
}

/// Spin until a new batch than `seen` is out (true) or the worker's
/// spin time is up (false).
fn spin_for_batch(shared: &Shared, seen: u64) -> bool {
    let started = Instant::now();
    let mut spins = 0u32;
    loop {
        if shared.claim.load(Ordering::Relaxed) >> INDEX_BITS != seen
            || shared.shutdown.load(Ordering::Relaxed)
        {
            return true;
        }
        std::hint::spin_loop();
        spins = spins.wrapping_add(1);
        if spins.is_multiple_of(64) && started.elapsed() >= shared.spin.worker {
            return false;
        }
    }
}

/// What the last worker to start got when it asked for real-time
/// priority, for the diagnostics.
pub static WORKER_PRIORITY: hardwave_audio_io::thread_priority::PriorityStatus =
    hardwave_audio_io::thread_priority::PriorityStatus::new();

fn worker_loop(shared: Arc<Shared>, index: usize) {
    // These threads run plug-ins and effects like the audio thread does,
    // and only the audio thread had denormals flushed: a reverb tail on a
    // worker took the slow path and spiked the load. The workers live as
    // long as the pool, so once is enough.
    hardwave_audio_io::denormals::flush_denormals_to_zero();
    // They also have to keep up with the audio thread they work for: at
    // normal priority any busy program on the machine delays a level,
    // and the audio thread waits for it. Same class as the audio thread.
    WORKER_PRIORITY.set(hardwave_audio_io::thread_priority::raise_current_thread());
    let bit = 1u64 << index;
    let mut seen = 0u64;
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            return;
        }
        let current = shared.claim.load(Ordering::Acquire);
        let epoch = current >> INDEX_BITS;
        if epoch != seen {
            seen = epoch;
            // More nodes left than this worker will take: pass the
            // wake-up on, so the audio thread pays for one at most.
            if current & INDEX_MASK > 1 {
                wake_one(&shared);
            }
            // The panic was reported where it happened; the audio thread
            // raises it again.
            drop(take_work(&shared, epoch, true).panicked);
            continue;
        }
        if spin_for_batch(&shared, seen) {
            continue;
        }
        // Say this worker is going to sleep, then look once more. The
        // publisher writes the claim word and then reads the bits, so
        // either it sees this bit and wakes the worker, or this read sees
        // its batch. A wake-up that comes before the sleep is kept for it.
        shared.sleeping.fetch_or(bit, Ordering::SeqCst);
        #[cfg(test)]
        if let Some(hook) = &shared.hooks.before_wait {
            hook();
        }
        if shared.claim.load(Ordering::SeqCst) >> INDEX_BITS != seen
            || shared.shutdown.load(Ordering::SeqCst)
        {
            shared.sleeping.fetch_and(!bit, Ordering::SeqCst);
            continue;
        }
        std::thread::park();
        // The waker took the bit off already; after a spurious wake-up it
        // is taken off here and set again before the next sleep.
        shared.sleeping.fetch_and(!bit, Ordering::SeqCst);
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
    use std::sync::atomic::{AtomicU32, AtomicUsize};
    use std::sync::{Condvar, Mutex};

    /// No spinning at all, so every wait goes to sleep and every level
    /// needs a wake-up: the paths a busy machine takes.
    const NO_SPIN: Spin = Spin::NONE;

    fn sleepy_pool(threads: usize) -> WorkerPool {
        WorkerPool::start_with(threads, NO_SPIN, TestHooks::default())
    }

    fn all_asleep(pool: &WorkerPool) -> bool {
        let all = (1u64 << pool.threads()) - 1;
        pool.shared.sleeping.load(Ordering::SeqCst) == all
    }

    /// Run `f` on a thread of its own and fail if it has not come back
    /// within `secs`: a lost wake-up shows as a hang, not a wrong value.
    fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (done, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(f());
        });
        done_rx
            .recv_timeout(std::time::Duration::from_secs(secs))
            .expect("came back in time (a lost wake-up hangs here)")
    }

    #[test]
    fn a_sleeping_worker_is_woken_for_the_next_level() {
        within(30, || {
            let pool = sleepy_pool(2);
            let nodes: Vec<usize> = (0..4).collect();
            for round in 0..50 {
                assert!(
                    wait_until(|| all_asleep(&pool)),
                    "round {round}: workers asleep"
                );
                let worker_ran = AtomicBool::new(false);
                pool.run_level(&nodes, &|node| {
                    if on_worker() {
                        worker_ran.store(true, Ordering::SeqCst);
                    } else if node == 3 {
                        // The first node the audio thread takes: hold the
                        // level open until a sleeping worker has joined.
                        assert!(wait_until(|| worker_ran.load(Ordering::SeqCst)));
                    }
                });
                assert!(worker_ran.load(Ordering::SeqCst), "round {round}");
            }
            assert!(pool.stats().worker_wakes >= 50);
        });
    }

    #[test]
    fn the_audio_thread_sleeps_and_the_last_worker_wakes_it() {
        within(30, || {
            let pool = sleepy_pool(1);
            let nodes: Vec<usize> = (0..2).collect();
            for _ in 0..20 {
                let worker_in = AtomicBool::new(false);
                pool.run_level(&nodes, &|_| {
                    if on_worker() {
                        worker_in.store(true, Ordering::SeqCst);
                        // Long past the audio thread's spin.
                        std::thread::sleep(Duration::from_millis(5));
                    } else {
                        assert!(wait_until(|| worker_in.load(Ordering::SeqCst)));
                    }
                });
            }
            assert!(pool.stats().waits_slept > 0, "the audio thread slept");
        });
    }

    #[test]
    fn wake_ups_are_never_lost_at_random_timings() {
        // Node times around the spin and wake-up times, so the sleeps and
        // wake-ups of both sides cross each other every way they can.
        within(120, || {
            let pool = sleepy_pool(3);
            let seed = AtomicU64::new(0x2545_f491_4f6c_dd1d);
            let random = || {
                let mut x = seed.load(Ordering::Relaxed);
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                seed.store(x, Ordering::Relaxed);
                x
            };
            let runs: Vec<AtomicU32> = (0..8).map(|_| AtomicU32::new(0)).collect();
            let all: Vec<usize> = (0..8).collect();
            for level in 0..5_000usize {
                let nodes = &all[..2 + level % 7];
                pool.run_level(nodes, &|node| {
                    runs[node].fetch_add(1, Ordering::SeqCst);
                    match random() % 8 {
                        0 => std::thread::sleep(Duration::from_micros(random() % 200)),
                        1 => std::thread::yield_now(),
                        _ => {}
                    }
                });
                for (node, count) in runs.iter().enumerate() {
                    let want = u32::from(node < nodes.len());
                    assert_eq!(count.swap(0, Ordering::SeqCst), want, "level {level}");
                }
                // Now and then let the workers fall asleep between levels.
                if random() % 16 == 0 {
                    std::thread::sleep(Duration::from_micros(random() % 300));
                }
            }
        });
    }

    #[test]
    fn levels_with_no_workers_run_in_order_and_never_wait() {
        within(10, || {
            let pool = WorkerPool::new(0);
            let order = vec![0, 1, 2, 3];
            let mut levels =
                Levels::build(&order, 4, |n| if n == 3 { vec![0, 1, 2] } else { vec![] });
            let seen: Mutex<Vec<usize>> = Mutex::new(Vec::new());
            for _ in 0..3 {
                pool.run_levels(&mut levels, &|node| seen.lock().unwrap().push(node));
            }
            assert_eq!(*seen.lock().unwrap(), [0, 1, 2, 3].repeat(3));
            assert_eq!(pool.stats(), PoolStats::default(), "nothing was handed out");
        });
    }

    #[test]
    fn a_level_is_shared_until_it_is_known_to_be_light() {
        // Decided from what the level cost before, not timed in the test
        // itself: a busy test machine stretches any timing.
        let mut cost = LevelCost::default();
        assert!(cost.worth_sharing(4), "not measured yet: shared");
        assert!(!cost.worth_sharing(1), "one node is never shared");
        cost.note(5_000, true);
        assert!(!cost.worth_sharing(4), "light: kept on the audio thread");
        cost.note(SHARE_ABOVE_NS - 1, false);
        assert!(!cost.worth_sharing(4), "not yet worth a wake-up");
        cost.note(SHARE_ABOVE_NS, false);
        assert!(cost.worth_sharing(4), "heavy from the next block on");
        // Shared, it stays shared down to the lower line, so a level near
        // the line does not switch every block.
        let mut shared = LevelCost::default();
        shared.note(SHARE_BELOW_NS, true);
        assert!(shared.worth_sharing(4));
        // Light blocks bring the estimate down a quarter of the way at a
        // time, so one quiet block does not send it back.
        cost.note(0, true);
        assert_eq!(cost.work_ns, SHARE_ABOVE_NS - SHARE_ABOVE_NS / 4);
        assert!(cost.worth_sharing(4));
        let mut blocks = 1;
        while cost.worth_sharing(4) {
            cost.note(0, true);
            blocks += 1;
        }
        assert_eq!(
            blocks, 3,
            "back on the audio thread after a few light blocks"
        );
        // ...and a heavy one up at once.
        cost.note(1_000_000, false);
        assert_eq!(cost.work_ns, 1_000_000);
        assert!(cost.worth_sharing(4));
    }

    #[test]
    fn run_levels_keeps_a_light_level_on_the_audio_thread_and_shares_a_heavy_one() {
        let pool = sleepy_pool(2);
        let order: Vec<usize> = (0..4).collect();
        let mut levels = Levels::build(&order, 4, |_| vec![]);
        let on_workers = AtomicUsize::new(0);
        let run = |_node: usize| {
            if on_worker() {
                on_workers.fetch_add(1, Ordering::SeqCst);
            }
        };
        levels.cost[0].note(1_000, true);
        pool.run_levels(&mut levels, &run);
        assert_eq!(
            on_workers.load(Ordering::SeqCst),
            0,
            "no node went to a worker"
        );
        assert_eq!(pool.stats().levels_shared, 0);
        assert_eq!(pool.stats().levels_alone, 1);
        assert!(levels.cost[0].measured && !levels.cost[0].shared);

        levels.cost[0].note(10_000_000, false);
        pool.run_levels(&mut levels, &run);
        assert_eq!(pool.stats().levels_shared, 1);
        assert_eq!(pool.stats().levels_alone, 1);
        assert!(levels.cost[0].shared, "noted as shared");
    }

    #[test]
    fn dropping_the_pool_while_every_worker_sleeps_lets_them_go() {
        let pool = sleepy_pool(3);
        let shared = Arc::downgrade(&pool.shared);
        assert!(wait_until(|| all_asleep(&pool)), "workers asleep");
        within(5, move || drop(pool));
        assert!(
            wait_until(|| shared.upgrade().is_none()),
            "every worker stopped"
        );
    }

    #[test]
    fn a_worker_that_panics_while_the_audio_thread_sleeps_still_wakes_it() {
        within(30, || {
            let pool = sleepy_pool(1);
            let nodes: Vec<usize> = (0..2).collect();
            let worker_in = AtomicBool::new(false);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.run_level(&nodes, &|_| {
                    if on_worker() {
                        worker_in.store(true, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        panic!("a node failed on a worker");
                    }
                    assert!(wait_until(|| worker_in.load(Ordering::SeqCst)));
                })
            }));
            assert!(result.is_err(), "the panic reached the caller");
            assert!(pool.stats().waits_slept > 0, "the audio thread slept");
            // And the pool still works.
            let counts: Vec<AtomicU32> = (0..2).map(|_| AtomicU32::new(0)).collect();
            pool.run_level(&nodes, &|node| {
                counts[node].fetch_add(1, Ordering::SeqCst);
            });
            assert!(counts.iter().all(|c| c.load(Ordering::SeqCst) == 1));
        });
    }

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

    fn hooks_before_claim(hook: impl Fn() + Send + Sync + 'static) -> TestHooks {
        TestHooks {
            before_claim: Some(Box::new(hook)),
            ..TestHooks::default()
        }
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
            hooks_before_claim(move || {
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
            }),
        );
        let gate_state = |f: &dyn Fn(&Gate) -> bool| f(&gate.0.lock().unwrap());

        let nodes: Vec<usize> = (0..8).collect();
        // Nodes are handed out from the end of the level: this is the
        // first one the audio thread takes.
        const FIRST: usize = 7;
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
            if node == FIRST && !on_worker() {
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
            if node == FIRST && !on_worker() {
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
            hooks_before_claim(move || {
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
            }),
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
        pool.shared.claim.store(pack(now, 2), Ordering::SeqCst);
        assert_eq!(claim(&pool.shared, 5), None);
        assert_eq!(claim(&pool.shared, now), Some(1));
        assert_eq!(claim(&pool.shared, now), Some(0));
        assert_eq!(claim(&pool.shared, now), None, "the batch is all taken");
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

    #[test]
    fn dropping_the_pool_lets_every_worker_go() {
        // A worker that has checked for shutdown and not yet started to
        // wait must still hear about it, and dropping the pool must not
        // wait for the workers to stop.
        let at_wait = Arc::new(AtomicUsize::new(0));
        let hook_at_wait = Arc::clone(&at_wait);
        let pool = WorkerPool::start(
            1,
            TestHooks {
                before_wait: Some(Box::new(move || {
                    hook_at_wait.fetch_add(1, Ordering::SeqCst);
                    // Stay between the check and the wait for a while.
                    std::thread::sleep(std::time::Duration::from_millis(100));
                })),
                ..TestHooks::default()
            },
        );
        let shared = Arc::downgrade(&pool.shared);
        assert!(
            wait_until(|| at_wait.load(Ordering::SeqCst) > 0),
            "worker idle"
        );
        let (dropped, dropped_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(pool);
            let _ = dropped.send(());
        });
        assert!(
            dropped_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_ok(),
            "dropping the pool returned"
        );
        assert!(
            wait_until(|| shared.upgrade().is_none()),
            "every worker stopped"
        );
    }

    #[test]
    fn two_threads_sharing_a_pool_each_run_every_node_once() {
        let pool = Arc::new(WorkerPool::new(2));
        let (done, done_rx) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let pool = Arc::clone(&pool);
            let done = done.clone();
            std::thread::spawn(move || {
                let counts: Vec<AtomicU32> = (0..16).map(|_| AtomicU32::new(0)).collect();
                let nodes: Vec<usize> = (0..16).collect();
                let run = |node: usize| {
                    counts[node].fetch_add(1, Ordering::SeqCst);
                };
                let mut wrong = 0;
                for _ in 0..2_000 {
                    pool.run_level(&nodes, &run);
                    wrong += counts
                        .iter()
                        .filter(|count| count.swap(0, Ordering::SeqCst) != 1)
                        .count();
                }
                let _ = done.send(wrong);
            });
        }
        for _ in 0..2 {
            let wrong = done_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("both threads got through their levels");
            assert_eq!(wrong, 0, "a node ran other than once");
        }
    }

    #[test]
    fn a_node_that_panics_on_a_worker_does_not_hang_the_audio_thread() {
        let pool = WorkerPool::new(1);
        let (done, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let nodes: Vec<usize> = (0..8).collect();
            let worker_ran = AtomicBool::new(false);
            let failing = |node: usize| {
                if on_worker() {
                    worker_ran.store(true, Ordering::SeqCst);
                    panic!("node {node} failed on a worker");
                }
                // Keep the level open until a worker has taken a node.
                if node == 0 {
                    assert!(wait_until(|| worker_ran.load(Ordering::SeqCst)));
                }
            };
            let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pool.run_level(&nodes, &failing)
            }));
            // The pool still works afterwards.
            let counts: Vec<AtomicU32> = (0..8).map(|_| AtomicU32::new(0)).collect();
            pool.run_level(&nodes, &|node| {
                counts[node].fetch_add(1, Ordering::SeqCst);
            });
            let counts: Vec<u32> = counts.iter().map(|c| c.load(Ordering::SeqCst)).collect();
            let _ = done.send((first.is_err(), counts));
        });
        let (panicked, counts) = done_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("run_level came back");
        assert!(panicked, "the worker's panic reached the caller");
        assert_eq!(counts, vec![1; 8]);
    }

    #[test]
    fn a_node_that_panics_on_the_audio_thread_waits_for_the_workers() {
        let pool = WorkerPool::new(1);
        let worker_in = AtomicBool::new(false);
        let worker_done = AtomicBool::new(false);
        let nodes: Vec<usize> = (0..2).collect();
        let run = |_node: usize| {
            if on_worker() {
                worker_in.store(true, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(100));
                worker_done.store(true, Ordering::SeqCst);
            } else {
                assert!(wait_until(|| worker_in.load(Ordering::SeqCst)));
                panic!("a node failed on the audio thread");
            }
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pool.run_level(&nodes, &run)
        }));
        assert!(result.is_err(), "the panic reached the caller");
        assert!(
            worker_done.load(Ordering::SeqCst),
            "run_level unwound while a worker was still in a node"
        );
    }

    /// What handing a level out costs, against the same work on one
    /// thread: levels of a few nodes with a known amount of work each,
    /// back to back, and paced with a gap between blocks of four levels
    /// (as between sound card blocks, where the workers go idle). A
    /// measurement, not a check; use a release build:
    ///   hw-daw-cargo test --release -j 1 -p hardwave-engine --lib \
    ///     pool_cost_per_level -- --ignored --nocapture
    #[test]
    #[ignore = "a measurement"]
    fn pool_cost_per_level() {
        fn work(us: u64) {
            let started = std::time::Instant::now();
            while started.elapsed() < std::time::Duration::from_micros(us) {
                std::hint::spin_loop();
            }
        }
        let max_workers: usize = std::env::var("HW_POOL_MAX_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        for workers in 0..=max_workers {
            let pool = WorkerPool::new(workers);
            for (count, us) in [(4usize, 5u64), (8, 25), (16, 50)] {
                let nodes: Vec<usize> = (0..count).collect();
                let run = |_node: usize| work(us);
                for paced in [false, true] {
                    let mut times = Vec::with_capacity(2000);
                    for level in 0..2000 {
                        if paced && level % 4 == 0 {
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        let started = std::time::Instant::now();
                        pool.run_level(&nodes, &run);
                        times.push(started.elapsed().as_nanos() as u64);
                    }
                    times.sort_unstable();
                    let mean = times.iter().sum::<u64>() as f64 / times.len() as f64 / 1e3;
                    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize] as f64 / 1e3;
                    eprintln!(
                        "workers {workers}, {count} nodes x {us} us ({} us of work), {}: mean {mean:.1} us, median {:.1}, p99 {:.1}, slowest {:.1}",
                        count as u64 * us,
                        if paced { "paced" } else { "back to back" },
                        at(0.5),
                        at(0.99),
                        at(1.0),
                    );
                }
            }
        }
    }
}
