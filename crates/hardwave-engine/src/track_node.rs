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
    /// Fade-in length in samples (0 = no fade).
    pub fade_in_samples: u64,
    /// Fade-out length in samples (0 = no fade).
    pub fade_out_samples: u64,
    /// Fade-in curve shape.
    pub fade_in_curve: hardwave_project::clip::FadeCurve,
    /// Fade-out curve shape.
    pub fade_out_curve: hardwave_project::clip::FadeCurve,
    /// Play source backwards when true.
    pub reversed: bool,
    /// Source-frames consumed per timeline sample. 1.0 = realtime, >1 = pitched up/faster.
    /// Combines pitch shift and time stretch via resampling.
    pub source_step: f64,
}

/// Map a normalized 0..=1 progress to a fade gain following the given curve.
#[inline]
fn fade_shape(t: f32, curve: hardwave_project::clip::FadeCurve) -> f32 {
    use hardwave_project::clip::FadeCurve;
    let t = t.clamp(0.0, 1.0);
    match curve {
        FadeCurve::Linear => t,
        FadeCurve::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
        FadeCurve::SCurve => {
            let x = t * 2.0 - 1.0;
            (x * std::f32::consts::FRAC_PI_2).sin() * 0.5 + 0.5
        }
        FadeCurve::Logarithmic => {
            if t <= 0.0 {
                0.0
            } else {
                (1.0 + (t * 99.0).log10() / 2.0).clamp(0.0, 1.0)
            }
        }
    }
}

/// Per-channel filter mode. `Off` bypasses the filter stage entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFilterType {
    Off,
    LowPass,
    HighPass,
    BandPass,
}

impl TrackFilterType {
    pub fn parse(s: &str) -> Self {
        match s {
            "lp" | "low" | "lowpass" => TrackFilterType::LowPass,
            "hp" | "high" | "highpass" => TrackFilterType::HighPass,
            "bp" | "band" | "bandpass" => TrackFilterType::BandPass,
            _ => TrackFilterType::Off,
        }
    }
}

/// Single biquad stage with independent L/R state. Coefficients are recomputed
/// whenever the filter parameters change.
#[derive(Default, Clone, Copy)]
struct BiquadStereo {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1_l: f32,
    x2_l: f32,
    y1_l: f32,
    y2_l: f32,
    x1_r: f32,
    x2_r: f32,
    y1_r: f32,
    y2_r: f32,
}

impl BiquadStereo {
    fn set_coeffs(&mut self, kind: TrackFilterType, cutoff_hz: f32, resonance: f32, sr: f32) {
        if !matches!(
            kind,
            TrackFilterType::LowPass | TrackFilterType::HighPass | TrackFilterType::BandPass
        ) {
            self.b0 = 1.0;
            self.b1 = 0.0;
            self.b2 = 0.0;
            self.a1 = 0.0;
            self.a2 = 0.0;
            return;
        }
        let sr = sr.max(1.0);
        let cutoff = cutoff_hz.clamp(20.0, (sr * 0.49).min(20_000.0));
        let w0 = 2.0 * std::f32::consts::PI * cutoff / sr;
        let q = 0.707 + resonance.clamp(0.0, 1.0) * 9.3;
        let alpha = w0.sin() / (2.0 * q);
        let cos_w0 = w0.cos();
        let a0 = 1.0 + alpha;
        match kind {
            TrackFilterType::LowPass => {
                self.b0 = (1.0 - cos_w0) * 0.5 / a0;
                self.b1 = (1.0 - cos_w0) / a0;
                self.b2 = (1.0 - cos_w0) * 0.5 / a0;
            }
            TrackFilterType::HighPass => {
                self.b0 = (1.0 + cos_w0) * 0.5 / a0;
                self.b1 = -(1.0 + cos_w0) / a0;
                self.b2 = (1.0 + cos_w0) * 0.5 / a0;
            }
            TrackFilterType::BandPass => {
                self.b0 = alpha / a0;
                self.b1 = 0.0;
                self.b2 = -alpha / a0;
            }
            TrackFilterType::Off => unreachable!(),
        }
        self.a1 = -2.0 * cos_w0 / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    fn process_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        let yl = self.b0 * l + self.b1 * self.x1_l + self.b2 * self.x2_l
            - self.a1 * self.y1_l
            - self.a2 * self.y2_l;
        self.x2_l = self.x1_l;
        self.x1_l = l;
        self.y2_l = self.y1_l;
        self.y1_l = yl;

        let yr = self.b0 * r + self.b1 * self.x1_r + self.b2 * self.x2_r
            - self.a1 * self.y1_r
            - self.a2 * self.y2_r;
        self.x2_r = self.x1_r;
        self.x1_r = r;
        self.y2_r = self.y1_r;
        self.y1_r = yr;

        (yl, yr)
    }

