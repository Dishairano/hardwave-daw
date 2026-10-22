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
fn isolate_track_for_render(
    proj: &mut Project,
    target: &str,
    keep_effects: bool,
    is_instrument: &dyn Fn(&str) -> bool,
) {
    for t in proj.tracks.iter_mut() {
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
        t.output_bus = None;
        if !keep_effects {
            t.inserts.retain(|s| is_instrument(&s.plugin_id));
            t.sends.clear();
            t.volume_db = 0.0;
            t.pan = 0.0;
        }
    }
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
#[allow(clippy::too_many_arguments)]
fn render_unlocked<F>(
    engine: parking_lot::MutexGuard<'_, DawEngine>,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    start_samples: u64,
    cancel: &Arc<AtomicBool>,
    on_progress: F,
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
    prepare: impl FnOnce(&mut Project),
) -> Result<Option<FileLoudness>, String>
where
    F: FnMut(u64, u64),
{
    let factory = offline_insert_factory(&engine);
    let render =
        engine.prepare_offline_render(fmt.sample_rate, start_samples, Some(&factory), prepare);
    drop(engine);
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
            let lsb = 1.0 / max;
            let mut x = s;
            match dither {
                DitherMode::None => {}
                DitherMode::Tpdf => x += state.rng.next_tpdf() * lsb,
                DitherMode::TpdfShaped => {
                    let n = state.rng.next_tpdf() * lsb;
                    let ch = channel & 1;
                    x = x + n - state.prev_err[ch];
                    state.prev_err[ch] = n;
                }
            }
            let v = (x.clamp(-1.0, 1.0) * max).round() as i16;
            wav.write_sample(v)
        }
        24 => {
            let max = 8_388_607.0_f32;
            let lsb = 1.0 / max;
            let mut x = s;
            match dither {
                DitherMode::None => {}
                DitherMode::Tpdf => x += state.rng.next_tpdf() * lsb,
                DitherMode::TpdfShaped => {
                    let n = state.rng.next_tpdf() * lsb;
                    let ch = channel & 1;
                    x = x + n - state.prev_err[ch];
                    state.prev_err[ch] = n;
                }
            }
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

fn spec_for(bit_depth: u32, sample_rate: u32) -> WavSpec {
    let (bits, format) = if bit_depth == 0 {
        (32u16, SampleFormat::Float)
    } else {
        (bit_depth as u16, SampleFormat::Int)
    };
    WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: bits,
        sample_format: format,
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
) -> Result<(), String> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let bits: u32 = match bit_depth {
        16 => 16,
        0 | 24 => 24,
        n => n.clamp(8, 24),
    };
    let max = ((1i64 << (bits - 1)) - 1) as f32;
    let mut samples: Vec<i32> = Vec::with_capacity(buffer.len());
    for &s in buffer {
        let v = (s.clamp(-1.0, 1.0) * max).round() as i32;
        samples.push(v);
    }

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
    write_flac_from_buffer(out_path, &buffer, fmt.sample_rate, fmt.bit_depth)?;
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
            |_| {},
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

            let fmt = RenderFormat {
                sample_rate,
                bit_depth: 32,
                normalize: NormalizeMode::parse("off"),
                normalize_target_db: -1.0,
                dither: DitherMode::parse("none"),
                mp3_bitrate_kbps: 320,
                mp3_vbr_quality: None,
                ogg_quality: 0.5,
            };

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
                // The glued clip stays on this track, through its effects
                // and fader and on to the master: the render has none of
                // them, only the instrument that plays its notes.
                move |proj: &mut Project| {
                    isolate_track_for_render(proj, &target, false, &|id| instruments.contains(id))
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
    crate::commands::audio::import_audio_file_blocking(
        state.clone(),
        source_track_id.clone(),
        path.clone(),
        Some(start_ticks),
    )?;
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

            let fmt = RenderFormat {
                sample_rate,
                bit_depth: 32,
                normalize: NormalizeMode::parse("off"),
                normalize_target_db: -1.0,
                dither: DitherMode::parse("none"),
                mp3_bitrate_kbps: 320,
                mp3_vbr_quality: None,
                ogg_quality: 0.5,
            };

            let target = track_id.clone();
            let completed = render_unlocked(
                engine_guard,
                &out_path,
                fmt,
                total_samples,
                0,
                &cancel,
                |_, _| {},
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
    crate::commands::audio::import_audio_file_blocking(
        state.clone(),
        new_track_id.clone(),
        path.clone(),
        Some(0),
    )?;
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
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

            let folder = PathBuf::from(&folder_path);
            fs::create_dir_all(&folder).map_err(|e| format!("create folder: {e}"))?;

            let track_infos: Vec<(String, String)> = {
                let project = engine_guard.project.lock();
                project
                    .tracks
                    .iter()
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
                    |proj: &mut Project| {
                        // Exclude rather than mute. A muted TrackNode returns
                        // before writing its output ports, which also stops it
                        // feeding any sidechain keyed off it and any send that
                        // originates from it — so muting the kick to render the
                        // bass stem produced a stem with NO ducking while the
                        // full mix ducked, and rendering a return's stem muted
                        // everything feeding it, giving silence. Excluded
                        // tracks keep processing and can still key a sidechain;
                        // they just don't reach master and their sends are
                        // dropped. This also preserves the user's own mute
                        // state instead of overwriting it.
                        for t in proj.tracks.iter_mut() {
                            t.stem_excluded = t.id != target_id;
                        }
                        if !respect_mute_solo {
                            for t in proj.tracks.iter_mut() {
                                t.soloed = false;
                                t.solo_safe = false;
                                t.muted = false;
                            }
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
                    |proj: &mut Project| {
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

        let fmt = RenderFormat {
            sample_rate,
            bit_depth: 32,
            normalize: NormalizeMode::parse("off"),
            normalize_target_db: -1.0,
            dither: DitherMode::parse("none"),
            mp3_bitrate_kbps: 320,
            mp3_vbr_quality: None,
            ogg_quality: 0.5,
        };

        let target = track_id.clone();
        let completed = render_unlocked(
            engine_guard,
            &out_path,
            fmt,
            total_samples,
            0,
            &cancel,
            |_, _| {},
            // Through its own effects, which rest while it is frozen, and
            // not through its bus or the master, which it still plays into.
            move |proj: &mut Project| isolate_track_for_render(proj, &target, true, &|_| true),
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
        if let Some(track) = project.track_mut(&source_track_id) {
            for placement in track.clips.iter_mut() {
                let is_render = match &placement.content {
                    hardwave_project::clip::ClipContent::Audio(ac) => ac.id == clip.clip_id,
                    hardwave_project::clip::ClipContent::Midi(_) => false,
                };
                if is_render {
                    placement.lane = hardwave_project::clip::FREEZE_LANE;
                }
            }
            track.frozen = true;
        }
    }
    state.engine.lock().rebuild_graph();
    Ok(rendered)
}

/// Put the live chain back. The render stays on the freeze lane, so
/// freezing again with nothing changed is instant.
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
        isolate_track_for_render(&mut p, &id, false, &|pid| pid.starts_with("synth."));
        let t = p.track(&id).expect("the kick");
        let kept: Vec<&str> = t.inserts.iter().map(|s| s.plugin_id.as_str()).collect();
        assert_eq!(kept, ["synth.kick"]);
        assert_eq!((t.volume_db, t.pan, t.output_bus.clone()), (0.0, 0.0, None));
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
