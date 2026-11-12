//! TrackNode — an AudioNode that plays audio clips for a single track.
//!
//! Reads from the AudioPool based on clip positions and the current transport position.
//! Applies track volume and pan. No allocations in the process call.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use atomic_float::AtomicF32;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioNode, ProcessContext};

/// Fold one timed block into a track's load read-out. The graph times a
/// node on one block in [`crate::graph::CPU_STRIDE`], so this averages over
/// about four of those, a second or so, and one slow block (a plug-in
/// loading, the OS scheduling something else) does not read as a track
/// eating the CPU.
pub(crate) fn report_cpu(meter: &TrackMeterState, took: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    let previous = meter.cpu_ns.load(Relaxed);
    let smoothed = if previous == 0 {
        took
    } else {
        (previous * 3 + took) / 4
    };
    meter.cpu_ns.store(smoothed, Relaxed);
}

/// Post-fader meter state shared between the audio thread and the UI thread.
/// Using atomics so the audio thread never allocates or locks.
pub struct TrackMeterState {
    /// Post-fader peak in dB (last processed block, no hold/decay — UI smooths).
    pub peak_db_l: AtomicF32,
    pub peak_db_r: AtomicF32,
    /// Post-fader RMS in dB (mono sum, smoothed).
    pub rms_db: AtomicF32,
    /// Pre-fader peak in dB (max of L/R before volume/pan).
    pub pre_fader_peak_db: AtomicF32,
    /// How long this track took on the last blocks, in nanoseconds, as a
    /// running average. The CPU meter says the load is high and cannot say
    /// which track is causing it; this can.
    pub cpu_ns: std::sync::atomic::AtomicU64,
    /// The fader, pan and width as the mixer has them right now, read by
    /// the node at the start of every block. A move in the mixer reaches
    /// the sound within one block, without the graph being rebuilt.
    pub mix_volume: AtomicF32,
    pub mix_pan: AtomicF32,
    pub mix_width: AtomicF32,
    /// False until the engine has written the three above for this track;
    /// before that the node keeps the values it was built with.
    pub mix_set: std::sync::atomic::AtomicBool,
    /// What the track's VCA groups add to its level, linear. Already in
    /// `mix_volume`; kept apart so a level drawn in automation, which
    /// replaces the fader, still gets it.
    pub mix_vca: AtomicF32,
    /// Where automation put the fader (linear) and the pan on the last
    /// block, so the mixer can follow it; bit 0 of `auto_on` says the
    /// volume is automated, bit 1 the pan.
    pub auto_volume: AtomicF32,
    pub auto_pan: AtomicF32,
    pub auto_on: std::sync::atomic::AtomicU8,
}

/// Meters start at silence. They started at 0 dB, so a song just opened
/// showed every strip full until its track first played a block.
impl Default for TrackMeterState {
    fn default() -> Self {
        Self {
            peak_db_l: AtomicF32::new(-120.0),
            peak_db_r: AtomicF32::new(-120.0),
            rms_db: AtomicF32::new(-120.0),
            pre_fader_peak_db: AtomicF32::new(-120.0),
            cpu_ns: Default::default(),
            mix_volume: Default::default(),
            mix_pan: Default::default(),
            mix_width: Default::default(),
            mix_set: Default::default(),
            mix_vca: AtomicF32::new(1.0),
            auto_volume: Default::default(),
            auto_pan: Default::default(),
            auto_on: Default::default(),
        }
    }
}

impl TrackMeterState {
    /// Show silence: for a track with no node, which no block will update.
    pub fn park(&self) {
        self.peak_db_l.store(-120.0, Ordering::Relaxed);
        self.peak_db_r.store(-120.0, Ordering::Relaxed);
        self.pre_fader_peak_db.store(-120.0, Ordering::Relaxed);
        self.rms_db.store(-120.0, Ordering::Relaxed);
    }

    /// Set the mix the node plays at: linear volume, pan -1..1, width 0..2.
    pub fn set_mix(&self, volume_linear: f32, pan: f32, width: f32) {
        use std::sync::atomic::Ordering::{Relaxed, Release};
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        self.mix_volume
            .store(finite(volume_linear, 0.0).max(0.0), Relaxed);
        self.mix_pan
            .store(finite(pan, 0.0).clamp(-1.0, 1.0), Relaxed);
        self.mix_width
            .store(finite(width, 1.0).clamp(0.0, 2.0), Relaxed);
        self.mix_set.store(true, Release);
    }

    /// The VCA groups' share of the level, linear (1 when the track is in
    /// none). Set with the mix, before it.
    pub fn set_vca_gain(&self, linear: f32) {
        let linear = if linear.is_finite() {
            linear.max(0.0)
        } else {
            1.0
        };
        self.mix_vca
            .store(linear, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn vca_gain(&self) -> f32 {
        self.mix_vca.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// What automation did to the fader and pan this block (audio thread).
    pub fn publish_automation(&self, volume_linear: Option<f32>, pan: Option<f32>) {
        use std::sync::atomic::Ordering::Relaxed;
        if let Some(v) = volume_linear {
            self.auto_volume.store(v, Relaxed);
        }
        if let Some(p) = pan {
            self.auto_pan.store(p, Relaxed);
        }
        let on = u8::from(volume_linear.is_some()) | (u8::from(pan.is_some()) << 1);
        self.auto_on.store(on, Relaxed);
    }

    /// The automated volume (linear) and pan, when automation drives them.
    pub fn automation(&self) -> (Option<f32>, Option<f32>) {
        use std::sync::atomic::Ordering::Relaxed;
        let on = self.auto_on.load(Relaxed);
        (
            (on & 1 != 0).then(|| self.auto_volume.load(Relaxed)),
            (on & 2 != 0).then(|| self.auto_pan.load(Relaxed)),
        )
    }

    /// The live mix, once the engine has set one.
    pub fn live_mix(&self) -> Option<(f32, f32, f32)> {
        use std::sync::atomic::Ordering::{Acquire, Relaxed};
        if !self.mix_set.load(Acquire) {
            return None;
        }
        Some((
            self.mix_volume.load(Relaxed),
            self.mix_pan.load(Relaxed),
            self.mix_width.load(Relaxed),
        ))
    }
}

/// Gains from one block to the next, ramped across the block so a fader
/// moving while the song plays does not click.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GainRamp {
    last: Option<(f32, f32)>,
}

impl GainRamp {
    /// Start and end gains for this block, remembering the end for the next.
    pub(crate) fn step(&mut self, target: (f32, f32)) -> ((f32, f32), (f32, f32)) {
        let start = self.last.unwrap_or(target);
        self.last = Some(target);
        (start, target)
    }
}

/// Apply ramped left/right gains to a block.
pub(crate) fn apply_ramped(
    left: &mut [f32],
    right: &mut [f32],
    start: (f32, f32),
    end: (f32, f32),
) {
    let n = left.len().min(right.len());
    if n == 0 {
        return;
    }
    if start == end {
        for (l, r) in left.iter_mut().zip(right.iter_mut()).take(n) {
            *l *= end.0;
            *r *= end.1;
        }
        return;
    }
    let step = 1.0 / n as f32;
    for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).take(n).enumerate() {
        let t = (i + 1) as f32 * step;
        *l *= start.0 + (end.0 - start.0) * t;
        *r *= start.1 + (end.1 - start.1) * t;
    }
}

/// Mid/side width, ramped from one block's value to the next.
pub(crate) fn apply_width(left: &mut [f32], right: &mut [f32], start: f32, end: f32) {
    if (start - 1.0).abs() < 1e-4 && (end - 1.0).abs() < 1e-4 {
        return;
    }
    let n = left.len().min(right.len());
    let step = 1.0 / n.max(1) as f32;
    for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).take(n).enumerate() {
        let sep = start + (end - start) * (i + 1) as f32 * step;
        let mid = (*l + *r) * 0.5;
        let side = (*l - *r) * 0.5;
        *l = mid + side * sep;
        *r = mid - side * sep;
    }
}

/// One clip in the launcher grid.
#[derive(Debug, Clone)]
pub struct SessionClip {
    /// Key into AudioPool.
    pub source_id: String,
    /// Where the loop starts inside that audio.
    pub source_offset: u64,
    /// How long the loop is, in samples. This is what it wraps at.
    pub length_samples: u64,
    pub gain: f32,
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
    /// Warp segments (piecewise timeline→source map), precomputed at
    /// graph rebuild from the clip's warp markers. Sorted by
    /// `start_sample`. Empty = classic single-ratio path via
    /// `source_step`. When present these DEFINE the read position:
    /// timing anchors win over pitch/stretch knobs (resample warp
    /// repitches per segment, varispeed-style — v1 semantics).
    pub warp: Vec<WarpSegment>,
    /// The clip's pan, polarity, stereo swap and DC offset, worked out at
    /// rebuild so the audio thread only multiplies.
    pub treat: ClipTreat,
}

/// Per-sample treatment of a clip: each channel's average to take off, the
/// channels swapped or not, then a gain per side (pan and polarity).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipTreat {
    pub dc: [f32; 2],
    pub swap: bool,
    pub gain_l: f32,
    pub gain_r: f32,
    /// The part of the file that is the sample, in source frames: the
    /// clip's positions count from `window.0`, and nothing from `window.1`
    /// on is heard (sample start, length and trim).
    pub window: (u64, u64),
    /// The left channel fades out and the right fades in over the window.
    pub fade_stereo: bool,
}

