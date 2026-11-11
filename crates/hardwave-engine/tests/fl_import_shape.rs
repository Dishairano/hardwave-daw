//! An FL song imported in FL's shape: what it is made of, whether its
//! clips still play FL's part of their files, and whether it sounds as the
//! import before it did. Measurements and a check, with a song from outside
//! the repository.
//!
//! With the song and its samples in one folder:
//!   HW_FL_SONG=/path/to/song.flp hw-daw-cargo test -j 1 -p hardwave-engine \
//!     --test fl_import_shape -- --ignored --nocapture --test-threads=1
//!
//! - `HW_FL_SAVE=/path/to/import.hwp` saves the import as this build makes
//!   it, samples finished, to compare a later build's import against.
//! - `HW_FL_OLD_SONG=/path/to/import.hwp` (one saved that way by an older
//!   build) renders both and compares them sample by sample.
//!   `HW_FL_SECONDS` renders only the start; `HW_FL_QUIET` names, with `|`
//!   between them, automation channels whose clips are left out of the
//!   new render (automation the older import could not bring).
//!
//! The song is never part of the repository.

use hardwave_engine::DawEngine;
use hardwave_project::clip::ClipContent;
use hardwave_project::fl_import::FlClipContent;
use hardwave_project::track::TrackKind;
use hardwave_project::Project;

const RATE: u32 = 48_000;

/// The song `HW_FL_SONG` names, read, imported as this build imports it
/// and finished as the app finishes it once its samples load. `None` when
/// the variable is not set.
fn import() -> Option<(
    hardwave_project::fl_import::FlProject,
    hardwave_project::fl_build::FlBuild,
    DawEngine,
)> {
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
    let mut built = hardwave_project::fl_build::build_project(&fl, "FL song", &resolve);
    let engine = DawEngine::new();
    engine
        .audio_source_loader(None)
        .load_sources_of(&built.project, |_, _| {});
    let song = &mut built.project;
    for fixup in &built.audio_fixups {
        let live = song
            .tracks
            .iter_mut()
            .filter(|t| t.id == fixup.track_id)
            .flat_map(|t| t.clips.iter_mut());
        let stored = song
            .arrangements
            .iter_mut()
            .filter_map(|a| a.timelines.get_mut(&fixup.track_id))
            .flat_map(|t| t.clips.iter_mut());
        for placement in live.chain(stored) {
            let ClipContent::Audio(clip) = &mut placement.content else {
                continue;
            };
            if clip.id != fixup.clip_id {
                continue;
            }
            if let Some(buffer) = engine.audio_pool.get(&clip.source_path) {
                fixup.apply(clip, buffer.num_frames as u64, buffer.sample_rate);
            }
        }
    }
    Some((fl, built, engine))
}

/// Every audio clip of a song, wherever it is (on its track or in a stored
/// arrangement), by its id.
type ClipWindow = (u64, u64, bool, String, String);

fn audio_clips(song: &Project) -> std::collections::HashMap<String, ClipWindow> {
    let live = song.tracks.iter().flat_map(|t| t.clips.iter());
    let stored = song
        .arrangements
        .iter()
        .flat_map(|a| a.timelines.values())
        .flat_map(|t| t.clips.iter());
    live.chain(stored)
        .filter_map(|p| match &p.content {
            ClipContent::Audio(a) => Some((
                a.id.clone(),
                (
                    a.source_start,
                    a.source_end,
                    a.reversed,
                    a.source_path.clone(),
                    a.name.clone(),
                ),
            )),
            ClipContent::Midi(_) => None,
        })
        .collect()
}

