use crate::AppState;
use hardwave_engine::{DawEngine, OfflineRender};
use hardwave_project::track::TrackKind;
use hardwave_project::Project;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

/// Build the offline-render insert factory: snapshot the scanner's
/// descriptors and return a closure that instantiates a fresh plug-in by
/// id (native / VST3 / CLAP). Passed to `render_offline_with` so exports
/// apply the same insert FX (EQ / comp / reverb / synth) as live playback
/// — without it, bounces render dry.
pub(crate) fn offline_insert_factory(
    engine: &DawEngine,
) -> impl Fn(&str) -> Option<Box<dyn hardwave_plugin_host::types::HostedPlugin>> {
    let descriptors: std::collections::HashMap<String, hardwave_plugin_host::PluginDescriptor> =
        engine
            .plugin_scanner
            .lock()
            .plugins()
            .iter()
            .map(|d| (d.id.clone(), d.clone()))
            .collect();
    move |plugin_id: &str| {
        descriptors
            .get(plugin_id)
            .and_then(|d| super::plugins::load_hosted(d).ok())
    }
}

/// Narrow a render's copy of the song to one track, for a bounce or a
/// consolidate. The other tracks are excluded from the mix rather than
/// muted, so they still key its sidechains. The master's inserts and fader
/// are stood down and the track plays straight into it: what comes out goes
/// back onto a track that plays through the master and its bus again. With
/// `keep_effects` false the track's own effects, fader and sends go too and
/// only an instrument stays: a consolidated clip stays on the track and
/// would otherwise be run through them twice.
///
/// The track is rendered as it sounds unmuted and with nothing soloed. Its
/// own mute button, or a solo on any other track, left it silent in the
/// render: freezing a muted track froze silence, and consolidating while
/// another track was soloed replaced the clips with an empty one.
fn isolate_track_for_render(
    proj: &mut Project,
    target: &str,
    keep_effects: bool,
    is_instrument: &dyn Fn(&str) -> bool,
) {
    for t in proj.tracks.iter_mut() {
        t.soloed = false;
        if matches!(t.kind, TrackKind::Master) {
            t.inserts.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            continue;
        }
        t.stem_excluded = t.id != target;
        if t.id != target {
            continue;
        }
        t.muted = false;
        t.output_bus = None;
        if !keep_effects {
            t.inserts.retain(|s| is_instrument(&s.plugin_id));
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
            // The channel's delay, swap, polarity, width and filter, which
            // the glued clip plays through on the track as well.
            t.phase_invert = false;
            t.swap_lr = false;
            t.delay_samples = 0;
            t.stereo_separation = 1.0;
            t.filter_type = "off".into();
        }
    }
}

/// Narrow a freeze or consolidate render further, to the track before its
/// fader. A frozen track, and a glued clip on its track, still play through
/// the fader and pan, their automation, the track's VCA groups and anything
/// modulating its level or pan; left in the render as well, they applied
/// twice: a track at -6 dB came back at -12 dB, panned twice as far.
fn leave_the_fader_out(proj: &mut Project, target: &str) {
    use hardwave_project::automation::AutomationTarget;
    use hardwave_project::modulation::ModTarget;
    let mix = |t: &AutomationTarget| {
        matches!(
            t,
            AutomationTarget::TrackVolume | AutomationTarget::TrackPan
        )
    };
    if let Some(t) = proj.track_mut(target) {
        t.volume_db = 0.0;
        t.pan = 0.0;
        t.automation_lanes.retain(|l| !mix(&l.target));
        t.automation_clips.retain(|c| !mix(&c.target));
    }
    for vca in proj.vcas.iter_mut() {
        vca.members.retain(|m| m != target);
    }
    proj.modulations.retain(|r| {
        !(r.track_id == target && matches!(r.target, ModTarget::TrackVolume | ModTarget::TrackPan))
    });
}

/// The ids of the plug-ins that make sound from notes, as the scanner knows
/// them: what a consolidated pattern clip keeps.
fn instrument_ids(engine: &DawEngine) -> std::collections::HashSet<String> {
    engine
        .plugin_scanner
        .lock()
        .plugins()
        .iter()
        .filter(|d| {
            matches!(
                d.category,
                hardwave_plugin_host::types::PluginCategory::Instrument
            )
        })
        .map(|d| d.id.clone())
        .collect()
}

/// Prepare a render under the engine's lock, let go of the lock, then
/// render and write. The render has its own copy of everything; holding
/// the lock through it froze every window command until the file was done.
///
/// `onto_a_track` is for audio that goes back onto a track (a bounce, a
/// freeze, a consolidate): see [`hardwave_engine::OfflineRender::for_a_track`].
#[allow(clippy::too_many_arguments)]
fn render_unlocked<F>(
    engine: parking_lot::MutexGuard<'_, DawEngine>,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    start_samples: u64,
    cancel: &Arc<AtomicBool>,
    on_progress: F,
    onto_a_track: bool,
    prepare: impl FnOnce(&mut Project),
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    render_unlocked_measured(
        engine,
        out_path,
        fmt,
        total_samples,
        start_samples,
        cancel,
        on_progress,
        onto_a_track,
        prepare,
    )
    .map(|loudness| loudness.is_some())
}

/// As `render_unlocked`, and say how loud the file came out: `None` when
/// the render was cancelled.
#[allow(clippy::too_many_arguments)]
fn render_unlocked_measured<F>(
    engine: parking_lot::MutexGuard<'_, DawEngine>,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    start_samples: u64,
    cancel: &Arc<AtomicBool>,
    on_progress: F,
    onto_a_track: bool,
    prepare: impl FnOnce(&mut Project),
) -> Result<Option<FileLoudness>, String>
where
    F: FnMut(u64, u64),
{
    let factory = offline_insert_factory(&engine);
    let render =
        engine.prepare_offline_render(fmt.sample_rate, start_samples, Some(&factory), prepare);
    drop(engine);
    let render = if onto_a_track {
        render.for_a_track()
    } else {
        render
    };
    let mut meter = LoudnessTap::new(fmt.sample_rate);
    let completed = write_render(
        render,
        out_path,
        fmt,
        total_samples,
        cancel,
        &mut meter,
        on_progress,
    )?;
    Ok(completed.then(|| meter.finish()))
}

/// How loud an exported file is: its sample peak, its true peak and its
/// integrated loudness, for the notice after an export.
#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileLoudness {
    pub peak_db: f32,
    pub true_peak_db: f32,
    pub lufs_integrated: Option<f32>,
}

/// Measures the samples on their way into the file: after normalizing and
/// before dither and the encoder. Every format path hands it what it
/// writes, so the notice describes the file, not the mix before it.
struct LoudnessTap {
    meter: hardwave_metering::ChannelMeter,
    left: Vec<f32>,
    right: Vec<f32>,
    peak: f32,
}

impl LoudnessTap {
    /// Frames the meter reads at a time.
    const BLOCK: usize = 512;

    fn new(sample_rate: u32) -> Self {
        Self {
            meter: hardwave_metering::ChannelMeter::new(sample_rate as f64),
            left: Vec::with_capacity(Self::BLOCK),
            right: Vec::with_capacity(Self::BLOCK),
            peak: 0.0,
        }
    }

    /// One sample of interleaved stereo; `channel` 0 is left.
    fn push(&mut self, sample: f32, channel: usize) {
        self.peak = self.peak.max(sample.abs());
        if channel == 0 {
            self.left.push(sample);
        } else {
            self.right.push(sample);
            if self.right.len() == Self::BLOCK {
                self.flush();
            }
        }
    }

    fn push_interleaved(&mut self, samples: &[f32]) {
        for (i, &s) in samples.iter().enumerate() {
            self.push(s, i & 1);
        }
    }

    fn flush(&mut self) {
        let n = self.left.len().min(self.right.len());
        if n > 0 {
            self.meter.process_block(&self.left[..n], &self.right[..n]);
        }
        self.left.clear();
        self.right.clear();
    }

    fn finish(mut self) -> FileLoudness {
        self.flush();
        FileLoudness {
            peak_db: if self.peak > 1e-9 {
                20.0 * self.peak.log10()
            } else {
                -100.0
            },
            true_peak_db: self.meter.true_peak_db(),
            lufs_integrated: self.meter.lufs_i(),
        }
    }
}

#[derive(serde::Serialize, Clone)]
pub struct ExportProgress {
    pub percent: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ExportResult {
    pub path: String,
    pub samples: u64,
    pub duration_secs: f64,
    pub cancelled: bool,
    /// How loud the file came out; none when cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loudness: Option<FileLoudness>,
}

#[derive(serde::Serialize)]
pub struct StemsExportResult {
    pub folder: String,
    pub files: Vec<String>,
    pub duration_secs: f64,
    pub cancelled: bool,
}

#[derive(Clone, Copy, Debug)]
enum NormalizeMode {
    Off,
    Peak,
    Lufs,
}

impl NormalizeMode {
    fn parse(s: &str) -> Self {
        match s {
            "peak" => Self::Peak,
            "lufs" => Self::Lufs,
            _ => Self::Off,
        }
    }
}

/// Integrated loudness (ITU-R BS.1770-4) of interleaved stereo, through the
/// same meter the export notice and the mix check read, so a file
/// normalized to a target reads that target. It had a K-filter of its own,
/// with the audio-EQ-cookbook shelf in place of the standard's, and read a
/// quarter LU low: a song normalized to -14 LUFS came out near -13.75.
/// `None` when the audio is too short or too quiet to measure.
fn integrated_lufs(buffer: &[f32], sample_rate: u32) -> Option<f32> {
    let mut tap = LoudnessTap::new(sample_rate);
    tap.push_interleaved(buffer);
    tap.finish().lufs_integrated
}

#[derive(Clone, Copy, Debug)]
enum DitherMode {
    None,
    Tpdf,
    TpdfShaped,
}

impl DitherMode {
    fn parse(s: &str) -> Self {
        match s {
            "triangular" | "tpdf" => Self::Tpdf,
            "tpdf_shaped" | "noise_shaped" | "shaped" => Self::TpdfShaped,
            _ => Self::None,
        }
    }
}

/// Small xorshift32 PRNG for dither noise. Seeded deterministically per
/// render so a given project produces byte-identical output each export —
/// which matters for audio diffing and user verification.
struct DitherRng {
    state: u32,
}

impl DitherRng {
    fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0xdead_beef } else { seed },
        }
    }
    fn next_u32(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }
    fn next_uniform(&mut self) -> f32 {
        let u = self.next_u32() as f32 / u32::MAX as f32;
        u * 2.0 - 1.0
    }
    fn next_tpdf(&mut self) -> f32 {
        (self.next_uniform() + self.next_uniform()) * 0.5
    }
}

struct DitherState {
    rng: DitherRng,
    prev_err: [f32; 2],
}

impl DitherState {
    fn new() -> Self {
        Self {
            rng: DitherRng::new(0x1234_5678),
            prev_err: [0.0; 2],
        }
    }
}

