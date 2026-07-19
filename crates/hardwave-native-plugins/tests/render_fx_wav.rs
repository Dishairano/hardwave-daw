//! Audible proof that a real native effect processes a track's audio through
//! the engine's offline render path.
//!
//! `hardwave-native-plugins` depends on `hardwave-engine`, so a test here can
//! drive `DawEngine::render_offline_with` AND attach a genuine shipping plugin
//! (`NativeDistortion`) as a track insert — the exact hydration the export
//! command performs via `build_offline_insert_factory`. It renders the same
//! project dry and wet, writes both to `.wav`, and asserts the wet render
//! carries harmonic content the dry sine does not.
//!
//! Run:
//!   HW_FX_WAV_DIR=/some/dir cargo test -p hardwave-native-plugins --test render_fx_wav -- --nocapture

use hardwave_engine::{AudioBuffer, DawEngine};
use hardwave_native_plugins::distortion::NativeDistortion;
use hardwave_plugin_host::types::HostedPlugin;
use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};
use hardwave_project::track::PluginSlot;
use hound::{SampleFormat, WavSpec, WavWriter};

const SR: u32 = 48_000;
const SECONDS: f32 = 3.0;

/// Build an engine with a single 110 Hz sine track (a bass-ish tone whose
/// distortion harmonics land at 220/330/440 Hz — easy to see and hear).
fn build_sine_engine() -> (DawEngine, String) {
    let engine = DawEngine::new();
    let frames = (SR as f32 * SECONDS) as usize;
    let mut ch = Vec::with_capacity(frames);
    for n in 0..frames {
        let t = n as f32 / SR as f32;
        ch.push((std::f32::consts::TAU * 110.0 * t).sin() * 0.5);
    }
    let buffer = AudioBuffer {
        channels: vec![ch.clone(), ch],
        sample_rate: SR,
        num_frames: frames,
    };
    engine.audio_pool.insert("fx-sine".to_string(), buffer);

    let mut project = engine.project.lock();
    let track_id = project.add_audio_track("Bass".into());
    if let Some(t) = project.track_mut(&track_id) {
        t.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "clip-fx".into(),
                name: "fx-sine".into(),
                source_path: "fx-sine".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: frames as u64,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
            }),
            track_id: track_id.clone(),
            position_ticks: 0,
            length_ticks: 1_000_000,
            lane: 0,
        });
    }
    drop(project);
    (engine, track_id)
}

/// Render the whole track to an interleaved stereo buffer.
fn render(engine: &DawEngine, factory: Option<&dyn Fn(&str) -> Option<Box<dyn HostedPlugin>>>) -> Vec<f32> {
    let total = (SR as f32 * SECONDS) as u64;
    let mut out = Vec::with_capacity(total as usize * 2);
    engine
        .render_offline_with(SR, total, 0, factory, |_| {}, |block| {
            out.extend_from_slice(block);
            true
        })
        .unwrap();
    out
}

fn write_wav(path: &str, interleaved: &[f32]) {
    let spec = WavSpec {
        channels: 2,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
    let mut w = WavWriter::create(path, spec).expect("create wav");
    for &s in interleaved {
        w.write_sample(s).expect("write");
    }
    w.finalize().expect("finalize");
}

/// Crude harmonic-energy estimate: fraction of total energy that a naive
/// one-bin Goertzel says sits at `freq`. Used only to show the wet render
/// grows energy at the 2nd/3rd harmonics that a pure sine lacks.
fn goertzel_mag(mono: &[f32], freq: f32) -> f64 {
    let w = std::f64::consts::TAU * freq as f64 / SR as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in mono {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
}

#[test]
fn render_distortion_fx_wav() {
    let (engine, track_id) = build_sine_engine();

    // Dry (no factory → insert skipped).
    let dry = render(&engine, None);

    // Attach a real Hardwave Distortion insert.
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&track_id) {
            t.inserts.push(PluginSlot {
                id: "slot-dist".into(),
                plugin_id: NativeDistortion::ID.into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
    }
    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == NativeDistortion::ID {
            let mut d = NativeDistortion::new();
            // Crank drive (PARAM_DRIVE = 0, normalized → ×24 dB): ~18 dB of
            // soft clipping so the harmonics are obvious to the ear + FFT.
            d.set_parameter_value(0, 0.75);
            Some(Box::new(d))
        } else {
            None
        }
    };
    let wet = render(&engine, Some(&factory));

    // Both non-silent.
    let dry_peak = dry.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let wet_peak = wet.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    assert!(dry_peak > 0.05, "dry render is silent (peak {dry_peak})");
    assert!(wet_peak > 0.05, "wet render is silent (peak {wet_peak})");

    // Harmonic proof: measure 2nd (220 Hz) + 3rd (330 Hz) harmonic energy
    // relative to the 110 Hz fundamental. A clean sine has ~none; soft-clip
    // distortion generates them.
    let dry_mono: Vec<f32> = dry.chunks(2).map(|f| f[0]).collect();
    let wet_mono: Vec<f32> = wet.chunks(2).map(|f| f[0]).collect();
    let dry_h = (goertzel_mag(&dry_mono, 220.0) + goertzel_mag(&dry_mono, 330.0))
        / goertzel_mag(&dry_mono, 110.0).max(1e-9);
    let wet_h = (goertzel_mag(&wet_mono, 220.0) + goertzel_mag(&wet_mono, 330.0))
        / goertzel_mag(&wet_mono, 110.0).max(1e-9);
    eprintln!("harmonic ratio (2nd+3rd / fundamental): dry={dry_h:.4} wet={wet_h:.4}");
    assert!(
        wet_h > dry_h * 3.0,
        "distortion insert must add harmonic content (dry {dry_h:.4} vs wet {wet_h:.4})"
    );

    if let Ok(dir) = std::env::var("HW_FX_WAV_DIR") {
        write_wav(&format!("{dir}/daw_fx_dry.wav"), &dry);
        write_wav(&format!("{dir}/daw_fx_wet.wav"), &wet);
        eprintln!("wrote {dir}/daw_fx_dry.wav + daw_fx_wet.wav");
    }
}
