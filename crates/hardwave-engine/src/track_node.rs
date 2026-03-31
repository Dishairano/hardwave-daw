//! TrackNode — an AudioNode that plays audio clips for a single track.
//!
//! Reads from the AudioPool based on clip positions and the current transport position.
//! Applies track volume and pan. No allocations in the process call.

use std::sync::Arc;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioNode, ProcessContext};

/// Description of a clip placed on this track, used by the audio thread.
/// This is a lightweight copy of the project-level ClipPlacement, pre-resolved
/// to sample positions so the audio thread doesn't need the tempo map.
#[derive(Debug, Clone)]
pub struct ClipRegion {
    /// Key into AudioPool.
    pub source_id: String,
    /// Where this clip starts on the timeline (in samples).
    pub timeline_start: u64,
    /// Where this clip ends on the timeline (in samples).
    pub timeline_end: u64,
    /// Offset into the source audio buffer (in samples).
    pub source_offset: u64,
    /// Gain multiplier (linear, from dB).
    pub gain: f32,
    /// Whether this clip is muted.
    pub muted: bool,
}

/// Audio node for a single track. Holds references to its clips and the shared audio pool.
pub struct TrackNode {
    name: String,
    pool: AudioPool,
    clips: Vec<ClipRegion>,
    volume: f32,   // linear
    pan: f32,      // -1.0 to 1.0
    muted: bool,
    soloed: bool,
    /// Cached Arc references to audio buffers, refreshed when clips change.
    cached_buffers: Vec<Option<Arc<AudioBuffer>>>,
}

impl TrackNode {
    pub fn new(name: String, pool: AudioPool) -> Self {
        Self {
            name,
            pool,
            clips: Vec::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            cached_buffers: Vec::new(),
        }
    }

    /// Update the clip list. Called from the engine thread (not the audio thread)
    /// when the project state changes.
    pub fn set_clips(&mut self, clips: Vec<ClipRegion>) {
        self.cached_buffers = clips.iter().map(|c| self.pool.get(&c.source_id)).collect();
        self.clips = clips;
    }

    pub fn set_volume_db(&mut self, db: f64) {
        self.volume = db_to_linear(db);
    }

    pub fn set_pan(&mut self, pan: f64) {
        self.pan = pan as f32;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_soloed(&mut self, soloed: bool) {
        self.soloed = soloed;
    }
}

impl AudioNode for TrackNode {
    fn name(&self) -> &str {
        &self.name
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        let buf_size = ctx.buffer_size as usize;

        // Ensure outputs are sized
        if outputs.len() < 2 {
            return;
        }
        outputs[0].resize(buf_size, 0.0);
        outputs[1].resize(buf_size, 0.0);
        outputs[0].fill(0.0);
        outputs[1].fill(0.0);

        if self.muted || !ctx.playing {
            return;
        }

        let pos = ctx.position_samples;

        // Sum all clips that overlap the current buffer window
        for (clip_idx, clip) in self.clips.iter().enumerate() {
            if clip.muted {
                continue;
            }

            let buf_end = pos + buf_size as u64;

            // Check if clip overlaps this buffer
            if clip.timeline_end <= pos || clip.timeline_start >= buf_end {
                continue;
            }

            let audio_buf = match &self.cached_buffers[clip_idx] {
                Some(b) => b,
                None => continue,
            };

            let num_channels = audio_buf.channels.len().min(2);
            if num_channels == 0 {
                continue;
            }

            for frame in 0..buf_size {
                let timeline_sample = pos + frame as u64;

                if timeline_sample < clip.timeline_start || timeline_sample >= clip.timeline_end {
                    continue;
                }

                let source_frame = (timeline_sample - clip.timeline_start + clip.source_offset) as usize;

                if source_frame >= audio_buf.num_frames {
                    continue;
                }

                let l = audio_buf.sample(0, source_frame) * clip.gain;
                let r = if num_channels > 1 {
                    audio_buf.sample(1, source_frame) * clip.gain
                } else {
                    l // mono → duplicate to both channels
                };

                outputs[0][frame] += l;
                outputs[1][frame] += r;
            }
        }

        // Apply track volume and pan
        let (pan_l, pan_r) = pan_law(self.pan);
        let vol = self.volume;

        for frame in 0..buf_size {
            outputs[0][frame] *= vol * pan_l;
            outputs[1][frame] *= vol * pan_r;
        }
    }

    fn reset(&mut self) {
        // Re-cache buffers in case pool contents changed
        self.cached_buffers = self.clips.iter().map(|c| self.pool.get(&c.source_id)).collect();
    }
}

/// Convert dB to linear gain.
#[inline]
fn db_to_linear(db: f64) -> f32 {
    if db <= -100.0 {
        0.0
    } else {
        10.0_f64.powf(db / 20.0) as f32
    }
}

/// Constant-power pan law. Returns (left_gain, right_gain).
#[inline]
fn pan_law(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}
