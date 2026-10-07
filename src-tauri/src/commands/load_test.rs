//! How many tracks this machine can play.
//!
//! "Will it run my project" has no answer until someone measures, and
//! the person best placed to measure is the one with the machine. The
//! test builds songs of test tracks in an engine of its own, beside the
//! open project and without touching it, and times every block at the
//! sound card's own buffer size and sample rate. It keeps doubling the
//! track count until blocks start running late, then narrows down on
//! the largest count that still plays.
//!
//! A block is late when it takes longer than the sound it makes. A
//! count passes when 99 blocks in 100 finish within 70% of that time:
//! the rest is for the sound card driver, the screen and the occasional
//! spike a real plug-in has.

use crate::AppState;
use hardwave_engine::DawEngine;
use hardwave_project::track::PluginSlot;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

const PPQ: u64 = hardwave_midi::PPQ;
const BAR: u64 = 4 * PPQ;

/// What every test track carries: a synth, then the three effects
/// nearly every channel in a hard dance mix has.
const TEST_CHAIN: [&str; 4] = [
    hardwave_native_plugins::NativeTripleOsc::ID,
    hardwave_native_plugins::NativeEq::ID,
    hardwave_native_plugins::NativeCompressor::ID,
    hardwave_native_plugins::NativeSaturator::ID,
];

/// The share of a block's time the engine may use and still pass.
const HEADROOM: f64 = 0.7;
/// The most tracks the test tries. A machine past this is not the
/// limit any more; the plug-ins someone chooses are.
const MOST: usize = 512;
/// Audio rendered per measurement, after the warm-up.
const MEASURED: Duration = Duration::from_millis(1500);
const WARM_UP: Duration = Duration::from_millis(250);

/// One measured track count.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub tracks: usize,
    /// How long 99 blocks in 100 took at most.
    pub p99_ms: f64,
    pub worst_ms: f64,
    pub mean_ms: f64,
    /// The time a block's sound lasts.
    pub budget_ms: f64,
    pub passed: bool,
    /// "now" for the current settings, "multicore" for the run with
    /// every core helping.
    pub run: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadTestResult {
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub budget_ms: f64,
    /// Threads helping the audio thread with the current settings.
    pub threads: usize,
    /// The most test tracks that played with the current settings.
    pub tracks: usize,
    /// True when even the most the test tries played without trouble.
    pub beyond_test: bool,
    /// With multi-core audio switched on, when it is off now.
    pub multicore: Option<MulticoreResult>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MulticoreResult {
    pub threads: usize,
    pub tracks: usize,
    pub beyond_test: bool,
}

/// Where the search ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub tracks: usize,
    pub beyond_test: bool,
}

