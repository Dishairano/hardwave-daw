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

/// Render and write one offline-rendered WAV file. Returns `Ok(true)` on
/// completion, `Ok(false)` when the caller requested cancellation mid-render.
#[allow(clippy::too_many_arguments)]
fn write_render<F>(
    engine: &DawEngine,
    out_path: &Path,
    sample_rate: u32,
    bit_depth: u32,
    total_samples: u64,
    cancel: &Arc<AtomicBool>,
    mut on_progress: F,
    prepare: impl FnOnce(&mut Project),
) -> Result<bool, String>
where
    F: FnMut(u64, u64),
{
    let bits = if bit_depth == 0 { 32 } else { bit_depth as u16 };
    let format = if bit_depth == 0 {
        SampleFormat::Float
    } else {
        SampleFormat::Int
    };
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: bits,
        sample_format: format,
    };

    let file = File::create(out_path).map_err(|e| format!("create: {e}"))?;
    let writer = BufWriter::new(file);
    let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
    let mut wav_err: Option<String> = None;
    let mut written_samples: u64 = 0;

    engine.render_offline_with(sample_rate, total_samples, prepare, |block| {
        if wav_err.is_some() {
            return false;
        }
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        for &s in block {
            let clamped = s.clamp(-1.0, 1.0);
            let write_res: Result<(), hound::Error> = match (bit_depth, format) {
                (16, _) => {
                    let v = (clamped * i16::MAX as f32) as i16;
                    wav.write_sample(v)
                }
                (24, _) => {
                    let v = (clamped * 8_388_607.0) as i32;
                    wav.write_sample(v)
                }
                _ => wav.write_sample(clamped),
            };
            if let Err(e) = write_res {
                wav_err = Some(format!("wav write: {e}"));
                return false;
            }
        }
        written_samples += (block.len() / 2) as u64;
        on_progress(written_samples, total_samples);
        true
    })?;

    if let Some(e) = wav_err {
        return Err(e);
    }
    wav.finalize().map_err(|e| format!("finalize: {e}"))?;
    let completed = !cancel.load(Ordering::Relaxed);
    Ok(completed)
}

#[tauri::command]
pub async fn export_project_wav(
    app: AppHandle,
    path: String,
    sample_rate: u32,
    bit_depth: u32,
    tail_secs: f32,
) -> Result<ExportResult, String> {
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);

    let out_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ExportResult, String> {
        let engine_guard = engine.lock();

        let body_samples = engine_guard.project_end_samples(sample_rate);
        if body_samples == 0 {
            return Err("Project is empty — nothing to export.".into());
        }
        let tail_samples = ((tail_secs.max(0.0) as f64) * sample_rate as f64) as u64;
        let total_samples = body_samples + tail_samples;

        let mut last_pct: i32 = -1;
        let app_for_progress = app.clone();
        let completed = write_render(
            &engine_guard,
            Path::new(&out_path),
            sample_rate,
            bit_depth,
            total_samples,
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
            // Best-effort cleanup of the partial file.
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
) -> Result<StemsExportResult, String> {
    let (engine, cancel) = {
        let state: State<AppState> = app.state();
        (state.engine.clone(), state.export_cancel.clone())
    };
    cancel.store(false, Ordering::Relaxed);

    let result =
        tauri::async_runtime::spawn_blocking(move || -> Result<StemsExportResult, String> {
            let engine_guard = engine.lock();

            let body_samples = engine_guard.project_end_samples(sample_rate);
            if body_samples == 0 {
                return Err("Project is empty — nothing to export.".into());
            }
            let tail_samples = ((tail_secs.max(0.0) as f64) * sample_rate as f64) as u64;
            let total_samples = body_samples + tail_samples;

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
                    sample_rate,
                    bit_depth,
                    total_samples,
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
                    sample_rate,
                    bit_depth,
                    total_samples,
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
