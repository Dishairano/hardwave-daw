//! Parameter changes on their way to the thread that plays a plug-in.
//!
//! A knob moved in a plug-in's own window, a preset picked from the host
//! and an automation value all reach the plug-in this way: whoever makes
//! the change pushes it here, and the instance takes everything waiting at
//! the start of its next block.
//!
//! This used to be a vector behind a mutex that the audio thread locked
//! every block. Since the audio thread runs at real-time priority, a window
//! thread holding that lock at the wrong moment (or being preempted while
//! it held it) made the audio thread wait on a normal-priority thread: the
//! classic priority inversion, heard as a dropout. The window side also
//! used `try_lock` and dropped the move when the audio thread was busy
//! draining, so the last move of a knob gesture could be lost.
//!
//! Now nothing on the playing side ever waits. Each parameter has a slot
//! holding its latest value, claimed the first time the parameter changes
//! and kept for the life of the plug-in, and a bit that says "changed since
//! last taken". A push stores the value and then sets the bit; the audio
//! thread clears the bit and then reads the value. A change can only be
//! folded into a newer value of the same parameter, never lost: the value
//! a plug-in ends up with is always the last one pushed. Changes to
//! different parameters taken in the same block arrive in slot order, not
//! push order.
//!
//! The table has a fixed number of slots, and a push looks through all of
//! them before it gives up. A plug-in with more distinct parameters changed
//! than that (rare: it counts parameters actually touched, not parameters
//! the plug-in has) sends the rest through a small overflow list, which the
//! audio thread only ever `try_lock`s and takes a bounded number from per
//! block: when a window thread holds it, those changes wait one block.
//!
//! A take that is cut short (a block's list is full) carries on next time
//! from the first change it left, so no parameter waits behind others that
//! keep changing.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Slots in the table. A power of two (the hash masks with it) and a
/// multiple of 64 (one dirty bit per slot, 64 to a word).
const SLOTS: usize = 1024;
const WORDS: usize = SLOTS / 64;

/// Overflow changes taken per take at most, so a busy overflow list never
/// makes one block long.
const MAX_OVERFLOW_PER_TAKE: usize = 64;

/// The id no parameter has: VST3's `kNoParamId` and CLAP's
/// `CLAP_INVALID_ID` are both all ones. Marks a free slot.
const EMPTY: u32 = u32::MAX;

struct Slot {
    id: AtomicU32,
    value: AtomicU64,
}

/// See the module documentation. Any number of threads may push; the
/// instance that plays the plug-in takes.
pub struct ParamChangeQueue {
    slots: Box<[Slot]>,
    /// One bit per slot: changed since it was last taken.
    dirty: [AtomicU64; WORDS],
    /// Some bit was set since the last take, so a take with nothing
    /// waiting reads one flag instead of every word.
    any: AtomicBool,
    /// The slot the next take starts at. Only the taking side moves it.
    next_slot: AtomicUsize,
    /// Changes that found no slot, newest value per parameter.
    overflow: Mutex<Vec<(u32, f64)>>,
    overflow_waiting: AtomicBool,
}

impl Default for ParamChangeQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ParamChangeQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParamChangeQueue")
            .field("pending", &self.has_pending())
            .finish()
    }
}

impl ParamChangeQueue {
    pub fn new() -> Self {
        Self {
            slots: (0..SLOTS)
                .map(|_| Slot {
                    id: AtomicU32::new(EMPTY),
                    value: AtomicU64::new(0),
                })
                .collect(),
            dirty: std::array::from_fn(|_| AtomicU64::new(0)),
            any: AtomicBool::new(false),
            next_slot: AtomicUsize::new(0),
            overflow: Mutex::new(Vec::new()),
            overflow_waiting: AtomicBool::new(false),
        }
    }

