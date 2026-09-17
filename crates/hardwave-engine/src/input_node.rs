//! InputNode — streams live audio from the input device into the graph.
//!
//! Holds the consumer side of a lock-free ring buffer that `hardwave-audio-io`'s
//! input callback pushes interleaved stereo samples into. On each process()
//! call the node drains one block worth of frames (or silence, when the ring
//! is empty or detached) and writes them to its stereo outputs, so armed
//! tracks hear themselves through their FX chain.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::graph::{AudioNode, ProcessContext};

pub type SharedInputConsumer = Arc<Mutex<Option<rtrb::Consumer<f32>>>>;

/// Where a recording is written while it happens.
///
/// The take used to be pushed into a `Vec` behind a mutex, with `reserve`
/// and `push` running on the audio thread: growing a buffer while recording
/// is an allocation in the audio callback, which is the classic cause of a
/// click in the take. Worse, the audio thread took the lock with `try_lock`
/// and dropped a whole block when the UI held it, so a take could lose 10 ms
/// of audio silently.
///
/// Now the space is reserved up front, on the UI thread, before recording
/// starts. The audio thread only writes into it and bumps a counter, so it
/// never allocates and never waits. A take that outgrows the reservation
/// stops growing rather than reallocating; `overflowed` says so, and the
/// caller can tell the user instead of handing them a truncated file that
/// looks complete.
pub struct CaptureTap {
    pub recording: AtomicBool,
    /// Frames of headroom, as interleaved stereo samples. Written only by
    /// the UI thread while recording is off.
    slots: Mutex<Vec<f32>>,
    /// Samples written so far by the audio thread.
    written: AtomicUsize,
    /// Set when the reservation ran out.
    overflowed: AtomicBool,
    /// Punch window in samples. The playlist has let people set a punch
    /// range, drawn it, and saved it with the project since it was written,
    /// and nothing read it: recording ignored the range completely.
    punch_enabled: AtomicBool,
    punch_in: AtomicU64,
    punch_out: AtomicU64,
}

impl Default for CaptureTap {
    fn default() -> Self {
        Self {
            recording: AtomicBool::new(false),
            slots: Mutex::new(Vec::new()),
            written: AtomicUsize::new(0),
            overflowed: AtomicBool::new(false),
            punch_enabled: AtomicBool::new(false),
            punch_in: AtomicU64::new(0),
            punch_out: AtomicU64::new(u64::MAX),
        }
    }
}

impl CaptureTap {
    /// Reserve room for `seconds` of stereo audio and start from zero.
    ///
    /// Called from the UI thread with recording off, so the allocation
    /// happens here rather than in the audio callback.
    pub fn arm(&self, sample_rate: u32, seconds: u32) {
        let needed = (sample_rate as usize).saturating_mul(seconds as usize) * 2;
        let mut slots = self.slots.lock();
        if slots.len() < needed {
            slots.resize(needed, 0.0);
        }
        self.written.store(0, Ordering::Relaxed);
        self.overflowed.store(false, Ordering::Relaxed);
    }

    /// Take the captured samples and reset for the next session.
    pub fn take(&self) -> Vec<f32> {
        let slots = self.slots.lock();
        let n = self.written.swap(0, Ordering::Relaxed).min(slots.len());
        slots[..n].to_vec()
    }

    /// Set the punch window, in samples on the project's timeline.
    ///
    /// With it enabled, only audio inside the window is captured, to the
    /// sample: a block that straddles punch-in keeps its tail, and one that
    /// straddles punch-out keeps its head.
    pub fn set_punch(&self, enabled: bool, in_samples: u64, out_samples: u64) {
        self.punch_enabled.store(enabled, Ordering::Relaxed);
        self.punch_in.store(in_samples, Ordering::Relaxed);
        self.punch_out
            .store(out_samples.max(in_samples), Ordering::Relaxed);
    }

    pub fn punch(&self) -> (bool, u64, u64) {
        (
            self.punch_enabled.load(Ordering::Relaxed),
            self.punch_in.load(Ordering::Relaxed),
            self.punch_out.load(Ordering::Relaxed),
        )
    }

    /// The part of a block that falls inside the punch window, as an offset
    /// and a length in frames. Pure, so the window arithmetic is testable
    /// without an audio device.
    fn punched_span(&self, block_start: u64, frames: usize) -> (usize, usize) {
        let (enabled, punch_in, punch_out) = self.punch();
        if !enabled {
            return (0, frames);
        }
        let block_end = block_start.saturating_add(frames as u64);
        if block_end <= punch_in || block_start >= punch_out {
            return (0, 0);
        }
        let start = punch_in.saturating_sub(block_start) as usize;
        let end = (punch_out.min(block_end) - block_start) as usize;
        (start.min(frames), end.min(frames).saturating_sub(start))
    }