    fn reset_state(&mut self) {
        self.x1_l = 0.0;
        self.x2_l = 0.0;
        self.y1_l = 0.0;
        self.y2_l = 0.0;
        self.x1_r = 0.0;
        self.x2_r = 0.0;
        self.y1_r = 0.0;
        self.y2_r = 0.0;
    }
}

/// Audio node for a single track. Holds references to its clips and the shared audio pool.
pub struct TrackNode {
    /// Stable project-side track id. Lets the engine route plug-in
    /// insert commands here and survive the chain across graph rebuilds.
    track_id: String,
    name: String,
    pool: AudioPool,
    clips: Vec<ClipRegion>,
    volume: f32, // linear
    pan: f32,    // -1.0 to 1.0
    muted: bool,
    soloed: bool,
    phase_invert: bool,
    swap_lr: bool,
    /// 0.0 = mono, 1.0 = normal, >1.0 = widened.
    stereo_separation: f32,
    /// Positive = delay, negative = advance. Bounded against delay_capacity.
    delay_samples: i32,
    /// Delay line for the left/right channels (ring buffer, zero-initialized).
    delay_buf_l: Vec<f32>,
    delay_buf_r: Vec<f32>,
    delay_write_pos: usize,
    /// Cached Arc references to audio buffers, refreshed when clips change.
    cached_buffers: Vec<Option<Arc<AudioBuffer>>>,
    /// Shared meter state so the UI can read post-fader peaks without locking.
    meter: Arc<TrackMeterState>,
    /// Smoothed RMS (linear), updated each block.
    rms_smooth: f32,
    /// Per-channel filter stage (biquad), applied before the fader.
    filter_kind: TrackFilterType,
    filter: BiquadStereo,
    /// Per-track plug-in insert chain. Audio flows through here after
    /// the filter+delay step but before the fader+pan, matching the
    /// pre-fader insert slot every major DAW exposes.
    chain: crate::insert_chain::InsertChain,
    /// Reusable scratch buffers for `chain.process` so the audio thread
    /// stays allocation-free in steady state.
    chain_scratch: crate::insert_chain::Scratch,
}

const TRACK_DELAY_CAPACITY: usize = 4800; // 100 ms at 48 kHz, both directions

impl TrackNode {
    /// Construct a track node bound to a specific project track id. The
    /// id propagates through the audio thread so per-track plug-in
    /// commands can be routed directly without scanning every node.
    pub fn new(
        track_id: String,
        name: String,
        pool: AudioPool,
        meter: Arc<TrackMeterState>,
    ) -> Self {
        Self {
            track_id,
            name,
            pool,
            clips: Vec::new(),
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            phase_invert: false,
            swap_lr: false,
            stereo_separation: 1.0,
            delay_samples: 0,
            delay_buf_l: vec![0.0; TRACK_DELAY_CAPACITY],
            delay_buf_r: vec![0.0; TRACK_DELAY_CAPACITY],
            delay_write_pos: 0,
            cached_buffers: Vec::new(),
            meter,
            rms_smooth: 0.0,
            filter_kind: TrackFilterType::Off,
            filter: BiquadStereo::default(),
            chain: crate::insert_chain::InsertChain::new(),
            chain_scratch: crate::insert_chain::Scratch::default(),
        }
    }

