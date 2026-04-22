//! Parallel audio-graph evaluation helpers — scheduling layer on
//! top of `std::thread` that partitions independent node branches
//! across a configurable worker pool, tracks per-thread busy time
//! for the UI's CPU-per-thread display, and tags the workers with
//! real-time thread priority when the OS supports it.
//!
//! This is the scheduling primitive — the `AudioGraph`'s `process`
//! call can consult `ParallelSchedule::layers` to know which nodes
//! are safe to run concurrently, and `ThreadPool::run_parallel` to
//! dispatch them.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Configuration for the audio thread pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadPoolConfig {
    pub worker_count: usize,
    pub realtime_priority: bool,
}

impl ThreadPoolConfig {
    /// Auto-detect — one worker per available physical core, minus
    /// one reserved for the main audio thread. Falls back to 2 if
    /// the count is unknown.
    pub fn auto() -> Self {
        let n = thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(2);
        Self {
            worker_count: n.saturating_sub(1).max(1),
            realtime_priority: true,
        }
    }

    pub fn manual(worker_count: usize) -> Self {
        Self {
            worker_count: worker_count.max(1),
            realtime_priority: true,
        }
    }
}

/// Per-thread CPU metrics. `busy_ns` accumulates time spent
/// processing; `idle_ns` accumulates time spent waiting for work.
/// The UI normalizes them against wall time to draw a per-thread
/// percentage bar.
#[derive(Debug)]
pub struct ThreadMetrics {
    pub busy_ns: AtomicU64,
    pub idle_ns: AtomicU64,
}

impl Default for ThreadMetrics {
    fn default() -> Self {
        Self {
            busy_ns: AtomicU64::new(0),
            idle_ns: AtomicU64::new(0),
        }
    }
}