/// A sample with dither added, one step of the output's bit depth wide,
/// ready to be rounded to it.
fn add_dither(
    s: f32,
    lsb: f32,
    dither: DitherMode,
    state: &mut DitherState,
    channel: usize,
) -> f32 {
    match dither {
        DitherMode::None => s,
        DitherMode::Tpdf => s + state.rng.next_tpdf() * lsb,
        DitherMode::TpdfShaped => {
            let n = state.rng.next_tpdf() * lsb;
            let ch = channel & 1;
            let x = s + n - state.prev_err[ch];
            state.prev_err[ch] = n;
            x
        }
    }
}

fn write_sample_dithered(
    wav: &mut WavWriter<BufWriter<File>>,
    s: f32,
    bit_depth: u32,
    dither: DitherMode,
    state: &mut DitherState,
    channel: usize,
) -> Result<(), hound::Error> {
    match bit_depth {
        16 => {
            let max = i16::MAX as f32;
            let x = add_dither(s, 1.0 / max, dither, state, channel);
            let v = (x.clamp(-1.0, 1.0) * max).round() as i16;
            wav.write_sample(v)
        }
        24 => {
            let max = 8_388_607.0_f32;
            let x = add_dither(s, 1.0 / max, dither, state, channel);
            let v = (x.clamp(-1.0, 1.0) * max).round() as i32;
            wav.write_sample(v)
        }
        _ => wav.write_sample(s.clamp(-1.0, 1.0)),
    }
}

#[derive(Clone, Copy)]
struct RenderFormat {
    sample_rate: u32,
    bit_depth: u32,
    normalize: NormalizeMode,
    normalize_target_db: f32,
    dither: DitherMode,
    /// Only consulted when the output path is `.mp3` and VBR is off.
    /// Ignored otherwise.
    mp3_bitrate_kbps: u32,
    /// When `Some(quality)`, MP3 export uses the VBR encoder with the
    /// given quality (`0` best, `9` smallest). When `None`, uses
    /// `mp3_bitrate_kbps` CBR. Only consulted for `.mp3` output.
    mp3_vbr_quality: Option<u32>,
    /// Vorbis QualityVbr target in the range `-0.2..=1.0`. Only consulted
    /// when the output path is `.ogg`. Ignored otherwise.
    ogg_quality: f32,
}

/// The WAV header for a render. It has to say what `write_sample_dithered`
/// writes: 16- and 24-bit integers, and 32-bit float for anything else. A
/// bounce, freeze or consolidate asks for 32 bits and was written as float
/// samples under a header that said 32-bit integers, so the file read back
/// as loud distortion in place of the part.
fn spec_for(bit_depth: u32, sample_rate: u32) -> WavSpec {
    let (bits, format) = match bit_depth {
        16 | 24 => (bit_depth as u16, SampleFormat::Int),
        _ => (32u16, SampleFormat::Float),
    };
    WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: bits,
        sample_format: format,
    }
}

/// How a bounce, freeze or consolidate is written: 32-bit float WAV at the
/// song's rate, nothing done to its level.
fn format_for_a_track(sample_rate: u32) -> RenderFormat {
    RenderFormat {
        sample_rate,
        bit_depth: 32,
        normalize: NormalizeMode::Off,
        normalize_target_db: -1.0,
        dither: DitherMode::None,
        mp3_bitrate_kbps: 320,
        mp3_vbr_quality: None,
        ogg_quality: 0.5,
    }
}

fn is_flac_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("flac"))
        .unwrap_or(false)
}

/// Encode normalized interleaved stereo `f32` samples to a FLAC file. FLAC
/// is lossless integer PCM only, so `bit_depth==0` (32-bit float requested)
/// falls back to 24-bit to preserve the maximum lossless precision flacenc
/// can represent.
fn write_flac_from_buffer(
    out_path: &Path,
    buffer: &[f32],
    sample_rate: u32,
    bit_depth: u32,
    dither: DitherMode,
) -> Result<(), String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let bits = flac_bits(bit_depth);
    let samples = flac_samples(buffer, bits, dither);

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("flac config: {e:?}"))?;
    let source =
        flacenc::source::MemSource::from_samples(&samples, 2, bits as usize, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| format!("flac encode: {e:?}"))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| format!("flac serialize: {e:?}"))?;
    fs::write(out_path, sink.as_slice()).map_err(|e| format!("flac file: {e}"))?;
    Ok(())
}

/// The bit depth a FLAC is written at: float has no FLAC form, so 24.
fn flac_bits(bit_depth: u32) -> u32 {
    match bit_depth {
        16 => 16,
        0 | 24 => 24,
        n => n.clamp(8, 24),
    }
}

/// Interleaved samples rounded to `bits`, with the dither the export asked
/// for. FLAC exports used to round without it whatever the setting said, so
/// a 16-bit FLAC's fades and quiet tails broke up into the distortion
/// dither is there to prevent, while the same export as WAV was dithered.
fn flac_samples(buffer: &[f32], bits: u32, dither: DitherMode) -> Vec<i32> {
    let max = ((1i64 << (bits - 1)) - 1) as f32;
    let lsb = 1.0 / max;
    let mut state = DitherState::new();
    buffer
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let x = add_dither(s, lsb, dither, &mut state, i & 1);
            (x.clamp(-1.0, 1.0) * max).round() as i32
        })
        .collect()
}

/// FLAC path: always buffer the full render, apply normalization, then encode.
/// FLAC has no streaming writer in this codebase, but the file sizes involved
/// (2.6 GB for a 4-hour session at 48k stereo f32) are acceptable for desktop.
#[allow(clippy::too_many_arguments)]
fn write_render_flac<F>(
    render: OfflineRender,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    cancel: &Arc<AtomicBool>,
    meter: &mut LoudnessTap,
    mut on_progress: F,
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    let capacity = (total_samples as usize).saturating_mul(2);
    let mut buffer: Vec<f32> = Vec::with_capacity(capacity);

    render.run(total_samples, |block| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        buffer.extend_from_slice(block);
        // Reserve ~85% of progress for render; encode is the final 15%.
        let frames = (buffer.len() / 2) as u64;
        on_progress((frames * 85) / 100, total_samples);
        true
    });

    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }

    let mut gain: f32 = 1.0;
    match fmt.normalize {
        NormalizeMode::Off => {}
        NormalizeMode::Peak => {
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            if peak > 1e-9 {
                gain = 10.0_f32.powf(fmt.normalize_target_db / 20.0) / peak;
            }
        }
        NormalizeMode::Lufs => {
            if let Some(lufs) = integrated_lufs(&buffer, fmt.sample_rate) {
                if lufs.is_finite() {
                    gain = 10.0_f32.powf((fmt.normalize_target_db - lufs) / 20.0);
                }
            }
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            let ceiling = 10.0_f32.powf(-1.0 / 20.0);
            if peak * gain > ceiling && peak > 1e-9 {
                gain = ceiling / peak;
            }
        }
    }

    if (gain - 1.0).abs() > f32::EPSILON {
        for s in buffer.iter_mut() {
            *s *= gain;
        }
    }

    on_progress((total_samples * 90) / 100, total_samples);
    meter.push_interleaved(&buffer);
    write_flac_from_buffer(
        out_path,
        &buffer,
        fmt.sample_rate,
        fmt.bit_depth,
        fmt.dither,
    )?;
    on_progress(total_samples, total_samples);
    Ok(true)
}

fn is_mp3_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("mp3"))
        .unwrap_or(false)
}