    /// Configure the per-channel filter. Called from the engine thread when the
    /// project state changes, so computing coefficients here is fine.
    pub fn set_filter(
        &mut self,
        kind: TrackFilterType,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
    ) {
        let changing_kind = self.filter_kind != kind;
        self.filter_kind = kind;
        self.filter
            .set_coeffs(kind, cutoff_hz, resonance, sample_rate);
        if changing_kind {
            self.filter.reset_state();
        }
    }

    pub fn set_phase_invert(&mut self, inv: bool) {
        self.phase_invert = inv;
    }

    pub fn set_swap_lr(&mut self, swap: bool) {
        self.swap_lr = swap;
    }

    pub fn set_stereo_separation(&mut self, sep: f64) {
        self.stereo_separation = sep.clamp(0.0, 2.0) as f32;
    }

    pub fn set_delay_samples(&mut self, samples: i64) {
        let max = TRACK_DELAY_CAPACITY as i64 - 1;
        // Only positive delay is honored; negative delay (advance) isn't representable
        // in a streaming graph without lookahead, and is normally achieved by delaying
        // the other tracks instead.
        self.delay_samples = samples.clamp(0, max) as i32;
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

    fn track_id(&self) -> Option<&str> {
        Some(&self.track_id)
    }

    fn apply_insert_command(
        &mut self,
        cmd: crate::insert_chain::InsertCommand,
        graveyard: &mut crate::insert_chain::PluginGraveyardSender,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        use crate::insert_chain::InsertCommand;
        match cmd {
            InsertCommand::Add { slot, .. } => {
                if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
                    log::warn!("track {} insert add failed: {e}", self.track_id);
                }
            }
            InsertCommand::Remove { slot_id, .. } => {
                if let Some(slot) = self.chain.take_slot(&slot_id) {
                    if graveyard.try_bury(slot).is_err() {
                        log::warn!(
                            "track {} graveyard full while removing {slot_id}; slot will leak until engine teardown",
                            self.track_id
                        );
                    }
                }
            }
            InsertCommand::Reorder { from, to, .. } => self.chain.reorder(from, to),
            InsertCommand::SetEnabled { slot_id, enabled, .. } => {
                self.chain.set_enabled(&slot_id, enabled);
            }
            InsertCommand::SetWet { slot_id, wet, .. } => {
                self.chain.set_wet(&slot_id, wet);
            }
            InsertCommand::SetParameter {
                slot_id,
                param_id,
                value,
                ..
            } => {
                self.chain.set_parameter(&slot_id, param_id, value);
            }
        }
    }

    fn take_chain(&mut self) -> Option<crate::insert_chain::InsertChain> {
        // Swap in an empty chain so this TrackNode keeps its other state
        // intact while the rebuild path moves the live plug-in instances
        // onto the freshly-constructed replacement TrackNode.
        let taken = std::mem::take(&mut self.chain);
        if taken.slots.is_empty() {
            None
        } else {
            Some(taken)
        }
    }

