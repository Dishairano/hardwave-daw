//! Render-to-WAV harness.
//!
//! Unlike `functional_smoke.rs` (which measures the render stream inline and
//! asserts on peak/RMS), this test writes the rendered master bus to an actual
//! `.wav` on disk so a human can *listen* to what the engine produced. It is a
//! `#[test]` so it reuses the exact same offline render path the smoke tests
//! exercise; point `HW_WAV_OUT` at a path to capture the file.
//!
//! Run:
//!   HW_WAV_OUT=/path/out.wav cargo test -p hardwave-engine --test render_wav -- --nocapture

mod common;

use common::*;
use hardwave_engine::DawEngine;
use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};
use hound::{SampleFormat, WavSpec, WavWriter};

fn write_stereo_wav(path: &str, sample_rate: u32, interleaved: &[f32]) {
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut w = WavWriter::create(path, spec).expect("create wav");
    for &s in interleaved {
        w.write_sample(s).expect("write");
    }
    w.finalize().expect("finalize");
}

/// Diagnostic: render a 440 Hz clip at stretch_ratio=1.0 vs 2.0 to show the
/// current time-stretch is varispeed (2× stretch drops the pitch an octave to
/// 220 Hz). Writes both WAVs when HW_STRETCH_WAV_DIR is set. Not an assertion
/// gate — it documents the coupling bug audibly.
#[test]
fn render_stretch_diagnostic() {
    let sample_rate = 48_000_u32;
    let render_clip = |stretch: f64| -> Vec<f32> {
        let engine = DawEngine::new();
        let buf = make_sine_buffer(sample_rate, 3.0, 440.0, 0.5);
        let frames = buf.num_frames as u64;
        engine.audio_pool.insert("stretch-src".to_string(), buf);
        {
            let mut project = engine.project.lock();
            let track_id = project.add_audio_track("Stretch".into());
            if let Some(t) = project.track_mut(&track_id) {
                let clip = AudioClip {
                    id: "clip-stretch".into(),
                    name: "stretch".into(),
                    source_path: "stretch-src".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: frames,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: stretch,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                };
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(clip),
                    track_id: track_id.clone(),
                    position_ticks: 0,
                    length_ticks: 1_000_000,
                    lane: 0,
                });
            }
        }
        let mut out = Vec::new();
        engine
            .render_offline(sample_rate, sample_rate as u64 * 2, |block| {
                out.extend_from_slice(block);
                true
            })
            .unwrap();
        out
    };

    let normal = render_clip(1.0);
    let stretched = render_clip(2.0);
    if let Ok(dir) = std::env::var("HW_STRETCH_WAV_DIR") {
        write_stereo_wav(&format!("{dir}/daw_stretch_normal.wav"), sample_rate, &normal);
        write_stereo_wav(&format!("{dir}/daw_stretch_2x.wav"), sample_rate, &stretched);
        eprintln!("wrote {dir}/daw_stretch_normal.wav + daw_stretch_2x.wav");
    }
    // Both audible.
    assert!(normal.iter().fold(0.0f32, |m, &s| m.max(s.abs())) > 0.05);
    assert!(stretched.iter().fold(0.0f32, |m, &s| m.max(s.abs())) > 0.05);
}

/// Build a short, musical two-track project and render it to a stereo WAV.
///
/// Track A: a 55 Hz sub sine (the "bass"), centre.
/// Track B: a 220 Hz sine (two octaves up) at lower amplitude (the "lead").
/// Together they mix on the master bus — proving multi-track summing, not just
/// a single passthrough tone.
#[test]
fn render_demo_wav() {
    let sample_rate = 44_100_u32;
    let duration_seconds = 4.0_f32;

    let engine = DawEngine::new();
    add_audio_track_with_sine(&engine, "Bass", "sine_bass", sample_rate, duration_seconds, 55.0, 0.45);
    add_audio_track_with_sine(&engine, "Lead", "sine_lead", sample_rate, duration_seconds, 220.0, 0.22);

    let total_samples = (sample_rate as f32 * duration_seconds) as u64;

    // Collect the interleaved L/R stream the engine streams to us block-by-block.
    let mut interleaved: Vec<f32> = Vec::with_capacity(total_samples as usize * 2);
    let result = engine.render_offline(sample_rate, total_samples, |block| {
        interleaved.extend_from_slice(block);
        true
    });
    assert!(result.is_ok(), "render_offline failed: {result:?}");
    assert!(!interleaved.is_empty(), "engine produced no samples");

    // Peak-check so a silent render fails loudly here rather than shipping a
    // silent file to the founder.
    let peak = interleaved.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(peak > 0.05, "render is effectively silent (peak {peak})");

    let out = std::env::var("HW_WAV_OUT").unwrap_or_else(|_| "/tmp/daw_demo.wav".to_string());
    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut writer = WavWriter::create(&out, spec).expect("create wav writer");
    for &s in &interleaved {
        writer.write_sample(s).expect("write sample");
    }
    writer.finalize().expect("finalize wav");

    eprintln!(
        "HW_WAV_OUT wrote {out} — {} frames, peak {peak:.3}",
        interleaved.len() / 2
    );
}