/// Encode normalized interleaved stereo `f32` samples to a CBR MP3 file via
/// shine-rs. Shine supports only MPEG-1/2/2.5 sample rates up to 48 kHz; the
/// caller enforces that constraint upstream so we can error cleanly here.
fn write_mp3_from_buffer(
    out_path: &Path,
    buffer: &[f32],
    sample_rate: u32,
    bitrate_kbps: u32,
) -> Result<(), String> {
    use shine_rs::{Mp3Encoder, Mp3EncoderConfig, StereoMode};

    const SHINE_RATES: &[u32] = &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
    if !SHINE_RATES.contains(&sample_rate) {
        return Err(format!(
            "MP3 export does not support {sample_rate} Hz — choose 32000/44100/48000"
        ));
    }

    let mut pcm: Vec<i16> = Vec::with_capacity(buffer.len());
    for &s in buffer {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i32;
        pcm.push(v.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }

    let cfg = Mp3EncoderConfig::new()
        .sample_rate(sample_rate)
        .bitrate(bitrate_kbps)
        .channels(2)
        .stereo_mode(StereoMode::JointStereo);
    let mut encoder = Mp3Encoder::new(cfg).map_err(|e| format!("mp3 init: {e:?}"))?;

    let mut out = Vec::with_capacity(pcm.len() / 8);
    let frame_len = encoder.samples_per_frame();
    let mut cursor = 0usize;
    while cursor < pcm.len() {
        let end = (cursor + frame_len).min(pcm.len());
        let frames = encoder
            .encode_interleaved(&pcm[cursor..end])
            .map_err(|e| format!("mp3 encode: {e:?}"))?;
        for f in frames {
            out.extend_from_slice(&f);
        }
        cursor = end;
    }
    let tail = encoder.finish().map_err(|e| format!("mp3 finish: {e:?}"))?;
    out.extend_from_slice(&tail);

    fs::write(out_path, &out).map_err(|e| format!("mp3 file: {e}"))?;
    Ok(())
}

/// Encode an interleaved stereo f32 buffer as a genuine VBR MP3 by
/// splitting it into fixed segments and choosing a bitrate per
/// segment based on the segment's RMS level. Quieter segments land
/// in a lower bitrate tier; louder ones in a higher tier. The
/// resulting file is freeform VBR — each frame header carries the
/// correct bitrate, so VLC / ffmpeg / any frame-by-frame MP3
/// decoder reads it natively.
///
/// The `quality` parameter in `[0, 9]` (0 = best, 9 = smallest)
/// biases the per-segment bitrate tiers up or down by one step.
fn write_mp3_vbr_from_buffer(
    out_path: &Path,
    buffer: &[f32],
    sample_rate: u32,
    quality: u32,
) -> Result<(), String> {
    use shine_rs::{Mp3Encoder, Mp3EncoderConfig, StereoMode};

    const SHINE_RATES: &[u32] = &[8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
    if !SHINE_RATES.contains(&sample_rate) {
        return Err(format!("MP3 VBR export does not support {sample_rate} Hz"));
    }

    // Quality → bitrate tiers (kbps). Quality 0 = highest average
    // bitrate; quality 9 = lowest. Each tier has a "quiet / medium /
    // loud" ladder that a per-segment RMS pick traverses.
    let quality = quality.min(9);
    let tiers: [[u32; 3]; 10] = [
        [224, 288, 320], // 0 best
        [192, 256, 320],
        [160, 224, 288],
        [128, 192, 256],
        [128, 160, 224],
        [112, 128, 192],
        [96, 128, 160],
        [80, 112, 128],
        [64, 96, 128],
        [64, 80, 96], // 9 smallest
    ];
    let ladder = tiers[quality as usize];

    // 2-second segments — large enough for shine to settle, small
    // enough for bitrate to track signal loudness.
    let segment_frames: usize = (sample_rate as usize * 2) * 2; // stereo samples
    let mut cursor = 0usize;
    let mut out = Vec::with_capacity(buffer.len());

    while cursor < buffer.len() {
        let end = (cursor + segment_frames).min(buffer.len());
        let segment = &buffer[cursor..end];
        // RMS of the segment.
        let mut sum_sq = 0.0_f64;
        for &s in segment {
            sum_sq += (s as f64) * (s as f64);
        }
        let rms = (sum_sq / segment.len() as f64).sqrt();
        // Pick a tier: < -24 dBFS → low, -24..-12 → mid, > -12 → high.
        let bitrate = if rms < 0.063 {
            ladder[0]
        } else if rms < 0.251 {
            ladder[1]
        } else {
            ladder[2]
        };

        let mut pcm: Vec<i16> = Vec::with_capacity(segment.len());
        for &s in segment {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i32;
            pcm.push(v.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        }
        let cfg = Mp3EncoderConfig::new()
            .sample_rate(sample_rate)
            .bitrate(bitrate)
            .channels(2)
            .stereo_mode(StereoMode::JointStereo);
        let mut encoder = Mp3Encoder::new(cfg).map_err(|e| format!("mp3 init: {e:?}"))?;
        let frame_len = encoder.samples_per_frame();
        let mut segment_cursor = 0usize;
        while segment_cursor < pcm.len() {
            let segment_end = (segment_cursor + frame_len).min(pcm.len());
            let frames = encoder
                .encode_interleaved(&pcm[segment_cursor..segment_end])
                .map_err(|e| format!("mp3 encode: {e:?}"))?;
            for f in frames {
                out.extend_from_slice(&f);
            }
            segment_cursor = segment_end;
        }
        let tail = encoder.finish().map_err(|e| format!("mp3 finish: {e:?}"))?;
        out.extend_from_slice(&tail);

        cursor = end;
    }

    fs::write(out_path, &out).map_err(|e| format!("mp3 file: {e}"))?;
    Ok(())
}

/// MP3 render path: buffer full render, normalize, then hand off to shine.
#[allow(clippy::too_many_arguments)]
fn write_render_mp3<F>(
    render: OfflineRender,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    cancel: &Arc<AtomicBool>,
    meter: &mut LoudnessTap,
    mut on_progress: F,
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    let capacity = (total_samples as usize).saturating_mul(2);
    let mut buffer: Vec<f32> = Vec::with_capacity(capacity);

    render.run(total_samples, |block| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        buffer.extend_from_slice(block);
        let frames = (buffer.len() / 2) as u64;
        on_progress((frames * 85) / 100, total_samples);
        true
    });

    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }

    let mut gain: f32 = 1.0;
    match fmt.normalize {
        NormalizeMode::Off => {}
        NormalizeMode::Peak => {
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            if peak > 1e-9 {
                gain = 10.0_f32.powf(fmt.normalize_target_db / 20.0) / peak;
            }
        }
        NormalizeMode::Lufs => {
            if let Some(lufs) = integrated_lufs(&buffer, fmt.sample_rate) {
                if lufs.is_finite() {
                    gain = 10.0_f32.powf((fmt.normalize_target_db - lufs) / 20.0);
                }
            }
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            let ceiling = 10.0_f32.powf(-1.0 / 20.0);
            if peak * gain > ceiling && peak > 1e-9 {
                gain = ceiling / peak;
            }
        }
    }

    if (gain - 1.0).abs() > f32::EPSILON {
        for s in buffer.iter_mut() {
            *s *= gain;
        }
    }

    on_progress((total_samples * 90) / 100, total_samples);
    meter.push_interleaved(&buffer);
    match fmt.mp3_vbr_quality {
        Some(quality) => {
            write_mp3_vbr_from_buffer(out_path, &buffer, fmt.sample_rate, quality)?;
        }
        None => {
            write_mp3_from_buffer(out_path, &buffer, fmt.sample_rate, fmt.mp3_bitrate_kbps)?;
        }
    }
    on_progress(total_samples, total_samples);
    Ok(true)
}

fn is_ogg_path(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|s| s.eq_ignore_ascii_case("ogg") || s.eq_ignore_ascii_case("oga"))
        .unwrap_or(false)
}

/// Encode normalized interleaved stereo `f32` samples to an Ogg Vorbis file
/// via vorbis_rs (aoTuV/Lancer-patched libvorbis). Uses QualityVbr mode so
/// the caller-provided quality in `-0.2..=1.0` maps directly to Vorbis's
/// perceptual quality scale (0.0 ≈ 64 kb/s, 0.5 ≈ 160 kb/s, 1.0 ≈ 500 kb/s).
fn write_ogg_from_buffer(
    out_path: &Path,
    buffer: &[f32],
    sample_rate: u32,
    quality: f32,
) -> Result<(), String> {
    use std::num::{NonZeroU32, NonZeroU8};
    use vorbis_rs::{VorbisBitrateManagementStrategy, VorbisEncoderBuilder};

    let sr = NonZeroU32::new(sample_rate)
        .ok_or_else(|| "ogg encode: sample rate must be non-zero".to_string())?;
    let channels = NonZeroU8::new(2).unwrap();
    let q = quality.clamp(-0.2, 1.0);

    let file = File::create(out_path).map_err(|e| format!("ogg create: {e}"))?;
    let writer = BufWriter::new(file);

    let mut encoder = VorbisEncoderBuilder::new(sr, channels, writer)
        .map_err(|e| format!("ogg init: {e}"))?
        .bitrate_management_strategy(VorbisBitrateManagementStrategy::QualityVbr {
            target_quality: q,
        })
        .build()
        .map_err(|e| format!("ogg build: {e}"))?;

    // Deinterleave in chunks to keep peak memory bounded; 8192 frames
    // (~170 ms at 48 kHz) is well above the vorbis internal block size
    // (max 2048 samples) so the analysis pipeline always has headroom.
    const CHUNK_FRAMES: usize = 8192;
    let total_frames = buffer.len() / 2;
    let mut left: Vec<f32> = Vec::with_capacity(CHUNK_FRAMES);
    let mut right: Vec<f32> = Vec::with_capacity(CHUNK_FRAMES);
    let mut i = 0;
    while i < total_frames {
        let end = (i + CHUNK_FRAMES).min(total_frames);
        left.clear();
        right.clear();
        for f in i..end {
            left.push(buffer[f * 2]);
            right.push(buffer[f * 2 + 1]);
        }
        let block: [&[f32]; 2] = [&left, &right];
        encoder
            .encode_audio_block(block)
            .map_err(|e| format!("ogg encode: {e}"))?;
        i = end;
    }
    let sink = encoder.finish().map_err(|e| format!("ogg finish: {e}"))?;
    drop(sink);
    Ok(())
}

/// OGG Vorbis render path: buffer full render, normalize, hand off to vorbis_rs.
#[allow(clippy::too_many_arguments)]
fn write_render_ogg<F>(
    render: OfflineRender,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    cancel: &Arc<AtomicBool>,
    meter: &mut LoudnessTap,
    mut on_progress: F,
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    let capacity = (total_samples as usize).saturating_mul(2);
    let mut buffer: Vec<f32> = Vec::with_capacity(capacity);

    render.run(total_samples, |block| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        buffer.extend_from_slice(block);
        let frames = (buffer.len() / 2) as u64;
        on_progress((frames * 85) / 100, total_samples);
        true
    });

    if cancel.load(Ordering::Relaxed) {
        return Ok(false);
    }

    let mut gain: f32 = 1.0;
    match fmt.normalize {
        NormalizeMode::Off => {}
        NormalizeMode::Peak => {
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            if peak > 1e-9 {
                gain = 10.0_f32.powf(fmt.normalize_target_db / 20.0) / peak;
            }
        }
        NormalizeMode::Lufs => {
            if let Some(lufs) = integrated_lufs(&buffer, fmt.sample_rate) {
                if lufs.is_finite() {
                    gain = 10.0_f32.powf((fmt.normalize_target_db - lufs) / 20.0);
                }
            }
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            let ceiling = 10.0_f32.powf(-1.0 / 20.0);
            if peak * gain > ceiling && peak > 1e-9 {
                gain = ceiling / peak;
            }
        }
    }

    if (gain - 1.0).abs() > f32::EPSILON {
        for s in buffer.iter_mut() {
            *s *= gain;
        }
    }

    on_progress((total_samples * 90) / 100, total_samples);
    meter.push_interleaved(&buffer);
    write_ogg_from_buffer(out_path, &buffer, fmt.sample_rate, fmt.ogg_quality)?;
    on_progress(total_samples, total_samples);
    Ok(true)
}

