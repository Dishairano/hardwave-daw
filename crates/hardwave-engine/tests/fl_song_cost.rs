//! What an FL song with thousands of clips costs: how long the audio graph
//! takes to build after every edit (off the audio thread), how long the
//! audio thread takes to swap it in, what a whole copy of the song weighs,
//! and what the undo history holds for small and big edits. A measurement,
//! not a check.
//!
//! Run with the song and its samples in one folder:
//!   HW_FL_SONG=/path/to/song.flp hw-daw-cargo test -j 1 -p hardwave-engine \
//!     --test fl_song_cost -- --ignored --nocapture
//! Run one test at a time (name it, or add `--test-threads=1`): the bytes
//! are counted for the whole process. `HW_UNDO_STEPS=32` measures fewer
//! undo steps.
//! The song is never part of the repository.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use hardwave_engine::DawEngine;
use hardwave_project::clip::ClipContent;

/// Counts the bytes handed out, so one copy of the song can be weighed.
struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

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
    // Opened as this build opens it: laid out and checked.
    let song = hardwave_project::Project::from_bytes(&song.to_bytes().unwrap()).unwrap();
    Some((engine, song))
}

#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn an_fl_songs_graph_build_and_undo_step() {
    let Some((engine, song)) = open_song() else {
        return;
    };
    let clips: usize = song.tracks.iter().map(|t| t.clips.len()).sum();
    let automation: usize = song.tracks.iter().map(|t| t.automation_clips.len()).sum();
    let busiest = song.tracks.iter().map(|t| t.clips.len()).max().unwrap_or(0);
    eprintln!(
        "{} tracks, {clips} clips ({automation} automation), at most {busiest} on one track, {} rows",
        song.tracks.len(),
        song.rows.len()
    );

    // A whole copy of the song: what each undo step held before steps
    // shared what did not change.
    let before = ALLOCATED.load(Ordering::Relaxed);
    let copy = song.clone();
    let one_copy = ALLOCATED.load(Ordering::Relaxed) - before;
    drop(copy);
    let file_size = song.to_bytes().unwrap().len();
    eprintln!(
        "a whole copy of the song holds {:.2} MB; the song file is {:.2} MB",
        one_copy as f64 / 1e6,
        file_size as f64 / 1e6
    );

    *engine.project.lock() = song;
    let times = engine.graph_build_times(12);
    // The first build fills caches; the rest are what an edit costs.
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
    let summary = |mut v: Vec<std::time::Duration>| {
        let first = v.remove(0);
        v.sort();
        (ms(first), ms(v[v.len() / 2]), ms(*v.last().unwrap()))
    };
    let (first, median, slowest) = summary(times.iter().map(|t| t.build).collect());
    eprintln!(
        "graph build, off the audio thread: first {first:.2} ms, then median {median:.2} ms, slowest {slowest:.2} ms"
    );
    let (first, median, slowest) = summary(times.iter().map(|t| t.swap).collect());
    eprintln!(
        "graph swap, on the audio thread: first {first:.3} ms, then median {median:.3} ms, slowest {slowest:.3} ms"
    );
}

/// The bytes held after `f` ran, and how long it took.
fn weigh(f: impl FnOnce()) -> (isize, std::time::Duration) {
    let before = ALLOCATED.load(Ordering::Relaxed) as isize;
    let start = std::time::Instant::now();
    f();
    let took = start.elapsed();
    (ALLOCATED.load(Ordering::Relaxed) as isize - before, took)
}

/// What the history itself counts it holds, to check its weights against
/// the allocator.
fn history_bytes(engine: &DawEngine) -> isize {
    engine.history.lock().bytes() as isize
}

