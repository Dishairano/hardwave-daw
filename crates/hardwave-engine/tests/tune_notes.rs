//! Notes tuned by hand in the sample window's NOTES view: heard on
//! playback and in an export alike, made off the audio thread, and the
//! renders nothing plays any more let go.

mod common;

use common::*;
use hardwave_engine::audio_pool::AudioBuffer;
use hardwave_engine::DawEngine;
use hardwave_project::clip::{ClipContent, ClipPlacement, TunedNote};

const SR: u32 = 48_000;

/// A sung-like line: silence, a 220 Hz note from 0.2 s to 0.8 s, silence
/// to 1.2 s. Stereo, the right side quieter.
fn take() -> AudioBuffer {
    let n = (1.2 * SR as f64) as usize;
    let left: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / SR as f64;
            if !(0.2..0.8).contains(&t) {
                return 0.0;
            }
            let edge = ((t - 0.2) / 0.01).min((0.8 - t) / 0.01).min(1.0);
            let p = std::f64::consts::TAU * 220.0 * t;
            ((p.sin() * 0.6 + (2.0 * p).sin() * 0.25 + (3.0 * p).sin() * 0.12) * edge * 0.6) as f32
        })
        .collect();
    let right = left.iter().map(|s| s * 0.7).collect();
    AudioBuffer {
        channels: vec![left, right],
        sample_rate: SR,
        num_frames: n,
    }
}

/// The note moved by `semitones`, everything else as sung.
fn moved(semitones: f64) -> Vec<TunedNote> {
    vec![TunedNote {
        start: 0.2,
        end: 0.8,
        sung: 57.0,
        pitch: 57.0 + semitones,
        drift: 1.0,
        new_start: 0.2,
        new_end: 0.8,
        formants: true,
    }]
}

/// An engine playing the take from the song's start, its notes as given.
fn engine_with(notes: Vec<TunedNote>, stretch: f64) -> DawEngine {
    let engine = DawEngine::new();
    let buffer = take();
    let frames = buffer.num_frames as u64;
    engine.audio_pool.insert("vocal".into(), buffer);
    let mut project = engine.project.lock();
    let track_id = project.add_audio_track("Vocal".into());
    let mut clip = make_audio_clip("vocal", frames);
    clip.tuned_notes = notes;
    clip.stretch_ratio = stretch;
    project
        .track_mut(&track_id)
        .unwrap()
        .clips
        .push(ClipPlacement {
            content: ClipContent::Audio(clip),
            track_id: track_id.clone(),
            position_ticks: 0,
            length_ticks: 1_000_000,
            lane: 0,
            color: None,
        });
    drop(project);
    engine
}

/// The left side of an export.
fn export(engine: &DawEngine, seconds: f64) -> Vec<f32> {
    let mut out = Vec::new();
    engine
        .render_offline(SR, (seconds * SR as f64) as u64, |block| {
            out.extend(block.chunks(2).map(|f| f[0]));
            true
        })
        .unwrap();
    out
}

/// The pitch between two times, averaged over windows, in Hz.
fn pitch(audio: &[f32], from: f64, to: f64) -> f64 {
    let settings = hardwave_dsp::audio_to_midi::Settings {
        sample_rate: SR as f64,
        ..Default::default()
    };
    let mut tracker = hardwave_dsp::audio_to_midi::PitchTracker::new(2048);
    let (a, b) = ((from * SR as f64) as usize, (to * SR as f64) as usize);
    let (mut sum, mut count) = (0.0, 0);
    let mut at = a;
    while at + 2048 <= b {
        if let Some(hz) = tracker.pitch(&audio[at..at + 2048], &settings) {
            sum += hz;
            count += 1;
        }
        at += 512;
    }
    assert!(count > 0, "no pitch between {from}s and {to}s");
    sum / count as f64
}

fn cents(heard: f64, want: f64) -> f64 {
    1200.0 * (heard / want).log2()
}