    /// Whether the last session ran out of reserved room.
    pub fn overflowed(&self) -> bool {
        self.overflowed.load(Ordering::Relaxed)
    }

    /// Seconds captured so far, for a recording time read-out.
    pub fn seconds_captured(&self, sample_rate: u32) -> f64 {
        if sample_rate == 0 {
            return 0.0;
        }
        self.written.load(Ordering::Relaxed) as f64 / 2.0 / sample_rate as f64
    }

    /// Same as the audio thread's write, exposed for tests that stand in for
    /// `InputNode::process`. `block_start` is the playhead in samples.
    #[doc(hidden)]
    pub fn write_test_block_at(
        &self,
        left: &[f32],
        right: &[f32],
        frames: usize,
        block_start: u64,
    ) {
        self.write_block(left, right, frames, block_start)
    }

    #[doc(hidden)]
    pub fn write_test_block(&self, left: &[f32], right: &[f32], frames: usize) {
        self.write_block(left, right, frames, 0)
    }

    /// Write one block of interleaved stereo. Audio thread only.
    ///
    /// `try_lock` here can only fail against `arm` or `take`, both of which
    /// run with recording off, so in a live session this never contends and
    /// never drops a block.
    fn write_block(&self, left: &[f32], right: &[f32], frames: usize, block_start: u64) {
        let (offset, take) = self.punched_span(block_start, frames);
        if take == 0 {
            return;
        }
        let Some(mut slots) = self.slots.try_lock() else {
            self.overflowed.store(true, Ordering::Relaxed);
            return;
        };
        let start = self.written.load(Ordering::Relaxed);
        let room = slots.len().saturating_sub(start) / 2;
        let n = take.min(room);
        if n < take {
            self.overflowed.store(true, Ordering::Relaxed);
        }
        for i in 0..n {
            slots[start + i * 2] = left[offset + i];
            slots[start + i * 2 + 1] = right[offset + i];
        }
        self.written.store(start + n * 2, Ordering::Relaxed);
    }
}

pub struct InputNode {
    consumer: SharedInputConsumer,
    capture: Option<Arc<CaptureTap>>,
}

impl InputNode {
    pub fn new(consumer: SharedInputConsumer) -> Self {
        Self {
            consumer,
            capture: None,
        }
    }

    /// Attach a capture tap. When the tap's `recording` flag is true and
    /// the input ring is producing samples, every block the node drains
    /// is also pushed to the tap's buffer. Detach by calling
    /// `set_capture(None)`.
    pub fn set_capture(&mut self, capture: Option<Arc<CaptureTap>>) {
        self.capture = capture;
    }
}

impl AudioNode for InputNode {
    fn name(&self) -> &str {
        "Input"
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        let buf_size = outputs.first().map(|o| o.len()).unwrap_or(0);
        for ch in outputs.iter_mut() {
            ch.fill(0.0);
        }
        if buf_size == 0 || outputs.len() < 2 {
            return;
        }

        // try_lock so a UI-thread rebuild that's swapping the consumer can
        // never stall the audio callback. If we can't lock (or no consumer
        // is attached), the outputs stay at silence for this block.
        let mut guard = match self.consumer.try_lock() {
            Some(g) => g,
            None => return,
        };
        let cons = match guard.as_mut() {
            Some(c) => c,
            None => return,
        };

        let (left, rest) = outputs.split_at_mut(1);
        let (right, _) = rest.split_at_mut(1);
        for (l_out, r_out) in left[0].iter_mut().zip(right[0].iter_mut()).take(buf_size) {
            *l_out = cons.pop().unwrap_or(0.0);
            *r_out = cons.pop().unwrap_or(0.0);
        }

        // Recording tap: the block just produced goes into the reserved
        // capture space, interleaved L, R, L, R, so a WAV writer can dump it
        // straight to disk on stop.
        if let Some(cap) = &self.capture {
            if cap.recording.load(Ordering::Relaxed) {
                cap.write_block(&left[0], &right[0], buf_size, ctx.position_samples);
            }
        }
    }
}

#[cfg(test)]
mod capture_tests {
    use super::*;

    fn tap(seconds: u32) -> CaptureTap {
        let t = CaptureTap::default();
        t.arm(48_000, seconds);
        t.recording.store(true, Ordering::Relaxed);
        t
    }

    #[test]
    fn a_block_is_captured_interleaved() {
        let t = tap(1);
        t.write_test_block(&[0.1, 0.2], &[-0.1, -0.2], 2);
        let out = t.take();
        assert_eq!(out, vec![0.1, -0.1, 0.2, -0.2]);
    }

