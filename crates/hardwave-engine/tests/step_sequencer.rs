//! The channel rack's steps are heard: in pattern mode an instrument
//! channel plays the active pattern, and in song mode it does not.

mod common;

use common::render_and_measure;
use hardwave_engine::engine::DawEngine;
use std::sync::atomic::Ordering;

const SAMPLE_RATE: u32 = 48_000;

fn engine_with_steps(steps: &str) -> DawEngine {
    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        let id = project.add_midi_track("Steps".to_string());
        let json = format!(
            r#"{{"v":1,"activeId":"p","patterns":[{{"id":"p","steps":{{"{id}":{steps}}}}}]}}"#
        );
        project.set_channel_rack_state(Some(json));
    }
    engine
}

#[test]
fn pattern_mode_plays_the_steps() {
    let engine = engine_with_steps("[1,0,0,0,1,0,0,0,1,0,0,0,1,0,0,0]");
    engine.transport.pattern_mode.store(true, Ordering::Relaxed);
    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64);
    assert_eq!(stats.nan_count, 0);
    assert!(
        stats.peak > 0.01,
        "four steps on the beat must sound, peak {:.5}",
        stats.peak
    );
}

#[test]
fn song_mode_does_not_play_the_pattern() {
    let engine = engine_with_steps("[1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1]");
    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 2);
    assert!(
        stats.peak < 1e-4,
        "no clips on the playlist, so silence; peak {:.5}",
        stats.peak
    );
}

#[test]
fn an_empty_pattern_is_silent_in_pattern_mode() {
    let engine = engine_with_steps("[0,0,0,0]");
    engine.transport.pattern_mode.store(true, Ordering::Relaxed);
    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 2);
    assert!(stats.peak < 1e-4, "peak {:.5}", stats.peak);
}

/// The pattern loops: the steps sound on the second pass too. The built-in
/// synths played a loop once and then fell silent.
#[test]
fn the_pattern_keeps_playing_after_it_loops() {
    let engine = engine_with_steps("[1,0,0,0,1,0,0,0,1,0,0,0,1,0,0,0]");
    engine.transport.pattern_mode.store(true, Ordering::Relaxed);
    // At 140 BPM one 16-step pattern is about 1.71 s; listen to the third pass.
    let bar = (SAMPLE_RATE as f64 * 60.0 / 140.0 * 4.0) as u64;
    let mut seen = 0u64;
    let mut late_peak = 0.0_f32;
    engine
        .render_offline(SAMPLE_RATE, bar * 3, |block| {
            for frame in block.chunks(2) {
                if seen > bar * 2 + 2_000 {
                    late_peak = late_peak.max(frame[0].abs()).max(frame[1].abs());
                }
                seen += 1;
            }
            true
        })
        .expect("render");
    assert!(
        late_peak > 0.01,
        "the third pass must sound too, peak {late_peak:.5}"
    );
}
