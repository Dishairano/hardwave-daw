//! InputNode — streams live audio from the input device into the graph.
//!
//! Holds the consumer side of a lock-free ring buffer that `hardwave-audio-io`'s
//! input callback pushes interleaved stereo samples into. On each process()
//! call the node drains one block worth of frames (or silence, when the ring
//! is empty or detached) and writes them to its stereo outputs, so armed
//! tracks hear themselves through their FX chain.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::graph::{AudioNode, ProcessContext};

pub type SharedInputConsumer = Arc<Mutex<Option<rtrb::Consumer<f32>>>>;

/// Lock-free recording capture target: when [`recording`] is true and
/// [`buffer`] is non-empty (i.e. a session is active), each input block
/// is appended interleaved (L, R, L, R …) to the buffer. The audio
/// thread try_lock's the buffer; if the UI thread is in the middle of a
/// take/clear it just drops the block, which is acceptable for a
/// recording start-up race.
#[derive(Default)]
pub struct CaptureTap {
    pub recording: AtomicBool,
    pub buffer: Mutex<Vec<f32>>,
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
        _ctx: &ProcessContext,
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

        // Recording tap. Push the block we just produced into the
        // capture buffer interleaved L, R, L, R … so a downstream WAV
        // writer can dump straight to disk on stop. We try_lock the
        // buffer; under contention (UI thread reading captured samples)
        // we drop one block, which is safer than stalling the audio
        // callback.
        if let Some(cap) = &self.capture {
            if cap.recording.load(Ordering::Relaxed) {
                if let Some(mut buf) = cap.buffer.try_lock() {
                    buf.reserve(buf_size * 2);
                    for i in 0..buf_size {
                        buf.push(left[0][i]);
                        buf.push(right[0][i]);
                    }
                }
            }
        }
    }
}
