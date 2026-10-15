//! How much a song's empty mixer inserts cost: a new project has 500 of
//! them, and on a small Windows VM playing an empty song ran above the
//! audio deadline. Prints the render's speed against real time.

use hardwave_engine::DawEngine;
use std::time::Instant;

/// Seconds of CPU for a render of `seconds` of audio, the song set up
/// with `tracks` empty tracks.
fn render_secs(tracks: usize, seconds: f64) -> f64 {
    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        // A new engine's song already has its mixer inserts; keep `tracks`.
        let ids: Vec<String> = project
            .tracks
            .iter()
            .filter(|t| !matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone())
            .collect();
        for id in ids.iter().skip(tracks) {
            project.remove_track(id);
        }
        for i in project.tracks.len().saturating_sub(1)..tracks {
            project.add_audio_track(format!("Insert {i}"));
        }
    }
    let sample_rate = 48_000;
    let started = Instant::now();
    engine
        .render_offline(sample_rate, (sample_rate as f64 * seconds) as u64, |_| true)
        .unwrap();
    started.elapsed().as_secs_f64()
}

#[test]
#[ignore = "a measurement, not a check: run with --ignored --nocapture"]
fn empty_inserts_cost() {
    for n in [0, 50, 125, 250, 500] {
        let short = render_secs(n, 2.0);
        let long = render_secs(n, 12.0);
        let per_second = (long - short) / 10.0;
        println!(
            "{n:>4} empty tracks: set-up {:>7.1} ms, playing {:>6.1} ms per second of audio",
            (short - 2.0 * per_second) * 1000.0,
            per_second * 1000.0
        );
    }
}