/// Render and write one offline-rendered WAV file. Returns `Ok(true)` on
/// completion, `Ok(false)` when the caller requested cancellation mid-render.
#[allow(clippy::too_many_arguments)]
fn write_render<F>(
    render: OfflineRender,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    cancel: &Arc<AtomicBool>,
    meter: &mut LoudnessTap,
    mut on_progress: F,
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    if is_flac_path(out_path) {
        return write_render_flac(
            render,
            out_path,
            fmt,
            total_samples,
            cancel,
            meter,
            on_progress,
        );
    }

    if is_mp3_path(out_path) {
        return write_render_mp3(
            render,
            out_path,
            fmt,
            total_samples,
            cancel,
            meter,
            on_progress,
        );
    }

    if is_ogg_path(out_path) {
        return write_render_ogg(
            render,
            out_path,
            fmt,
            total_samples,
            cancel,
            meter,
            on_progress,
        );
    }

    let spec = spec_for(fmt.bit_depth, fmt.sample_rate);

    match fmt.normalize {
        NormalizeMode::Off => {
            let file = File::create(out_path).map_err(|e| format!("create: {e}"))?;
            let writer = BufWriter::new(file);
            let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
            let mut wav_err: Option<String> = None;
            let mut dither = DitherState::new();
            let mut written_frames: u64 = 0;

            render.run(total_samples, |block| {
                if wav_err.is_some() || cancel.load(Ordering::Relaxed) {
                    return false;
                }
                for (i, &s) in block.iter().enumerate() {
                    meter.push(s, i & 1);
                    if let Err(e) = write_sample_dithered(
                        &mut wav,
                        s,
                        fmt.bit_depth,
                        fmt.dither,
                        &mut dither,
                        i & 1,
                    ) {
                        wav_err = Some(format!("wav write: {e}"));
                        return false;
                    }
                }
                written_frames += (block.len() / 2) as u64;
                on_progress(written_frames, total_samples);
                true
            });

            if let Some(e) = wav_err {
                return Err(e);
            }
            wav.finalize().map_err(|e| format!("finalize: {e}"))?;
        }
        NormalizeMode::Peak => {
            // Peak normalize requires two passes: first buffer all samples
            // to find the absolute peak, then scale and write. A 10-min
            // stereo render at 48 kHz buffers ~230 MB — acceptable on desktop.
            let capacity = (total_samples as usize).saturating_mul(2);
            let mut buffer: Vec<f32> = Vec::with_capacity(capacity);
            let mut peak: f32 = 0.0;

            render.run(total_samples, |block| {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                for &s in block {
                    let a = s.abs();
                    if a > peak {
                        peak = a;
                    }
                    buffer.push(s);
                }
                // Reserve half of progress for render, half for write.
                let frames = (buffer.len() / 2) as u64;
                on_progress(frames / 2, total_samples);
                true
            });

            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }

            let target_lin = 10.0_f32.powf(fmt.normalize_target_db / 20.0);
            let gain = if peak > 1e-9 { target_lin / peak } else { 1.0 };

            let file = File::create(out_path).map_err(|e| format!("create: {e}"))?;
            let writer = BufWriter::new(file);
            let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
            let mut dither = DitherState::new();
            let half = total_samples / 2;

            for (i, s) in buffer.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let scaled = *s * gain;
                meter.push(scaled, i & 1);
                write_sample_dithered(
                    &mut wav,
                    scaled,
                    fmt.bit_depth,
                    fmt.dither,
                    &mut dither,
                    i & 1,
                )
                .map_err(|e| format!("wav write: {e}"))?;
                if i % 4096 == 0 {
                    let frames = (i / 2) as u64;
                    on_progress(half + frames / 2, total_samples);
                }
            }
            wav.finalize().map_err(|e| format!("finalize: {e}"))?;
            on_progress(total_samples, total_samples);
        }
        NormalizeMode::Lufs => {
            // LUFS normalize mirrors the peak two-pass approach: render first,
            // then measure BS.1770 integrated loudness and scale to target. A
            // hard peak ceiling of -1 dBFS after scaling prevents overshoot on
            // tracks with high peak-to-loudness ratios (drum stems, sfx).
            let capacity = (total_samples as usize).saturating_mul(2);
            let mut buffer: Vec<f32> = Vec::with_capacity(capacity);

            render.run(total_samples, |block| {
                if cancel.load(Ordering::Relaxed) {
                    return false;
                }
                buffer.extend_from_slice(block);
                let frames = (buffer.len() / 2) as u64;
                on_progress(frames / 2, total_samples);
                true
            });

            if cancel.load(Ordering::Relaxed) {
                return Ok(false);
            }

            let measured = integrated_lufs(&buffer, fmt.sample_rate);
            let mut gain = match measured {
                Some(lufs) if lufs.is_finite() => {
                    let delta_db = fmt.normalize_target_db - lufs;
                    10.0_f32.powf(delta_db / 20.0)
                }
                _ => 1.0,
            };

            // Clamp so post-gain peaks stay below -1 dBFS true-peak budget.
            let mut peak: f32 = 0.0;
            for &s in buffer.iter() {
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
            }
            let peak_ceiling = 10.0_f32.powf(-1.0 / 20.0);
            if peak * gain > peak_ceiling && peak > 1e-9 {
                gain = peak_ceiling / peak;
            }

            let file = File::create(out_path).map_err(|e| format!("create: {e}"))?;
            let writer = BufWriter::new(file);
            let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
            let mut dither = DitherState::new();
            let half = total_samples / 2;

            for (i, s) in buffer.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(false);
                }
                let scaled = *s * gain;
                meter.push(scaled, i & 1);
                write_sample_dithered(
                    &mut wav,
                    scaled,
                    fmt.bit_depth,
                    fmt.dither,
                    &mut dither,
                    i & 1,
                )
                .map_err(|e| format!("wav write: {e}"))?;
                if i % 4096 == 0 {
                    let frames = (i / 2) as u64;
                    on_progress(half + frames / 2, total_samples);
                }
            }
            wav.finalize().map_err(|e| format!("finalize: {e}"))?;
            on_progress(total_samples, total_samples);
        }
    }

    let completed = !cancel.load(Ordering::Relaxed);
    Ok(completed)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn export_project_wav(
    app: AppHandle,
    path: String,
    sample_rate: u32,
    bit_depth: u32,
    tail_secs: f32,
    start_samples: Option<u64>,
    end_samples: Option<u64>,
    normalize_mode: Option<String>,
    normalize_target_db: Option<f32>,
    dither_mode: Option<String>,
    mp3_bitrate_kbps: Option<u32>,
    mp3_vbr_quality: Option<u32>,
    ogg_quality: Option<f32>,
) -> Result<ExportResult, String> {
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);

    let fmt = RenderFormat {
        sample_rate,
        bit_depth,
        normalize: NormalizeMode::parse(normalize_mode.as_deref().unwrap_or("off")),
        normalize_target_db: normalize_target_db.unwrap_or(-1.0),
        dither: DitherMode::parse(dither_mode.as_deref().unwrap_or("none")),
        mp3_bitrate_kbps: mp3_bitrate_kbps.unwrap_or(320).clamp(8, 320),
        mp3_vbr_quality: mp3_vbr_quality.map(|q| q.min(9)),
        ogg_quality: ogg_quality.unwrap_or(0.5).clamp(-0.2, 1.0),
    };

    let out_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ExportResult, String> {
        let engine_guard = engine.lock();

        let body_samples = engine_guard.project_end_samples(sample_rate);
        if body_samples == 0 {
            return Err("Project is empty — nothing to export.".into());
        }
        let tail_samples = ((tail_secs.max(0.0) as f64) * sample_rate as f64) as u64;
        let (render_start, body_len) = match (start_samples, end_samples) {
            (Some(s), Some(e)) if e > s => (s, e - s),
            _ => (0, body_samples),
        };
        let total_samples = body_len + tail_samples;
        let range_end = (render_start + body_len < body_samples).then_some(render_start + body_len);

        let mut last_pct: i32 = -1;
        let app_for_progress = app.clone();
        let loudness = render_unlocked_measured(
            engine_guard,
            Path::new(&out_path),
            fmt,
            total_samples,
            render_start,
            &cancel,
            |written, total| {
                let pct_i = ((written as f64 / total as f64) * 100.0) as i32;
                if pct_i != last_pct {
                    last_pct = pct_i;
                    let _ = app_for_progress.emit(
                        "export-progress",
                        ExportProgress {
                            percent: pct_i as f32,
                            label: None,
                        },
                    );
                }
            },
            false,
            |proj: &mut Project| {
                if let Some(end) = range_end {
                    end_the_song_at(proj, end, sample_rate);
                }
            },
        )?;
        let completed = loudness.is_some();

        if !completed {
            let _ = fs::remove_file(&out_path);
        }

        Ok(ExportResult {
            path: out_path,
            samples: total_samples,
            duration_secs: total_samples as f64 / sample_rate as f64,
            cancelled: !completed,
            loudness,
        })
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    Ok(result)
}

#[tauri::command]
pub fn cancel_export(app: AppHandle) {
    let state: State<AppState> = app.state();
    state.export_cancel.store(true, Ordering::Relaxed);
}

fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' || ch == ' ' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim().trim_matches('.');
    if trimmed.is_empty() {
        "track".into()
    } else {
        trimmed.to_string()
    }
}

/// End the song at the end of an exported range, so the tail rendered after
/// it is only what was still sounding there: a reverb, a delay, notes
/// letting go. The tail used to play on into the song, so a loop exported
/// with the usual two seconds of tail carried the first beats of the next
/// bar. A clip running past the end is cut there, its notes let go there.
fn end_the_song_at(proj: &mut Project, end_samples: u64, sample_rate: u32) {
    use hardwave_project::clip::ClipContent;
    let end = proj
        .tempo_map
        .samples_to_tick(end_samples, sample_rate as f64);
    for t in proj.tracks.iter_mut() {
        t.clips.retain(|c| c.position_ticks < end);
        for c in t.clips.iter_mut() {
            let keep = end - c.position_ticks;
            if c.length_ticks <= keep {
                continue;
            }
            match &mut c.content {
                // A reversed clip plays from its end backwards, so a shorter
                // one would play other audio inside the range. It rings on.
                ClipContent::Audio(a) if a.reversed => continue,
                // Cut with the clip's short declick rather than a fade-out
                // that would now start inside the range.
                ClipContent::Audio(a) => a.fade_out_ticks = 0,
                // Notes drawn past a clip's end can still play, so the
                // cut is made on the notes themselves.
                ClipContent::Midi(m) => {
                    m.clip.notes.retain(|n| n.start_tick < keep);
                    for n in m.clip.notes.iter_mut() {
                        n.duration_ticks = n.duration_ticks.min(keep - n.start_tick);
                    }
                    m.clip.controls.retain(|k| k.tick < keep);
                }
            }
            c.length_ticks = keep;
        }
    }
}

/// Whether a track gets a stem: it plays something of its own, from its
/// clips or from notes another track routes to it. Every track used to get
/// one, so a new song's 500 empty inserts and the master track each took a
/// full-length render, and the folder filled with hundreds of silent files.
/// The master has a stem of its own to tick.
fn has_a_stem(project: &Project, track: &hardwave_project::track::Track) -> bool {
    use hardwave_project::clip::FREEZE_LANE;
    if !track.kind.is_audio_bearing() {
        return false;
    }
    // A frozen track plays its render and nothing else; a live one plays
    // everything but the render.
    let plays = |t: &hardwave_project::track::Track| {
        t.clips.iter().any(|c| (c.lane == FREEZE_LANE) == t.frozen)
    };
    plays(track)
        || project
            .tracks
            .iter()
            .any(|s| s.id != track.id && s.midi_route_to.contains(&track.id) && plays(s))
}

/// Narrow a stem's copy of the song to one track's own sound, on its whole
/// way out: through the buses it plays into and the returns it sends to, on
/// to the master. The buses and returns on the way carry only this track,
/// not their own clips, and every other track is excluded.
///
/// Exclude rather than mute. A muted track stops feeding any sidechain keyed
/// off it, so muting the kick to render the bass stem produced a stem with
/// no ducking while the full mix ducked. Excluded tracks keep running and
/// still key sidechains; they just don't reach the master and their sends
/// are dropped. This also keeps the producer's own mute state.
///
/// The buses and returns used to be excluded too, so a track routed into a
/// bus came out silent (the bus never reached the master) and no stem had
/// the reverb its sends fed. Each stem now carries its own share of them,
/// so the stems add up to the mix.
fn isolate_for_stem(proj: &mut Project, target: &str) {
    let way_out = hardwave_project::mixer::way_out(&proj.tracks, target);
    for t in proj.tracks.iter_mut() {
        if t.id == target {
            t.stem_excluded = false;
        } else if way_out.contains(&t.id) {
            t.stem_excluded = false;
            t.clips.clear();
        } else {
            t.stem_excluded = true;
        }
        // Notes another track routes to a bus on the way would play there.
        if t.id != target {
            t.midi_route_to.retain(|to| !way_out.contains(to));
        }
    }
}

/// Where the song's last clip ends, in ticks: how far a bounce or a freeze
/// renders.
fn song_end_ticks(project: &Project) -> u64 {
    project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.position_ticks + c.length_ticks)
        .max()
        .unwrap_or(0)
}