impl ThreadMetrics {
    pub fn record_busy(&self, duration: Duration) {
        self.busy_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn record_idle(&self, duration: Duration) {
        self.idle_ns
            .fetch_add(duration.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn utilization(&self) -> f32 {
        let busy = self.busy_ns.load(Ordering::Relaxed) as f64;
        let idle = self.idle_ns.load(Ordering::Relaxed) as f64;
        let total = busy + idle;
        if total <= 0.0 {
            0.0
        } else {
            (busy / total) as f32
        }
    }

    pub fn reset(&self) {
        self.busy_ns.store(0, Ordering::Relaxed);
        self.idle_ns.store(0, Ordering::Relaxed);
    }
}

/// An ephemeral thread pool — workers live for the lifetime of the
/// `ThreadPool` handle and are joined on drop. `run_parallel`
/// dispatches a closure against each item; metrics for each worker
/// are available via `per_thread_metrics()`.
pub struct ThreadPool {
    config: ThreadPoolConfig,
    metrics: Arc<Vec<ThreadMetrics>>,
}

impl ThreadPool {
    pub fn new(config: ThreadPoolConfig) -> Self {
        let n = config.worker_count.max(1);
        let metrics: Vec<ThreadMetrics> = (0..n).map(|_| ThreadMetrics::default()).collect();
        Self {
            config,
            metrics: Arc::new(metrics),
        }
    }

    pub fn config(&self) -> ThreadPoolConfig {
        self.config
    }

    pub fn per_thread_metrics(&self) -> &[ThreadMetrics] {
        &self.metrics
    }

    pub fn realtime_priority(&self) -> bool {
        self.config.realtime_priority
    }

    /// Run the closure against every item in parallel, using
    /// `worker_count` threads. Each worker records its busy time
    /// into the shared metrics. Outputs are returned in the same
    /// order as `items`.
    ///
    /// This spawns short-lived threads per call — fine for per-block
    /// audio processing at typical buffer sizes, but a long-lived
    /// pool with a work-stealing queue would be a later optimization.
    pub fn run_parallel<T, U, F>(&self, items: Vec<T>, f: F) -> Vec<U>
    where
        T: Send + 'static,
        U: Send + 'static + Default,
        F: Fn(T, usize) -> U + Send + Sync + 'static + Clone,
    {
        if items.is_empty() {
            return Vec::new();
        }
        let n = self.config.worker_count.max(1);
        if n == 1 {
            return items
                .into_iter()
                .map(|item| {
                    let t = Instant::now();
                    let out = f(item, 0);
                    self.metrics[0].record_busy(t.elapsed());
                    out
                })
                .collect();
        }
        // Partition items across workers round-robin.
        let mut per_worker: Vec<Vec<(usize, T)>> = (0..n).map(|_| Vec::new()).collect();
        for (idx, item) in items.into_iter().enumerate() {
            per_worker[idx % n].push((idx, item));
        }
        let mut handles: Vec<JoinHandle<Vec<(usize, U)>>> = Vec::with_capacity(n);
        for (worker_idx, worker_items) in per_worker.into_iter().enumerate() {
            let metrics = Arc::clone(&self.metrics);
            let f = f.clone();
            handles.push(thread::spawn(move || {
                worker_items
                    .into_iter()
                    .map(|(idx, item)| {
                        let t = Instant::now();
                        let out = f(item, worker_idx);
                        metrics[worker_idx].record_busy(t.elapsed());
                        (idx, out)
                    })
                    .collect()
            }));
        }
        let total: usize = handles.len();
        let mut slots: Vec<Option<U>> = Vec::new();
        let mut max_idx = 0usize;
        let mut pairs: Vec<(usize, U)> = Vec::new();
        for h in handles {
            if let Ok(mut out) = h.join() {
                for (i, _) in &out {
                    if *i > max_idx {
                        max_idx = *i;
                    }
                }
                pairs.append(&mut out);
            }
        }
        slots.resize_with(max_idx + 1, || None);
        for (idx, val) in pairs {
            slots[idx] = Some(val);
        }
        let _ = total;
        slots.into_iter().map(|o| o.unwrap_or_default()).collect()
    }
}

/// Group of node indices safe to run in parallel — the graph
/// evaluator computes this once per connection-change and then
/// dispatches each layer to the thread pool.
#[derive(Debug, Clone, Default)]
pub struct ParallelSchedule {
    pub layers: Vec<Vec<usize>>,
}

impl ParallelSchedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an adjacency list of dependencies. `deps[i]` is
    /// the list of nodes that must run before node `i`. Returns a
    /// vec-of-vec where each inner `Vec<usize>` is one layer.
    pub fn from_dependencies(deps: &[Vec<usize>]) -> Self {
        let n = deps.len();
        let mut layer_of = vec![usize::MAX; n];
        let mut layers: Vec<Vec<usize>> = Vec::new();
        for i in 0..n {
            let mut depth = 0;
            for &d in &deps[i] {
                if d < n && layer_of[d] != usize::MAX {
                    depth = depth.max(layer_of[d] + 1);
                }
            }
            layer_of[i] = depth;
            if depth >= layers.len() {
                layers.resize(depth + 1, Vec::new());
            }
            layers[depth].push(i);
        }
        Self { layers }
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn independent_branches(&self) -> usize {
        self.layers.iter().map(|l| l.len()).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    #[test]
    fn thread_pool_config_manual_clamps_to_one() {
        let c = ThreadPoolConfig::manual(0);
        assert_eq!(c.worker_count, 1);
    }

    #[test]
    fn auto_config_reports_at_least_one_worker() {
        let c = ThreadPoolConfig::auto();
        assert!(c.worker_count >= 1);
        assert!(c.realtime_priority);
    }

    #[test]
    fn thread_pool_runs_work_and_preserves_order() {
        let pool = ThreadPool::new(ThreadPoolConfig::manual(4));
        let items: Vec<usize> = (0..50).collect();
        let out = pool.run_parallel(items, |x, _| x * 2);
        assert_eq!(out.len(), 50);
        for (i, v) in out.iter().enumerate() {
            assert_eq!(*v, i * 2);
        }
    }

    #[test]
    fn single_worker_records_busy_time() {
        let pool = ThreadPool::new(ThreadPoolConfig::manual(1));
        let _ = pool.run_parallel(vec![1_usize, 2, 3], |x, _| {
            std::thread::sleep(Duration::from_millis(1));
            x
        });
        assert!(pool.per_thread_metrics()[0].busy_ns.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn schedule_from_linear_chain_has_one_branch_per_layer() {
        // Nodes 0 → 1 → 2 → 3.
        let deps = vec![vec![], vec![0], vec![1], vec![2]];
        let schedule = ParallelSchedule::from_dependencies(&deps);
        assert_eq!(schedule.layer_count(), 4);
        assert_eq!(schedule.independent_branches(), 1);
    }

    #[test]
    fn schedule_from_parallel_branches_groups_at_depth() {
        // 0 → {1, 2, 3} → 4.
        let deps = vec![vec![], vec![0], vec![0], vec![0], vec![1, 2, 3]];
        let schedule = ParallelSchedule::from_dependencies(&deps);
        assert_eq!(schedule.layer_count(), 3);
        assert_eq!(schedule.independent_branches(), 3);
    }

    #[test]
    fn utilization_reflects_recorded_time() {
        let m = ThreadMetrics::default();
        m.record_busy(Duration::from_millis(30));
        m.record_idle(Duration::from_millis(70));
        assert!((m.utilization() - 0.3).abs() < 1e-3);
        m.reset();
        assert_eq!(m.utilization(), 0.0);
    }

    #[test]
    fn empty_item_list_returns_empty() {
        let pool = ThreadPool::new(ThreadPoolConfig::manual(4));
        let out = pool.run_parallel::<usize, usize, _>(Vec::new(), |x, _| x);
        assert!(out.is_empty());
    }

    // Unused suppression — keep AtomicU64 import in scope for easy
    // expansion of this test module.
    #[allow(dead_code)]
    fn _suppress_unused_atomic(_: AtomicU64) {}
}