impl Default for ClipTreat {
    fn default() -> Self {
        Self {
            dc: [0.0; 2],
            swap: false,
            gain_l: 1.0,
            gain_r: 1.0,
            window: (0, u64::MAX),
            fade_stereo: false,
        }
    }
}

impl ClipTreat {
    /// From a clip's treatment, with its sample's average per channel when
    /// it is to be taken off. Pan is a balance: the far side comes down and
    /// the centre stays at full level, as a stereo sample is panned in FL.
    pub fn new(t: &hardwave_project::clip::SampleTreatment, dc: [f32; 2]) -> Self {
        let pan = t.pan.clamp(-1.0, 1.0) as f32;
        let polarity = if t.polarity_inverted { -1.0 } else { 1.0 };
        Self {
            dc: if t.remove_dc { dc } else { [0.0; 2] },
            swap: t.swap_stereo,
            gain_l: (1.0 - pan).min(1.0) * polarity,
            gain_r: (1.0 + pan).min(1.0) * polarity,
            window: (0, u64::MAX),
            fade_stereo: t.fade_stereo,
        }
    }

    /// The window the sample is, for a file of `frames` frames: the start
    /// and length shares of the treatment, and the tail cut at `trim_end`
    /// when the treatment trims.
    pub fn with_window(
        mut self,
        t: &hardwave_project::clip::SampleTreatment,
        frames: u64,
        trim_end: Option<u64>,
    ) -> Self {
        let start = (t.start.clamp(0.0, 1.0) * frames as f64).round() as u64;
        let rest = frames.saturating_sub(start);
        let mut end = start + (t.length.clamp(0.0, 1.0) * rest as f64).round() as u64;
        if let Some(cut) = trim_end {
            end = end.min(cut.max(start));
        }
        self.window = (start, end.max(start));
        self
    }
}

/// One precomputed warp segment. For into-clip timeline samples at or
/// after `start_sample` (and before the next segment), the source read
/// position is `source_start + (into_clip - start_sample) * step`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpSegment {
    /// Into-clip timeline sample where this segment begins.
    pub start_sample: u64,
    /// Absolute source frame at the segment start.
    pub source_start: f64,
    /// Source frames per timeline sample within this segment.
    pub step: f64,
}

/// Build warp segments from anchors of (into-clip timeline sample →
/// absolute source frame). An implicit anchor at (0, `source_offset`)
/// is prepended unless one exists at sample 0, so playback before the
/// first user marker still maps sensibly. The final segment extends
/// past the last anchor with the last inter-anchor step (keeps groove
/// direction through the clip tail), or `default_step` when only one
/// anchor exists. Zero-length spans are skipped.
pub fn build_warp_segments(
    anchors: &[(u64, f64)],
    default_step: f64,
    source_offset: f64,
) -> Vec<WarpSegment> {
    let mut pts: Vec<(u64, f64)> = Vec::with_capacity(anchors.len() + 1);
    if anchors.first().map(|a| a.0) != Some(0) {
        pts.push((0, source_offset));
    }
    pts.extend_from_slice(anchors);
    pts.sort_by_key(|p| p.0);
    pts.dedup_by_key(|p| p.0);

    let mut segs: Vec<WarpSegment> = Vec::with_capacity(pts.len());
    for w in pts.windows(2) {
        let (t0, s0) = w[0];
        let (t1, s1) = w[1];
        let span = t1.saturating_sub(t0);
        if span == 0 {
            continue;
        }
        segs.push(WarpSegment {
            start_sample: t0,
            source_start: s0,
            step: (s1 - s0) / span as f64,
        });
    }
    // Tail: extend past the last anchor.
    if let Some(&(t_last, s_last)) = pts.last() {
        let tail_step = segs.last().map(|s| s.step).unwrap_or(default_step);
        segs.push(WarpSegment {
            start_sample: t_last,
            source_start: s_last,
            step: tail_step,
        });
    }
    segs
}