/// Give a rendered clip the length, in ticks, of the stretch of song it was
/// rendered from. Importing the file works its length out from its duration
/// at one tempo (the one under the playhead), so in a song with a tempo
/// change a frozen or bounced part came out short and went silent before
/// the end of the song, or ran on past it.
fn fit_render_clip(
    project: &mut Project,
    track_id: &str,
    clip_id: &str,
    start_ticks: u64,
    end_ticks: u64,
) {
    use hardwave_project::clip::ClipContent;
    let Some(track) = project.track_mut(track_id) else {
        return;
    };
    for placement in track.clips.iter_mut() {
        if matches!(&placement.content, ClipContent::Audio(ac) if ac.id == clip_id) {
            placement.position_ticks = start_ticks;
            placement.length_ticks = end_ticks.saturating_sub(start_ticks);
        }
    }
}

/// Glue a track's clips between two points into one audio clip.
///
/// Several takes, chopped and rearranged, are fine to work with and awkward
/// to move: dragging the arrangement means dragging fifteen pieces and
/// hoping none slipped. Consolidating renders that stretch of the track,
/// with its plug-ins, into one clip and replaces the pieces with it.
///
/// Returns the path of the rendered file.
#[tauri::command]
pub async fn consolidate_track_range(
    app: AppHandle,
    track_id: String,
    start_ticks: u64,
    end_ticks: u64,
) -> Result<String, String> {
    if end_ticks <= start_ticks {
        return Err("select a range with some length in it".into());
    }
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);

    let source_track_id = track_id.clone();
    let rendered =
        tauri::async_runtime::spawn_blocking(move || -> Result<(String, u64, u64), String> {
            let engine_guard = engine.lock();
            let sample_rate = engine_guard.current_sample_rate();
            let (start_samples, end_samples, track_name) = {
                let project = engine_guard.project.lock();
                let name = project
                    .track(&track_id)
                    .map(|t| t.name.clone())
                    .ok_or_else(|| format!("Track not found: {track_id}"))?;
                let map = &project.tempo_map;
                (
                    map.tick_to_samples(start_ticks, sample_rate as f64),
                    map.tick_to_samples(end_ticks, sample_rate as f64),
                    name,
                )
            };
            let total_samples = end_samples.saturating_sub(start_samples);
            if total_samples == 0 {
                return Err("that range is empty".into());
            }

            let dir = match engine_guard.project_dir() {
                Some(d) => d.join("Bounces"),
                None => dirs::data_dir()
                    .ok_or_else(|| "No writable folder for bounces on this system".to_string())?
                    .join("Hardwave")
                    .join("Bounces"),
            };
            fs::create_dir_all(&dir).map_err(|e| format!("create folder: {e}"))?;
            let safe: String = track_name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == ' ' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
            let out_path = dir.join(format!("{safe} consolidated {stamp}.wav"));

            let fmt = format_for_a_track(sample_rate);

            let target = track_id.clone();
            let instruments = instrument_ids(&engine_guard);
            let completed = render_unlocked(
                engine_guard,
                &out_path,
                fmt,
                total_samples,
                start_samples,
                &cancel,
                |_, _| {},
                true,
                // The glued clip stays on this track, through its effects
                // and fader and on to the master: the render has none of
                // them, only the instrument that plays its notes.
                move |proj: &mut Project| {
                    isolate_track_for_render(proj, &target, false, &|id| instruments.contains(id));
                    leave_the_fader_out(proj, &target);
                },
            )?;
            if !completed {
                let _ = fs::remove_file(&out_path);
                return Err("consolidating was cancelled".into());
            }
            Ok((
                out_path.to_string_lossy().into_owned(),
                start_samples,
                end_samples,
            ))
        })
        .await
        .map_err(|e| e.to_string())??;

    let (path, _, _) = rendered;

    // Replacing the pieces with the render is one step, so one undo puts
    // the original clips back.
    let state: State<AppState> = app.state();
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&source_track_id) {
            // Only what lies inside the range goes: a clip that merely
            // overlaps the edge is left alone rather than silently cut.
            track.clips.retain(|c| {
                let clip_end = c.position_ticks + c.length_ticks;
                !(c.position_ticks >= start_ticks && clip_end <= end_ticks)
            });
        }
    }
    let clip = crate::commands::audio::import_audio_file_blocking(
        state.clone(),
        source_track_id.clone(),
        path.clone(),
        Some(start_ticks),
    )?;
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        fit_render_clip(
            &mut project,
            &source_track_id,
            &clip.clip_id,
            start_ticks,
            end_ticks,
        );
    }
    state.engine.lock().rebuild_graph();
    Ok(path)
}

/// Render one track through its plug-ins to audio, put the result on a new/// Render one track through its plug-ins to audio, put the result on a new
/// track, and mute the original.
///
/// A track with three plug-ins on it costs that CPU on every block, for the
/// whole song, whether or not the part is finished. Every DAW lets you turn
/// a finished part into audio to get that back. The engine has had a freeze
/// registry since it was written, with nothing wired to it; this does the
/// half that is honest to build blind, using the same render path the stem
/// export already uses in anger.
///
/// Returns the path of the rendered file.
#[tauri::command]
pub async fn bounce_track_to_audio(app: AppHandle, track_id: String) -> Result<String, String> {
    // The render runs on a blocking thread and takes its own copy of the id.
    let source_track_id = track_id.clone();
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);
    // How far the render goes, in ticks, for the clip that plays it.
    let end_ticks = song_end_ticks(&engine.lock().project.lock());

    let rendered =
        tauri::async_runtime::spawn_blocking(move || -> Result<(String, String), String> {
            let engine_guard = engine.lock();
            let sample_rate = engine_guard.current_sample_rate();
            let total_samples = engine_guard.project_end_samples(sample_rate);
            if total_samples == 0 {
                return Err("there is nothing on the timeline to bounce".into());
            }

            let track_name = {
                let project = engine_guard.project.lock();
                project
                    .track(&track_id)
                    .map(|t| t.name.clone())
                    .ok_or_else(|| format!("Track not found: {track_id}"))?
            };

            // Beside the project's takes, so a bounced part travels with the
            // song the way a recording does.
            let dir = match engine_guard.project_dir() {
                Some(d) => d.join("Bounces"),
                None => dirs::data_dir()
                    .ok_or_else(|| "No writable folder for bounces on this system".to_string())?
                    .join("Hardwave")
                    .join("Bounces"),
            };
            fs::create_dir_all(&dir).map_err(|e| format!("create folder: {e}"))?;
            let safe: String = track_name
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == ' ' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
            let out_path = dir.join(format!("{safe} {stamp}.wav"));

            let fmt = format_for_a_track(sample_rate);

            let target = track_id.clone();
            let completed = render_unlocked(
                engine_guard,
                &out_path,
                fmt,
                total_samples,
                0,
                &cancel,
                |_, _| {},
                true,
                // Through its own effects and fader, which the new track
                // does without, and not through its bus or the master,
                // which the new track plays into again.
                move |proj: &mut Project| isolate_track_for_render(proj, &target, true, &|_| true),
            )?;
            if !completed {
                let _ = fs::remove_file(&out_path);
                return Err("the bounce was cancelled".into());
            }
            Ok((out_path.to_string_lossy().into_owned(), track_name))
        })
        .await
        .map_err(|e| e.to_string())??;

    let (path, track_name) = rendered;

    // Placing it is one undo step together with muting the original.
    let state: State<AppState> = app.state();
    state.engine.lock().snapshot_before_mutation();
    let new_track_id = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        // Into the same bus as the original, so the bus's effects still
        // shape it; the render left them out.
        let bus = project
            .track(&source_track_id)
            .and_then(|t| t.output_bus.clone());
        let id = project.add_audio_track(format!("{track_name} (bounced)"));
        if let Some(t) = project.track_mut(&id) {
            t.output_bus = bus;
        }
        id
    };
    let clip = crate::commands::audio::import_audio_file_blocking(
        state.clone(),
        new_track_id.clone(),
        path.clone(),
        Some(0),
    )?;
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        fit_render_clip(&mut project, &new_track_id, &clip.clip_id, 0, end_ticks);
        if let Some(t) = project.track_mut(&source_track_id) {
            t.muted = true;
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(path)
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn export_project_stems(
    app: AppHandle,
    folder_path: String,
    project_name: String,
    sample_rate: u32,
    bit_depth: u32,
    tail_secs: f32,
    include_master: bool,
    respect_mute_solo: bool,
    start_samples: Option<u64>,
    end_samples: Option<u64>,
    normalize_mode: Option<String>,
    normalize_target_db: Option<f32>,
    dither_mode: Option<String>,
    stem_format: Option<String>,
    mp3_bitrate_kbps: Option<u32>,
    mp3_vbr_quality: Option<u32>,
    ogg_quality: Option<f32>,
) -> Result<StemsExportResult, String> {
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);

    let fmt = RenderFormat {
        sample_rate,
        bit_depth,
        normalize: NormalizeMode::parse(normalize_mode.as_deref().unwrap_or("off")),
        normalize_target_db: normalize_target_db.unwrap_or(-1.0),
        dither: DitherMode::parse(dither_mode.as_deref().unwrap_or("none")),
        mp3_bitrate_kbps: mp3_bitrate_kbps.unwrap_or(320).clamp(8, 320),
        mp3_vbr_quality: mp3_vbr_quality.map(|q| q.min(9)),
        ogg_quality: ogg_quality.unwrap_or(0.5).clamp(-0.2, 1.0),
    };

    let stem_ext: &str = match stem_format.as_deref().unwrap_or("wav") {
        "flac" => "flac",
        "mp3" => "mp3",
        "ogg" => "ogg",
        _ => "wav",
    };

    let result =
        tauri::async_runtime::spawn_blocking(move || -> Result<StemsExportResult, String> {
            let engine_guard = engine.lock();

            let body_samples = engine_guard.project_end_samples(sample_rate);
            if body_samples == 0 {
                return Err("Project is empty — nothing to export.".into());
            }
            let tail_samples = ((tail_secs.max(0.0) as f64) * sample_rate as f64) as u64;
            let (render_start, body_len) = match (start_samples, end_samples) {
                (Some(s), Some(e)) if e > s => (s, e - s),
                _ => (0, body_samples),
            };
            let total_samples = body_len + tail_samples;
            let range_end =
                (render_start + body_len < body_samples).then_some(render_start + body_len);

            let folder = PathBuf::from(&folder_path);
            fs::create_dir_all(&folder).map_err(|e| format!("create folder: {e}"))?;

            let track_infos: Vec<(String, String)> = {
                let project = engine_guard.project.lock();
                project
                    .tracks
                    .iter()
                    .filter(|t| has_a_stem(&project, t))
                    .map(|t| (t.id.clone(), t.name.clone()))
                    .collect()
            };
            if track_infos.is_empty() {
                return Err("No tracks to export.".into());
            }
            // Each stem takes the lock only while it is prepared.
            drop(engine_guard);

            let project_slug = sanitize_filename(&project_name);
            let mut files: Vec<String> = Vec::new();
            let total_renders = track_infos.len() + if include_master { 1 } else { 0 };
            let mut render_idx: usize = 0;
            let mut cancelled = false;

            for (track_id, track_name) in track_infos.iter() {
                if cancel.load(Ordering::Relaxed) {
                    cancelled = true;
                    break;
                }
                let stem_name = format!(
                    "{}_{}.{}",
                    project_slug,
                    sanitize_filename(track_name),
                    stem_ext
                );
                let stem_path = folder.join(&stem_name);
                let mut last_pct: i32 = -1;
                let app_for_progress = app.clone();
                let target_id = track_id.clone();
                let label = track_name.clone();
                let idx = render_idx;

                let completed = render_unlocked(
                    engine.lock(),
                    &stem_path,
                    fmt,
                    total_samples,
                    render_start,
                    &cancel,
                    |written, total| {
                        let pct_i = ((written as f64 / total as f64) * 100.0) as i32;
                        if pct_i != last_pct {
                            last_pct = pct_i;
                            let inner = written as f64 / total as f64;
                            let overall = ((idx as f64 + inner) / total_renders as f64) * 100.0;
                            let _ = app_for_progress.emit(
                                "export-progress",
                                ExportProgress {
                                    percent: overall as f32,
                                    label: Some(label.clone()),
                                },
                            );
                        }
                    },
                    false,
                    |proj: &mut Project| {
                        isolate_for_stem(proj, &target_id);
                        if !respect_mute_solo {
                            for t in proj.tracks.iter_mut() {
                                t.soloed = false;
                                t.solo_safe = false;
                                t.muted = false;
                            }
                        }
                        if let Some(end) = range_end {
                            end_the_song_at(proj, end, sample_rate);
                        }
                    },
                )?;

                if !completed {
                    let _ = fs::remove_file(&stem_path);
                    cancelled = true;
                    break;
                }

                files.push(stem_path.to_string_lossy().into_owned());
                render_idx += 1;
            }

            if !cancelled && include_master {
                let master_name = format!("{}_master.{}", project_slug, stem_ext);
                let master_path = folder.join(&master_name);
                let mut last_pct: i32 = -1;
                let app_for_progress = app.clone();
                let idx = render_idx;

                let completed = render_unlocked(
                    engine.lock(),
                    &master_path,
                    fmt,
                    total_samples,
                    render_start,
                    &cancel,
                    |written, total| {
                        let pct_i = ((written as f64 / total as f64) * 100.0) as i32;
                        if pct_i != last_pct {
                            last_pct = pct_i;
                            let inner = written as f64 / total as f64;
                            let overall = ((idx as f64 + inner) / total_renders as f64) * 100.0;
                            let _ = app_for_progress.emit(
                                "export-progress",
                                ExportProgress {
                                    percent: overall as f32,
                                    label: Some("master".into()),
                                },
                            );
                        }
                    },
                    false,
                    |proj: &mut Project| {
                        if let Some(end) = range_end {
                            end_the_song_at(proj, end, sample_rate);
                        }
                        if !respect_mute_solo {
                            for t in proj.tracks.iter_mut() {
                                t.muted = false;
                                t.soloed = false;
                                t.solo_safe = false;
                            }
                        }
                    },
                )?;

                if !completed {
                    let _ = fs::remove_file(&master_path);
                    cancelled = true;
                } else {
                    files.push(master_path.to_string_lossy().into_owned());
                }
            }

            let _ = app.emit(
                "export-progress",
                ExportProgress {
                    percent: 100.0,
                    label: None,
                },
            );

            Ok(StemsExportResult {
                folder: folder_path,
                files,
                duration_secs: total_samples as f64 / sample_rate as f64,
                cancelled,
            })
        })
        .await
        .map_err(|e| format!("join: {e}"))??;

    Ok(result)
}

