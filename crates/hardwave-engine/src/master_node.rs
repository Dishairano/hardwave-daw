//! MasterNode — sums all track inputs and applies master volume.

use crate::graph::{AudioNode, ProcessContext};

pub struct MasterNode {
    volume: f32,
}

impl MasterNode {
    pub fn new() -> Self {
        Self { volume: 1.0 }
    }

    pub fn set_volume_db(&mut self, db: f64) {
        self.volume = if db <= -100.0 { 0.0 } else { 10.0_f64.powf(db / 20.0) as f32 };
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

        // Copy inputs through (the graph already sums connected sources)
        for (i, sample) in outputs[0].iter_mut().enumerate() {
            *sample = inputs.get(0).and_then(|ch| ch.get(i)).copied().unwrap_or(0.0) * self.volume;
        }
        for (i, sample) in outputs[1].iter_mut().enumerate() {
            *sample = inputs.get(1).and_then(|ch| ch.get(i)).copied().unwrap_or(0.0) * self.volume;
        }
    }
}