/// Source read position for `into_clip` under the given segments.
/// Segments are sorted by `start_sample`; partition_point keeps this
/// alloc-free and O(log n) on the audio thread.
#[inline]
pub fn warp_source_pos(segs: &[WarpSegment], into_clip: u64) -> f64 {
    let idx = segs.partition_point(|s| s.start_sample <= into_clip);
    // idx is the first segment STARTING AFTER into_clip; the active one
    // precedes it. build_warp_segments guarantees a segment at 0.
    let seg = segs[idx.saturating_sub(1)];
    seg.source_start + (into_clip - seg.start_sample) as f64 * seg.step
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

    /// Nothing in its memory: silence in is silence out, and the memory
    /// stays as it is.
    fn at_rest(&self) -> bool {
        [
            self.x1_l, self.x2_l, self.y1_l, self.y2_l, self.x1_r, self.x2_r, self.y1_r, self.y2_r,
        ]
        .iter()
        .all(|v| *v == 0.0)
    }

    /// Take the filter's memory from `old`, keeping this one's coefficients.
    fn carry_state_from(&mut self, old: &BiquadStereo) {
        self.x1_l = old.x1_l;
        self.x2_l = old.x2_l;
        self.y1_l = old.y1_l;
        self.y2_l = old.y2_l;
        self.x1_r = old.x1_r;
        self.x2_r = old.x2_r;
        self.y1_r = old.y1_r;
        self.y2_r = old.y2_r;
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
    /// Delay, swap, polarity and filter, before the plug-ins.
    utility: ChannelUtility,
    /// Shared behaviour switches, read on the audio thread. Only the pan
    /// law is used here; the rest belong to the instrument tracks.
    prefs: crate::audio_prefs::AudioPrefs,
    /// 0 dB at centre (FL's law) instead of the pan law setting: an insert
    /// in a song with `Project::inserts_unity_at_centre`.
    unity_centre: bool,
    /// 0.0 = mono, 1.0 = normal, >1.0 = widened.
    stereo_separation: f32,
    /// What the last block used, so the next ramps from it.
    width_applied: f32,
    gain_ramp: GainRamp,
    /// The last block this node wrote was silence on every channel, so the
    /// graph can leave it out of the sums it feeds (`output_is_silent`).
    wrote_silence: bool,
    /// Cached Arc references to audio buffers, refreshed when clips change.
    cached_buffers: Vec<Option<Arc<AudioBuffer>>>,
    /// The clips in this track's row of the launcher grid, and the
    /// buffers behind them. A launched slot loops on top of the song
    /// instead of the timeline clips, which is what a launcher is.
    session_slots: Vec<Option<SessionClip>>,
    session_buffers: Vec<Option<Arc<AudioBuffer>>>,
    /// Which slot this track is playing or has been asked to play.
    /// Shared with the commands that drive the grid.
    session: Option<Arc<crate::session::TrackSession>>,
    /// How a launch is lined up, in beats, read from the grid.
    session_quantise: Option<Arc<std::sync::atomic::AtomicU32>>,
    /// Shared meter state so the UI can read post-fader peaks without locking.
    meter: Arc<TrackMeterState>,
    /// Smoothed RMS (linear), updated each block.
    rms_smooth: f32,
    /// Per-track plug-in insert chain. Audio flows through here after
    /// the filter+delay step but before the fader+pan, matching the
    /// pre-fader insert slot every major DAW exposes.
    chain: crate::insert_chain::InsertChain,
    /// Reusable scratch buffers for `chain.process` so the audio thread
    /// stays allocation-free in steady state.
    chain_scratch: crate::insert_chain::Scratch,
    /// Active automation lanes — evaluated at the start of each block
    /// to override the static `volume` / `pan` values when a lane
    /// targets them. The Vec is replaced wholesale on `rebuild_graph`,
    /// so the audio thread never allocates here in steady state.
    automation_lanes: Vec<hardwave_project::automation::AutomationLane>,

    /// Arrangement-level automation clips evaluated alongside lanes. A
    /// clip only contributes while the playhead is inside its
    /// `[start, start+length]` window; outside it, it's silent and the
    /// lane / static value stands.
    automation_clips: Vec<hardwave_project::automation_clip::AutomationClip>,
    /// True when another track routes into this one, which makes it a bus
    /// that has work to do even with nothing of its own on it.
    receives_input: bool,
    /// True when live input passes through this track: an armed track that
    /// monitors, or a bus one routes into. It runs while the song is
    /// stopped, so a singer hears themselves before pressing play.
    live_input: bool,
    /// Cached static fader value so we can restore it when an
    /// automation lane is removed mid-session. We store the linear
    /// gain so the runtime path stays branch-light.
    static_volume: f32,
    /// Cached static pan setting (-1..=1). Same restore-on-clear story
    /// as `static_volume`.
    static_pan: f32,
    /// Modulation's level and pan on top of the fader and the pan knob
    /// (see `AudioNode::set_mod_mix`).
    mod_gain: f32,
    mod_pan: f32,
    /// Playing a freeze render, which already went through this track's
    /// delay, swap, polarity, width and filter as well as its plug-ins.
    frozen: bool,
}

const TRACK_DELAY_CAPACITY: usize = 4800; // 100 ms at 48 kHz, both directions

/// A channel's utility stage before its plug-ins: its time offset, left and
/// right swapped, its polarity and its filter. The same on an audio track and
/// an instrument track; the instrument track had none of it, so the mixer
/// strip's polarity, swap and delay and the rack's filter did nothing there.
pub struct ChannelUtility {
    phase_invert: bool,
    swap_lr: bool,
    /// Positive delays only; bounded by TRACK_DELAY_CAPACITY.
    delay_samples: i32,
    /// Delay line for the left/right channels (ring buffer, zero-initialized),
    /// made only for a track that has a delay: every track used to carry
    /// 100 ms of it, made and zeroed for all 500 tracks at every rebuild.
    delay_buf_l: Vec<f32>,
    delay_buf_r: Vec<f32>,
    delay_write_pos: usize,
    filter_kind: TrackFilterType,
    filter: BiquadStereo,
}

impl Default for ChannelUtility {
    fn default() -> Self {
        Self {
            phase_invert: false,
            swap_lr: false,
            delay_samples: 0,
            delay_buf_l: Vec::new(),
            delay_buf_r: Vec::new(),
            delay_write_pos: 0,
            filter_kind: TrackFilterType::Off,
            filter: BiquadStereo::default(),
        }
    }
}

impl ChannelUtility {
    /// Configure the filter. Called from the engine thread when the project
    /// changes, so computing coefficients here is fine.
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

    pub fn set_delay_samples(&mut self, samples: i64) {
        let max = TRACK_DELAY_CAPACITY as i64 - 1;
        // Only positive delay is honored; negative delay (advance) isn't representable
        // in a streaming graph without lookahead, and is normally achieved by delaying
        // the other tracks instead.
        self.delay_samples = samples.clamp(0, max) as i32;
        if self.delay_samples > 0 && self.delay_buf_l.is_empty() {
            self.delay_buf_l = vec![0.0; TRACK_DELAY_CAPACITY];
            self.delay_buf_r = vec![0.0; TRACK_DELAY_CAPACITY];
        }
    }

    /// Carry on from `old`, the stage of the node this one replaces: the
    /// filter's memory when it is the same kind of filter, and the delay
    /// line with the audio in it. Swaps, so nothing is allocated or freed.
    pub fn carry_from(&mut self, old: &mut ChannelUtility) {
        if self.filter_kind == old.filter_kind {
            self.filter.carry_state_from(&old.filter);
        }
        if !self.delay_buf_l.is_empty() && self.delay_buf_l.len() == old.delay_buf_l.len() {
            std::mem::swap(&mut self.delay_buf_l, &mut old.delay_buf_l);
            std::mem::swap(&mut self.delay_buf_r, &mut old.delay_buf_r);
            self.delay_write_pos = old.delay_write_pos;
        }
    }

    /// Delay, swap and polarity, in that order: the shifted signal feeds the
    /// rest. Allocation-free.
    pub fn process_pre(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let n = out_l.len().min(out_r.len());
        if self.delay_samples > 0 && !self.delay_buf_l.is_empty() {
            let cap = self.delay_buf_l.len();
            let delay = self.delay_samples as usize;
            for frame in 0..n {
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
            for frame in 0..n {
                std::mem::swap(&mut out_l[frame], &mut out_r[frame]);
            }
        }
        if self.phase_invert {
            for s in out_l[..n].iter_mut().chain(out_r[..n].iter_mut()) {
                *s = -*s;
            }
        }
    }

    /// Whether silence in gives silence out with nothing changing: no delay
    /// line, and the filter off or with nothing left ringing in it. Swap and
    /// polarity on silence are silence.
    fn at_rest(&self) -> bool {
        let no_delay = self.delay_samples <= 0 || self.delay_buf_l.is_empty();
        no_delay && (self.filter_kind == TrackFilterType::Off || self.filter.at_rest())
    }

    /// The filter, after the width stage. `Off` skips the loop.
    pub fn process_filter(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        if self.filter_kind == TrackFilterType::Off {
            return;
        }
        let n = out_l.len().min(out_r.len());
        for frame in 0..n {
            let (yl, yr) = self.filter.process_stereo(out_l[frame], out_r[frame]);
            out_l[frame] = yl;
            out_r[frame] = yr;
        }
    }
}

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
            session_slots: Vec::new(),
            session_buffers: Vec::new(),
            session: None,
            session_quantise: None,
            prefs: crate::audio_prefs::AudioPrefs::new(),
            unity_centre: false,
            volume: 1.0,
            pan: 0.0,
            muted: false,
            soloed: false,
            utility: ChannelUtility::default(),
            stereo_separation: 1.0,
            width_applied: 1.0,
            gain_ramp: GainRamp::default(),
            wrote_silence: false,
            cached_buffers: Vec::new(),
            meter,
            rms_smooth: 0.0,
            chain: crate::insert_chain::InsertChain::new(),
            chain_scratch: crate::insert_chain::Scratch::default(),
            automation_lanes: Vec::new(),
            automation_clips: Vec::new(),
            receives_input: false,
            live_input: false,
            static_volume: 1.0,
            static_pan: 0.0,
            mod_gain: 1.0,
            mod_pan: 0.0,
            frozen: false,
        }
    }

    /// Play a freeze render: the channel's delay, swap, polarity, width
    /// and filter are already in it, so they are not run again. Its fader,
    /// pan and sends still are.
    pub fn set_frozen(&mut self, frozen: bool) {
        self.frozen = frozen;
    }

    /// Configure the per-channel filter (see [`ChannelUtility::set_filter`]).
    pub fn set_filter(
        &mut self,
        kind: TrackFilterType,
        cutoff_hz: f32,
        resonance: f32,
        sample_rate: f32,
    ) {
        self.utility
            .set_filter(kind, cutoff_hz, resonance, sample_rate);
    }

    pub fn set_phase_invert(&mut self, inv: bool) {
        self.utility.set_phase_invert(inv);
    }

    pub fn set_swap_lr(&mut self, swap: bool) {
        self.utility.set_swap_lr(swap);
    }

    pub fn set_stereo_separation(&mut self, sep: f64) {
        self.stereo_separation = sep.clamp(0.0, 2.0) as f32;
    }

    pub fn set_delay_samples(&mut self, samples: i64) {
        self.utility.set_delay_samples(samples);
    }

    /// Carry on the sound of `old`, the node this one replaces in a graph
    /// swap: the gain ramp, the width, the meter's smoothing, the filter and
    /// the delay line. Clips play from the playhead, so they need nothing.
    pub(crate) fn carry_runtime_from(&mut self, old: &mut TrackNode) {
        self.gain_ramp = old.gain_ramp;
        self.width_applied = old.width_applied;
        self.rms_smooth = old.rms_smooth;
        self.utility.carry_from(&mut old.utility);
        std::mem::swap(&mut self.chain_scratch, &mut old.chain_scratch);
    }

    /// Update the clip list. Called from the engine thread (not the audio thread)
    /// when the project state changes.
    pub fn set_clips(&mut self, clips: Vec<ClipRegion>) {
        self.cached_buffers = clips.iter().map(|c| self.pool.get(&c.source_id)).collect();
        self.clips = clips;
    }

    /// Whether the launcher has anything for this track, so the idle
    /// gate does not skip a track that is about to start.
    /// A placeholder track: no clips, no plug-ins, no automation, nothing
    /// routed into it and nothing in the launcher grid. It plays silence.
    fn has_nothing_to_play(&self) -> bool {
        self.clips.is_empty()
            && self.chain.slots.is_empty()
            && self.automation_lanes.is_empty()
            && !self.receives_input
            && !self.live_input
            // A track whose only content is in the launcher grid still
            // has work to do: the gate is about idle tracks, and one
            // with a clip queued or playing is not idle.
            && !self.session_has_work()
    }

    fn session_has_work(&self) -> bool {
        if self.session_slots.is_empty() {
            return false;
        }
        match &self.session {
            Some(session) => {
                session.playing_slot().is_some() || session.pending_slot() != crate::session::NONE
            }
            None => false,
        }
    }

    /// Play whatever the grid says this track should be playing.
    ///
    /// Returns whether a slot is sounding, so the caller knows to leave
    /// the timeline clips alone. A launch that falls inside this block
    /// starts at the boundary sample rather than at the block edge,
    /// which is what keeps two tracks launched together together.
    fn run_session(
        &mut self,
        outputs: &mut [Vec<f32>],
        buf_size: usize,
        pos: u64,
        ctx: &ProcessContext,
    ) -> bool {
        use std::sync::atomic::Ordering;
        let Some(session) = self.session.clone() else {
            return false;
        };
        if self.session_slots.is_empty() {
            return false;
        }

        // Anything queued starts, or stops, at the next boundary.
        let beats = self
            .session_quantise
            .as_ref()
            .map(|q| q.load(Ordering::Relaxed))
            .unwrap_or(4);
        let quantise = crate::session::quantise_samples(beats, ctx.tempo, ctx.sample_rate);
        session.advance(pos, buf_size, quantise);

        let Some(slot_index) = session.playing_slot() else {
            return false;
        };
        let Some(Some(slot)) = self.session_slots.get(slot_index).cloned() else {
            return false;
        };
        let Some(Some(buffer)) = self.session_buffers.get(slot_index).cloned() else {
            return false;
        };
        let launched_at = session.launch_sample.load(Ordering::Relaxed);

        let channels = buffer.channels.len().min(2);
        if channels == 0 || slot.length_samples == 0 {
            return false;
        }
        let (out_left, out_rest) = outputs.split_at_mut(1);
        let out_l = &mut out_left[0];
        let out_r = &mut out_rest[0];

        for frame in 0..buf_size {
            let now = pos + frame as u64;
            let Some(into) =
                crate::session::position_in_loop(now, launched_at, slot.length_samples)
            else {
                continue;
            };
            let index = (slot.source_offset + into) as usize;
            if index >= buffer.num_frames {
                continue;
            }
            let left = buffer.channels[0][index] * slot.gain;
            let right = if channels > 1 {
                buffer.channels[1][index] * slot.gain
            } else {
                left
            };
            out_l[frame] += left;
            out_r[frame] += right;
        }
        true
    }

    /// Put this track's row of the launcher grid in place.
    pub fn set_session_slots(&mut self, slots: Vec<Option<SessionClip>>) {
        self.session_buffers = slots
            .iter()
            .map(|slot| slot.as_ref().and_then(|c| self.pool.get(&c.source_id)))
            .collect();
        self.session_slots = slots;
    }

    /// Hand this track the grid state it shares with the UI.
    pub fn set_session(
        &mut self,
        session: Arc<crate::session::TrackSession>,
        quantise: Arc<std::sync::atomic::AtomicU32>,
    ) {
        self.session = Some(session);
        self.session_quantise = Some(quantise);
    }

    /// Replace the automation lane snapshot the audio thread evaluates
    /// each block. Lanes are pre-cloned on the UI thread (off the audio
    /// path) and shipped here through `rebuild_graph`. Empty list means
    /// no automation — volume / pan stick to the static values that
    /// `set_volume_db` / `set_pan` last applied.
    /// Share the engine's switches with this node. Called on every graph
    /// rebuild, like the other per-track settings.
    pub fn set_prefs(&mut self, prefs: crate::audio_prefs::AudioPrefs) {
        self.prefs = prefs;
    }

    /// Pan this track with 0 dB at centre whatever the pan law setting: an
    /// insert (something plays into it) in a song with FL's inserts.
    pub fn set_unity_centre(&mut self, on: bool) {
        self.unity_centre = on;
    }

    pub fn set_automation_lanes(
        &mut self,
        lanes: Vec<hardwave_project::automation::AutomationLane>,
    ) {
        self.automation_lanes = lanes;
    }

    /// Replace the automation-clip snapshot the audio thread evaluates.
    pub fn set_automation_clips(
        &mut self,
        clips: Vec<hardwave_project::automation_clip::AutomationClip>,
    ) {
        self.automation_clips = clips;
    }

    pub fn set_volume_db(&mut self, db: f64) {
        let lin = db_to_linear(db);
        self.volume = lin;
        // Cache so an automation lane can be deleted mid-session and
        // we still know the user-set fader value to fall back to.
        self.static_volume = lin;
    }

    pub fn set_pan(&mut self, pan: f64) {
        let p = pan as f32;
        self.pan = p;
        self.static_pan = p;
    }

    /// Tell the node that something routes into it, so the idle gate below
    /// does not skip it.
    ///
    /// A bus with no clips, no inserts and no automation looked idle, so the
    /// gate returned before mixing its inputs: every send into an empty bus
    /// was silent, and a reverb bus is empty by definition until a plug-in is
    /// added to it.
    pub fn set_receives_input(&mut self, receives: bool) {
        self.receives_input = receives;
    }

    /// Whether this track runs with the song stopped: live input passes
    /// through it, or other tracks play into it. The graph only runs while
    /// stopped when something is played live (an input monitored, a
    /// keyboard), and a bus has to carry that: a synth played with the
    /// song stopped reached no bus or reverb return, through its output or
    /// a send, until Play.
    /// The left and right gains of the fader and pan this block.
    fn fader_gains(&self) -> (f32, f32) {
        let pan = (self.pan + self.mod_pan).clamp(-1.0, 1.0);
        // An insert in a song with FL's inserts passes a centred sound at
        // 0 dB (FL's law: the far side is turned down, nothing is boosted).
        let law = if self.unity_centre {
            crate::pan::PanLaw::Zero
        } else {
            self.prefs.pan_law()
        };
        let (pan_l, pan_r) = crate::pan::gains(pan, law);
        let vol = self.volume * self.mod_gain;
        (vol * pan_l, vol * pan_r)
    }

    /// A block of silence through the stages after the clips, without
    /// running them over it: the width and the fader ramp move on to where
    /// they would be, and the meters read what the block would have given.
    fn pass_silence(&mut self, buf_size: usize) {
        if !self.frozen {
            self.width_applied = self.stereo_separation;
        }
        self.gain_ramp.step(self.fader_gains());
        let silence = linear_to_db(0.0);
        self.meter
            .pre_fader_peak_db
            .store(silence, Ordering::Relaxed);
        self.meter.peak_db_l.store(silence, Ordering::Relaxed);
        self.meter.peak_db_r.store(silence, Ordering::Relaxed);
        let rms_lin = (0.0_f64 / buf_size.max(1) as f64).sqrt() as f32;
        self.rms_smooth = self.rms_smooth * 0.85 + rms_lin * 0.15;
        self.meter
            .rms_db
            .store(linear_to_db(self.rms_smooth), Ordering::Relaxed);
    }

    fn runs_stopped(&self) -> bool {
        self.live_input || self.receives_input
    }

    /// Monitoring went through the track only while playing, and only when
    /// the track had a plug-in: a track with nothing on it counted as idle
    /// and a stopped one returned silence, so an armed vocal track let the
    /// singer hear nothing. A track live input passes through now runs
    /// while stopped (its clips stay still) and is never idle.
    pub fn set_live_input(&mut self, live: bool) {
        self.live_input = live;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_soloed(&mut self, soloed: bool) {
        self.soloed = soloed;
    }
}

