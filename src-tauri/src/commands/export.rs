use crate::AppState;
use hardwave_engine::DawEngine;
use hardwave_project::Project;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

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

/// BS.1770 K-weighting biquad in Direct Form I (64-bit to preserve headroom
/// across the pre-filter + RLB cascade).
struct KBiquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl KBiquad {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

fn k_shelf(fs: f64) -> KBiquad {
    let a = f64::powf(10.0, 3.999_843_853_973_347 / 40.0);
    let w = 2.0 * std::f64::consts::PI * 1_681.974_450_955_533 / fs;
    let alpha = w.sin() / (2.0 * 0.707_175_236_955_419_6);
    let c = w.cos();
    let sq = a.sqrt();
    KBiquad::new(
        a * ((a + 1.0) + (a - 1.0) * c + 2.0 * sq * alpha),
        -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
        a * ((a + 1.0) + (a - 1.0) * c - 2.0 * sq * alpha),
        (a + 1.0) - (a - 1.0) * c + 2.0 * sq * alpha,
        2.0 * ((a - 1.0) - (a + 1.0) * c),
        (a + 1.0) - (a - 1.0) * c - 2.0 * sq * alpha,
    )
}

fn k_hpf(fs: f64) -> KBiquad {
    let w = 2.0 * std::f64::consts::PI * 38.135_470_876_024_44 / fs;
    let alpha = w.sin() / (2.0 * 0.500_327_037_323_877_3);
    let c = w.cos();
    KBiquad::new(
        (1.0 + c) / 2.0,
        -(1.0 + c),
        (1.0 + c) / 2.0,
        1.0 + alpha,
        -2.0 * c,
        1.0 - alpha,
    )
}

/// Integrated loudness per ITU-R BS.1770-4: K-weight both channels, compute
/// mean-square over 400 ms blocks with 75 % overlap (100 ms hop), gate
/// absolute at -70 LUFS, then gate relative at integrated - 10 LU. Input
/// buffer is interleaved stereo `[l0, r0, l1, r1, ...]`. Returns `None` when
/// the buffer is too short for a single gated block.
fn integrated_lufs(buffer: &[f32], sample_rate: u32) -> Option<f32> {
    let fs = sample_rate as f64;
    let block = (0.4 * fs).round() as usize;
    let hop = (0.1 * fs).round() as usize;
    if block == 0 || hop == 0 {
        return None;
    }

    let frames = buffer.len() / 2;
    if frames < block {
        return None;
    }

    let mut kl_shelf = k_shelf(fs);
    let mut kl_hpf = k_hpf(fs);
    let mut kr_shelf = k_shelf(fs);
    let mut kr_hpf = k_hpf(fs);

    let mut kl = vec![0.0_f64; frames];
    let mut kr = vec![0.0_f64; frames];
    for i in 0..frames {
        let l = buffer[i * 2] as f64;
        let r = buffer[i * 2 + 1] as f64;
        kl[i] = kl_hpf.process(kl_shelf.process(l));
        kr[i] = kr_hpf.process(kr_shelf.process(r));
    }

    let mut blocks_z: Vec<f64> = Vec::with_capacity((frames.saturating_sub(block)) / hop + 1);
    let mut start = 0usize;
    while start + block <= frames {
        let mut sl = 0.0_f64;
        let mut sr = 0.0_f64;
        for i in 0..block {
            let a = kl[start + i];
            let b = kr[start + i];
            sl += a * a;
            sr += b * b;
        }
        let z = (sl + sr) / block as f64;
        blocks_z.push(z);
        start += hop;
    }
    if blocks_z.is_empty() {
        return None;
    }

    // Absolute gate: L >= -70 LUFS <=> z >= 10^(-6.9309) after the -0.691 offset.
    let abs_z = 10f64.powf((-70.0 - (-0.691)) / 10.0);
    let pass_abs: Vec<f64> = blocks_z.iter().copied().filter(|&z| z >= abs_z).collect();
    if pass_abs.is_empty() {
        return None;
    }

    // Relative gate: -10 LU below integrated loudness of the abs-gated set.
    let mean_abs = pass_abs.iter().sum::<f64>() / pass_abs.len() as f64;
    let rel_lufs = -0.691 + 10.0 * mean_abs.log10() - 10.0;
    let rel_z = 10f64.powf((rel_lufs - (-0.691)) / 10.0);

    let pass_rel: Vec<f64> = pass_abs.into_iter().filter(|&z| z >= rel_z).collect();
    if pass_rel.is_empty() {
        return None;
    }
    let mean = pass_rel.iter().sum::<f64>() / pass_rel.len() as f64;
    if mean <= 0.0 {
        return None;
    }
    Some((-0.691 + 10.0 * mean.log10()) as f32)
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

/// Render and write one offline-rendered WAV file. Returns `Ok(true)` on
/// completion, `Ok(false)` when the caller requested cancellation mid-render.
#[allow(clippy::too_many_arguments)]
fn write_render<F>(
    engine: &DawEngine,
    out_path: &Path,
    fmt: RenderFormat,
    total_samples: u64,
    start_samples: u64,
    cancel: &Arc<AtomicBool>,
    mut on_progress: F,
    prepare: impl FnOnce(&mut Project),
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    let spec = spec_for(fmt.bit_depth, fmt.sample_rate);

    match fmt.normalize {
        NormalizeMode::Off => {
            let file = File::create(out_path).map_err(|e| format!("create: {e}"))?;
            let writer = BufWriter::new(file);
            let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
            let mut wav_err: Option<String> = None;
            let mut dither = DitherState::new();
            let mut written_frames: u64 = 0;

            engine.render_offline_with(
                fmt.sample_rate,
                total_samples,
                start_samples,
                prepare,
                |block| {
                    if wav_err.is_some() || cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    for (i, &s) in block.iter().enumerate() {
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
                },
            )?;

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

            engine.render_offline_with(
                fmt.sample_rate,
                total_samples,
                start_samples,
                prepare,
                |block| {
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
                },
            )?;

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

            engine.render_offline_with(
                fmt.sample_rate,
                total_samples,
                start_samples,
                prepare,
                |block| {
                    if cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    buffer.extend_from_slice(block);
                    let frames = (buffer.len() / 2) as u64;
                    on_progress(frames / 2, total_samples);
                    true
                },
            )?;

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
        let completed = write_render(
            &engine_guard,
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

        if !completed {
            let _ = fs::remove_file(&out_path);
        }

        Ok(ExportResult {
            path: out_path,
            samples: total_samples,
            duration_secs: total_samples as f64 / sample_rate as f64,
            cancelled: !completed,
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
                let stem_name = format!("{}_{}.wav", project_slug, sanitize_filename(track_name));
                let stem_path = folder.join(&stem_name);
                let mut last_pct: i32 = -1;
                let app_for_progress = app.clone();
                let target_id = track_id.clone();
                let label = track_name.clone();
                let idx = render_idx;

                let completed = write_render(
                    &engine_guard,
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
                        for t in proj.tracks.iter_mut() {
                            t.muted = t.id != target_id;
                        }
                        if !respect_mute_solo {
                            for t in proj.tracks.iter_mut() {
                                t.soloed = false;
                                t.solo_safe = false;
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
                let master_name = format!("{}_master.wav", project_slug);
                let master_path = folder.join(&master_name);
                let mut last_pct: i32 = -1;
                let app_for_progress = app.clone();
                let idx = render_idx;

                let completed = write_render(
                    &engine_guard,
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
