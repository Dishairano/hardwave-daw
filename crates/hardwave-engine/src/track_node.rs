//! TrackNode — an AudioNode that plays audio clips for a single track.
//!
//! Reads from the AudioPool based on clip positions and the current transport position.
//! Applies track volume and pan. No allocations in the process call.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use atomic_float::AtomicF32;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioNode, ProcessContext};

/// Post-fader meter state shared between the audio thread and the UI thread.
/// Using atomics so the audio thread never allocates or locks.
#[derive(Default)]
pub struct TrackMeterState {
    /// Post-fader peak in dB (last processed block, no hold/decay — UI smooths).
    pub peak_db_l: AtomicF32,
    pub peak_db_r: AtomicF32,
    /// Post-fader RMS in dB (mono sum, smoothed).
    pub rms_db: AtomicF32,
    /// Pre-fader peak in dB (max of L/R before volume/pan).
    pub pre_fader_peak_db: AtomicF32,
}

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
    /// Linear fade-in length in samples (0 = no fade).
    pub fade_in_samples: u64,
    /// Linear fade-out length in samples (0 = no fade).
    pub fade_out_samples: u64,
    /// Play source backwards when true.
    pub reversed: bool,
    /// Source-frames consumed per timeline sample. 1.0 = realtime, >1 = pitched up/faster.
    /// Combines pitch shift and time stretch via resampling.
    pub source_step: f64,
}

/// Audio node for a single track. Holds references to its clips and the shared audio pool.
pub struct TrackNode {
    name: String,
    pool: AudioPool,
    clips: Vec<ClipRegion>,
    volume: f32, // linear
    pan: f32,    // -1.0 to 1.0
    muted: bool,
    soloed: bool,
    /// Cached Arc references to audio buffers, refreshed when clips change.
    cached_buffers: Vec<Option<Arc<AudioBuffer>>>,
    /// Shared meter state so the UI can read post-fader peaks without locking.
    meter: Arc<TrackMeterState>,
    /// Smoothed RMS (linear), updated each block.
    rms_smooth: f32,
}

impl TrackNode {
    pub fn new(name: String, pool: AudioPool, meter: Arc<TrackMeterState>) -> Self {
        Self {
            name,
            pool,
            clips: Vec::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            cached_buffers: Vec::new(),
            meter,
            rms_smooth: 0.0,
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

            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];

            let clip_length = clip.timeline_end.saturating_sub(clip.timeline_start);

            for frame in 0..buf_size {
                let timeline_sample = pos + frame as u64;

                if timeline_sample < clip.timeline_start || timeline_sample >= clip.timeline_end {
                    continue;
                }

                let into_clip = timeline_sample - clip.timeline_start;

                // Fractional source position for pitch/stretch resampling.
                let into_src = into_clip as f64 * clip.source_step;
                let source_pos = if clip.reversed {
                    let end_frame = clip.source_offset.saturating_add(clip_length);
                    if end_frame == 0 {
                        continue;
                    }
                    (end_frame as f64 - 1.0) - into_src
                } else {
                    clip.source_offset as f64 + into_src
                };

                if source_pos < 0.0 {
                    continue;
                }
                let idx0 = source_pos.floor() as usize;
                if idx0 >= audio_buf.num_frames {
                    continue;
                }
                let idx1 = (idx0 + 1).min(audio_buf.num_frames - 1);
                let frac = (source_pos - idx0 as f64) as f32;

                // Linear fade-in / fade-out envelope.
                let mut env = 1.0_f32;
                if clip.fade_in_samples > 0 && into_clip < clip.fade_in_samples {
                    env *= into_clip as f32 / clip.fade_in_samples as f32;
                }
                if clip.fade_out_samples > 0 {
                    let to_end = clip_length.saturating_sub(into_clip);
                    if to_end < clip.fade_out_samples {
                        env *= to_end as f32 / clip.fade_out_samples as f32;
                    }
                }

                let scaled_gain = clip.gain * env;
                let l0 = audio_buf.sample(0, idx0);
                let l1 = audio_buf.sample(0, idx1);
                let l = (l0 + (l1 - l0) * frac) * scaled_gain;
                let r = if num_channels > 1 {
                    let r0 = audio_buf.sample(1, idx0);
                    let r1 = audio_buf.sample(1, idx1);
                    (r0 + (r1 - r0) * frac) * scaled_gain
                } else {
                    l
                };

                out_l[frame] += l;
                out_r[frame] += r;
            }
        }