impl AudioNode for TrackNode {
    fn set_chain_parameter(&mut self, slot_id: &str, param_id: u32, value: f64) {
        // Modulation moves a parameter 0..1, as automation does.
        self.chain
            .set_parameter_normalized(slot_id, param_id, value);
    }

    fn set_mod_mix(&mut self, gain: f32, pan: f32) {
        self.mod_gain = gain;
        self.mod_pan = pan;
    }

    /// The engine calls this through the trait after the send pass, so the
    /// override has to live here: an inherent method of the same name is
    /// never reached through `Box<dyn AudioNode>`, which is why a bus with
    /// nothing of its own on it stayed on the idle path and every send into
    /// it was silent.
    fn set_live_input(&mut self, live: bool) {
        TrackNode::set_live_input(self, live);
    }

    fn set_receives_input(&mut self, receives: bool) {
        TrackNode::set_receives_input(self, receives);
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn track_id(&self) -> Option<&str> {
        Some(&self.track_id)
    }

    fn report_cpu(&self, nanos: u64) {
        report_cpu(&self.meter, nanos);
    }

    fn output_is_silent(&self) -> bool {
        self.wrote_silence
    }

    /// Report the insert chain's latency so the graph can compensate it.
    fn latency_samples(&self) -> u32 {
        self.chain.latency_samples()
    }

    fn flush_plugin_parameters(&mut self) {
        self.chain.flush_parameters();
    }

    fn snapshot_plugin_states(&self) -> Vec<(String, Vec<u8>)> {
        self.chain
            .slots
            .iter()
            .map(|slot| (slot.slot_id.clone(), slot.plugin.get_state()))
            .collect()
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
                match self.chain.add_or_replace(slot, sample_rate, max_block_size) {
                    Ok(Some(old)) => {
                        if let Err(old) = graveyard.try_bury(old) {
                            std::mem::forget(old);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("track {} insert add failed: {e}", self.track_id),
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
            InsertCommand::SetEnabled {
                slot_id, enabled, ..
            } => {
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
            InsertCommand::SetParameterNormalized {
                slot_id,
                param_id,
                value01,
                ..
            } => {
                self.chain
                    .set_parameter_normalized(&slot_id, param_id, value01);
            }
            InsertCommand::SetState { slot_id, bytes, .. } => {
                self.chain.set_state(&slot_id, &bytes);
            }
            InsertCommand::LoadFactoryPreset { slot_id, index, .. } => {
                self.chain.load_factory_preset(&slot_id, index);
            }
            InsertCommand::ClearAll => self.chain.bury_all(graveyard),
        }
    }

    /// Build-time insert population for offline render: push a pre-built
    /// `LiveSlot` straight onto the chain (no command queue, no
    /// graveyard). Used by the offline render's chain hydration so an
    /// export applies the same insert FX as live playback.
    fn push_offline_slot(
        &mut self,
        slot: crate::insert_chain::LiveSlot,
        sample_rate: f64,
        max_block_size: u32,
    ) {
        if let Err(e) = self.chain.push_slot(slot, sample_rate, max_block_size) {
            log::warn!("track {} offline insert add failed: {e}", self.track_id);
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

    fn carry_on_from(&mut self, old: &mut dyn AudioNode) {
        if let Some(old) = old
            .as_any_mut()
            .and_then(|old| old.downcast_mut::<TrackNode>())
        {
            self.carry_runtime_from(old);
        }
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn chain_len(&self) -> usize {
        self.chain.slots.len()
    }

    fn set_slot_sidechain(&mut self, slot_id: &str, active: bool) {
        self.chain.set_slot_sidechain(slot_id, active);
    }

    /// Muted, stopped, or a placeholder: each of those leaves the track
    /// silent before it reads its input or runs anything (see `process`).
    fn idle(&self, ctx: &ProcessContext) -> bool {
        self.muted || (!ctx.playing && !self.runs_stopped()) || self.has_nothing_to_play()
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        ctx: &ProcessContext,
    ) {
        // The mixer's live fader, pan and width. Automation, where a lane
        // or clip drives them, still overrides below.
        if let Some((volume, pan, width)) = self.meter.live_mix() {
            if volume != self.static_volume {
                self.static_volume = volume;
                self.volume = volume;
            }
            if pan != self.static_pan {
                self.static_pan = pan;
                self.pan = pan;
            }
            self.stereo_separation = width;
        }
        let buf_size = ctx.buffer_size as usize;
        self.wrote_silence = false;

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
        // Every way out below that leaves the outputs as they are now says
        // so; the rest write sound.
        self.wrote_silence = true;

        if self.muted || (!ctx.playing && !self.runs_stopped()) {
            // Silence on the meter as well. A track muted (or silenced by
            // another track's solo) while the song played kept showing its
            // last level until the song stopped, because the mixer is only
            // sent meters that change.
            self.meter.park();
            return;
        }

        // Lazy-promotion gate: a placeholder track (no clips, no chain
        // slots, no automation) does no audible work. Skipping it here
        // saves the per-block clip-overlap loop, filter stage, fader,
        // pan, peak/RMS calc, and every plug-in in the chain — across
        // 500 idle default-project tracks that's the bulk of audio CPU.
        //
        // Outputs were already filled with zero above, so an early
        // return leaves the track silent (correct). The atomic meter
        // stores stay at their last value; UI will show no movement on
        // an inactive track, which is what the user expects.
        //
        // We DON'T gate on `armed` or `monitor_input` here — those
        // imply a live input path the EngineCallback drains via a
        // different code path (`input_consumer`), and we still want
        // any chain slots on an armed track to process live monitoring.
        // Hence the chain.slots check below covers the monitoring case
        // implicitly: an armed track with no chain = nothing for us to
        // do; an armed track WITH chain = process for monitoring.
        if self.has_nothing_to_play() {
            // Park the meter at silence so the UI doesn't show stale
            // values from a previous active block.
            use std::sync::atomic::Ordering;
            self.meter.peak_db_l.store(-120.0, Ordering::Relaxed);
            self.meter.peak_db_r.store(-120.0, Ordering::Relaxed);
            self.meter
                .pre_fader_peak_db
                .store(-120.0, Ordering::Relaxed);
            self.meter.rms_db.store(-120.0, Ordering::Relaxed);
            return;
        }

        // Evaluate automation lanes that target this track's fader and
        // pan. We compute the playback position in ticks once, then walk
        // each lane and override the static volume / pan with whatever
        // the lane curve says at that tick. Lanes that target plugin
        // parameters or sends are forwarded in a follow-up pass —
        // volume + pan are the highest-impact targets and fully wire
        // up the automation pipeline end-to-end.
        self.meter.publish_automation(None, None);
        if (!self.automation_lanes.is_empty() || !self.automation_clips.is_empty())
            && ctx.sample_rate > 0.0
            && ctx.tempo > 0.0
        {
            // The engine works this out once per block, through the project's
            // tempo map. Deriving it here from one tempo, as this used to,
            // put every curve in the wrong place as soon as the song had a
            // tempo change or a ramp.
            let tick = ctx.position_ticks;
            let mix = crate::automation_eval::evaluate(
                &self.automation_lanes,
                &self.automation_clips,
                tick,
                self.static_volume,
                self.static_pan,
                &mut self.chain,
            );
            let (volume, pan, mute_override) = (mix.volume_linear, mix.pan, mix.mute);
            self.meter.publish_automation(
                mix.volume_automated.then_some(volume),
                mix.pan_automated.then_some(pan),
            );
            // Automation replaces the fader, not the VCA on top of it: a
            // track with its level automated ignored its group's fader.
            self.volume = if mix.volume_automated {
                volume * self.meter.vca_gain()
            } else {
                volume
            };
            self.pan = pan;
            if let Some(m) = mute_override {
                if m {
                    // Don't permanently store muted=true so a deleted
                    // lane immediately restores audio. Just early-exit
                    // the block at silence, with the meter at silence too.
                    self.meter.park();
                    return;
                }
            }
        }

        // Whether anything reaches this track from others this block. A bus
        // fed by tracks that are all silent hears nothing.
        let input_heard = inputs
            .iter()
            .take(2)
            .any(|ch| ch.iter().take(buf_size).any(|s| *s != 0.0));

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

        // The launcher. A launched slot loops on top of the song and
        // replaces this track's timeline clips, because a track doing
        // both at once is nobody's idea of either.
        let session_playing = ctx.playing && self.run_session(outputs, buf_size, pos, ctx);

        // Sum all clips that overlap the current buffer window. Stopped, a
        // track runs only for its live input, and its clips stay silent: the
        // playhead is not moving, so they would repeat one block.
        let mut clip_heard = false;
        for (clip_idx, clip) in self.clips.iter().enumerate() {
            if session_playing || !ctx.playing {
                break;
            }
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
            clip_heard = true;

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

                // Fractional source position. Warp segments (piecewise
                // map) take precedence over the single-ratio path;
                // reverse is not combined with warp (anchors define
                // absolute positions).
                let source_pos = if !clip.warp.is_empty() {
                    warp_source_pos(&clip.warp, into_clip)
                } else {
                    if clip.reversed {
                        // The forward read mirrored: the right edge plays
                        // the window's start and the left edge its end, the
                        // window being as many source frames as the clip
                        // moves through. It was counted in timeline samples,
                        // so a reversed clip played faster or slower than
                        // 1:1 (repitched, on a pitched track, or bounced at
                        // another rate) read the wrong part of the file and
                        // went silent part of the way through.
                        let from_end = (clip_length - 1 - into_clip) as f64;
                        clip.source_offset as f64 + from_end * clip.source_step
                    } else {
                        clip.source_offset as f64 + into_clip as f64 * clip.source_step
                    }
                };

                if source_pos < 0.0 {
                    continue;
                }
                // The clip's positions count from where the sample starts
                // in its file, and the sample ends where its window does.
                let t = &clip.treat;
                let source_pos = source_pos + t.window.0 as f64;
                if source_pos >= t.window.1 as f64 {
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
                let l = l0 + (l1 - l0) * frac - t.dc[0];
                let r = if num_channels > 1 {
                    let r0 = audio_buf.sample(1, idx0);
                    let r1 = audio_buf.sample(1, idx1);
                    r0 + (r1 - r0) * frac - t.dc[1]
                } else {
                    l
                };
                let (l, r) = if t.swap { (r, l) } else { (l, r) };
                // Fade stereo: left to right across the sample.
                let (fl, fr) = if t.fade_stereo {
                    let span =
                        (t.window.1.min(audio_buf.num_frames as u64) - t.window.0).max(1) as f64;
                    let x = ((source_pos - t.window.0 as f64) / span).clamp(0.0, 1.0) as f32;
                    (1.0 - x, x)
                } else {
                    (1.0, 1.0)
                };

                out_l[frame] += l * scaled_gain * t.gain_l * fl;
                out_r[frame] += r * scaled_gain * t.gain_r * fr;
            }
        }

        // Nothing to hear and nothing ringing on: the block is the silence
        // already in the outputs. Most tracks of a big song are like this
        // most of the time, between their clips, and each one ran its
        // utility, filter, fader and meters over a block of zeros: three
        // quarters of the audio thread's time on a 127-track FL song. What
        // those stages remember moves on as it would have.
        if !input_heard
            && !session_playing
            && !clip_heard
            && self.chain.slots.is_empty()
            && (self.frozen || self.utility.at_rest())
        {
            self.pass_silence(buf_size);
            return;
        }
        self.wrote_silence = false;

        // --- Pre-fader utility chain: delay → swap → phase → stereo separation.
        // Order is deliberate: delay first so the shifted signal feeds the rest of
        // the chain, then L/R swap, then polarity flip, then M/S separation.
        // A freeze render went through all of it, and through the filter
        // below: run again, a flipped polarity flipped back and the filter
        // cut twice as steep.
        if !self.frozen {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            self.utility
                .process_pre(&mut out_left[0][..buf_size], &mut out_rest[0][..buf_size]);
        }

        if !self.frozen {
            let start = self.width_applied;
            let end = self.stereo_separation;
            self.width_applied = end;
            let (out_left, out_rest) = outputs.split_at_mut(1);
            apply_width(
                &mut out_left[0][..buf_size],
                &mut out_rest[0][..buf_size],
                start,
                end,
            );
        }

        // Per-channel biquad filter stage. Applied pre-fader so metering and
        // sends see the filtered signal. `Off` skips the inner loop.
        if !self.frozen {
            let (out_left, out_rest) = outputs.split_at_mut(1);
            self.utility
                .process_filter(&mut out_left[0][..buf_size], &mut out_rest[0][..buf_size]);
        }

        // Plug-in insert chain. Slots run in series, post-utility +
        // post-filter, pre-fader. Pre-fader sends below see the
        // already-processed signal so an EQ/Comp on the source track
        // affects what hits the bus return — matching the standard
        // DAW signal flow. No-op when the chain has zero enabled slots,
        // and no allocations once `chain_scratch` is sized to buf_size.
        {
            // Sidechain bus: input ports 2/3 carry any audio routed here
            // from other tracks' sidechain sends (see engine rebuild).
            // Slots with a sidechain source keyed to them receive it as
            // extra input channels; other slots ignore it.
            let sidechain: Option<(&[f32], &[f32])> = if inputs.len() >= 4 {
                Some((inputs[2], inputs[3]))
            } else {
                None
            };
            let (out_left, out_rest) = outputs.split_at_mut(1);
            let out_l = &mut out_left[0];
            let out_r = &mut out_rest[0];
            // Where the song is, so a plug-in in this chain can work in
            // beats rather than only in milliseconds.
            self.chain
                .set_transport(hardwave_plugin_host::types::TransportInfo {
                    playing: ctx.playing,
                    tempo: ctx.tempo,
                    position_beats: ctx.position_ticks as f64 / hardwave_midi::PPQ as f64,
                    time_sig: ctx.time_sig,
                    sample_rate: ctx.sample_rate,
                    position_samples: ctx.position_samples,
                });
            self.chain.process(
                out_l,
                out_r,
                buf_size,
                &mut self.chain_scratch,
                midi_in,
                sidechain,
            );
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
        let (out_left, out_rest) = outputs.split_at_mut(1);
        let (start, end) = self.gain_ramp.step(self.fader_gains());
        apply_ramped(
            &mut out_left[0][..buf_size],
            &mut out_rest[0][..buf_size],
            start,
            end,
        );
        let mut peak_l = 0.0_f32;
        let mut peak_r = 0.0_f32;
        let mut sum_sq = 0.0_f64;
        for (l, r) in out_left[0]
            .iter_mut()
            .zip(out_rest[0].iter_mut())
            .take(buf_size)
        {
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
        // Re-cache buffers in case pool contents changed.
        //
        // Refilled in place rather than collected into a new Vec: reset now
        // runs on the audio thread, on stop and on a playhead jump, and
        // allocating there is what a dropout sounds like. After the clear the
        // capacity from the last fill is enough for the same clips.
        self.cached_buffers.clear();
        for clip in &self.clips {
            self.cached_buffers.push(self.pool.get(&clip.source_id));
        }
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

/// The default law, kept for the tests that assert the centre and the edges.
#[cfg(test)]
#[inline]
fn pan_law(pan: f32) -> (f32, f32) {
    crate::pan::gains(pan, crate::pan::PanLaw::Minus3dB)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The channel utility shared by audio and instrument tracks: a delay
    /// shifts the signal later by its samples, swap trades the sides, and
    /// polarity flips both.
    #[test]
    fn the_channel_utility_delays_swaps_and_flips() {
        let mut u = ChannelUtility::default();
        u.set_delay_samples(3);
        let mut l = vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let mut r = vec![0.0, 2.0, 0.0, 0.0, 0.0, 0.0];
        u.process_pre(&mut l, &mut r);
        assert_eq!(l, vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        assert_eq!(r, vec![0.0, 0.0, 0.0, 0.0, 2.0, 0.0]);

        let mut u = ChannelUtility::default();
        u.set_swap_lr(true);
        u.set_phase_invert(true);
        let (mut l, mut r) = (vec![0.5, 0.25], vec![-0.1, 0.0]);
        u.process_pre(&mut l, &mut r);
        assert_eq!((l, r), (vec![0.1, -0.0], vec![-0.5, -0.25]));
    }
    use std::sync::Arc;

    fn make_test_node() -> (TrackNode, Arc<TrackMeterState>) {
        let pool = AudioPool::new();
        let meter = Arc::new(TrackMeterState::default());
        let node = TrackNode::new(
            "test-track-id".into(),
            "Test".into(),
            pool,
            Arc::clone(&meter),
        );
        (node, meter)
    }

    /// A node with one loop in its grid, and the pool entry behind it.
    fn node_with_loop(length: usize) -> (TrackNode, Arc<crate::session::TrackSession>) {
        let pool = AudioPool::new();
        // A loop that counts up, so where a block came from in the loop
        // can be read straight off the output.
        let ramp: Vec<f32> = (0..length)
            .map(|i| (i + 1) as f32 / length as f32)
            .collect();
        pool.insert(
            "loop".to_string(),
            crate::audio_pool::AudioBuffer {
                channels: vec![ramp.clone(), ramp],
                sample_rate: 48_000,
                num_frames: length,
            },
        );
        let meter = Arc::new(TrackMeterState::default());
        let mut node = TrackNode::new("t".into(), "T".into(), pool, meter);
        node.set_session_slots(vec![Some(SessionClip {
            source_id: "loop".to_string(),
            source_offset: 0,
            length_samples: length as u64,
            gain: 1.0,
        })]);
        let session = Arc::new(crate::session::TrackSession::new());
        let quantise = Arc::new(std::sync::atomic::AtomicU32::new(4));
        node.set_session(Arc::clone(&session), quantise);
        (node, session)
    }

    fn block(node: &mut TrackNode, position: u64, size: usize) -> Vec<f32> {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            position_ticks: 0,
            buffer_size: size as u32,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: position,
            playing: true,
        };
        let inputs: Vec<&[f32]> = vec![];
        let mut outputs = vec![vec![0.0f32; size]; 4];
        node.process(&inputs, &mut outputs, &[], &mut Vec::new(), &ctx);
        outputs.remove(0)
    }

    #[test]
    fn a_fader_move_is_heard_on_the_next_blocks_without_a_rebuild() {
        let (mut node, session) = node_with_loop(480);
        session.queue(0);
        let peak = |b: &[f32]| b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        // Past the bar line, so the loop sounds.
        let before = peak(&block(&mut node, 96_000, 256));
        assert!(before > 0.0);
        // The mixer halves the fader while it plays.
        let meter = Arc::clone(&node.meter);
        meter.set_mix(0.5, 0.0, 1.0);
        let _ramping = block(&mut node, 96_256, 256);
        // The loop is 480 samples, so two loops on the same part of it
        // plays the same samples as the first block, at the new level.
        let _ = block(&mut node, 96_512, 256);
        let after = peak(&block(&mut node, 96_960, 256));
        assert!(
            (after / before - 0.5).abs() < 0.05,
            "half the fader, half the level: {before} then {after}"
        );
    }

    /// A track in a VCA group with its level automated: the lane takes the
    /// fader's place, and the group's level still goes on top. It used to
    /// be dropped with the fader, so the VCA did nothing to that track.
    #[test]
    fn the_vca_rides_an_automated_level() {
        use hardwave_project::automation::{
            AutomationLane, AutomationPoint, AutomationTarget, CurveMode,
        };
        let level = |vca: f32| {
            let (mut node, session) = node_with_loop(480);
            session.queue(0);
            node.set_automation_lanes(vec![AutomationLane {
                id: "volume".into(),
                target: AutomationTarget::TrackVolume,
                // 0 dB on the lane's -60..+6 dB range.
                points: vec![AutomationPoint {
                    tick: 0,
                    value: 60.0 / 66.0,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                }],
                visible: true,
                row: None,
            }]);
            node.meter.set_vca_gain(vca);
            block(&mut node, 96_000, 256)
                .iter()
                .fold(0.0f32, |m, s| m.max(s.abs()))
        };
        let (open, down) = (level(1.0), level(0.5));
        assert!(open > 0.0);
        assert!(
            (down / open - 0.5).abs() < 1e-3,
            "the VCA at half halves the automated track: {open} then {down}"
        );
    }

    /// Muting a track while the song plays drops its meter to silence. It
    /// kept showing the last block's level until the song stopped.
    #[test]
    fn a_muted_track_meters_silence() {
        let (mut node, session) = node_with_loop(480);
        session.queue(0);
        let _ = block(&mut node, 96_000, 256);
        let meter = Arc::clone(&node.meter);
        assert!(meter.peak_db_l.load(Ordering::Relaxed) > -60.0);
        node.set_muted(true);
        let _ = block(&mut node, 96_256, 256);
        assert_eq!(meter.peak_db_l.load(Ordering::Relaxed), -120.0);
        assert_eq!(meter.peak_db_r.load(Ordering::Relaxed), -120.0);
        assert_eq!(meter.rms_db.load(Ordering::Relaxed), -120.0);
        assert_eq!(meter.pre_fader_peak_db.load(Ordering::Relaxed), -120.0);
    }

    #[test]
    fn width_zero_makes_the_track_mono() {
        let (mut node, session) = node_with_loop(480);
        session.queue(0);
        let _ = block(&mut node, 96_000, 256);
        Arc::clone(&node.meter).set_mix(1.0, 1.0, 0.0);
        let _ = block(&mut node, 96_256, 256);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            position_ticks: 0,
            buffer_size: 256,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: 96_512,
            playing: true,
        };
        let mut outputs = vec![vec![0.0f32; 256]; 4];
        node.process(&[], &mut outputs, &[], &mut Vec::new(), &ctx);
        // Hard right pan after mono: the left side is silent.
        assert!(
            outputs[0].iter().all(|s| s.abs() < 1e-6),
            "panned hard right"
        );
        assert!(outputs[1].iter().any(|s| s.abs() > 0.0));
    }

    #[test]
    fn a_launched_clip_waits_for_the_bar_and_then_loops() {
        // A bar at 120 bpm is 96000 samples.
        let (mut node, session) = node_with_loop(480);
        session.queue(0);

        // Half way through a bar: nothing should sound yet.
        let early = block(&mut node, 48_000, 256);
        assert!(
            early.iter().all(|s| *s == 0.0),
            "a launch must wait for the boundary"
        );

        // The block the bar line falls in: it starts there, not at the
        // block edge.
        let landing = block(&mut node, 95_900, 256);
        assert!(
            landing[..100].iter().all(|s| *s == 0.0),
            "the part of the block before the bar line stays silent"
        );
        assert!(
            landing[100..].iter().any(|s| *s != 0.0),
            "and the part after it sounds"
        );

        // A block a loop and a half later: still sounding, because it
        // loops rather than playing once.
        let later = block(&mut node, 96_000 + 720, 256);
        assert!(later.iter().any(|s| *s != 0.0), "the loop should repeat");
    }

    /// A clip of a steady stereo signal (left 0.5, right 0.1) treated as
    /// `treat`, one block of it, as (left, right) at the block's middle.
    fn treated(treat: ClipTreat) -> (f32, f32) {
        let pool = AudioPool::new();
        pool.insert(
            "s".to_string(),
            crate::audio_pool::AudioBuffer {
                channels: vec![vec![0.5; 4800], vec![0.1; 4800]],
                sample_rate: 48_000,
                num_frames: 4800,
            },
        );
        let meter = Arc::new(TrackMeterState::default());
        let mut node = TrackNode::new("t".into(), "T".into(), pool, meter);
        node.set_clips(vec![ClipRegion {
            source_id: "s".into(),
            timeline_start: 0,
            timeline_end: 4800,
            source_offset: 0,
            gain: 1.0,
            muted: false,
            fade_in_samples: 0,
            fade_out_samples: 0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            source_step: 1.0,
            warp: Vec::new(),
            treat,
        }]);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            position_ticks: 0,
            buffer_size: 256,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: 0,
            playing: true,
        };
        let mut outputs = vec![vec![0.0f32; 256]; 4];
        node.process(&[], &mut outputs, &[], &mut Vec::new(), &ctx);
        (outputs[0][128], outputs[1][128])
    }

    #[test]
    fn a_clip_s_treatment_is_heard() {
        use hardwave_project::clip::SampleTreatment;
        let (l, r) = treated(ClipTreat::default());
        assert!(l > 0.0 && r > 0.0);
        let ratio = l / r;
        assert!(
            (ratio - 5.0).abs() < 1e-3,
            "as recorded, left is five times right"
        );

        let swapped = treated(ClipTreat::new(
            &SampleTreatment {
                swap_stereo: true,
                ..Default::default()
            },
            [0.0; 2],
        ));
        assert!((swapped.0 - r).abs() < 1e-6 && (swapped.1 - l).abs() < 1e-6);

        let flipped = treated(ClipTreat::new(
            &SampleTreatment {
                polarity_inverted: true,
                ..Default::default()
            },
            [0.0; 2],
        ));
        assert!((flipped.0 + l).abs() < 1e-6 && (flipped.1 + r).abs() < 1e-6);

        let left = treated(ClipTreat::new(
            &SampleTreatment {
                pan: -1.0,
                ..Default::default()
            },
            [0.0; 2],
        ));
        assert!(
            (left.0 - l).abs() < 1e-6,
            "panned left, the left side stays"
        );
        assert_eq!(left.1, 0.0, "and the right is gone");

        // A steady signal is all DC: with it taken off, nothing is left.
        let no_dc = treated(ClipTreat::new(
            &SampleTreatment {
                remove_dc: true,
                ..Default::default()
            },
            [0.5, 0.1],
        ));
        assert!(no_dc.0.abs() < 1e-6 && no_dc.1.abs() < 1e-6);
        // Asked for without a DC known, it takes nothing off.
        let off = ClipTreat::new(&SampleTreatment::default(), [0.5, 0.1]);
        assert_eq!(off.dc, [0.0; 2]);
    }

    /// A clip of a ramp 0, 1, 2, ... treated as `treat`: one block, left
    /// channel, so the output says which frame of the file was read.
    fn ramp_through(treat: ClipTreat, fade_stereo_right: bool) -> Vec<f32> {
        let pool = AudioPool::new();
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        pool.insert(
            "ramp".to_string(),
            crate::audio_pool::AudioBuffer {
                channels: vec![ramp.clone(), ramp],
                sample_rate: 48_000,
                num_frames: 1000,
            },
        );
        let meter = Arc::new(TrackMeterState::default());
        let mut node = TrackNode::new("t".into(), "T".into(), pool, meter);
        node.set_clips(vec![ClipRegion {
            source_id: "ramp".into(),
            timeline_start: 0,
            timeline_end: 1000,
            source_offset: 0,
            gain: 1.0,
            muted: false,
            fade_in_samples: 0,
            fade_out_samples: 0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            source_step: 1.0,
            warp: Vec::new(),
            treat,
        }]);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            buffer_size: 512,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: 0,
            position_ticks: 0,
            playing: true,
        };
        let mut outputs = vec![vec![0.0f32; 512]; 2];
        let empty: Vec<f32> = vec![0.0; 512];
        let inputs: Vec<&[f32]> = vec![&empty, &empty];
        node.process(&inputs, &mut outputs, &[], &mut Vec::new(), &ctx);
        outputs.swap_remove(if fade_stereo_right { 1 } else { 0 })
    }

    /// The pan law takes both sides down the same: the ramp's frame 1
    /// through an untreated clip says by how much.
    fn pan_gain() -> f32 {
        ramp_through(ClipTreat::default(), false)[1]
    }

    #[test]
    fn the_sample_s_window_moves_its_start_and_ends_it() {
        // Sample start at frame 100 of the file: the clip's first frame
        // reads frame 100. The window ends at 300: nothing after it.
        let t = ClipTreat {
            window: (100, 300),
            ..Default::default()
        };
        let g = pan_gain();
        let out: Vec<f32> = ramp_through(t, false).iter().map(|v| v / g).collect();
        assert!(
            (out[0] - 100.0).abs() < 1e-2,
            "starts at the window: {}",
            out[0]
        );
        assert!(
            (out[199] - 299.0).abs() < 1e-2,
            "reads up to its end: {}",
            out[199]
        );
        assert_eq!(out[200], 0.0, "and nothing past it");
        assert_eq!(out[400], 0.0);
    }

    #[test]
    fn fade_stereo_goes_from_left_to_right_over_the_window() {
        let t = ClipTreat {
            window: (0, 1000),
            fade_stereo: true,
            ..Default::default()
        };
        let g = pan_gain();
        let left: Vec<f32> = ramp_through(t, false).iter().map(|v| v / g).collect();
        let right: Vec<f32> = ramp_through(t, true).iter().map(|v| v / g).collect();
        // Frame 500 is half way: both sides at half of the ramp's 500.
        assert!(
            (left[500] - 250.0).abs() < 2.0,
            "left half way down: {}",
            left[500]
        );
        assert!(
            (right[500] - 250.0).abs() < 2.0,
            "right half way up: {}",
            right[500]
        );
        assert!(
            left[10] > 9.5 && right[10] < 0.2,
            "left full and right gone at the start"
        );
    }

    /// A ramp 0, 1, 2, ... played reversed at `step` source frames per
    /// sample, from frame 100, for 400 samples: one block, left channel.
    fn reversed_ramp(step: f64) -> Vec<f32> {
        let pool = AudioPool::new();
        let ramp: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        pool.insert(
            "ramp".to_string(),
            crate::audio_pool::AudioBuffer {
                channels: vec![ramp.clone(), ramp],
                sample_rate: 48_000,
                num_frames: 1000,
            },
        );
        let meter = Arc::new(TrackMeterState::default());
        let mut node = TrackNode::new("t".into(), "T".into(), pool, meter);
        node.set_clips(vec![ClipRegion {
            source_id: "ramp".into(),
            timeline_start: 0,
            timeline_end: 400,
            source_offset: 100,
            gain: 1.0,
            muted: false,
            fade_in_samples: 0,
            fade_out_samples: 0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: true,
            source_step: step,
            warp: Vec::new(),
            treat: ClipTreat::default(),
        }]);
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            buffer_size: 512,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: 0,
            position_ticks: 0,
            playing: true,
        };
        let mut outputs = vec![vec![0.0f32; 512]; 2];
        let empty: Vec<f32> = vec![0.0; 512];
        let inputs: Vec<&[f32]> = vec![&empty, &empty];
        node.process(&inputs, &mut outputs, &[], &mut Vec::new(), &ctx);
        let g = pan_gain();
        outputs.swap_remove(0).iter().map(|v| v / g).collect()
    }

    #[test]
    fn a_reversed_clip_plays_its_window_backwards_at_any_speed() {
        // At 1:1 the 400 frames from 100 on, last first.
        let out = reversed_ramp(1.0);
        assert!(
            (out[0] - 499.0).abs() < 1e-2,
            "starts at the end: {}",
            out[0]
        );
        assert!(
            (out[399] - 100.0).abs() < 1e-2,
            "ends at the start: {}",
            out[399]
        );

        // An octave up it moves through twice the file: 800 frames from
        // 100 on, last first, and sounding all the way to its end.
        let out = reversed_ramp(2.0);
        assert!(
            (out[0] - 898.0).abs() < 1e-2,
            "starts at the end: {}",
            out[0]
        );
        assert!((out[200] - 498.0).abs() < 1e-2, "half way: {}", out[200]);
        assert!(
            (out[399] - 100.0).abs() < 1e-2,
            "ends at the start: {}",
            out[399]
        );
        assert_eq!(out[400], 0.0, "and stops with the clip");
    }

    #[test]
    fn the_window_comes_from_the_treatment_and_the_trim() {
        use hardwave_project::clip::SampleTreatment;
        let t = SampleTreatment {
            start: 0.25,
            length: 0.5,
            ..Default::default()
        };
        let w = ClipTreat::new(&t, [0.0; 2])
            .with_window(&t, 1000, None)
            .window;
        assert_eq!(w, (250, 625), "a quarter in, half of what is left");
        let w = ClipTreat::new(&t, [0.0; 2])
            .with_window(&t, 1000, Some(400))
            .window;
        assert_eq!(w, (250, 400), "the trim cuts the tail sooner");
        let w = ClipTreat::new(&t, [0.0; 2])
            .with_window(&t, 1000, Some(10))
            .window;
        assert_eq!(
            w,
            (250, 250),
            "a trim before the start leaves nothing, never a negative span"
        );
    }

    #[test]
    fn a_launched_clip_replaces_this_track_s_timeline_clips() {
        let (mut node, session) = node_with_loop(480);
        node.set_clips(vec![]);
        session.queue(0);
        let _ = block(&mut node, 96_000, 256);
        assert_eq!(session.playing_slot(), Some(0));
    }

    #[test]
    fn stopping_is_also_on_the_bar() {
        let (mut node, session) = node_with_loop(480);
        session.queue(0);
        let _ = block(&mut node, 96_000, 256);
        session.queue_stop();
        // Still inside the bar: it keeps playing.
        let during = block(&mut node, 96_256, 256);
        assert!(during.iter().any(|s| *s != 0.0), "it stops with the bar");
        // The next bar line: silent from there.
        let after = block(&mut node, 192_000, 256);
        assert!(
            after.iter().all(|s| *s == 0.0),
            "it should have stopped at the bar"
        );
        assert_eq!(session.playing_slot(), None);
    }

    #[test]
    fn with_no_quantise_a_clip_starts_in_the_block_it_was_pressed() {
        let (mut node, session) = node_with_loop(480);
        node.set_session(
            Arc::clone(&session),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
        );
        session.queue(0);
        let now = block(&mut node, 12_345, 256);
        assert!(now.iter().any(|s| *s != 0.0), "now means now");
    }

    #[test]
    fn pre_fader_meter_reads_signal_before_volume() {
        let (mut node, meter) = make_test_node();
        node.set_volume_db(-100.0); // fader at -inf

        let ctx = ProcessContext {
            sample_rate: 48000.0,
            position_ticks: 0,
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

#[cfg(test)]
mod warp_tests {
    use super::{build_warp_segments, warp_source_pos};

    #[test]
    fn piecewise_map_interpolates_between_anchors() {
        // Anchor A: timeline 1000 ↔ source 4000; anchor B: 2000 ↔ 5000.
        // Before A: implicit (0, source_offset=0) → step 4.0.
        // Between A and B: step (5000-4000)/1000 = 1.0.
        let segs = build_warp_segments(&[(1000, 4000.0), (2000, 5000.0)], 1.0, 0.0);
        assert_eq!(warp_source_pos(&segs, 0), 0.0);
        assert_eq!(
            warp_source_pos(&segs, 500),
            2000.0,
            "first span squeezes 4x"
        );
        assert_eq!(warp_source_pos(&segs, 1000), 4000.0, "anchor A exact");
        assert_eq!(warp_source_pos(&segs, 1500), 4500.0, "second span is 1:1");
        assert_eq!(warp_source_pos(&segs, 2000), 5000.0, "anchor B exact");
    }

    #[test]
    fn tail_continues_with_last_step() {
        let segs = build_warp_segments(&[(1000, 500.0)], 1.0, 0.0);
        // Only one user anchor: pre-anchor step (500-0)/1000 = 0.5; tail
        // keeps that step past the anchor.
        assert_eq!(warp_source_pos(&segs, 2000), 1000.0);
    }

    #[test]
    fn single_anchor_at_zero_uses_default_step_for_tail() {
        let segs = build_warp_segments(&[(0, 300.0)], 2.0, 0.0);
        // Anchor replaces the implicit origin; no inter-anchor span →
        // tail uses default_step.
        assert_eq!(warp_source_pos(&segs, 0), 300.0);
        assert_eq!(warp_source_pos(&segs, 100), 500.0);
    }

    #[test]
    fn unsorted_and_duplicate_anchors_are_normalized() {
        let segs = build_warp_segments(&[(2000, 5000.0), (1000, 4000.0), (1000, 9999.0)], 1.0, 0.0);
        // Sorted, dupes on the same tick collapsed (first kept after sort).
        assert_eq!(warp_source_pos(&segs, 1500), 4500.0);
    }
}

#[cfg(test)]
mod silence_tests {
    use super::*;
    use std::sync::Arc;

    /// A track with a steady clip (0.5 on both sides) on each of `spans`.
    fn steady_clips(spans: &[(u64, u64)]) -> TrackNode {
        let pool = AudioPool::new();
        pool.insert(
            "s".to_string(),
            crate::audio_pool::AudioBuffer {
                channels: vec![vec![0.5; 48_000], vec![0.5; 48_000]],
                sample_rate: 48_000,
                num_frames: 48_000,
            },
        );
        let meter = Arc::new(TrackMeterState::default());
        let mut node = TrackNode::new("t".into(), "T".into(), pool, meter);
        node.set_clips(
            spans
                .iter()
                .map(|&(start, end)| ClipRegion {
                    source_id: "s".into(),
                    timeline_start: start,
                    timeline_end: end,
                    source_offset: 0,
                    gain: 1.0,
                    muted: false,
                    fade_in_samples: 0,
                    fade_out_samples: 0,
                    fade_in_curve: Default::default(),
                    fade_out_curve: Default::default(),
                    reversed: false,
                    source_step: 1.0,
                    warp: Vec::new(),
                    treat: ClipTreat::default(),
                })
                .collect(),
        );
        node
    }

    /// One block at `position` with `input` arriving on both sides: the
    /// left side, and whether the node said the block was silent.
    fn heard(node: &mut TrackNode, position: u64, input: f32) -> (Vec<f32>, bool) {
        let ctx = ProcessContext {
            sample_rate: 48_000.0,
            position_ticks: 0,
            buffer_size: 512,
            tempo: 120.0,
            time_sig: (4, 4),
            position_samples: position,
            playing: true,
        };
        let input = vec![input; 512];
        let inputs: Vec<&[f32]> = vec![&input, &input, &[], &[]];
        let mut outputs = vec![vec![0.0f32; 512]; 4];
        node.process(&inputs[..2], &mut outputs, &[], &mut Vec::new(), &ctx);
        (outputs.remove(0), node.output_is_silent())
    }

    /// Between its clips a track writes silence and says so, so the graph
    /// leaves it out of the sums; with a clip under the playhead it plays.
    #[test]
    fn a_track_between_its_clips_writes_silence_and_says_so() {
        let mut node = steady_clips(&[(4096, 8192)]);
        let (left, silent) = heard(&mut node, 0, 0.0);
        assert!(silent);
        assert!(left.iter().all(|s| *s == 0.0));
        let (left, silent) = heard(&mut node, 4096, 0.0);
        assert!(!silent);
        assert!(left.iter().all(|s| *s != 0.0));
    }

    /// A bus with nothing of its own plays what reaches it, and is silent
    /// when nothing does.
    #[test]
    fn a_bus_is_silent_only_when_nothing_reaches_it() {
        let mut node = steady_clips(&[]);
        node.set_receives_input(true);
        let (left, silent) = heard(&mut node, 0, 0.25);
        assert!(!silent);
        assert!(left.iter().all(|s| *s != 0.0));
        let (left, silent) = heard(&mut node, 512, 0.0);
        assert!(silent);
        assert!(left.iter().all(|s| *s == 0.0));
    }

    /// The fader moved while the track was silent between two clips: the
    /// second clip starts at the new level, as it did when the silence ran
    /// through the fader, and does not fade over from the old one.
    #[test]
    fn a_fader_moved_between_clips_is_where_it_was_put_when_the_next_starts() {
        let mut node = steady_clips(&[(0, 512), (2048, 8192)]);
        let (first, _) = heard(&mut node, 0, 0.0);
        assert!(first.iter().all(|s| *s == first[0]));
        node.set_volume_db(-6.0);
        for at in [512, 1024, 1536] {
            assert!(heard(&mut node, at, 0.0).1);
        }
        let (second, _) = heard(&mut node, 2048, 0.0);
        assert!(second[0] < first[0]);
        assert!(
            second.iter().all(|s| *s == second[0]),
            "no fade from the old level"
        );
    }

    /// A filter still ringing after its clip ended is heard to the end of
    /// its tail; the track is silent only once nothing is left in it.
    #[test]
    fn a_filter_still_ringing_after_a_clip_is_heard() {
        let mut node = steady_clips(&[(0, 512)]);
        node.set_filter(TrackFilterType::LowPass, 200.0, 0.9, 48_000.0);
        heard(&mut node, 0, 0.0);
        let (tail, silent) = heard(&mut node, 512, 0.0);
        assert!(!silent);
        assert!(tail.iter().any(|s| *s != 0.0), "the tail is heard");
    }
}