/// Freeze a track: play a render of it instead of its clips and plug-ins.
///
/// Bouncing gives the CPU back but replaces the part with audio on a new
/// track. Freezing leaves the part, the plug-ins and the automation
/// exactly where they are and plays a render instead, so unfreezing costs
/// nothing but the CPU it gives back.
///
/// The render is a clip on the track's freeze lane, so it loads with the
/// project like any other audio and travels with the song.
#[tauri::command]
pub async fn freeze_track(app: AppHandle, track_id: String) -> Result<String, String> {
    let source_track_id = track_id.clone();
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);
    // How far the render goes, in ticks, for the clip that plays it.
    let end_ticks = song_end_ticks(&engine.lock().project.lock());

    let rendered = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let engine_guard = engine.lock();
        let sample_rate = engine_guard.current_sample_rate();
        let total_samples = engine_guard.project_end_samples(sample_rate);
        if total_samples == 0 {
            return Err("there is nothing on the timeline to freeze".into());
        }
        let track_name = {
            let project = engine_guard.project.lock();
            let track = project
                .track(&track_id)
                .ok_or_else(|| format!("Track not found: {track_id}"))?;
            if track.frozen {
                return Err("that track is already frozen".into());
            }
            track.name.clone()
        };

        let dir = match engine_guard.project_dir() {
            Some(d) => d.join("Freeze"),
            None => dirs::data_dir()
                .ok_or_else(|| "No writable folder for freezes on this system".to_string())?
                .join("Hardwave")
                .join("Freeze"),
        };
        fs::create_dir_all(&dir).map_err(|e| format!("create folder: {e}"))?;
        let safe: String = track_name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S");
        let out_path = dir.join(format!("{safe} frozen {stamp}.wav"));

        let fmt = format_for_a_track(sample_rate);

        let target = track_id.clone();
        let completed = render_unlocked(
            engine_guard,
            &out_path,
            fmt,
            total_samples,
            0,
            &cancel,
            |_, _| {},
            true,
            // Through its own effects, which rest while it is frozen, and
            // not through its fader, its bus or the master, which it still
            // plays through.
            move |proj: &mut Project| {
                isolate_track_for_render(proj, &target, true, &|_| true);
                leave_the_fader_out(proj, &target);
            },
        )?;
        if !completed {
            let _ = fs::remove_file(&out_path);
            return Err("the freeze was cancelled".into());
        }
        Ok(out_path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())??;

    // Placing the render and flipping the switch are one undo step.
    let state: State<AppState> = app.state();
    state.engine.lock().snapshot_before_mutation();
    let clip = crate::commands::audio::import_audio_file_blocking(
        state.clone(),
        source_track_id.clone(),
        rendered.clone(),
        Some(0),
    )?;
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        fit_render_clip(&mut project, &source_track_id, &clip.clip_id, 0, end_ticks);
        if let Some(track) = project.track_mut(&source_track_id) {
            place_freeze_render(track, &clip.clip_id);
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(rendered)
}

/// Move a freeze render onto the track's freeze lane and freeze the track.
/// The render an earlier freeze left there goes: a frozen track plays every
/// clip on that lane, so freezing, unfreezing and freezing again played the
/// old render and the new one together, crossfading from one to the other
/// across the whole song.
fn place_freeze_render(track: &mut hardwave_project::track::Track, clip_id: &str) {
    use hardwave_project::clip::{ClipContent, FREEZE_LANE};
    track
        .clips
        .retain(|placement| placement.lane != FREEZE_LANE);
    for placement in track.clips.iter_mut() {
        if matches!(&placement.content, ClipContent::Audio(ac) if ac.id == clip_id) {
            placement.lane = FREEZE_LANE;
        }
    }
    track.frozen = true;
}

/// Put the live chain back. The render stays on the freeze lane until the
/// track is frozen again.
#[tauri::command]
pub fn unfreeze_track(state: State<AppState>, track_id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        if !track.frozen {
            return Err("that track is not frozen".into());
        }
        track.frozen = false;
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Which tracks are frozen, for the app to show.
#[tauri::command]
pub fn list_frozen_tracks(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .tracks
        .iter()
        .filter(|t| t.frozen)
        .map(|t| t.id.clone())
        .collect()
}

#[cfg(test)]
mod isolate_tests {
    use super::isolate_track_for_render;
    use hardwave_project::track::{PluginSlot, TrackKind};
    use hardwave_project::Project;

    fn slot(id: &str) -> PluginSlot {
        PluginSlot {
            id: id.into(),
            plugin_id: id.into(),
            enabled: true,
            state: None,
            sidechain_source: None,
            wet: 1.0,
        }
    }

    /// A kick track with a synth and an EQ, turned down and panned, into a
    /// bus; a limiter and a lowered fader on the master.
    fn song() -> (Project, String) {
        let mut p = Project::default();
        let master = p
            .tracks
            .iter_mut()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .expect("a master");
        master.inserts.push(slot("fx.limiter"));
        master.volume_db = -3.0;
        let id = p.add_midi_track("Kick".into());
        let t = p.track_mut(&id).expect("the kick");
        t.inserts = vec![slot("synth.kick"), slot("fx.eq")];
        t.volume_db = -6.0;
        t.pan = 0.3;
        t.output_bus = Some("insert-001".into());
        (p, id)
    }

    fn master(p: &Project) -> &hardwave_project::track::Track {
        p.tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .expect("a master")
    }

    #[test]
    fn a_consolidate_keeps_only_the_instrument_and_plays_straight_into_a_bare_master() {
        let (mut p, id) = song();
        {
            let t = p.track_mut(&id).expect("the kick");
            t.phase_invert = true;
            t.swap_lr = true;
            t.delay_samples = 64;
            t.stereo_separation = 0.5;
            t.filter_type = "lp".into();
        }
        isolate_track_for_render(&mut p, &id, false, &|pid| pid.starts_with("synth."));
        super::leave_the_fader_out(&mut p, &id);
        let t = p.track(&id).expect("the kick");
        let kept: Vec<&str> = t.inserts.iter().map(|s| s.plugin_id.as_str()).collect();
        assert_eq!(kept, ["synth.kick"]);
        assert_eq!((t.volume_db, t.pan, t.output_bus.clone()), (0.0, 0.0, None));
        assert_eq!(
            (
                t.phase_invert,
                t.swap_lr,
                t.delay_samples,
                t.stereo_separation,
                t.filter_type.as_str()
            ),
            (false, false, 0, 1.0, "off"),
            "the glued clip plays through the channel's own settings on the track"
        );
        assert!(!t.stem_excluded);
        assert!(master(&p).inserts.is_empty());
        assert_eq!(master(&p).volume_db, 0.0);
        assert!(
            p.tracks
                .iter()
                .filter(|t| !matches!(t.kind, TrackKind::Master) && t.id != id)
                .all(|t| t.stem_excluded),
            "every other track is left out of the mix"
        );
    }

    #[test]
    fn a_bounce_keeps_the_track_s_own_effects_and_fader() {
        let (mut p, id) = song();
        isolate_track_for_render(&mut p, &id, true, &|_| false);
        let t = p.track(&id).expect("the kick");
        assert_eq!(t.inserts.len(), 2);
        assert_eq!(
            (t.volume_db, t.pan, t.output_bus.clone()),
            (-6.0, 0.3, None)
        );
        assert!(master(&p).inserts.is_empty());
    }

    #[test]
    fn a_freeze_keeps_the_effects_and_leaves_the_fader_and_pan_to_the_frozen_track() {
        use hardwave_project::automation::{AutomationLane, AutomationTarget};
        use hardwave_project::modulation::{ModRoute, ModSource, ModTarget};
        use hardwave_project::vca::Vca;
        let (mut p, id) = song();
        let lane = |target: AutomationTarget| AutomationLane {
            id: format!("{target:?}"),
            target,
            points: Vec::new(),
            visible: true,
            row: None,
        };
        let t = p.track_mut(&id).expect("the kick");
        t.automation_lanes = vec![
            lane(AutomationTarget::TrackVolume),
            lane(AutomationTarget::TrackPan),
            lane(AutomationTarget::TrackMute),
            lane(AutomationTarget::PluginParam {
                slot_id: "fx.eq".into(),
                param_id: 0,
            }),
        ];
        let mut group = Vca::new("v".into(), "Drums".into());
        group.gain_db = -4.0;
        group.members = vec![id.clone(), "insert-002".into()];
        p.vcas.push(group);
        let route = |target: ModTarget| ModRoute {
            id: "r".into(),
            name: "r".into(),
            enabled: true,
            source: ModSource::MidiCc { cc: 1 },
            track_id: id.clone(),
            target,
            center: 0.5,
            depth: 0.5,
        };
        p.modulations = vec![
            route(ModTarget::TrackVolume),
            route(ModTarget::PluginParam {
                slot_id: "fx.eq".into(),
                param_id: 0,
            }),
        ];

        isolate_track_for_render(&mut p, &id, true, &|_| false);
        super::leave_the_fader_out(&mut p, &id);

        let t = p.track(&id).expect("the kick");
        assert_eq!(t.inserts.len(), 2, "the effects are in the render");
        assert_eq!((t.volume_db, t.pan), (0.0, 0.0));
        let lanes: Vec<&str> = t.automation_lanes.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            lanes,
            [
                "TrackMute",
                "PluginParam { slot_id: \"fx.eq\", param_id: 0 }"
            ],
            "the plug-in's automation stays in the render, the fader's and pan's do not"
        );
        assert_eq!(p.vcas[0].members, ["insert-002".to_string()]);
        assert_eq!(p.modulations.len(), 1);
        assert!(matches!(
            p.modulations[0].target,
            ModTarget::PluginParam { .. }
        ));
    }
}

