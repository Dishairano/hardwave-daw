//! What playing an FL song costs the audio thread, block by block, through
//! the callback playback runs, with the built-in sampler on every sampler
//! channel and the built-in effects the import puts in for FL's, as the
//! app opens it, in the blocks a sound card asks for (512
//! frames at 48 kHz by default): the whole callback per block (median,
//! 99th percentile, slowest), the share each kind of node takes, and the
//! nodes that cost the most. A measurement, not a check. Use a release
//! build for the numbers:
//!   HW_FL_SONG=/path/to/song.flp hw-daw-cargo test --release -j 1 \
//!     -p hardwave-native-plugins --test fl_song_playback -- --ignored --nocapture
//! `HW_DEVICE_FRAMES` sets the device block, `HW_PLAY_SECONDS` plays only
//! the start of the song, `HW_PLAY_DUMP=<file>` writes the sound played
//! (raw little-endian f32 stereo) and `HW_PLAY_COMPARE=<file>` compares
//! the sound with one written before, sample for sample.
//! `HW_SMART_DISABLE=0` plays with smart disable off (it is on by default),
//! so its cost and its sound can be set against smart disable on.
//! `HW_WORKERS` sets the helper threads, `HW_BUSY_THREADS` runs that many
//! busy loops beside, standing in for other programs, and `HW_PACED=1`
//! starts each block when a sound card would rather than at once. A
//! second test
//! plays the song and counts every allocation and free made on the thread
//! playing it, after the first blocks (which build the graph). The song is
//! never part of the repository.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use hardwave_engine::engine::{OfflineInsertFactory, LIVE_PROFILE_KINDS};
use hardwave_engine::DawEngine;
use hardwave_native_plugins::sampler::{self as hs, NativeSampler};
use hardwave_project::clip::ClipContent;
use hardwave_project::track::PluginSlot;

/// Counts allocations and frees on a thread while it is watched.
struct Counting;

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