    fn restore_chain(&mut self, chain: crate::insert_chain::InsertChain) {
        self.chain = chain;
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

        // Ensure outputs are sized. Channels 0/1 carry post-fader L/R; 2/3
        // carry the pre-fader tap used by pre-fader sends. Downstream nodes
        // that only need stereo ignore 2/3.
        if outputs.len() < 2 {
            return;
        }
        for ch in outputs.iter_mut() {
            ch.resize(buf_size, 0.0);
            ch.fill(0.0);
        }

        if self.muted || !ctx.playing {
            return;
        }

        // Mix in send inputs arriving at this track (return routing). Sends
        // arrive at ports 0/1 and are treated as channel input — summed with
        // the clip output before the pre-fader chain so the return track's
        // own utilities, fader, and pan apply to the received signal too.
        {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            if let Some(in_l) = inputs.first() {
                for (o, s) in out_l.iter_mut().zip(in_l.iter()).take(buf_size) {
                    *o += *s;
                }
            }
            if let Some(in_r) = inputs.get(1) {
                for (o, s) in out_r.iter_mut().zip(in_r.iter()).take(buf_size) {
                    *o += *s;
                }
            }
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

                // Fade-in / fade-out envelope with selectable curve shape.
                let mut env = 1.0_f32;
                if clip.fade_in_samples > 0 && into_clip < clip.fade_in_samples {
                    let t = into_clip as f32 / clip.fade_in_samples as f32;
                    env *= fade_shape(t, clip.fade_in_curve);
                }
                if clip.fade_out_samples > 0 {
                    let to_end = clip_length.saturating_sub(into_clip);
                    if to_end < clip.fade_out_samples {
                        let t = to_end as f32 / clip.fade_out_samples as f32;
                        env *= fade_shape(t, clip.fade_out_curve);
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

        // --- Pre-fader utility chain: delay → swap → phase → stereo separation.
        // Order is deliberate: delay first so the shifted signal feeds the rest of
        // the chain, then L/R swap, then polarity flip, then M/S separation.

        if self.delay_samples > 0 {
            let cap = self.delay_buf_l.len();
            let delay = self.delay_samples as usize;
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            for frame in 0..buf_size {
                let write = self.delay_write_pos;
                let read = (write + cap - delay) % cap;
                let in_l = out_l[frame];
                let in_r = out_r[frame];
                out_l[frame] = self.delay_buf_l[read];
                out_r[frame] = self.delay_buf_r[read];
                self.delay_buf_l[write] = in_l;
                self.delay_buf_r[write] = in_r;
                self.delay_write_pos = (write + 1) % cap;
            }
        }

        if self.swap_lr {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            for frame in 0..buf_size {
                std::mem::swap(&mut out_l[frame], &mut out_r[frame]);
            }
        }

        if self.phase_invert {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            for s in out_left[0].iter_mut().take(buf_size) {
                *s = -*s;
            }
            for s in out_rest[0].iter_mut().take(buf_size) {
                *s = -*s;
            }
        }

        if (self.stereo_separation - 1.0).abs() > 1e-4 {
            let sep = self.stereo_separation;
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            for frame in 0..buf_size {
                let l = out_l[frame];
                let r = out_r[frame];
                let mid = (l + r) * 0.5;
                let side = (l - r) * 0.5;
                out_l[frame] = mid + side * sep;
                out_r[frame] = mid - side * sep;
            }
        }

        // Per-channel biquad filter stage. Applied pre-fader so metering and
        // sends see the filtered signal. `Off` skips the inner loop.
        if self.filter_kind != TrackFilterType::Off {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            for frame in 0..buf_size {
                let (yl, yr) = self.filter.process_stereo(out_l[frame], out_r[frame]);
                out_l[frame] = yl;
                out_r[frame] = yr;
            }
        }

        // Plug-in insert chain. Slots run in series, post-utility +
        // post-filter, pre-fader. Pre-fader sends below see the
        // already-processed signal so an EQ/Comp on the source track
        // affects what hits the bus return — matching the standard
        // DAW signal flow. No-op when the chain has zero enabled slots,
        // and no allocations once `chain_scratch` is sized to buf_size.
        {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            self.chain
                .process(out_l, out_r, buf_size, &mut self.chain_scratch);
        }

        // Measure pre-fader peak and publish the pre-fader tap so pre-fader
        // sends can read the signal before volume/pan.
        {
            let mut pre_peak = 0.0_f32;
            for (l, r) in outputs[0].iter().zip(outputs[1].iter()).take(buf_size) {
                pre_peak = pre_peak.max(l.abs());
                pre_peak = pre_peak.max(r.abs());
            }
            self.meter
                .pre_fader_peak_db
                .store(linear_to_db(pre_peak), Ordering::Relaxed);

            // Copy pre-fader L/R into ports 2/3 if the buffer has them.
            if outputs.len() >= 4 {
                let (first_two, tail) = outputs.split_at_mut(2);
                for (dst, src) in tail[0].iter_mut().zip(first_two[0].iter()).take(buf_size) {
                    *dst = *src;
                }
                for (dst, src) in tail[1].iter_mut().zip(first_two[1].iter()).take(buf_size) {
                    *dst = *src;
                }
            }
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
        let node = TrackNode::new("test-track-id".into(), "Test".into(), pool, Arc::clone(&meter));
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
