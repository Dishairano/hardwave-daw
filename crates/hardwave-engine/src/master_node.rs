//! MasterNode — sums all track inputs and applies master volume.

use crate::graph::{AudioNode, ProcessContext};
use atomic_float::AtomicF64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct MasterNode {
    /// dB value read from the shared transport state each process block.
    /// Using an atomic keeps volume changes lock-free and graph-rebuild-free.
    volume_db: Arc<AtomicF64>,
}

impl MasterNode {
    pub fn new(volume_db: Arc<AtomicF64>) -> Self {
        Self { volume_db }
    }
}

impl AudioNode for MasterNode {
    fn name(&self) -> &str {
        "Master"
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        let buf_size = ctx.buffer_size as usize;
        if outputs.len() < 2 {
            return;
        }
        outputs[0].resize(buf_size, 0.0);
        outputs[1].resize(buf_size, 0.0);

        let db = self.volume_db.load(Ordering::Relaxed);
        let gain = if db <= -100.0 {
            0.0
        } else {
            10.0_f64.powf(db / 20.0) as f32
        };

        for (i, sample) in outputs[0].iter_mut().enumerate() {
            *sample = inputs
                .first()
                .and_then(|ch| ch.get(i))
                .copied()
                .unwrap_or(0.0)
                * gain;
        }
        for (i, sample) in outputs[1].iter_mut().enumerate() {
            *sample = inputs
                .get(1)
                .and_then(|ch| ch.get(i))
                .copied()
                .unwrap_or(0.0)
                * gain;
        }
    }
}
