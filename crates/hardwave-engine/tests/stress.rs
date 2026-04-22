//! Stress tests for the DAW engine. These lock in the roadmap performance
//! claims (100+ tracks, 1000+ MIDI notes) by driving the real engine
//! through `render_offline` and verifying it completes without panicking
//! and produces finite samples.
//!
//! These are integration tests living under `tests/` so they see the
//! engine through its public API only, the same way the Tauri command
//! layer does.

use hardwave_engine::DawEngine;

#[test]
fn engine_renders_with_120_audio_tracks() {
    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        for i in 0..120 {
            project.add_audio_track(format!("Track {i}"));
        }
    }

    let sample_rate = 48_000;
    // 0.5 seconds of audio — long enough to exercise the rebuild_graph +
    // several process blocks, short enough to stay quick in CI.
    let total_samples = (sample_rate as u64) / 2;

    let mut any_nan = false;
    let mut any_inf = false;
    let mut total_blocks = 0_usize;
    let result = engine.render_offline(sample_rate, total_samples, |block| {
        total_blocks += 1;
        for &s in block {
            if s.is_nan() {
                any_nan = true;
            }
            if s.is_infinite() {
                any_inf = true;
            }
        }
        true
    });

    assert!(result.is_ok(), "render_offline failed: {result:?}");
    assert!(total_blocks > 0, "render produced no blocks");
    assert!(!any_nan, "engine produced NaN samples with 120 tracks");
    assert!(!any_inf, "engine produced infinite samples with 120 tracks");
}

#[test]
fn engine_handles_120_midi_tracks() {
    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        for i in 0..120 {
            project.add_midi_track(format!("Midi {i}"));
        }
    }

    let sample_rate = 48_000;
    let total_samples = (sample_rate as u64) / 4;

    let mut total_blocks = 0_usize;
    let result = engine.render_offline(sample_rate, total_samples, |_block| {
        total_blocks += 1;
        true
    });

    assert!(result.is_ok(), "midi stress render failed: {result:?}");
    assert!(total_blocks > 0);
}

#[test]
fn engine_renders_pattern_with_1500_midi_notes() {
    use hardwave_midi::{MidiClip, MidiNote};
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};

    let engine = DawEngine::new();
    let track_id = {
        let mut project = engine.project.lock();
        project.add_midi_track("Stress Pattern".to_string())
    };

    // Pack 1500 short notes into a 16-bar (61440-tick) pattern.
    let ppq: u64 = 960;
    let length_ticks: u64 = 16 * 4 * ppq;
    let mut notes: Vec<MidiNote> = Vec::with_capacity(1500);
    for i in 0..1500_u64 {
        let start_tick = (i * length_ticks / 1500).min(length_ticks.saturating_sub(1));
        notes.push(MidiNote {
            start_tick,
            duration_ticks: 24,
            pitch: ((i % 72) + 24) as u8,
            velocity: 0.75,
            channel: 0,
            muted: false,
        });
    }

    let mut midi = MidiClip::new(
        "stress-clip".to_string(),
        "stress".to_string(),
        length_ticks,
    );
    midi.notes = notes;

    {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .expect("freshly-added track must resolve");
        track.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "stress-clip".to_string(),
                clip: midi,
            }),
            track_id: track_id.clone(),
            position_ticks: 0,
            length_ticks,
            lane: 0,
        });
    }

    let sample_rate = 48_000;
    let total_samples = (sample_rate as u64) / 2;
    let mut any_nan = false;
    let result = engine.render_offline(sample_rate, total_samples, |block| {
        for &s in block {
            if s.is_nan() || s.is_infinite() {
                any_nan = true;
            }
        }
        true
    });

    assert!(result.is_ok(), "1500-note render failed: {result:?}");
    assert!(!any_nan, "engine produced NaN/Inf on 1500-note pattern");
}

#[test]
fn track_churn_leaves_baseline_track_count() {
    // Add + remove tracks in a long loop, verifying the project's track
    // count returns to its original baseline after each pair. Catches
    // cases where `remove_track` leaves dangling entries behind and where
    // `rebuild_graph` fails to clean up per-track resources — either of
    // which would show up as a growing track count or a failed render.
    let engine = DawEngine::new();
    let sample_rate = 48_000;
    let baseline = engine.project.lock().tracks.len();

    for cycle in 0..40 {
        let created_ids: Vec<String> = {
            let mut project = engine.project.lock();
            (0..10)
                .map(|i| project.add_audio_track(format!("Churn {cycle}-{i}")))
                .collect()
        };
        assert!(engine.render_offline(sample_rate, 512, |_| true).is_ok());
        {
            let mut project = engine.project.lock();
            for id in &created_ids {
                project.remove_track(id);
            }
        }
        assert!(engine.render_offline(sample_rate, 512, |_| true).is_ok());
        let count_now = engine.project.lock().tracks.len();
        assert_eq!(
            count_now, baseline,
            "cycle {cycle} leaked tracks: expected {baseline}, got {count_now}"
        );
    }
}

#[test]
fn engine_survives_rapid_rebuild_with_many_tracks() {
    // Add tracks one at a time, re-rendering briefly between each addition.
    // Verifies that `rebuild_graph` stays stable as the graph grows and
    // that renderer state survives repeated rebuilds.
    let engine = DawEngine::new();
    let sample_rate = 48_000;

    for i in 0..50 {
        {
            let mut project = engine.project.lock();
            project.add_audio_track(format!("Track {i}"));
        }
        let result = engine.render_offline(sample_rate, 512, |_| true);
        assert!(
            result.is_ok(),
            "render_offline failed after adding track {i}: {result:?}"
        );
    }

    let final_count = engine.project.lock().tracks.len();
    // 50 adds + the default Master track.
    assert!(
        final_count >= 50,
        "expected >= 50 tracks, got {final_count}"
    );
}