#[cfg(test)]
mod stem_and_freeze_tests {
    use super::has_a_stem;
    use super::isolate_for_stem;
    use hardwave_engine::{AudioBuffer, DawEngine};
    use hardwave_project::clip::FREEZE_LANE;
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement};
    use hardwave_project::mixer::Send;
    use hardwave_project::Project;

    const SR: u32 = 48_000;
    const LEN_TICKS: u64 = 1920;

    fn audio(id: &str, source: &str, frames: u64, track: &str) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: id.into(),
                name: id.into(),
                source_path: source.into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: frames,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                source_file: String::new(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: track.into(),
            position_ticks: 0,
            length_ticks: LEN_TICKS,
            lane: 0,
            color: None,
            row: None,
        }
    }

    fn midi(id: &str, track: &str) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(hardwave_project::clip::MidiClipRef {
                id: id.into(),
                clip: hardwave_midi::MidiClip::new(id.into(), id.into(), LEN_TICKS),
            }),
            track_id: track.into(),
            position_ticks: 0,
            length_ticks: LEN_TICKS,
            lane: 0,
            color: None,
            row: None,
        }
    }

    fn send(target: &str) -> Send {
        Send {
            target: target.into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: true,
        }
    }

    #[test]
    fn freezing_again_replaces_the_render_the_last_freeze_left() {
        let mut p = Project::default();
        let id = p.add_audio_track("Vox".into());
        let t = p.track_mut(&id).unwrap();
        t.clips.push(audio("take", "take", 100, &id));
        let mut old = audio("old-render", "old", 100, &id);
        old.lane = FREEZE_LANE;
        t.clips.push(old);
        // Unfrozen, then frozen again: the new render lands on lane 0 first.
        t.clips.push(audio("new-render", "new", 100, &id));

        super::place_freeze_render(t, "new-render");

        assert!(t.frozen);
        let on_lane: Vec<(&str, u32)> = t
            .clips
            .iter()
            .map(|c| match &c.content {
                ClipContent::Audio(a) => (a.id.as_str(), c.lane),
                ClipContent::Midi(m) => (m.id.as_str(), c.lane),
            })
            .collect();
        assert_eq!(
            on_lane,
            [("take", 0), ("new-render", FREEZE_LANE)],
            "one render on the freeze lane, the part where it was"
        );
    }

    #[test]
    fn only_tracks_that_play_something_get_a_stem() {
        let mut p = Project::default();
        let vox = p.add_audio_track("Vox".into());
        p.track_mut(&vox)
            .unwrap()
            .clips
            .push(audio("v", "v", 100, &vox));
        let lead = p.add_midi_track("Lead".into());
        let layer = p.add_midi_track("Layer".into());
        {
            let t = p.track_mut(&lead).unwrap();
            t.clips.push(midi("m", &lead));
            t.midi_route_to.push(layer.clone());
        }
        let frozen = p.add_audio_track("Frozen".into());
        {
            let t = p.track_mut(&frozen).unwrap();
            let mut render = audio("r", "r", 100, &frozen);
            render.lane = FREEZE_LANE;
            t.clips.push(render);
            t.frozen = true;
        }
        let bus = p.add_audio_track("Bus".into());
        p.track_mut(&vox).unwrap().output_bus = Some(bus.clone());

        let stems: Vec<&str> = p
            .tracks
            .iter()
            .filter(|t| has_a_stem(&p, t))
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(
            stems,
            ["Vox", "Lead", "Layer", "Frozen"],
            "not the master, the 500 empty inserts or the empty bus"
        );
    }

    #[test]
    fn a_stem_goes_out_through_its_bus_and_the_returns_it_sends_to() {
        let mut p = Project::default();
        let bus = p.add_audio_track("Bus".into());
        let verb = p.add_audio_track("Verb".into());
        let lead = p.add_audio_track("Lead".into());
        let pad = p.add_audio_track("Pad".into());
        let other = p.add_midi_track("Other".into());
        for (id, clip) in [(&lead, "l"), (&pad, "p"), (&bus, "b")] {
            p.track_mut(id)
                .unwrap()
                .clips
                .push(audio(clip, clip, 100, id));
        }
        {
            let t = p.track_mut(&lead).unwrap();
            t.output_bus = Some(bus.clone());
            t.sends.push(send(&verb));
        }
        p.track_mut(&pad).unwrap().output_bus = Some(bus.clone());
        p.track_mut(&other).unwrap().midi_route_to.push(bus.clone());

        isolate_for_stem(&mut p, &lead);

        let t = |id: &str| p.track(id).unwrap();
        assert!(!t(&lead).stem_excluded);
        assert!(
            !t(&bus).stem_excluded && !t(&verb).stem_excluded,
            "the bus and the return carry the lead to the master"
        );
        assert!(t(&bus).clips.is_empty(), "with nothing of the bus's own");
        assert!(t(&pad).stem_excluded, "the pad is not in the lead's stem");
        assert!(
            t(&other).midi_route_to.is_empty(),
            "nor notes another track plays on the bus"
        );
    }

    /// The stems, added up, are the mix: each track routed into a bus, one
    /// of them sending to a return. Both came out silent: the bus was left
    /// out of each stem and never reached the master.
    #[test]
    fn the_stems_add_up_to_the_mix() {
        let engine = DawEngine::new();
        let frames = SR as u64;
        for (source, hz) in [("a", 220.0f32), ("b", 330.0)] {
            let ch: Vec<f32> = (0..frames)
                .map(|n| (std::f32::consts::TAU * hz * n as f32 / SR as f32).sin() * 0.3)
                .collect();
            engine.audio_pool.insert(
                source.to_string(),
                AudioBuffer {
                    channels: vec![ch.clone(), ch],
                    sample_rate: SR,
                    num_frames: frames as usize,
                },
            );
        }
        let len = {
            let mut p = engine.project.lock();
            let bus = p.add_audio_track("Bus".into());
            let verb = p.add_audio_track("Verb".into());
            p.track_mut(&bus).unwrap().volume_db = -3.0;
            for source in ["a", "b"] {
                let id = p.add_audio_track(source.into());
                let t = p.track_mut(&id).unwrap();
                t.clips.push(audio(source, source, frames, &id));
                t.output_bus = Some(bus.clone());
                if source == "a" {
                    t.sends.push(send(&verb));
                }
            }
            p.tempo_map.tick_to_samples(LEN_TICKS, SR as f64)
        };
        let render = |prepare: &dyn Fn(&mut Project)| -> Vec<f32> {
            let mut out = Vec::new();
            engine
                .render_offline_with(SR, len, 0, None, prepare, |b| {
                    out.extend_from_slice(b);
                    true
                })
                .expect("it renders");
            out
        };
        let mix = render(&|_| {});
        let stems: Vec<String> = {
            let p = engine.project.lock();
            p.tracks
                .iter()
                .filter(|t| has_a_stem(&p, t))
                .map(|t| t.id.clone())
                .collect()
        };
        assert_eq!(stems.len(), 2, "a stem for each track that plays");
        let mut sum = vec![0.0f32; mix.len()];
        for id in &stems {
            let stem = render(&|p: &mut Project| isolate_for_stem(p, id));
            for (s, v) in sum.iter_mut().zip(stem) {
                *s += v;
            }
        }
        let peak = mix.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let worst = mix
            .iter()
            .zip(&sum)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(peak > 0.05, "the mix plays: {peak}");
        assert!(worst < 1e-3, "the stems add up to the mix: off by {worst}");
    }

    /// A range export ends the song where the range does, so its tail is
    /// what was still sounding and not the next bar.
    #[test]
    fn a_range_export_s_tail_has_nothing_from_after_the_range() {
        let mut p = Project::default();
        let id = p.add_midi_track("Lead".into());
        let bar = 4 * hardwave_midi::PPQ;
        {
            let t = p.track_mut(&id).unwrap();
            let mut across = midi("across", &id);
            across.length_ticks = 2 * bar;
            if let ClipContent::Midi(m) = &mut across.content {
                m.clip.length_ticks = 2 * bar;
                for (start, len) in [(0, bar / 4), (bar - bar / 4, bar / 2), (bar + 10, 10)] {
                    m.clip.notes.push(hardwave_midi::MidiNote {
                        start_tick: start,
                        duration_ticks: len,
                        pitch: 60,
                        velocity: 0.8,
                        ..Default::default()
                    });
                }
            }
            t.clips.push(across);
            let mut after = audio("after", "after", 100, &id);
            after.position_ticks = bar;
            t.clips.push(after);
            let mut faded = audio("faded", "faded", 100, &id);
            faded.length_ticks = 2 * bar;
            if let ClipContent::Audio(a) = &mut faded.content {
                a.fade_out_ticks = bar / 2;
            }
            t.clips.push(faded);
        }
        let end = p.tempo_map.tick_to_samples(bar, SR as f64);

        super::end_the_song_at(&mut p, end, SR);

        let t = p.track(&id).unwrap();
        assert_eq!(t.clips.len(), 2, "the clip after the range is gone");
        assert!(
            t.clips.iter().all(|c| c.length_ticks == bar),
            "cut at the end"
        );
        let ClipContent::Midi(m) = &t.clips[0].content else {
            panic!("the MIDI clip first");
        };
        let notes: Vec<(u64, u64)> = m
            .clip
            .notes
            .iter()
            .map(|n| (n.start_tick, n.duration_ticks))
            .collect();
        assert_eq!(
            notes,
            [(0, bar / 4), (bar - bar / 4, bar / 4)],
            "a note held over the end is let go there, one after it is gone"
        );
        let ClipContent::Audio(a) = &t.clips[1].content else {
            panic!("the audio clip second");
        };
        assert_eq!(a.fade_out_ticks, 0, "no fade-out moved into the range");
    }

    /// A rendered clip covers the stretch of song it was rendered from, in
    /// a song whose tempo changes. Its length came from the file's duration
    /// at one tempo, so it ended before the render did.
    #[test]
    fn a_render_s_clip_covers_its_render_when_the_tempo_changes() {
        let mut p = Project::default();
        let bar = 4 * hardwave_midi::PPQ;
        let mut faster = p.tempo_map.entries[0].clone();
        faster.tick = 4 * bar;
        faster.bpm = 175.0;
        p.tempo_map.entries.push(faster);
        let id = p.add_audio_track("Lead".into());
        let mut part = audio("part", "part", 100, &id);
        part.length_ticks = 8 * bar;
        p.track_mut(&id).unwrap().clips.push(part);
        let end = super::song_end_ticks(&p);
        assert_eq!(end, 8 * bar);
        let rendered = p.tempo_map.tick_to_samples(end, SR as f64);

        // What importing the render gives it: its duration at 140 BPM.
        let secs = rendered as f64 / SR as f64;
        let imported = (secs * 140.0 / 60.0 * hardwave_midi::PPQ as f64).round() as u64;
        let mut render = audio("render", "render", rendered, &id);
        render.length_ticks = imported;
        p.track_mut(&id).unwrap().clips.push(render);

        super::fit_render_clip(&mut p, &id, "render", 0, end);

        let placed = p.track(&id).unwrap().clips.last().unwrap();
        let plays_to = p
            .tempo_map
            .tick_to_samples(placed.position_ticks + placed.length_ticks, SR as f64);
        assert!(imported < end, "the import alone falls short");
        assert_eq!(plays_to, rendered, "the clip plays the whole render");
    }

    /// A bounced part plays back as it played, through the file the bounce
    /// writes and the import that reads it back: a 24-bit stereo sample on
    /// a centred track at 0 dB. The file's header said 32-bit integers while
    /// it held float samples, so the bounce read back as loud distortion,
    /// which a peak meter showed as the part coming back about 2 dB down.
    #[test]
    fn a_bounced_24_bit_stereo_sample_plays_back_as_it_played() {
        use std::sync::atomic::AtomicBool;
        use std::sync::Arc;
        let dir = tempfile::tempdir().expect("a folder");
        let source = dir.path().join("loop.wav");
        {
            let spec = hound::WavSpec {
                channels: 2,
                sample_rate: SR,
                bits_per_sample: 24,
                sample_format: hound::SampleFormat::Int,
            };
            let mut w = hound::WavWriter::create(&source, spec).expect("a file");
            let full = 8_388_607.0f32;
            let tau = std::f32::consts::TAU;
            for n in 0..SR {
                let t = n as f32 / SR as f32;
                // A different sound on each side, one dying away to the
                // very quiet samples a reverb tail ends in.
                let l = (tau * 110.0 * t).sin() * 0.55;
                let r = (tau * 165.0 * t).sin() * 0.45 * (-6.0 * t).exp();
                w.write_sample((l * full).round() as i32).expect("a sample");
                w.write_sample((r * full).round() as i32).expect("a sample");
            }
            w.finalize().expect("the file");
        }

        // Imported onto a track, as the app does.
        let engine = parking_lot::Mutex::new(DawEngine::new());
        let loop_id = {
            let e = engine.lock();
            let (source_id, info) = e.load_audio_file(&source).expect("it imports");
            let mut p = e.project.lock();
            let id = p.add_audio_track("Loop".into());
            let mut clip = audio("loop", &source_id, info.total_frames, &id);
            clip.length_ticks = p.tempo_map.samples_to_tick(info.total_frames, SR as f64);
            p.track_mut(&id).unwrap().clips.push(clip);
            id
        };
        let len = engine.lock().project_end_samples(SR);
        let mix = |e: &DawEngine| -> Vec<f32> {
            let mut out = Vec::new();
            e.render_offline_with(
                SR,
                len,
                0,
                None,
                |_| {},
                |b| {
                    out.extend_from_slice(b);
                    true
                },
            )
            .expect("it renders");
            out
        };
        let before = mix(&engine.lock());

        // The bounce: the command's render, written the way it writes it,
        // read back and placed the way it places it.
        let bounced = dir.path().join("bounce.wav");
        let end = super::song_end_ticks(&engine.lock().project.lock());
        let target = loop_id.clone();
        let finished = super::render_unlocked(
            engine.lock(),
            &bounced,
            super::format_for_a_track(SR),
            len,
            0,
            &Arc::new(AtomicBool::new(false)),
            |_, _| {},
            true,
            move |p: &mut Project| super::isolate_track_for_render(p, &target, true, &|_| true),
        )
        .expect("it bounces");
        assert!(finished);
        {
            let e = engine.lock();
            let (source_id, info) = e.load_audio_file(&bounced).expect("the bounce imports");
            let mut p = e.project.lock();
            let id = p.add_audio_track("Loop (bounced)".into());
            let clip = audio("bounced", &source_id, info.total_frames, &id);
            p.track_mut(&id).unwrap().clips.push(clip);
            super::fit_render_clip(&mut p, &id, "bounced", 0, end);
            p.track_mut(&loop_id).unwrap().muted = true;
        }
        let after = mix(&engine.lock());

        let peak_db = |s: &[f32]| 20.0 * s.iter().fold(0.0f32, |m, v| m.max(v.abs())).log10();
        let rms_db = |s: &[f32]| {
            let sum: f64 = s.iter().map(|&v| (v as f64) * (v as f64)).sum();
            10.0 * (sum / s.len() as f64).log10() as f32
        };
        let (peak_before, peak_after) = (peak_db(&before), peak_db(&after));
        assert!(
            (peak_before - peak_after).abs() < 0.1,
            "the bounce peaks where the part did: {peak_before:.2} dB before, {peak_after:.2} dB after"
        );
        let (rms_before, rms_after) = (rms_db(&before), rms_db(&after));
        assert!(
            (rms_before - rms_after).abs() < 0.1,
            "and is as loud all through: {rms_before:.2} dB before, {rms_after:.2} dB after"
        );
    }

    /// A muted track, or one silenced by a solo elsewhere, renders as it
    /// sounds unmuted: freezing or bouncing it rendered silence.
    #[test]
    fn a_muted_track_or_a_solo_elsewhere_does_not_silence_the_render() {
        let mut p = Project::default();
        let vox = p.add_audio_track("Vox".into());
        let drums = p.add_audio_track("Drums".into());
        p.track_mut(&vox).unwrap().muted = true;
        p.track_mut(&drums).unwrap().soloed = true;
        p.track_mut(&drums).unwrap().muted = true;

        super::isolate_track_for_render(&mut p, &vox, true, &|_| false);

        assert!(!p.track(&vox).unwrap().muted);
        assert!(p.tracks.iter().all(|t| !t.soloed), "no solo silences it");
        assert!(
            p.track(&drums).unwrap().muted,
            "another track's own mute stays, as it keys sidechains as heard"
        );
    }
}