    /// Hand a parameter its new value. From any thread; a later push of the
    /// same parameter before the next take replaces this one.
    pub fn push(&self, id: u32, value: f64) {
        if id != EMPTY {
            if let Some(index) = self.slot_for(id) {
                self.slots[index]
                    .value
                    .store(value.to_bits(), Ordering::Release);
                // After the value: whoever sees the bit sees the value.
                self.dirty[index / 64].fetch_or(1 << (index % 64), Ordering::Release);
                self.any.store(true, Ordering::Release);
                return;
            }
        }
        let mut overflow = self.overflow.lock();
        match overflow.iter_mut().find(|(p, _)| *p == id) {
            Some(entry) => entry.1 = value,
            None => overflow.push((id, value)),
        }
        self.overflow_waiting.store(true, Ordering::Release);
    }

    /// The slot that holds `id`, claiming a free one the first time.
    fn slot_for(&self, id: u32) -> Option<usize> {
        let start = (id.wrapping_mul(0x9E37_79B1) >> (32 - SLOTS.trailing_zeros())) as usize;
        for step in 0..SLOTS {
            let index = (start + step) & (SLOTS - 1);
            let slot = &self.slots[index].id;
            let held = slot.load(Ordering::Acquire);
            if held == id {
                return Some(index);
            }
            if held == EMPTY {
                match slot.compare_exchange(EMPTY, id, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => return Some(index),
                    // Another push claimed it for the same parameter.
                    Err(now) if now == id => return Some(index),
                    Err(_) => {}
                }
            }
        }
        None
    }

    /// Whether anything is waiting. Never blocks.
    pub fn has_pending(&self) -> bool {
        self.any.load(Ordering::Acquire) || self.overflow_waiting.load(Ordering::Acquire)
    }

