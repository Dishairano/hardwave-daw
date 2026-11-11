//! What the first export of a part of an FL song costs right after the
//! import, against the second: the stretched clips are baked before a
//! render, and that bake used to run with the engine locked. A
//! measurement, not a check.
//!
//! Run with the song and its samples in one folder:
//!   HW_FL_SONG=/path/to/song.flp hw-daw-cargo test -j 1 -p hardwave-engine \
//!     --test fl_export_bake -- --ignored --nocapture
//! `HW_EXPORT_FROM` and `HW_EXPORT_TO` set the range in seconds (2:37 to
//! 2:57 by default). The song is never part of the repository.

use std::time::{Duration, Instant};

use hardwave_engine::DawEngine;
use hardwave_project::clip::ClipContent;

/// The song `HW_FL_SONG` names, opened as this build opens it, and the
/// engine that loaded its samples. `None` when the variable is not set.
fn open_song() -> Option<(DawEngine, hardwave_project::Project)> {
    let Ok(file) = std::env::var("HW_FL_SONG") else {
        eprintln!("HW_FL_SONG is not set; skipped");
        return None;
    };
    let file = std::path::PathBuf::from(file);
    let dir = file.parent().unwrap().to_path_buf();
    let fl = hardwave_project::flp_parser::parse(&std::fs::read(&file).unwrap()).unwrap();
    let resolve = |p: &str| {
        let name = p.rsplit(['\\', '/']).next()?;
        let f = dir.join(name);
        f.is_file().then(|| f.to_string_lossy().into_owned())
    };
    let built = hardwave_project::fl_build::build_project(&fl, "FL song", &resolve);
    let engine = DawEngine::new();
    engine
        .audio_source_loader(None)
        .load_sources_of(&built.project, |_, _| {});
    let mut song = built.project;
    for fixup in &built.audio_fixups {
        for track in song.tracks.iter_mut().filter(|t| t.id == fixup.track_id) {
            for placement in &mut track.clips {
                if let ClipContent::Audio(clip) = &mut placement.content {
                    if clip.id != fixup.clip_id {
                        continue;
                    }
                    if let Some(buffer) = engine.audio_pool.get(&clip.source_path) {
                        fixup.apply(clip, buffer.num_frames as u64, buffer.sample_rate);
                    }
                }
            }
        }
    }
    let song = hardwave_project::Project::from_bytes(&song.to_bytes().unwrap()).unwrap();
    Some((engine, song))
}

fn secs_env(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// The stretched clips of the song, and how much audio their sources hold.
fn describe_stretches(engine: &DawEngine, song: &hardwave_project::Project, range: (f64, f64)) {
    let mut all = std::collections::HashSet::new();
    let mut in_range = std::collections::HashSet::new();
    for track in &song.tracks {
        for clip in &track.clips {
            let ClipContent::Audio(a) = &clip.content else {
                continue;
            };
            let stretched = (a.stretch_ratio > 0.01 && (a.stretch_ratio - 1.0).abs() > 1e-4)
                || a.pitch_semitones.abs() > 1e-4;
            if !stretched || !a.warp_markers.is_empty() || a.treatment.resample {
                continue;
            }
            let key = (
                a.source_path.clone(),
                format!("{:.6}:{:.6}", a.stretch_ratio, a.pitch_semitones),
            );
            let from = song.tempo_map.secs_at_tick(clip.position_ticks);
            let to = song
                .tempo_map
                .secs_at_tick(clip.position_ticks + clip.length_ticks);
            if from < range.1 && to > range.0 {
                in_range.insert(key.clone());
            }
            all.insert(key);
        }
    }
    let secs = |set: &std::collections::HashSet<(String, String)>| -> f64 {
        set.iter()
            .filter_map(|(s, _)| engine.audio_pool.get(s))
            .map(|b| b.num_frames as f64 / b.sample_rate.max(1) as f64 * b.channels.len() as f64)
            .sum()
    };
    eprintln!(
        "stretched variants: {} in the song ({:.1} s of channel audio), {} in the range ({:.1} s)",
        all.len(),
        secs(&all),
        in_range.len(),
        secs(&in_range)
    );
}

/// The first and second export of the range right after the import, as
/// the app runs them: the engine behind a lock, the background bake the
/// import starts already running, and a window command trying the lock
/// every 10 ms the whole time.
#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn the_first_export_after_an_fl_import() {
    let Some((engine, song)) = open_song() else {
        return;
    };
    let from = secs_env("HW_EXPORT_FROM", 157.0);
    let to = secs_env("HW_EXPORT_TO", 177.0);
    describe_stretches(&engine, &song, (from, to));
    *engine.project.lock() = song;
    // The import asks for a graph, which starts the background bake.
    engine.rebuild_graph();

    let engine = std::sync::Arc::new(parking_lot::Mutex::new(engine));
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    // What the window thread meets: the longest wait for the engine.
    let window = {
        let engine = std::sync::Arc::clone(&engine);
        let done = std::sync::Arc::clone(&done);
        std::thread::spawn(move || {
            let mut longest = Duration::ZERO;
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                let asked = Instant::now();
                drop(engine.lock());
                longest = longest.max(asked.elapsed());
                std::thread::sleep(Duration::from_millis(10));
            }
            longest
        })
    };

    let rate = 48_000_u32;
    let start = (from * rate as f64) as u64;
    let total = ((to - from) * rate as f64) as u64;
    for run in ["first", "second"] {
        let began = Instant::now();
        let mut rendered = 0_u64;
        // As the export command runs it: the bakes listed under the lock,
        // made without it, then the render prepared under it again.
        let asked = Instant::now();
        let mut guard = engine.lock();
        let span = Some((start, start + total));
        let bakes = guard.offline_bakes(rate, span);
        let mut locked_for = asked.elapsed();
        let count = bakes.len();
        let baking = parking_lot::MutexGuard::unlocked(&mut guard, || {
            let began = Instant::now();
            assert!(bakes.run(&std::sync::atomic::AtomicBool::new(false), |_, _| {}));
            began.elapsed()
        });
        let relocked = Instant::now();
        let render = guard.prepare_offline_span(rate, start, span, None, |_| {});
        drop(guard);
        locked_for = locked_for.max(relocked.elapsed());
        eprintln!(
            "{run} export: {count} bakes in {:.1} s with the engine free",
            baking.as_secs_f64()
        );
        render.run(total, |block| {
            rendered += (block.len() / 2) as u64;
            true
        });
        assert_eq!(rendered, total);
        eprintln!(
            "{run} export of {from:.0} s to {to:.0} s: {:.1} s, the engine locked for at most {:.2} s",
            began.elapsed().as_secs_f64(),
            locked_for.as_secs_f64()
        );
    }
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let longest = window.join().unwrap();
    eprintln!(
        "the window waited for the engine at most {:.2} s",
        longest.as_secs_f64()
    );
}