/// The pool id of the clip's tuned take.
fn tuned_key(engine: &DawEngine) -> Option<String> {
    let project = engine.project.lock();
    project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find_map(|c| match &c.content {
            ClipContent::Audio(a) if a.has_tuned_notes() => Some(format!(
                "vocal#hw-tune:{:016x}",
                hardwave_project::clip::tuned_notes_hash(&a.tuned_notes)
            )),
            _ => None,
        })
}

#[test]
fn a_note_tuned_by_hand_is_heard_in_the_export() {
    let untouched = export(&engine_with(Vec::new(), 1.0), 1.0);
    assert!(cents(pitch(&untouched, 0.3, 0.7), 220.0).abs() < 5.0);

    let engine = engine_with(moved(3.0), 1.0);
    let out = export(&engine, 1.0);
    let want = 220.0 * 2f64.powf(3.0 / 12.0);
    let heard = pitch(&out, 0.3, 0.7);
    assert!(
        cents(heard, want).abs() < 5.0,
        "the export should have the note 3 semitones up: {heard:.1} Hz, want {want:.1}"
    );
    // The file in the pool is the take as recorded.
    let file = engine.audio_pool.get("vocal").unwrap();
    assert!(cents(pitch(&file.channels[0], 0.3, 0.7), 220.0).abs() < 5.0);
}

#[test]
fn a_stretched_clip_is_stretched_from_its_tuned_take() {
    let engine = engine_with(moved(-2.0), 1.5);
    let out = export(&engine, 1.6);
    let want = 220.0 * 2f64.powf(-2.0 / 12.0);
    // The note now runs from 0.3 s to 1.2 s.
    let heard = pitch(&out, 0.45, 1.05);
    assert!(
        cents(heard, want).abs() < 8.0,
        "stretched, the tuned note keeps its new pitch: {heard:.1} Hz, want {want:.1}"
    );
    let key = tuned_key(&engine).unwrap();
    assert!(
        engine
            .audio_pool
            .contains(&format!("{key}#hw-stretch:1.500000:0.000000")),
        "the stretch is made from the tuned take"
    );
}

#[test]
fn playback_bakes_the_tuned_take_off_the_audio_thread_and_the_export_uses_it() {
    let engine = engine_with(moved(2.0), 1.0);
    let key = tuned_key(&engine).unwrap();
    assert!(
        !engine.audio_pool.contains(&key),
        "nothing is baked by itself"
    );
    engine.prebake_stretch_sources_async();
    let mut landed = false;
    for _ in 0..600 {
        if engine.audio_pool.contains(&key) {
            landed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(landed, "the tuned take never landed in the pool");
    let baked = engine.audio_pool.get(&key).unwrap();
    // The export finds the same render rather than making its own.
    let out = export(&engine, 1.0);
    assert!(std::sync::Arc::ptr_eq(
        &baked,
        &engine.audio_pool.get(&key).unwrap()
    ));
    let want = 220.0 * 2f64.powf(2.0 / 12.0);
    assert!(cents(pitch(&out, 0.3, 0.7), want).abs() < 5.0);
}

#[test]
fn a_new_move_lets_go_of_the_render_nothing_plays_any_more() {
    let engine = engine_with(moved(2.0), 1.0);
    let wait_for = |key: &str| {
        for _ in 0..600 {
            if engine.audio_pool.contains(key) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    };
    engine.prebake_stretch_sources_async();
    let first = tuned_key(&engine).unwrap();
    assert!(wait_for(&first));

    // The note moved again.
    {
        let mut project = engine.project.lock();
        for track in &mut project.tracks {
            for clip in &mut track.clips {
                if let ClipContent::Audio(a) = &mut clip.content {
                    a.tuned_notes = moved(4.0);
                }
            }
        }
    }
    engine.prebake_stretch_sources_async();
    let second = tuned_key(&engine).unwrap();
    assert_ne!(first, second);
    assert!(wait_for(&second));
    let mut gone = false;
    for _ in 0..200 {
        if !engine.audio_pool.contains(&first) {
            gone = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        gone,
        "the first render should be let go once the second has landed"
    );
    assert!(engine.audio_pool.contains("vocal"), "the file itself stays");
}