    /// Take up to `max` waiting changes, calling `apply(id, value)` for each.
    /// Never blocks and never allocates: safe on the audio thread. What does
    /// not fit stays waiting for the next take, which starts where this one
    /// stopped; overflow changes wait too when a pushing thread holds their
    /// list at this moment. Returns how many were taken.
    pub fn drain(&self, max: usize, mut apply: impl FnMut(u32, f64)) -> usize {
        if max == 0 {
            return 0;
        }
        let mut taken = 0;
        if self.any.swap(false, Ordering::Acquire) {
            let start = self.next_slot.load(Ordering::Relaxed) % SLOTS;
            let (first_word, first_bit) = (start / 64, start % 64);
            // Every word once from the start slot on, and last the low bits
            // of the first word, below the start slot.
            for k in 0..=WORDS {
                let w = (first_word + k) % WORDS;
                let mask = match k {
                    0 => u64::MAX << first_bit,
                    _ if k == WORDS && first_bit == 0 => break,
                    _ if k == WORDS => (1u64 << first_bit) - 1,
                    _ => u64::MAX,
                };
                let word = &self.dirty[w];
                if word.load(Ordering::Relaxed) & mask == 0 {
                    continue;
                }
                let mut bits = word.fetch_and(!mask, Ordering::Acquire) & mask;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    if taken == max {
                        // Put back what was not taken, and start there next
                        // time.
                        word.fetch_or(bits, Ordering::Release);
                        self.any.store(true, Ordering::Release);
                        self.next_slot.store(w * 64 + bit, Ordering::Relaxed);
                        return taken;
                    }
                    bits &= bits - 1;
                    let slot = &self.slots[w * 64 + bit];
                    // The bit is clear before the value is read, so a push
                    // landing now sets it again and is taken next time.
                    apply(
                        slot.id.load(Ordering::Acquire),
                        f64::from_bits(slot.value.load(Ordering::Acquire)),
                    );
                    taken += 1;
                }
            }
        }
        if self.overflow_waiting.load(Ordering::Acquire) {
            if let Some(mut overflow) = self.overflow.try_lock() {
                let n = overflow.len().min(max - taken).min(MAX_OVERFLOW_PER_TAKE);
                for (id, value) in overflow.drain(..n) {
                    apply(id, value);
                }
                taken += n;
                self.overflow_waiting
                    .store(!overflow.is_empty(), Ordering::Release);
            }
        }
        taken
    }

    /// Drop whatever is waiting, for a state load: changes made before it
    /// are older than the loaded state and would put old values back over
    /// it. Never blocks; overflow changes a pushing thread holds at this
    /// moment stay.
    pub fn discard(&self) {
        while self.drain(SLOTS, |_, _| {}) > 0 {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Take until nothing is left, the way blocks would one after another.
    fn take_all(q: &ParamChangeQueue) -> Vec<(u32, f64)> {
        let mut out = Vec::new();
        while q.drain(usize::MAX, |id, v| out.push((id, v))) > 0 {}
        out
    }

    #[test]
    fn a_push_is_taken_once() {
        let q = ParamChangeQueue::new();
        assert!(!q.has_pending());
        q.push(7, 0.25);
        assert!(q.has_pending());
        assert_eq!(take_all(&q), vec![(7, 0.25)]);
        assert!(!q.has_pending());
        assert!(take_all(&q).is_empty());
    }

    #[test]
    fn a_parameter_moved_twice_arrives_with_its_last_value() {
        let q = ParamChangeQueue::new();
        q.push(3, 0.1);
        q.push(9, 0.5);
        q.push(3, 0.9);
        let mut got = take_all(&q);
        got.sort_by_key(|e| e.0);
        assert_eq!(got, vec![(3, 0.9), (9, 0.5)]);
    }

    #[test]
    fn what_does_not_fit_in_a_block_waits_for_the_next() {
        let q = ParamChangeQueue::new();
        for id in 0..10 {
            q.push(id, id as f64 / 10.0);
        }
        let mut first = Vec::new();
        assert_eq!(q.drain(4, |id, v| first.push((id, v))), 4);
        assert!(q.has_pending());
        let rest = take_all(&q);
        assert_eq!(rest.len(), 6);
        let mut all: Vec<u32> = first.iter().chain(&rest).map(|e| e.0).collect();
        all.sort();
        assert_eq!(all, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn more_parameters_than_slots_go_through_the_overflow_and_still_arrive() {
        let q = ParamChangeQueue::new();
        let n = SLOTS as u32 * 2;
        for id in 0..n {
            q.push(id, 0.5);
        }
        // The id no parameter has still goes through, as it always did.
        q.push(EMPTY, 0.25);
        let mut got: Vec<u32> = take_all(&q).into_iter().map(|e| e.0).collect();
        got.sort();
        let mut want: Vec<u32> = (0..n).collect();
        want.push(EMPTY);
        assert_eq!(got, want);
        assert!(!q.has_pending());
    }

    #[test]
    fn a_take_while_a_window_holds_the_overflow_does_not_wait_or_lose_it() {
        let q = ParamChangeQueue::new();
        for id in 0..(SLOTS as u32 * 2) {
            q.push(id, 1.0);
        }
        let held = q.overflow.lock();
        // The table part is taken; the overflow part waits.
        let first = {
            let mut n = 0;
            q.drain(usize::MAX, |_, _| n += 1);
            n
        };
        assert!(first > 0 && first < SLOTS * 2);
        assert!(q.has_pending());
        drop(held);
        assert_eq!(first + take_all(&q).len(), SLOTS * 2);
    }

    #[test]
    fn the_whole_table_is_used_before_anything_overflows() {
        let q = ParamChangeQueue::new();
        // Ids that crowd into the same part of the table.
        for i in 0..SLOTS as u32 {
            q.push(i * SLOTS as u32, 0.5);
        }
        assert!(q.overflow.lock().is_empty());
        assert!(!q.overflow_waiting.load(Ordering::Relaxed));
        q.push(7, 0.5);
        assert_eq!(q.overflow.lock().len(), 1, "only past a full table");
    }

    #[test]
    fn a_take_hands_over_a_bounded_number_of_overflow_changes() {
        let q = ParamChangeQueue::new();
        for id in 0..(SLOTS + 200) as u32 {
            q.push(id, 0.5);
        }
        assert_eq!(
            q.drain(usize::MAX, |_, _| {}),
            SLOTS + MAX_OVERFLOW_PER_TAKE
        );
        assert_eq!(take_all(&q).len(), 200 - MAX_OVERFLOW_PER_TAKE);
    }

    /// A take cut short by a full block list starts next time where it
    /// stopped: a parameter in a high slot is not starved by one in a lower
    /// slot that changes before every block.
    #[test]
    fn a_short_take_does_not_starve_the_slots_after_it() {
        let q = ParamChangeQueue::new();
        let (low, high) = {
            // Two ids, ordered by the slot each lands in.
            q.push(1, 0.0);
            q.push(2, 0.0);
            let (a, b) = (q.slot_for(1).unwrap(), q.slot_for(2).unwrap());
            if a < b {
                (1, 2)
            } else {
                (2, 1)
            }
        };
        let mut seen = Vec::new();
        for _ in 0..3 {
            q.push(low, 1.0);
            q.drain(1, |id, _| seen.push(id));
        }
        assert!(seen.contains(&high), "taken: {seen:?}");
    }

    #[test]
    fn a_discard_drops_what_is_waiting() {
        let q = ParamChangeQueue::new();
        for id in 0..(SLOTS + 100) as u32 {
            q.push(id, 0.5);
        }
        q.discard();
        assert!(!q.has_pending());
        assert!(take_all(&q).is_empty());
        q.push(3, 0.25);
        assert_eq!(take_all(&q), vec![(3, 0.25)]);
    }

    /// Window threads push while the audio thread takes every "block". The
    /// audio side must end up with every parameter's last value, and must
    /// never see a value that was not pushed for that parameter.
    #[test]
    fn concurrent_pushes_never_lose_the_latest_value() {
        const PRODUCERS: u32 = 3;
        const PARAMS: u32 = 40;
        const ROUNDS: u32 = 20_000;
        let q = Arc::new(ParamChangeQueue::new());
        let done = Arc::new(AtomicBool::new(false));
        let producers: Vec<_> = (0..PRODUCERS)
            .map(|p| {
                let q = Arc::clone(&q);
                std::thread::spawn(move || {
                    // Each producer owns its own parameters, so its last
                    // push per parameter is that parameter's final value.
                    for round in 0..ROUNDS {
                        let id = p * 1000 + round % PARAMS;
                        q.push(id, (round as f64) + p as f64 * 0.001);
                    }
                })
            })
            .collect();
        let consumer = {
            let q = Arc::clone(&q);
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let mut seen: HashMap<u32, f64> = HashMap::new();
                loop {
                    let finished = done.load(Ordering::Acquire);
                    q.drain(64, |id, v| {
                        let p = id / 1000;
                        assert!(p < PRODUCERS, "unknown id {id}");
                        assert_eq!(
                            (v.fract() * 1000.0).round() as u32,
                            p,
                            "value {v} was never pushed for {id}"
                        );
                        if let Some(prev) = seen.insert(id, v) {
                            assert!(v >= prev, "{id} went back from {prev} to {v}");
                        }
                    });
                    if finished && !q.has_pending() {
                        return seen;
                    }
                    std::hint::spin_loop();
                }
            })
        };
        for p in producers {
            p.join().unwrap();
        }
        done.store(true, Ordering::Release);
        let seen = consumer.join().unwrap();
        for p in 0..PRODUCERS {
            for param in 0..PARAMS {
                let id = p * 1000 + param;
                let last_round = (0..ROUNDS).rev().find(|r| r % PARAMS == param).unwrap();
                assert_eq!(
                    seen.get(&id).copied(),
                    Some(last_round as f64 + p as f64 * 0.001),
                    "parameter {id} lost its last value"
                );
            }
        }
    }
}