#[cfg(test)]
mod dither_tests {
    use super::{flac_bits, flac_samples, DitherMode};

    /// A FLAC export is dithered when the export says so. A level under
    /// one 16-bit step rounded to nothing; dithered, it averages out at
    /// its level, as it does in a dithered WAV.
    #[test]
    fn a_16_bit_flac_is_dithered_when_asked() {
        let step = 1.0 / i16::MAX as f32;
        let quiet = vec![0.3 * step; 96_000];
        let bits = flac_bits(16);
        let plain = flac_samples(&quiet, bits, DitherMode::None);
        assert!(plain.iter().all(|&v| v == 0), "rounded away undithered");
        let dithered = flac_samples(&quiet, bits, DitherMode::Tpdf);
        let mean = dithered.iter().map(|&v| v as f64).sum::<f64>() / dithered.len() as f64;
        assert!(
            (mean - 0.3).abs() < 0.05,
            "dithered, the level survives on average: {mean}"
        );
    }
}

#[cfg(test)]
mod loudness_tests {
    use super::{integrated_lufs, LoudnessTap};

    /// Ten seconds of a 1 kHz sine at -6 dBFS on both sides, interleaved.
    fn sine(sample_rate: u32) -> Vec<f32> {
        let amp = 10f32.powf(-6.0 / 20.0);
        (0..sample_rate as usize * 10)
            .flat_map(|i| {
                let v = amp
                    * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn the_notice_reads_a_known_sine_right() {
        let samples = sine(48_000);
        let mut tap = LoudnessTap::new(48_000);
        tap.push_interleaved(&samples);
        let l = tap.finish();
        assert!((l.peak_db + 6.0).abs() < 0.05, "peak {}", l.peak_db);
        assert!(
            (l.true_peak_db + 6.0).abs() < 0.2,
            "true peak {}",
            l.true_peak_db
        );
        let lufs = l.lufs_integrated.expect("ten seconds is enough to measure");
        assert!((lufs + 6.0).abs() < 0.3, "LUFS {lufs}");
    }

    /// One meter for both: a file normalized to a target reads the target.
    #[test]
    fn the_notice_agrees_with_what_lufs_normalizing_aimed_at() {
        let samples = sine(44_100);
        let mut tap = LoudnessTap::new(44_100);
        tap.push_interleaved(&samples);
        let shown = tap.finish().lufs_integrated.unwrap();
        let aimed = integrated_lufs(&samples, 44_100).unwrap();
        assert!(
            (shown - aimed).abs() < 0.1,
            "notice {shown} against normalize {aimed}"
        );
    }

    #[test]
    fn sample_by_sample_and_whole_buffers_measure_the_same() {
        let samples = sine(48_000);
        let mut whole = LoudnessTap::new(48_000);
        whole.push_interleaved(&samples);
        let mut one_by_one = LoudnessTap::new(48_000);
        for (i, &s) in samples.iter().enumerate() {
            one_by_one.push(s, i & 1);
        }
        let (a, b) = (whole.finish(), one_by_one.finish());
        assert_eq!(a.peak_db, b.peak_db);
        assert_eq!(a.lufs_integrated, b.lufs_integrated);
    }
}