/// Find the largest count that passes, measuring as few as it can.
///
/// It doubles from 8 until a count fails, then halves the gap between
/// the last pass and the first failure until the two are within a
/// tenth of each other. Measuring is the slow part, so a typical
/// machine is done in about ten measurements.
pub fn search(mut passes: impl FnMut(usize) -> bool, most: usize) -> Capacity {
    let mut good = 0usize;
    let mut n = 8usize.min(most);
    let mut bad = loop {
        if !passes(n) {
            break n;
        }
        good = n;
        if n >= most {
            return Capacity {
                tracks: n,
                beyond_test: true,
            };
        }
        n = (n * 2).min(most);
    };
    while bad - good > 1 && bad - good > good / 10 {
        let mid = good + (bad - good) / 2;
        if passes(mid) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    Capacity {
        tracks: good,
        beyond_test: false,
    }
}

/// Summarise block times against the time each block's sound lasts.
fn summarise(times: &[Duration], budget: Duration, tracks: usize, run: &str) -> Step {
    let mut ms: Vec<f64> = times.iter().map(|t| t.as_secs_f64() * 1000.0).collect();
    ms.sort_by(|a, b| a.total_cmp(b));
    let budget_ms = budget.as_secs_f64() * 1000.0;
    let p99_ms = ms
        .get((ms.len() * 99).div_ceil(100).saturating_sub(1))
        .copied()
        .unwrap_or(0.0);
    let worst_ms = ms.last().copied().unwrap_or(0.0);
    let mean_ms = if ms.is_empty() {
        0.0
    } else {
        ms.iter().sum::<f64>() / ms.len() as f64
    };
    Step {
        tracks,
        p99_ms,
        worst_ms,
        mean_ms,
        budget_ms,
        passed: !ms.is_empty() && p99_ms <= budget_ms * HEADROOM,
        run: run.to_string(),
    }
}

/// Fill a project with test tracks: each a synth playing a held chord
/// through the test chain, so every plug-in has something to work on
/// in every block.
pub(crate) fn add_test_tracks(
    project: &mut hardwave_project::Project,
    count: usize,
) -> Vec<String> {
    // Am, F, C, G, an octave apart per track so voices do not all
    // stack on the same notes.
    let chords: [[u8; 3]; 4] = [[57, 60, 64], [53, 57, 60], [48, 52, 55], [55, 59, 62]];
    let bars = 8u64;
    let mut ids = Vec::with_capacity(count);
    for i in 0..count {
        // On the playlist's own rows, so the test song can be looked at.
        let id = super::starter::claim_row(project, i + 1, &format!("Test {}", i + 1), true);
        let shift = ((i % 3) as i8 - 1) * 12;
        let mut notes = Vec::new();
        for bar in 0..bars {
            for &pitch in &chords[(bar % 4) as usize] {
                let pitch = (pitch as i8 + shift).clamp(24, 108) as u8;
                notes.push(super::starter::note(bar * BAR, BAR - PPQ / 8, pitch, 0.7));
            }
        }
        if let Some(track) = project.track_mut(&id) {
            track.volume_db = -24.0;
            track.clips.push(super::starter::midi_clip(
                &id,
                "Test chords",
                0,
                bars * BAR,
                notes,
            ));
            for plugin_id in TEST_CHAIN {
                track.inserts.push(PluginSlot {
                    id: uuid::Uuid::new_v4().to_string(),
                    plugin_id: plugin_id.to_string(),
                    enabled: true,
                    state: None,
                    sidechain_source: None,
                    wet: 1.0,
                });
            }
        }
        ids.push(id);
    }
    ids
}

fn native(id: &str) -> Option<Box<dyn hardwave_plugin_host::types::HostedPlugin>> {
    let descriptor = hardwave_native_plugins::native_plugin_descriptors()
        .into_iter()
        .find(|d| d.id == id)?;
    super::plugins::instantiate_plugin(&descriptor).ok()
}

/// Time one track count in an engine of its own.
fn measure(tracks: usize, sample_rate: u32, buffer_size: u32, threads: usize, run: &str) -> Step {
    let engine = DawEngine::new();
    if threads > 0 {
        engine.set_worker_threads(threads);
    }
    {
        let mut project = engine.project.lock();
        if let Some(entry) = project.tempo_map.entries.get_mut(0) {
            entry.bpm = 150.0;
        }
        add_test_tracks(&mut project, tracks);
    }
    engine.rebuild_graph();

    let samples = |d: Duration| (d.as_secs_f64() * sample_rate as f64) as u64;
    let times = engine.measure_block_times(
        sample_rate,
        buffer_size as usize,
        samples(WARM_UP + MEASURED),
        Some(&native),
    );
    let skip = samples(WARM_UP).div_ceil(buffer_size as u64) as usize;
    let budget = Duration::from_secs_f64(buffer_size as f64 / sample_rate as f64);
    summarise(&times[skip.min(times.len())..], budget, tracks, run)
}

/// Run the whole test with the sound card's settings.
///
/// Playback stops first: the test uses every core it is allowed, and
/// whatever was playing would crackle and make the numbers worse.
#[tauri::command]
pub async fn run_load_test(app: AppHandle) -> Result<LoadTestResult, String> {
    let (sample_rate, buffer_size, threads) = {
        let state = app.state::<AppState>();
        let engine = state.engine.lock();
        engine.send_command(hardwave_engine::transport::TransportCommand::Stop);
        let (_, sample_rate, buffer_size) = engine.audio_config();
        (
            sample_rate.max(8_000),
            buffer_size.max(32),
            engine.worker_threads(),
        )
    };
    let progress = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut steps = Vec::new();
        let run = |name: &str, threads: usize, steps: &mut Vec<Step>| {
            search(
                |n| {
                    let step = measure(n, sample_rate, buffer_size, threads, name);
                    let _ = progress.emit("load-test-step", &step);
                    let passed = step.passed;
                    steps.push(step);
                    passed
                },
                MOST,
            )
        };
        let now = run("now", threads, &mut steps);
        // With multi-core audio off, show what switching it on would
        // give, because that is the one setting that changes the answer.
        let multicore = if threads == 0 {
            let helpers = hardwave_engine::parallel::default_worker_count();
            (helpers > 0).then(|| {
                let found = run("multicore", helpers, &mut steps);
                MulticoreResult {
                    threads: helpers,
                    tracks: found.tracks,
                    beyond_test: found.beyond_test,
                }
            })
        } else {
            None
        };
        LoadTestResult {
            sample_rate,
            buffer_size,
            budget_ms: buffer_size as f64 / sample_rate as f64 * 1000.0,
            threads,
            tracks: now.tracks,
            beyond_test: now.beyond_test,
            multicore,
            steps,
        }
    })
    .await
    .map_err(|e| format!("the performance test stopped: {e}"))
}

