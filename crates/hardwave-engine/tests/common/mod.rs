//! Shared helpers for functional smoke tests.
//!
//! These bypass the file system by inserting synthetic audio buffers
//! directly into the engine's `audio_pool`, then constructing clips that
//! reference the synthetic buffer by id. That keeps tests fast and
//! hermetic — no temp files, no decode path, just engine graph behavior.

use hardwave_engine::AudioBuffer;
use hardwave_engine::DawEngine;
use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};

/// Build an in-memory stereo `AudioBuffer` containing a sine tone.
pub fn make_sine_buffer(
    sample_rate: u32,
    duration_seconds: f32,
    frequency_hz: f32,
    amplitude: f32,
) -> AudioBuffer {
    let num_frames = (sample_rate as f32 * duration_seconds) as usize;
    let mut left = Vec::with_capacity(num_frames);
    let mut right = Vec::with_capacity(num_frames);
    let two_pi = std::f32::consts::TAU;
    for n in 0..num_frames {
        let t = n as f32 / sample_rate as f32;
        let s = (two_pi * frequency_hz * t).sin() * amplitude;
        left.push(s);
        right.push(s);
    }
    AudioBuffer {
        channels: vec![left, right],
        sample_rate,
        num_frames,
    }
}

/// Construct an `AudioClip` that references the given source id (the key
/// you used when inserting a buffer into `audio_pool`).
pub fn make_audio_clip(source_id: impl Into<String>, num_frames: u64) -> AudioClip {
    let id = source_id.into();
    AudioClip {
        id: format!("clip-{}", id),
        name: "test clip".to_string(),
        source_path: id,
        source_hash: String::new(),
        source_start: 0,
        source_end: num_frames,
        gain_db: 0.0,
        fade_in_ticks: 0,
        fade_out_ticks: 0,
        muted: false,
        reversed: false,
        pitch_semitones: 0.0,
        stretch_ratio: 1.0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_curve: FadeCurve::Linear,
    }
}

/// Inject a sine buffer into the audio pool and place a clip referencing
/// it on the named track. Returns the track id.
pub fn add_audio_track_with_sine(
    engine: &DawEngine,
    track_name: &str,
    source_id: &str,
    sample_rate: u32,
    duration_seconds: f32,
    frequency_hz: f32,
    amplitude: f32,
) -> String {
    let buffer = make_sine_buffer(sample_rate, duration_seconds, frequency_hz, amplitude);
    let num_frames = buffer.num_frames as u64;
    engine.audio_pool.insert(source_id.to_string(), buffer);

    let mut project = engine.project.lock();
    let track_id = project.add_audio_track(track_name.to_string());
    let clip = make_audio_clip(source_id, num_frames);
    if let Some(track) = project.track_mut(&track_id) {
        track.clips.push(ClipPlacement {
            content: ClipContent::Audio(clip),
            track_id: track_id.clone(),
            position_ticks: 0,
            // Generous length: 100_000 ticks ≈ 45 seconds at 140 BPM, 960 PPQ.
            // The clip stops naturally when source_end is reached.
            length_ticks: 100_000,
            lane: 0,
        });
    }
    track_id
}

/// Result of an offline render — peak (max abs sample), RMS (root-mean-square),
/// and counts that flag NaN / Inf for sanity.
#[derive(Debug, Clone, Copy)]
pub struct RenderStats {
    pub peak: f32,
    pub rms: f32,
    pub frames: usize,
    pub nan_count: usize,
    pub inf_count: usize,
}

pub fn render_and_measure(
    engine: &DawEngine,
    sample_rate: u32,
    total_samples: u64,
) -> RenderStats {
    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut frames = 0_usize;
    let mut nan_count = 0_usize;
    let mut inf_count = 0_usize;

    let result = engine.render_offline(sample_rate, total_samples, |block| {
        for &s in block {
            if s.is_nan() {
                nan_count += 1;
                continue;
            }
            if s.is_infinite() {
                inf_count += 1;
                continue;
            }
            let abs = s.abs();
            if abs > peak {
                peak = abs;
            }
            sum_sq += (s as f64) * (s as f64);
            frames += 1;
        }
        true
    });
    assert!(result.is_ok(), "render_offline failed: {result:?}");

    let rms = if frames > 0 {
        ((sum_sq / frames as f64).sqrt()) as f32
    } else {
        0.0
    };
    RenderStats {
        peak,
        rms,
        frames,
        nan_count,
        inf_count,
    }
}
