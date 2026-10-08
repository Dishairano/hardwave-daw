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
