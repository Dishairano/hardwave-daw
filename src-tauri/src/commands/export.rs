use crate::AppState;
use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(serde::Serialize, Clone)]
pub struct ExportProgress {
    pub percent: f32,
}

#[derive(serde::Serialize)]
pub struct ExportResult {
    pub path: String,
    pub samples: u64,
    pub duration_secs: f64,
}

#[tauri::command]
pub async fn export_project_wav(
    app: AppHandle,
    path: String,
    sample_rate: u32,
    bit_depth: u32,
    tail_secs: f32,
) -> Result<ExportResult, String> {
    // Pull the shared engine handle out of the state so the blocking render
    // loop can own a clone; we don't hold `State` across the await.
    let engine = {
        let state: State<AppState> = app.state();
        state.engine.clone()
    };

    let out_path = path.clone();
    let result = tauri::async_runtime::spawn_blocking(move || -> Result<ExportResult, String> {
        let engine_guard = engine.lock();

        let body_samples = engine_guard.project_end_samples(sample_rate);
        if body_samples == 0 {
            return Err("Project is empty — nothing to export.".into());
        }
        let tail_samples = ((tail_secs.max(0.0) as f64) * sample_rate as f64) as u64;
        let total_samples = body_samples + tail_samples;

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

        let file = File::create(&out_path).map_err(|e| format!("create: {e}"))?;
        let writer = BufWriter::new(file);
        let mut wav = WavWriter::new(writer, spec).map_err(|e| format!("wav: {e}"))?;
        let mut wav_err: Option<String> = None;

        let mut written_samples: u64 = 0;
        let mut last_reported_pct: i32 = -1;
        let app_for_progress = app.clone();

        engine_guard.render_offline(sample_rate, total_samples, |block| {
            if wav_err.is_some() {
                return;
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
                    return;
                }
            }
            written_samples += (block.len() / 2) as u64;
            let pct_i = ((written_samples as f64 / total_samples as f64) * 100.0) as i32;
            if pct_i != last_reported_pct {
                last_reported_pct = pct_i;
                let _ = app_for_progress.emit(
                    "export-progress",
                    ExportProgress {
                        percent: pct_i as f32,
                    },
                );
            }
        })?;

        if let Some(e) = wav_err {
            return Err(e);
        }

        wav.finalize().map_err(|e| format!("finalize: {e}"))?;

        Ok(ExportResult {
            path: out_path,
            samples: total_samples,
            duration_secs: total_samples as f64 / sample_rate as f64,
        })
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    Ok(result)
}
