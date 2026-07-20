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

/// Single-bin Goertzel magnitude at `freq` — enough to tell whether a
/// rendered tone still sits at its original pitch.
fn goertzel_mag(mono: &[f32], sample_rate: u32, freq: f32) -> f64 {
    let w = std::f64::consts::TAU * freq as f64 / sample_rate as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in mono {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
}

/// Regression gate for pitch-preserving time-stretch.
///
/// Before the bake path landed, `source_step = pitch_factor / stretch` meant a
/// clip at `stretch_ratio = 2.0` played back an octave DOWN (measured 439.5 Hz
/// → 219.7 Hz). Now the stretch is baked with `apply_stretch`, so duration and
/// pitch are independent: a 440 Hz clip stays at 440 Hz however it's stretched.
///
/// Writes the rendered WAVs when HW_STRETCH_WAV_DIR is set, so the artefact
/// quality can be judged by ear as well as by this assertion.
#[test]
fn stretch_preserves_pitch() {
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
    let half = render_clip(0.5);
    if let Ok(dir) = std::env::var("HW_STRETCH_WAV_DIR") {
        write_stereo_wav(&format!("{dir}/daw_stretch_normal.wav"), sample_rate, &normal);
        write_stereo_wav(&format!("{dir}/daw_stretch_2x.wav"), sample_rate, &stretched);
        write_stereo_wav(&format!("{dir}/daw_stretch_half.wav"), sample_rate, &half);
        eprintln!("wrote {dir}/daw_stretch_{{normal,2x,half}}.wav");
    }

    // Skip the clip onset, where the phase vocoder's first frames settle.
    let mono = |v: &[f32]| -> Vec<f32> {
        v.chunks(2)
            .skip(sample_rate as usize / 4)
            .map(|f| f[0])
            .collect()
    };
    for (label, buf, ratio) in [
        ("normal", &normal, 1.0f64),
        ("2x", &stretched, 2.0),
        ("half", &half, 0.5),
    ] {
        let m = mono(buf);
        // Level is checked over the WHOLE render, not the pitch-analysis
        // window: the normalisation spikes this guards against live at the
        // head and tail, exactly where `mono()` skips.
        let peak = buf.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
        assert!(peak > 0.05, "{label} render is silent (peak {peak})");
        // Stretching must not change the level. The source is a 0.5 sine
        // (~0.354 at the master after pan-centre), so anything near or past
        // full scale means the stretch normalisation blew up.
        assert!(
            peak < 0.6,
            "{label} render level exploded (peak {peak}) — stretch normalisation regression"
        );
        assert!(
            buf.iter().all(|s| s.is_finite()),
            "{label} render contains NaN/inf"
        );
        let f440 = goertzel_mag(&m, sample_rate, 440.0);
        let f220 = goertzel_mag(&m, sample_rate, 220.0);
        let f880 = goertzel_mag(&m, sample_rate, 880.0);
        eprintln!("stretch {ratio}: 440Hz={f440:.0} 220Hz={f220:.0} 880Hz={f880:.0}");
        // The fundamental must stay put: stretching changes duration, not pitch.
        assert!(
            f440 > f220 * 4.0 && f440 > f880 * 4.0,
            "stretch_ratio={ratio} must preserve the 440 Hz fundamental \
             (440={f440:.0}, 220={f220:.0}, 880={f880:.0}) — varispeed regression?"
        );
    }
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

/// The other half of the decoupling promise: `pitch_semitones` must change
/// pitch WITHOUT changing duration.
///
/// On the old resample path a +12 semitone clip played back at twice the
/// source step, so it finished in half the time — pitch and length moved
/// together. With the bake, the source is pitch-shifted in place, so the clip
/// keeps sounding for its whole length at the new pitch.
#[test]
fn pitch_shift_preserves_duration() {
    let sample_rate = 48_000_u32;
    // Source is 2 s long; render 2 s. Under the old path a +12 clip would be
    // silent through the second half (source exhausted at 2x read speed).
    let render_clip = |semis: f64| -> Vec<f32> {
        let engine = DawEngine::new();
        let buf = make_sine_buffer(sample_rate, 2.0, 440.0, 0.5);
        let frames = buf.num_frames as u64;
        engine.audio_pool.insert("pitch-src".to_string(), buf);
        {
            let mut project = engine.project.lock();
            let track_id = project.add_audio_track("Pitch".into());
            if let Some(t) = project.track_mut(&track_id) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "clip-pitch".into(),
                        name: "pitch".into(),
                        source_path: "pitch-src".into(),
                        source_hash: String::new(),
                        source_start: 0,
                        source_end: frames,
                        gain_db: 0.0,
                        fade_in_ticks: 0,
                        fade_out_ticks: 0,
                        muted: false,
                        reversed: false,
                        pitch_semitones: semis,
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

    let shifted = render_clip(12.0);
    let left: Vec<f32> = shifted.chunks(2).map(|f| f[0]).collect();
    let half = left.len() / 2;

    // Duration preserved: the second half must still be sounding.
    let rms = |v: &[f32]| -> f64 {
        (v.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / v.len().max(1) as f64).sqrt()
    };
    let first = rms(&left[..half]);
    let second = rms(&left[half..]);
    eprintln!("pitch +12: first-half rms={first:.4} second-half rms={second:.4}");
    assert!(
        second > first * 0.5,
        "a +12 semitone clip must still sound through its full length \
         (first={first:.4}, second={second:.4}) — pitch is changing duration"
    );

    // Pitch actually moved up an octave: 880 Hz should now dominate 440 Hz.
    let mid = &left[left.len() / 4..left.len() / 2];
    let f440 = goertzel_mag(mid, sample_rate, 440.0);
    let f880 = goertzel_mag(mid, sample_rate, 880.0);
    eprintln!("pitch +12: 440Hz={f440:.0} 880Hz={f880:.0}");
    assert!(
        f880 > f440 * 2.0,
        "+12 semitones should move the fundamental to 880 Hz \
         (440={f440:.0}, 880={f880:.0})"
    );
}

/// The stretch bake must stay OFF the audio thread.
///
/// `rebuild_graph` runs inside `AudioCallback::process`, and baking is an FFT
/// over the whole source plus multi-megabyte allocations — running it there
/// would stall the callback for as long as the file is big. So the audio
/// thread only ever looks a bake up, and `prebake_stretch_sources` (UI thread,
/// also called by `rebuild_graph` and before an offline render) is what
/// actually populates the pool.
///
/// This asserts that contract from the outside: a stretched clip adds nothing
/// to the pool until the explicit prebake runs.
#[test]
fn stretch_bake_is_explicit_and_off_the_audio_thread() {
    let sample_rate = 48_000_u32;
    let engine = DawEngine::new();
    let buf = make_sine_buffer(sample_rate, 1.0, 440.0, 0.5);
    let frames = buf.num_frames as u64;
    engine.audio_pool.insert("bake-src".to_string(), buf);
    {
        let mut project = engine.project.lock();
        let track_id = project.add_audio_track("Bake".into());
        if let Some(t) = project.track_mut(&track_id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-bake".into(),
                    name: "bake".into(),
                    source_path: "bake-src".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: frames,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 2.0,
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
    }

    // Only the raw source is resident — nothing baked yet.
    let before = engine.audio_pool.stats().entry_count;
    assert_eq!(before, 1, "expected just the raw source, got {before} entries");

    // The explicit off-thread bake is what materialises the variant.
    engine.prebake_stretch_sources();
    let after = engine.audio_pool.stats().entry_count;
    assert_eq!(
        after, 2,
        "prebake_stretch_sources must add the baked variant (before={before}, after={after})"
    );

    // Idempotent: a second pass reuses the cache rather than re-baking.
    engine.prebake_stretch_sources();
    assert_eq!(
        engine.audio_pool.stats().entry_count,
        2,
        "prebake must be idempotent — a cached variant should not be rebaked"
    );
}

/// The async bake must actually land, and must not stampede.
///
/// Baking a long stem takes seconds (~16 s for 30 s of stereo), so the UI path
/// hands it to a background thread and lets the clip play varispeed until it
/// lands. Dragging a stretch control fires a rebuild per frame, so repeated
/// calls must coalesce rather than spawn a bake per frame.
#[test]
fn async_stretch_bake_lands_and_does_not_stampede() {
    let sample_rate = 48_000_u32;
    let engine = DawEngine::new();
    let buf = make_sine_buffer(sample_rate, 1.0, 440.0, 0.5);
    let frames = buf.num_frames as u64;
    engine.audio_pool.insert("async-src".to_string(), buf);
    {
        let mut project = engine.project.lock();
        let track_id = project.add_audio_track("Async".into());
        if let Some(t) = project.track_mut(&track_id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-async".into(),
                    name: "async".into(),
                    source_path: "async-src".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: frames,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.5,
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
    }
    assert_eq!(engine.audio_pool.stats().entry_count, 1);

    // Simulate a control drag: many rebuilds in quick succession.
    for _ in 0..8 {
        engine.prebake_stretch_sources_async();
    }

    // The bake is off-thread, so poll for it rather than assuming timing.
    let mut landed = false;
    for _ in 0..100 {
        if engine.audio_pool.stats().entry_count >= 2 {
            landed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(landed, "async bake never landed in the pool");

    // Exactly one variant — the 8 calls coalesced instead of stampeding.
    let count = engine.audio_pool.stats().entry_count;
    assert_eq!(
        count, 2,
        "expected the raw source plus one baked variant, got {count} — bakes stampeded"
    );
}

/// Rendering at a rate other than the source's must not change its pitch.
///
/// Pool buffers are normalised to the *device* rate on import, but an export
/// renders at whatever rate the user chose. `source_step` carried no
/// pool-rate/render-rate term, so bouncing at 44.1 kHz from a 48 kHz device
/// read every audio clip 8.8% too slow — flat and long — while MIDI/synth
/// tracks, generated at the render rate, stayed in tune. Samples and synths
/// came out of the same bounce in different keys.
#[test]
fn render_rate_does_not_detune_audio_clips() {
    let source_rate = 48_000_u32;
    // Build the source at 48k, then render it at several rates.
    let render_at = |render_rate: u32| -> Vec<f32> {
        let engine = DawEngine::new();
        let buf = make_sine_buffer(source_rate, 2.0, 440.0, 0.5);
        let frames = buf.num_frames as u64;
        engine.audio_pool.insert("rate-src".to_string(), buf);
        {
            let mut project = engine.project.lock();
            let track_id = project.add_audio_track("Rate".into());
            if let Some(t) = project.track_mut(&track_id) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "clip-rate".into(),
                        name: "rate".into(),
                        source_path: "rate-src".into(),
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
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Linear,
                    }),
                    track_id: track_id.clone(),
                    position_ticks: 0,
                    length_ticks: 1_000_000,
                    lane: 0,
                });
            }
        }
        let mut out = Vec::new();
        engine
            .render_offline(render_rate, render_rate as u64, |block| {
                out.extend_from_slice(block);
                true
            })
            .unwrap();
        out
    };

    for rate in [48_000_u32, 44_100, 96_000] {
        let buf = render_at(rate);
        let mono: Vec<f32> = buf.chunks(2).skip(rate as usize / 8).map(|f| f[0]).collect();
        let f440 = goertzel_mag(&mono, rate, 440.0);
        // 44.1k from a 48k source detuned to ~404 Hz; 96k to ~880 Hz.
        let f404 = goertzel_mag(&mono, rate, 404.0);
        let f880 = goertzel_mag(&mono, rate, 880.0);
        eprintln!("render @{rate}: 440Hz={f440:.0} 404Hz={f404:.0} 880Hz={f880:.0}");
        assert!(
            f440 > f404 * 3.0 && f440 > f880 * 3.0,
            "rendering a 48 kHz source at {rate} Hz must keep it at 440 Hz \
             (440={f440:.0}, 404={f404:.0}, 880={f880:.0})"
        );
    }
}