fn mb(bytes: isize) -> f64 {
    bytes as f64 / 1e6
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The median and the slowest of some timings.
fn spread(times: &mut [std::time::Duration]) -> String {
    times.sort();
    format!(
        "{:.2} ms (median), {:.2} ms (slowest)",
        ms(times[times.len() / 2]),
        ms(*times.last().unwrap())
    )
}

/// What the undo history holds for small and big edits on a big song,
/// how long taking a step takes (it runs before every edit), and that
/// undoing and redoing every step gives back the very same song.
#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn an_fl_songs_undo_history() {
    let Some((engine, song)) = open_song() else {
        return;
    };
    // The busiest track, and a track with clips to delete from.
    let busiest = (0..song.tracks.len())
        .max_by_key(|&i| song.tracks[i].clips.len())
        .unwrap();
    *engine.project.lock() = song;
    engine.reset_history();
    let saved = |e: &DawEngine| e.project.lock().to_bytes().unwrap();
    let move_one_clip = |e: &DawEngine, n: u64| {
        let mut p = e.project.lock();
        p.tracks[busiest].clips[(n as usize * 7) % 400].position_ticks += 96;
    };
    let turn_one_knob = |e: &DawEngine, n: u64| {
        e.project.lock().tracks[busiest].volume_db = -(n as f64 % 12.0);
    };
    let delete_100_clips = |e: &DawEngine| {
        let mut p = e.project.lock();
        let mut left = 100;
        for t in p.tracks.iter_mut() {
            let n = t.clips.len().min(left).min(5);
            t.clips.drain(..n);
            left -= n;
        }
        assert_eq!(left, 0);
    };
    let nudge_every_clip = |e: &DawEngine| {
        for t in e.project.lock().tracks.iter_mut() {
            for c in t.clips.iter_mut() {
                c.position_ticks += 96;
            }
        }
    };

    // A step holds the song before its edit; with steps that share what
    // did not change, the edit's cost shows when the next step is taken.
    // So: step, edit, then weigh the next step.
    let (first, _) = weigh(|| engine.snapshot_before_mutation());
    eprintln!("the first step: {:.3} MB", mb(first));
    move_one_clip(&engine, 1);
    let (moved, moved_took) = weigh(|| engine.snapshot_before_mutation());
    turn_one_knob(&engine, 1);
    let (knob, knob_took) = weigh(|| engine.snapshot_before_mutation());
    delete_100_clips(&engine);
    let (deleted, deleted_took) = weigh(|| engine.snapshot_before_mutation());
    nudge_every_clip(&engine);
    let (nudged, nudged_took) = weigh(|| engine.snapshot_before_mutation());
    eprintln!(
        "after moving one clip: {:.4} MB in {:.2} ms; one knob: {:.4} MB in {:.2} ms",
        mb(moved),
        ms(moved_took),
        mb(knob),
        ms(knob_took)
    );
    eprintln!(
        "after deleting 100 clips: {:.4} MB in {:.2} ms; after moving every clip: {:.3} MB in {:.2} ms",
        mb(deleted),
        ms(deleted_took),
        mb(nudged),
        ms(nudged_took)
    );

    // 256 small edits, then undo and redo all of them. HW_UNDO_STEPS asks
    // for fewer, to measure a build whose steps each copy the whole song
    // without holding half a gigabyte.
    let steps: usize = std::env::var("HW_UNDO_STEPS")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(256);
    engine.reset_history();
    let mut songs = Vec::new();
    let mut took = Vec::new();
    let (held, _) = weigh(|| {
        for n in 0..steps as u64 {
            songs.push(saved(&engine));
            let start = std::time::Instant::now();
            engine.snapshot_before_mutation();
            took.push(start.elapsed());
            if n % 2 == 0 {
                move_one_clip(&engine, n);
            } else {
                turn_one_knob(&engine, n);
            }
        }
        songs.push(saved(&engine));
    });
    let songs_bytes: usize = songs.iter().map(|s| s.capacity()).sum();
    eprintln!(
        "{steps} small edits: the history holds {:.2} MB (it counts {:.2} MB); a step takes {}",
        mb(held - songs_bytes as isize),
        mb(history_bytes(&engine)),
        spread(&mut took)
    );
    assert_eq!(engine.history_sizes(), (steps, 0));
    let mut undos = Vec::new();
    for n in (0..steps).rev() {
        let start = std::time::Instant::now();
        assert!(engine.undo());
        undos.push(start.elapsed());
        assert!(saved(&engine) == songs[n], "undo {n} gives back the song");
    }
    for (n, song) in songs.iter().enumerate().skip(1) {
        assert!(engine.redo());
        assert!(saved(&engine) == *song, "redo {n} gives back the song");
    }
    eprintln!(
        "undo and redo give back every song; an undo takes {}",
        spread(&mut undos)
    );

    // Moves of every clip: nothing to share between the steps.
    engine.reset_history();
    let (held, _) = weigh(|| {
        for _ in 0..steps {
            engine.snapshot_before_mutation();
            nudge_every_clip(&engine);
        }
    });
    let (kept, _) = engine.history_sizes();
    eprintln!(
        "{steps} moves of every clip: the history holds {:.1} MB (it counts {:.1} MB) in {kept} steps",
        mb(held),
        mb(history_bytes(&engine))
    );
}