/// Put test tracks into the open project, so the screen can be tried
/// at a track count as well as the sound.
///
/// The window starts a new project first. The plug-ins go in through
/// the same path as a person adding them, so they are live.
#[tauri::command]
pub fn open_load_test_song(state: State<AppState>, tracks: usize) -> Result<usize, String> {
    let tracks = tracks.clamp(1, MOST);
    state.engine.lock().snapshot_before_mutation();
    let ids = {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let ids = add_test_tracks(&mut project, tracks);
        // The slots are added again below, live; drop the planned ones.
        for id in &ids {
            if let Some(track) = project.track_mut(id) {
                track.inserts.clear();
            }
        }
        ids
    };
    {
        let engine = state.engine.lock();
        engine.sync_track_meters();
        engine.rebuild_graph();
    }
    for id in ids {
        for plugin_id in TEST_CHAIN {
            super::plugins::add_plugin_to_track_quietly(
                state.clone(),
                id.clone(),
                plugin_id.to_string(),
            )?;
        }
    }
    Ok(tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_finds_the_limit_without_measuring_everything() {
        for limit in [1, 5, 8, 9, 40, 100, 137, 300, 511] {
            let mut measured = 0;
            let found = search(
                |n| {
                    measured += 1;
                    n <= limit
                },
                MOST,
            );
            assert!(!found.beyond_test);
            assert!(found.tracks <= limit, "never more than plays: {limit}");
            assert!(
                found.tracks + found.tracks / 10 + 1 >= limit,
                "within a tenth of the real limit {limit}, found {}",
                found.tracks
            );
            assert!(measured <= 16, "{measured} measurements for {limit}");
        }
    }

    #[test]
    fn a_machine_past_the_test_says_so() {
        let found = search(|_| true, MOST);
        assert_eq!(
            found,
            Capacity {
                tracks: MOST,
                beyond_test: true
            }
        );
    }

    #[test]
    fn a_machine_that_cannot_play_one_says_none() {
        let found = search(|_| false, MOST);
        assert_eq!(found.tracks, 0);
        assert!(!found.beyond_test);
    }

    #[test]
    fn a_count_passes_on_its_slowest_blocks_not_its_average() {
        let budget = Duration::from_millis(10);
        // Mostly quick, but two blocks in a hundred run over: that is
        // a crackle twice a second, which is a fail.
        let mut times = vec![Duration::from_millis(2); 98];
        times.extend([Duration::from_millis(12); 2]);
        let step = summarise(&times, budget, 10, "now");
        assert!(step.mean_ms < 5.0);
        assert!(!step.passed, "p99 {} over the budget", step.p99_ms);

        let step = summarise(&[Duration::from_millis(6); 100], budget, 10, "now");
        assert!(step.passed, "6 ms of 10 is within the headroom");
        let step = summarise(&[Duration::from_millis(8); 100], budget, 10, "now");
        assert!(!step.passed, "8 ms of 10 leaves too little");
    }

    #[test]
    fn test_tracks_carry_the_whole_chain_and_make_a_sound() {
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let ids = add_test_tracks(&mut project, 3);
            for id in &ids {
                let track = project.track(id).unwrap();
                assert_eq!(track.inserts.len(), TEST_CHAIN.len());
                assert!(!track.clips.is_empty());
            }
        }
        engine.rebuild_graph();
        let mut peak = 0.0f32;
        engine
            .render_offline_with(
                48_000,
                24_000,
                0,
                Some(&native),
                |_| {},
                |block| {
                    for s in block {
                        peak = peak.max(s.abs());
                    }
                    true
                },
            )
            .unwrap();
        assert!(peak > 0.001, "the test chords are audible: {peak}");
    }

    #[test]
    fn a_measurement_times_every_block_after_the_warm_up() {
        let step = measure(2, 48_000, 256, 0, "now");
        assert!(
            step.budget_ms > 5.0 && step.budget_ms < 5.5,
            "256 samples at 48 kHz"
        );
        assert!(step.p99_ms > 0.0);
        assert!(step.worst_ms >= step.p99_ms);
    }
}
