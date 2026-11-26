//! Sharing a level out allocates nothing, on the audio thread or on the
//! helpers, from the very first level a thread hands out: the wake-up of a
//! sleeping helper, the audio thread's own sleep while the last node runs,
//! and a wait that runs past the block's end.
//!
//! An allocation there can wait on the system allocator's lock, which is
//! a click. The thread that plays is new to the pool here, as a sound
//! card's thread is (an ASIO driver makes its own): nothing on it was
//! prepared beforehand. Every thread's allocations are counted while the
//! levels run, so this file holds one test: another test running beside
//! it would be counted too.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use hardwave_engine::parallel::{Levels, WorkerPool};

struct Counting;

static WATCHING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

fn note() {
    if WATCHING.load(Ordering::Relaxed) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn on_worker() -> bool {
    std::thread::current()
        .name()
        .is_some_and(|n| n.starts_with("hardwave-audio-worker"))
}

#[test]
fn the_first_shared_levels_allocate_nothing_on_any_thread() {
    let pool = WorkerPool::new(2);
    // One level of four nodes, heavy enough to be shared every block.
    let order: Vec<usize> = (0..4).collect();
    let mut levels = Levels::build(&order, 4, |_| vec![]);
    let worker_in = AtomicBool::new(false);
    let slow_worker = AtomicBool::new(false);
    let run = |node: usize| {
        if on_worker() {
            worker_in.store(true, Ordering::SeqCst);
            if slow_worker.load(Ordering::SeqCst) {
                // Long past the audio thread's spin and the block's end.
                std::thread::sleep(Duration::from_millis(20));
            }
        } else if node == 3 {
            // The audio thread's first node: hold the level open until a
            // sleeping helper has been woken and joined.
            let started = Instant::now();
            while !worker_in.load(Ordering::SeqCst) && started.elapsed() < Duration::from_secs(5) {
                std::hint::spin_loop();
            }
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_micros(100) {
            std::hint::spin_loop();
        }
    };
    // The helpers fall asleep.
    std::thread::sleep(Duration::from_millis(50));

    let (done, done_rx) = std::sync::mpsc::channel();
    let before = pool.stats();
    std::thread::scope(|scope| {
        let pool = &pool;
        let levels = &mut levels;
        let run = &run;
        let worker_in = &worker_in;
        let slow_worker = &slow_worker;
        let done = done.clone();
        // A thread that has never handed a level out, as a new stream's.
        scope.spawn(move || {
            WATCHING.store(true, Ordering::SeqCst);
            // First level: wakes a sleeping helper.
            pool.run_levels(levels, run, None);
            // Then the audio thread has to sleep for the last node, and
            // the wait runs past the block's end.
            worker_in.store(false, Ordering::SeqCst);
            slow_worker.store(true, Ordering::SeqCst);
            pool.run_levels(levels, run, Some(Instant::now() + Duration::from_millis(2)));
            slow_worker.store(false, Ordering::SeqCst);
            for _ in 0..50 {
                worker_in.store(false, Ordering::SeqCst);
                pool.run_levels(
                    levels,
                    run,
                    Some(Instant::now() + Duration::from_millis(10)),
                );
            }
            WATCHING.store(false, Ordering::SeqCst);
            let _ = done.send(());
        });
    });
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the levels ran");
    let after = pool.stats();
    assert!(
        after.worker_wakes > before.worker_wakes,
        "a helper was woken"
    );
    assert!(
        after.waits_slept > before.waits_slept,
        "the audio thread slept"
    );
    assert!(
        after.stalls > before.stalls,
        "a wait past the block's end was counted"
    );
    assert_eq!(
        ALLOCATIONS.load(Ordering::SeqCst),
        0,
        "allocations or frees while the levels ran"
    );
}
