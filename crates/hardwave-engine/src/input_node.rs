//! InputNode — streams live audio from the input device into the graph.
//!
//! Holds the consumer side of a lock-free ring buffer that `hardwave-audio-io`'s
//! input callback pushes interleaved stereo samples into. On each process()
//! call the node drains one block worth of frames (or silence, when the ring
//! is empty or detached) and writes them to its stereo outputs, so armed
//! tracks hear themselves through their FX chain.

use parking_lot::Mutex;
use std::sync::Arc;

use crate::graph::{AudioNode, ProcessContext};

pub type SharedInputConsumer = Arc<Mutex<Option<rtrb::Consumer<f32>>>>;

pub struct InputNode {
    consumer: SharedInputConsumer,
}

impl InputNode {
    pub fn new(consumer: SharedInputConsumer) -> Self {
        Self { consumer }
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
    }
}