    #[test]
    fn consecutive_blocks_keep_their_order() {
        let t = tap(1);
        t.write_test_block(&[0.1], &[0.1], 1);
        t.write_test_block(&[0.2], &[0.2], 1);
        t.write_test_block(&[0.3], &[0.3], 1);
        let out = t.take();
        assert_eq!(out.len(), 6);
        assert_eq!(out[0], 0.1);
        assert_eq!(out[2], 0.2);
        assert_eq!(out[4], 0.3);
    }

    /// The tap used to push into a Vec, so a long take grew a buffer on the
    /// audio thread. The room is reserved before recording starts, and the
    /// writing path must not change the allocation.
    #[test]
    fn writing_does_not_grow_the_reservation() {
        let t = tap(1);
        let capacity_before = t.slots.lock().capacity();
        for _ in 0..200 {
            t.write_test_block(&[0.5; 64], &[0.5; 64], 64);
        }
        assert_eq!(t.slots.lock().capacity(), capacity_before);
    }

    #[test]
    fn a_take_past_the_reservation_is_reported_not_grown() {
        // One second of room, two seconds of audio.
        let t = tap(1);
        for _ in 0..(48_000 / 480 * 2) {
            t.write_test_block(&[0.4; 480], &[0.4; 480], 480);
        }
        assert!(t.overflowed(), "running out of room must be reported");
        let out = t.take();
        assert_eq!(out.len(), 48_000 * 2, "kept exactly what fit");
    }

    #[test]
    fn taking_a_session_resets_for_the_next_one() {
        let t = tap(1);
        t.write_test_block(&[0.1, 0.2], &[0.1, 0.2], 2);
        assert_eq!(t.take().len(), 4);
        assert!(t.take().is_empty(), "a second take must be empty");

        t.write_test_block(&[0.9], &[0.9], 1);
        assert_eq!(t.take(), vec![0.9, 0.9], "and the tap still works after");
    }

    #[test]
    fn the_time_read_out_follows_what_was_captured() {
        let t = tap(2);
        t.write_test_block(&[0.1; 24_000], &[0.1; 24_000], 24_000);
        assert!((t.seconds_captured(48_000) - 0.5).abs() < 1e-9);
    }

    /// The punch range was drawn in the playlist and saved with the project,
    /// and recording ignored it: it captured the whole pass.
    #[test]
    fn with_punch_off_the_whole_block_is_captured() {
        let t = tap(4);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert_eq!(t.take().len(), 960);
    }

    #[test]
    fn a_block_before_punch_in_is_not_captured() {
        let t = tap(4);
        t.set_punch(true, 48_000, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 0);
        assert!(t.take().is_empty(), "audio before the punch point got in");
    }

    #[test]
    fn a_block_after_punch_out_is_not_captured() {
        let t = tap(4);
        t.set_punch(true, 48_000, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert!(t.take().is_empty(), "audio after the punch point got in");
    }

    #[test]
    fn a_block_straddling_punch_in_keeps_only_its_tail() {
        let t = tap(4);
        // Punch in 100 frames into this block.
        t.set_punch(true, 48_100, 96_000);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(
            out.len(),
            (480 - 100) * 2,
            "kept the wrong number of frames"
        );
        assert_eq!(out[0], 100.0, "the first captured frame is the punch point");
    }

    #[test]
    fn a_block_straddling_punch_out_keeps_only_its_head() {
        let t = tap(4);
        t.set_punch(true, 48_000, 48_200);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(out.len(), 200 * 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[out.len() - 2], 199.0, "cut exactly at the punch point");
    }

    #[test]
    fn a_punch_window_inside_one_block_keeps_only_that_slice() {
        let t = tap(4);
        t.set_punch(true, 48_100, 48_150);
        let left: Vec<f32> = (0..480).map(|i| i as f32).collect();
        t.write_test_block_at(&left, &left, 480, 48_000);
        let out = t.take();
        assert_eq!(out.len(), 50 * 2);
        assert_eq!(out[0], 100.0);
    }

    #[test]
    fn a_reversed_punch_range_is_treated_as_empty_not_infinite() {
        let t = tap(4);
        // Out before in: clamped so the window is zero-length, not the whole
        // song, which would silently record everything.
        t.set_punch(true, 96_000, 48_000);
        let (_, punch_in, punch_out) = t.punch();
        assert_eq!(punch_in, 96_000);
        assert_eq!(punch_out, 96_000);
        t.write_test_block_at(&[0.5; 480], &[0.5; 480], 480, 96_000);
        assert!(t.take().is_empty());
    }

    #[test]
    fn arming_clears_the_previous_overflow() {
        let t = tap(1);
        for _ in 0..300 {
            t.write_test_block(&[0.4; 480], &[0.4; 480], 480);
        }
        assert!(t.overflowed());
        t.arm(48_000, 1);
        assert!(!t.overflowed(), "a new take starts clean");
    }
}