fn note(size: usize) {
    if WATCHING.try_with(|w| w.get()).unwrap_or(false) {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LARGEST.fetch_max(size, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

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
    // What the app's import does (src-tauri commands/flp.rs): each sampler
    // channel gets the built-in sampler with its sample and FL's settings,
    // and each FL effect with a built-in counterpart gets that, set as FL
    // had it.
    for sampler in &built.sampler_samples {
        let Ok((info, channels)) =
            hardwave_dsp::audio_file::AudioFileReader::read(std::path::Path::new(&sampler.file))
        else {
            eprintln!("sampler sample {} could not be read", sampler.file);
            continue;
        };
        let on = |b: bool| if b { 1.0 } else { 0.0 };
        let mut settings = hs::settings_from_fl(&sampler.setup, sampler.root_note);
        settings.extend([
            (hs::PARAM_REVERSE, on(sampler.reversed)),
            (hs::PARAM_SWAP_STEREO, on(sampler.swap_stereo)),
            (hs::PARAM_POLARITY, on(sampler.polarity_inverted)),
        ]);
        let state = hs::encode_sample_state_with(
            info.sample_rate,
            sampler.root_note,
            channels,
            &sampler.file,
            settings,
        );
        let slot_id = format!("sampler-{}", sampler.track_id);
        if let Some(track) = song.track_mut(&sampler.track_id) {
            track.inserts.insert(
                0,
                PluginSlot {
                    allow_smart_disable: None,
                    id: slot_id.clone(),
                    plugin_id: NativeSampler::ID.to_string(),
                    enabled: true,
                    state: None,
                    sidechain_source: None,
                    wet: 1.0,
                },
            );
            song.set_plugin_state(slot_id, "native-sampler", state);
        }
    }
    for effect in &built.native_effects {
        if let Some(mut plugin) = hardwave_native_plugins::create_native(effect.plugin_id) {
            for (id, value) in &effect.params {
                plugin.set_parameter_value(*id, *value);
            }
            song.set_plugin_state(effect.slot_id.clone(), "native", plugin.get_state());
        }
    }
    eprintln!(
        "{} sampler channels, {} built-in effects",
        built.sampler_samples.len(),
        built.native_effects.len()
    );
    for track in song.tracks.iter().filter(|t| !t.inserts.is_empty()) {
        let plugins: Vec<&str> = track.inserts.iter().map(|s| s.plugin_id.as_str()).collect();
        eprintln!("  {}: {}", track.name, plugins.join(", "));
    }
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

fn env_num(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn ms(ns: u64) -> f64 {
    ns as f64 / 1e6
}

/// Median, 99th percentile and the slowest of some nanoseconds.
fn spread(mut v: Vec<u64>) -> (f64, f64, f64) {
    v.sort_unstable();
    let at = |q: f64| ms(v[((v.len() - 1) as f64 * q) as usize]);
    (at(0.5), at(0.99), ms(*v.last().unwrap()))
}

#[test]
#[ignore = "a measurement with a song from outside the repository"]
fn an_fl_songs_playback_block_by_block() {
    let Some((engine, song)) = open_song() else {
        return;
    };
    let rate = 48_000u32;
    let device_frames = env_num("HW_DEVICE_FRAMES", 512.0) as usize;
    let end_tick = song
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .map(|c| c.position_ticks + c.length_ticks)
        .max()
        .unwrap_or(0);
    let song_samples = song.tempo_map.tick_to_samples(end_tick, rate as f64);
    let samples = match env_num("HW_PLAY_SECONDS", 0.0) {
        s if s > 0.0 => ((s * rate as f64) as u64).min(song_samples),
        _ => song_samples,
    };
    let kinds = |k: hardwave_project::TrackKind| song.tracks.iter().filter(|t| t.kind == k).count();
    let clips: usize = song.tracks.iter().map(|t| t.clips.len()).sum();
    eprintln!(
        "{} tracks ({} audio, {} instrument, {} bus, {} return), {clips} clips; playing {:.1} s of {:.1} s in {device_frames}-frame blocks",
        song.tracks.len(),
        kinds(hardwave_project::TrackKind::Audio),
        kinds(hardwave_project::TrackKind::Midi),
        kinds(hardwave_project::TrackKind::Bus),
        kinds(hardwave_project::TrackKind::Return),
        samples as f64 / rate as f64,
        song_samples as f64 / rate as f64,
    );
    *engine.project.lock() = song;
    let smart = std::env::var("HW_SMART_DISABLE").map_or(true, |v| v != "0");
    engine.audio_prefs().set_smart_disable(smart);
    eprintln!("smart disable {}", if smart { "on" } else { "off" });
    // Helper threads beside the playing thread, as the audio settings set
    // them, and busy threads standing in for other programs on the machine.
    let workers = env_num("HW_WORKERS", 0.0) as usize;
    if workers > 0 {
        engine.set_worker_threads(workers);
    }
    let busy = env_num("HW_BUSY_THREADS", 0.0) as usize;
    let stop_busy = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let busy_threads: Vec<_> = (0..busy)
        .map(|_| {
            let stop = std::sync::Arc::clone(&stop_busy);
            std::thread::spawn(move || {
                let mut x = 1u64;
                while !stop.load(Ordering::Relaxed) {
                    x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                    std::hint::black_box(x);
                }
            })
        })
        .collect();
    eprintln!("{workers} helper threads, {busy} busy threads beside");
    if workers > 0 {
        eprintln!("(with helper threads the nodes are not timed one by one)");
    }

    // The sound is written or compared as it is played, a block at a time.
    use std::io::{Read, Write};
    let mut dump = std::env::var("HW_PLAY_DUMP")
        .ok()
        .map(|f| std::io::BufWriter::new(std::fs::File::create(f).unwrap()));
    let mut compare = std::env::var("HW_PLAY_COMPARE")
        .ok()
        .map(|f| std::io::BufReader::new(std::fs::File::open(f).unwrap()));
    let (mut compared, mut differ, mut worst, mut peak) = (0usize, 0usize, 0.0f32, 0.0f32);
    let mut bytes = Vec::new();
    let started = std::time::Instant::now();
    let factory = hardwave_native_plugins::create_native;
    let factory: OfflineInsertFactory<'_> = &factory;
    // Paced, each block starts when a sound card would ask for it, so the
    // helper threads go idle between blocks as they do in playback.
    let paced = env_num("HW_PACED", 0.0) > 0.0;
    let block_time = Duration::from_secs_f64(device_frames as f64 / rate as f64);
    let mut next_block = std::time::Instant::now();
    let profile =
        engine.profile_live_blocks(rate, samples, device_frames, Some(factory), |block| {
            if paced {
                next_block += block_time;
                let now = std::time::Instant::now();
                if next_block > now {
                    std::thread::sleep(next_block - now);
                } else {
                    next_block = now;
                }
            }
            if let Some(out) = dump.as_mut() {
                for s in block {
                    out.write_all(&s.to_le_bytes()).unwrap();
                }
            }
            if let Some(input) = compare.as_mut() {
                bytes.resize(block.len() * 4, 0);
                input
                    .read_exact(&mut bytes)
                    .expect("as many samples as before");
                for (b, now) in bytes.as_chunks::<4>().0.iter().zip(block) {
                    let was = f32::from_le_bytes(*b);
                    compared += 1;
                    if was.to_bits() != now.to_bits() {
                        differ += 1;
                    }
                    worst = worst.max((was - now).abs());
                    peak = peak.max(was.abs());
                }
            }
        });
    eprintln!(
        "ran in {:.1} s (bakes included); {} graph swaps during play",
        started.elapsed().as_secs_f64(),
        profile.swaps
    );
    stop_busy.store(true, Ordering::Relaxed);
    for t in busy_threads {
        t.join().unwrap();
    }
    if let Some(stats) = engine.worker_pool_stats() {
        eprintln!("helper threads: {stats:?}");
    }

    let budget = Duration::from_secs_f64(device_frames as f64 / rate as f64);
    let blocks: Vec<u64> = profile.blocks.iter().map(|d| d.as_nanos() as u64).collect();
    let over = profile.blocks.iter().filter(|d| **d > budget).count();
    let mean = blocks.iter().sum::<u64>() as f64 / blocks.len() as f64;
    let (median, p99, slowest) = spread(blocks.clone());
    let budget_ms = budget.as_secs_f64() * 1e3;
    eprintln!(
        "{} blocks; a block: mean {:.3} ms ({:.1} %), median {median:.3} ms ({:.1} %), p99 {p99:.3} ms ({:.1} %), slowest {slowest:.3} ms ({:.1} %); {over} over the {budget_ms:.2} ms budget",
        blocks.len(),
        mean / 1e6,
        mean / 1e6 / budget_ms * 100.0,
        median / budget_ms * 100.0,
        p99 / budget_ms * 100.0,
        slowest / budget_ms * 100.0,
    );
    // The slowest blocks, and where in the song they are.
    let mut slow: Vec<usize> = (0..blocks.len()).collect();
    slow.sort_by_key(|&i| std::cmp::Reverse(blocks[i]));
    for &i in slow.iter().take(8) {
        let k = profile.kinds[i];
        eprintln!(
            "  slow block {i} at {:.2} s: {:.3} ms; {}",
            (i * device_frames) as f64 / rate as f64,
            ms(blocks[i]),
            LIVE_PROFILE_KINDS
                .iter()
                .zip(k)
                .map(|(n, v)| format!("{n} {:.3}", ms(v)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let in_graph: Vec<u64> = profile.kinds.iter().map(|k| k.iter().sum()).collect();
    let outside: Vec<u64> = blocks
        .iter()
        .zip(&in_graph)
        .map(|(b, g)| b.saturating_sub(*g))
        .collect();
    let (m, p, s) = spread(outside);
    eprintln!("  outside the nodes (engine work, idle checks): median {m:.3}, p99 {p:.3}, slowest {s:.3} ms");
    for (k, name) in LIVE_PROFILE_KINDS.iter().enumerate() {
        let per: Vec<u64> = profile.kinds.iter().map(|v| v[k]).collect();
        let total: u64 = per.iter().sum();
        let (m, p, s) = spread(per);
        eprintln!(
            "  {name}: median {m:.3}, p99 {p:.3}, slowest {s:.3} ms per block; {:.1} % of all time",
            total as f64 / blocks.iter().sum::<u64>() as f64 * 100.0
        );
    }
    let mut nodes = profile.nodes.clone();
    nodes.sort_by_key(|n| std::cmp::Reverse(n.gather_ns + n.run_ns));
    let ran = nodes.len();
    eprintln!("{ran} nodes ran; the costliest:");
    for n in nodes.iter().take(25) {
        eprintln!(
            "  {:<10} {:<32} {:>8.1} ms total ({:.3} ms gather), {:>6} blocks, mean {:.4} ms, slowest {:.3} ms",
            n.kind,
            n.name,
            ms(n.gather_ns + n.run_ns),
            ms(n.gather_ns),
            n.blocks,
            ms(n.gather_ns + n.run_ns) / n.blocks.max(1) as f64,
            ms(n.max_ns)
        );
    }

    if let Some(mut out) = dump {
        out.flush().unwrap();
        eprintln!("wrote the sound played");
    }
    if compare.is_some() {
        eprintln!(
            "against the sound before: {differ} of {compared} samples differ; the largest difference is {worst:e} ({:.1} dB below the peak {peak})",
            20.0 * (worst.max(1e-30) / peak.max(1e-30)).log10()
        );
    }
}

/// Playing the song allocates nothing on the thread that plays it, once
/// the first blocks have built the graph: an allocation there can wait on
/// the system allocator, which is heard as a dropout.
#[test]
#[ignore = "a check with a song from outside the repository"]
fn an_fl_songs_playback_allocates_nothing() {
    let Some((engine, song)) = open_song() else {
        return;
    };
    let rate = 48_000u32;
    let device_frames = env_num("HW_DEVICE_FRAMES", 512.0) as usize;
    let seconds = env_num("HW_PLAY_SECONDS", 120.0);
    *engine.project.lock() = song;
    let factory = hardwave_native_plugins::create_native;
    let factory: OfflineInsertFactory<'_> = &factory;
    const WARM_UP_BLOCKS: usize = 32;
    let samples = (seconds * rate as f64) as u64;
    // Watched from the end of the warm-up to the last block: what follows
    // that is the playing being taken down, not playing.
    let total = samples.div_ceil(device_frames as u64) as usize;
    let mut blocks = 0usize;
    engine
        .play_live_in_device_blocks(rate, samples, device_frames, Some(factory), |_| {
            blocks += 1;
            WATCHING.with(|w| w.set(blocks >= WARM_UP_BLOCKS && blocks < total));
            true
        })
        .unwrap();
    WATCHING.with(|w| w.set(false));
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    eprintln!(
        "{allocations} allocations or frees over {} blocks of playing, the largest {} bytes",
        blocks - WARM_UP_BLOCKS,
        LARGEST.load(Ordering::Relaxed)
    );
    assert_eq!(allocations, 0);
}