/// The song's three lists, as the plan's table counts them, and the clips
/// that play FL's part of their file.
#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn the_fl_song_in_fl_s_shape() {
    let Some((fl, built, engine)) = import() else {
        return;
    };
    let song = Project::from_bytes(&built.project.to_bytes().unwrap()).unwrap();
    let channels: Vec<_> = song
        .tracks
        .iter()
        .filter(|t| t.rack_group.is_some())
        .collect();
    let inserts: Vec<_> = song
        .tracks
        .iter()
        .filter(|t| t.mixer_number.is_some_and(|n| n > 0))
        .collect();
    eprintln!(
        "{} tracks: {} channels, {} inserts, the master",
        song.tracks.len(),
        channels.len(),
        inserts.len()
    );
    for (g, name) in song.rack_groups.iter().enumerate() {
        let n = channels
            .iter()
            .filter(|t| t.rack_group == Some(g as u16))
            .count();
        eprintln!("  rack group {name}: {n} channels");
    }
    let mut numbers: Vec<_> = inserts
        .iter()
        .map(|t| (t.mixer_number.unwrap(), t.name.clone(), t.inserts.len()))
        .collect();
    numbers.sort();
    for (n, name, fx) in &numbers {
        eprintln!("  insert {n}: {name} ({fx} effects)");
    }
    let clips: usize = song.tracks.iter().map(|t| t.clips.len()).sum();
    let automation: usize = song.tracks.iter().map(|t| t.automation_clips.len()).sum();
    let on_rows = song
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.row))
        .chain(
            song.tracks
                .iter()
                .flat_map(|t| t.automation_clips.iter().map(|c| c.row)),
        )
        .filter(Option::is_some)
        .count();
    let rows_used: std::collections::BTreeSet<u32> = song
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.row))
        .chain(
            song.tracks
                .iter()
                .flat_map(|t| t.automation_clips.iter().map(|c| c.row)),
        )
        .flatten()
        .collect();
    let on_automation_channels = song
        .tracks
        .iter()
        .filter(|t| matches!(t.kind, TrackKind::Automation))
        .map(|t| t.automation_clips.len())
        .sum::<usize>();
    eprintln!(
        "{} playlist rows ({} with clips); {} clips ({} audio or notes, {} automation, {} of them on automation channels), {} on a row",
        song.rows.len(),
        rows_used.len(),
        clips + automation,
        clips,
        automation,
        on_automation_channels,
        on_rows
    );
    eprintln!("report: {:?}", built.report);

    // FL's part of each file: its own start and end fields, in the file's
    // milliseconds (-1 is "not trimmed"). The fix-ups come in FL's
    // playlist order.
    let fl_audio: Vec<(f32, f32)> = fl
        .playlist_clips
        .iter()
        .filter_map(|c| match &c.content {
            FlClipContent::AudioSample {
                start_offset_ms,
                end_offset_ms,
                ..
            } => Some((*start_offset_ms, *end_offset_ms)),
            _ => None,
        })
        .collect();
    assert_eq!(fl_audio.len(), built.audio_fixups.len());
    let ours = audio_clips(&built.project);
    let (mut same, mut checked) = (0, 0);
    let mut off = Vec::new();
    for ((start_ms, end_ms), fixup) in fl_audio.iter().zip(&built.audio_fixups) {
        let Some((from, to, reversed, path, name)) = ours.get(&fixup.clip_id) else {
            continue;
        };
        let Some(buffer) = engine.audio_pool.get(path) else {
            continue;
        };
        checked += 1;
        let rate = buffer.sample_rate as f64;
        let frames = buffer.num_frames as u64;
        // In FL's terms: a reversed clip counts from the file's end.
        let (a, b) = if *reversed {
            (frames - to, frames - from)
        } else {
            (*from, *to)
        };
        let (a, b) = (a as f64 / rate * 1e3, b as f64 / rate * 1e3);
        let start_ok = (a - (*start_ms).max(0.0) as f64).abs() <= 3.0;
        let end_ok = *end_ms < 0.0 || (b - *end_ms as f64).abs() <= 3.0;
        if start_ok && end_ok {
            same += 1;
        } else {
            off.push(format!(
                "{name}: FL {start_ms:.1} to {end_ms:.1} ms, here {a:.1} to {b:.1} ms"
            ));
        }
    }
    eprintln!(
        "{same} of {} audio clips play FL's part of their file (within 3 ms); {checked} had their file",
        fl_audio.len()
    );
    for line in off.iter().take(12) {
        eprintln!("  not FL's part: {line}");
    }
}

/// Save this build's import, finished, for a later build to compare with.
#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn save_the_fl_import() {
    let Ok(out) = std::env::var("HW_FL_SAVE") else {
        eprintln!("HW_FL_SAVE is not set; skipped");
        return;
    };
    let Some((_, built, _)) = import() else {
        return;
    };
    std::fs::write(&out, built.project.to_bytes().unwrap()).unwrap();
    eprintln!("saved to {out}");
}