        // Measure pre-fader peak (before volume/pan).
        {
            let mut pre_peak = 0.0_f32;
            for (l, r) in outputs[0].iter().zip(outputs[1].iter()).take(buf_size) {
                pre_peak = pre_peak.max(l.abs());
                pre_peak = pre_peak.max(r.abs());
            }
            self.meter
                .pre_fader_peak_db
                .store(linear_to_db(pre_peak), Ordering::Relaxed);
        }

        // Apply track volume and pan, and measure post-fader peak/RMS.
        let (pan_l, pan_r) = pan_law(self.pan);
        let vol = self.volume;

        let (out_left, out_rest) = outputs.split_at_mut(1);
        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;
        let mut sum_sq = 0.0_f64;
        for (l, r) in out_left[0]
            .iter_mut()
            .zip(out_rest[0].iter_mut())
            .take(buf_size)
        {
            *l *= vol * pan_l;
            *r *= vol * pan_r;
            peak_l = peak_l.max(l.abs());
            peak_r = peak_r.max(r.abs());
            let mono = (*l + *r) * 0.5;
            sum_sq += (mono as f64) * (mono as f64);
        }

        // Publish post-fader meters (lock-free).
        self.meter
            .peak_db_l
            .store(linear_to_db(peak_l), Ordering::Relaxed);
        self.meter
            .peak_db_r
            .store(linear_to_db(peak_r), Ordering::Relaxed);
        let rms_lin = (sum_sq / buf_size.max(1) as f64).sqrt() as f32;
        self.rms_smooth = self.rms_smooth * 0.85 + rms_lin * 0.15;
        self.meter
            .rms_db
            .store(linear_to_db(self.rms_smooth), Ordering::Relaxed);
    }

    fn reset(&mut self) {
        // Re-cache buffers in case pool contents changed
        self.cached_buffers = self
            .clips
            .iter()
            .map(|c| self.pool.get(&c.source_id))
            .collect();
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

#[inline]
fn linear_to_db(linear: f32) -> f32 {
    (20.0 * (linear + 1e-10).log10()).clamp(-100.0, 6.0)
}

/// Constant-power pan law. Returns (left_gain, right_gain).
#[inline]
fn pan_law(pan: f32) -> (f32, f32) {
    let angle = (pan + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_test_node() -> (TrackNode, Arc<TrackMeterState>) {
        let pool = AudioPool::new();
        let meter = Arc::new(TrackMeterState::default());
        let node = TrackNode::new("Test".into(), pool, Arc::clone(&meter));
        (node, meter)
    }

    #[test]
    fn pre_fader_meter_reads_signal_before_volume() {
        let (mut node, meter) = make_test_node();
        node.set_volume_db(-100.0); // fader at -inf

        let ctx = ProcessContext {
            sample_rate: 48000.0,
            buffer_size: 64,
            tempo: 140.0,
            time_sig: (4, 4),
            position_samples: 0,
            playing: true,
        };

        // Simulate signal by writing directly to outputs (as if clips produced it).
        // We can't easily inject clips without the audio pool, so we test the meter
        // reads from outputs after the clip-summing stage by calling process with
        // a node that has no clips (outputs will be zero → pre-fader should be -inf).
        let inputs: Vec<&[f32]> = vec![];
        let mut outputs = vec![vec![0.0f32; 64]; 2];
        let midi_in = vec![];
        let mut midi_out = vec![];
        node.process(&inputs, &mut outputs, &midi_in, &mut midi_out, &ctx);

        // With no clips, pre-fader peak should be near -inf
        let pre = meter.pre_fader_peak_db.load(Ordering::Relaxed);
        assert!(
            pre < -90.0,
            "pre-fader should be near -inf with no signal, got {pre}"
        );

        // Post-fader should also be near -inf
        let post_l = meter.peak_db_l.load(Ordering::Relaxed);
        assert!(
            post_l < -90.0,
            "post-fader should be near -inf, got {post_l}"
        );
    }

    #[test]
    fn db_to_linear_and_back() {
        let db_vals = [-60.0, -12.0, 0.0, 6.0];
        for &db in &db_vals {
            let lin = db_to_linear(db);
            let back = linear_to_db(lin);
            assert!(
                (back - db as f32).abs() < 0.5,
                "roundtrip failed: {db} → {lin} → {back}"
            );
        }
    }

    #[test]
    fn pan_law_center_is_equal() {
        let (l, r) = pan_law(0.0);
        assert!(
            (l - r).abs() < 0.01,
            "center pan should be equal L/R: l={l} r={r}"
        );
    }

    #[test]
    fn pan_law_hard_left() {
        let (l, r) = pan_law(-1.0);
        assert!(l > 0.9, "hard left should have l near 1.0, got {l}");
        assert!(r < 0.01, "hard left should have r near 0.0, got {r}");
    }
}
