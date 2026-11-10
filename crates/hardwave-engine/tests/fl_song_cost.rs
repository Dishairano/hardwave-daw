//! What an FL song with thousands of clips costs: how long the audio graph
//! takes to build (it is rebuilt on the audio thread after every edit) and
//! how much memory one undo step holds (each keeps a whole copy of the
//! song). A measurement, not a check.
//!
//! Run with the song and its samples in one folder:
//!   HW_FL_SONG=/path/to/song.flp hw-daw-cargo test -j 1 -p hardwave-engine \
//!     --test fl_song_cost -- --ignored --nocapture
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

#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn an_fl_songs_graph_build_and_undo_step() {
    let Ok(file) = std::env::var("HW_FL_SONG") else {
        eprintln!("HW_FL_SONG is not set; skipped");
        return;
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
    let clips: usize = song.tracks.iter().map(|t| t.clips.len()).sum();
    let automation: usize = song.tracks.iter().map(|t| t.automation_clips.len()).sum();
    let busiest = song.tracks.iter().map(|t| t.clips.len()).max().unwrap_or(0);
    eprintln!(
        "{} tracks, {clips} clips ({automation} automation), at most {busiest} on one track, {} rows",
        song.tracks.len(),
        song.rows.len()
    );

    // One undo step: a whole copy of the song.
    let before = ALLOCATED.load(Ordering::Relaxed);
    let copy = song.clone();
    let one_copy = ALLOCATED.load(Ordering::Relaxed) - before;
    drop(copy);
    let file_size = song.to_bytes().unwrap().len();
    eprintln!(
        "one undo step holds {:.2} MB ({} steps: {:.0} MB); the song file is {:.2} MB",
        one_copy as f64 / 1e6,
        256,
        one_copy as f64 * 256.0 / 1e6,
        file_size as f64 / 1e6
    );

    *engine.project.lock() = song;
    let mut times = engine.graph_build_times(12);
    // The first build fills caches; the rest are what an edit costs.
    let first = times.remove(0);
    times.sort();
    eprintln!(
        "graph build: first {:.2} ms, then median {:.2} ms, slowest {:.2} ms",
        first.as_secs_f64() * 1e3,
        times[times.len() / 2].as_secs_f64() * 1e3,
        times.last().unwrap().as_secs_f64() * 1e3
    );
}