/// Where the song ends, in seconds: its last clip's end, plus a second.
fn song_seconds(song: &Project) -> f64 {
    let end = song
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(|c| c.position_ticks + c.length_ticks))
        .max()
        .unwrap_or(0);
    song.tempo_map.tick_to_samples(end, RATE as f64) as f64 / RATE as f64 + 1.0
}

/// Render a song from its samples on disk.
fn render(song: Project, seconds: f64, mut on_block: impl FnMut(&[f32])) {
    let engine = DawEngine::new();
    engine
        .audio_source_loader(None)
        .load_sources_of(&song, |_, _| {});
    *engine.project.lock() = song;
    engine
        .render_offline(RATE, (RATE as f64 * seconds) as u64, |block| {
            on_block(block);
            true
        })
        .unwrap();
}

/// The import of the older build (`HW_FL_OLD_SONG`) and this one's,
/// rendered and compared sample by sample.
#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn renders_of_the_old_and_new_import_match() {
    let Ok(old) = std::env::var("HW_FL_OLD_SONG") else {
        eprintln!("HW_FL_OLD_SONG is not set; skipped");
        return;
    };
    let Some((_, built, engine)) = import() else {
        return;
    };
    drop(engine);
    let old = Project::from_bytes(&std::fs::read(old).unwrap()).unwrap();
    let mut new = Project::from_bytes(&built.project.to_bytes().unwrap()).unwrap();
    drop(built);
    let quiet: Vec<String> = std::env::var("HW_FL_QUIET")
        .map(|v| v.split('|').map(str::to_string).collect())
        .unwrap_or_default();
    for track in new.tracks.iter_mut() {
        if quiet.contains(&track.name) {
            eprintln!(
                "left out of the new render: {} automation clips of {}",
                track.automation_clips.len(),
                track.name
            );
            track.automation_clips.clear();
        }
    }
    let seconds = std::env::var("HW_FL_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| song_seconds(&old).max(song_seconds(&new)));
    eprintln!(
        "rendering {seconds:.1} s of each: {} tracks before, {} now",
        old.tracks.len(),
        new.tracks.len()
    );
    let started = std::time::Instant::now();
    let mut before = Vec::new();
    render(old, seconds, |b| before.extend_from_slice(b));
    eprintln!(
        "the old import rendered in {:.0} s",
        started.elapsed().as_secs_f64()
    );
    let mut at = 0usize;
    let mut exact = 0usize;
    let mut worst = 0f32;
    let mut worst_at = 0usize;
    let mut signal = 0f64;
    let mut error = 0f64;
    let mut first_off: Option<usize> = None;
    // Per second: the largest difference.
    let mut per_second: Vec<f32> = Vec::new();
    render(new, seconds, |block| {
        for s in block {
            let b = before.get(at).copied().unwrap_or(0.0);
            let d = (s - b).abs();
            if d == 0.0 {
                exact += 1;
            } else if first_off.is_none() && d > 1e-6 {
                first_off = Some(at);
            }
            if d > worst {
                worst = d;
                worst_at = at;
            }
            signal += (b as f64) * (b as f64);
            error += (d as f64) * (d as f64);
            let second = at / 2 / RATE as usize;
            if per_second.len() <= second {
                per_second.resize(second + 1, 0.0);
            }
            per_second[second] = per_second[second].max(d);
            at += 1;
        }
    });
    let at_s = |i: usize| i as f64 / 2.0 / RATE as f64;
    let db = |x: f64| 10.0 * x.max(1e-30).log10();
    eprintln!(
        "{at} samples; {exact} the same to the bit; largest difference {worst:.2e} at {:.3} s; difference {:.1} dB below the song",
        at_s(worst_at),
        db(signal) - db(error)
    );
    match first_off {
        Some(i) => eprintln!("first difference over 1e-6 at {:.3} s", at_s(i)),
        None => eprintln!("no difference over 1e-6 anywhere"),
    }
    let loud: Vec<String> = per_second
        .iter()
        .enumerate()
        .filter(|(_, d)| **d > 1e-4)
        .map(|(s, d)| format!("{s}s:{d:.1e}"))
        .collect();
    eprintln!(
        "seconds with a difference over 1e-4: {}",
        if loud.is_empty() {
            "none".to_string()
        } else {
            loud.join(" ")
        }
    );
}
