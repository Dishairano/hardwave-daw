//! The audio thread must not allocate.
//!
//! An allocation in the audio callback can wait on the system allocator's
//! lock, and a wait there is a click: the classic cause of a dropout that
//! comes and goes. Nothing checked it. This test plays a song with the
//! common moving parts (an audio track through an EQ and a compressor, a
//! send to a reverb bus, a synth playing notes, volume automation, and a
//! loop that wraps) and counts every allocation and free made on the thread
//! running the blocks, once the first blocks have built the graph. Any is a
//! failure, with how many and how big, so the culprit can be found.
//!
//! The graph is built on the audio thread when the song changes; that is a
//! known cost of an edit, not of playing, so the count starts after it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

use hardwave_engine::engine::OfflineInsertFactory;
use hardwave_engine::{AudioBuffer, DawEngine};
use hardwave_native_plugins::compressor::NativeCompressor;
use hardwave_native_plugins::eq::NativeEq;
use hardwave_native_plugins::reverb::NativeReverb;
use hardwave_native_plugins::triple_osc::NativeTripleOsc;
use hardwave_plugin_host::types::HostedPlugin;
use hardwave_project::automation::{AutomationLane, AutomationPoint, AutomationTarget, CurveMode};
use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef};
use hardwave_project::mixer::Send;
use hardwave_project::track::PluginSlot;

struct Counting;

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACING: Cell<bool> = const { Cell::new(false) };
}
/// HW_ALLOC_TRACE=1: the first allocations' backtraces, captured
/// unresolved (cheap) and resolved after the run, to find where they come
/// from. Resolving thousands of them took more memory than HQ has.
static TRACES: std::sync::Mutex<Vec<(usize, std::backtrace::Backtrace)>> =
    std::sync::Mutex::new(Vec::new());
static TRACE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const MAX_TRACES: usize = 60;

fn note(size: usize) {
    if WATCHING.try_with(|w| w.get()).unwrap_or(false) {
        let n = ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        LARGEST.fetch_max(size, Ordering::Relaxed);
        if TRACE_ON.load(Ordering::Relaxed)
            && n < MAX_TRACES
            && !TRACING.try_with(|t| t.replace(true)).unwrap_or(true)
        {
            if let Ok(mut t) = TRACES.lock() {
                if t.len() < MAX_TRACES {
                    t.push((size, std::backtrace::Backtrace::force_capture()));
                }
            }
            let _ = TRACING.try_with(|t| t.set(false));
        }
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

const SR: u32 = 48_000;
const BAR: u64 = 3840;

fn slot(id: &str, plugin_id: &str) -> PluginSlot {
    PluginSlot {
        id: id.into(),
        plugin_id: plugin_id.into(),
        enabled: true,
        state: None,
        sidechain_source: None,
        wet: 1.0,
    }
}

/// The song: drums through an EQ and a compressor with a send to a reverb
/// bus and volume automation, a synth playing a chord every beat, and a
/// two-bar loop.
fn song() -> DawEngine {
    let engine = DawEngine::new();
    let frames = SR as usize * 4;
    let tone: Vec<f32> = (0..frames)
        .map(|n| (std::f32::consts::TAU * 110.0 * n as f32 / SR as f32).sin() * 0.4)
        .collect();
    engine.audio_pool.insert(
        "rt-tone".into(),
        AudioBuffer {
            channels: vec![tone.clone(), tone],
            sample_rate: SR,
            num_frames: frames,
        },
    );
    let mut project = engine.project.lock();
    let bus = project.add_audio_track("Reverb bus".into());
    if let Some(t) = project.track_mut(&bus) {
        t.inserts.push(slot("slot-verb", NativeReverb::ID));
    }
    let drums = project.add_audio_track("Drums".into());
    if let Some(t) = project.track_mut(&drums) {
        t.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "clip-rt".into(),
                name: "rt-tone".into(),
                source_path: "rt-tone".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: frames as u64,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: String::new(),
                treatment: Default::default(),
            }),
            track_id: drums.clone(),
            position_ticks: 0,
            length_ticks: 4 * BAR,
            lane: 0,
            color: None,
        });
        t.inserts.push(slot("slot-eq", NativeEq::ID));
        t.inserts.push(slot("slot-comp", NativeCompressor::ID));
        t.sends.push(Send {
            target: bus.clone(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: true,
        });
        let point = |tick: u64, value: f64| AutomationPoint {
            tick,
            value,
            curve: CurveMode::Linear,
            tension: 0.0,
        };
        t.automation_lanes.push(AutomationLane {
            id: "lane-vol".into(),
            target: AutomationTarget::TrackVolume,
            points: vec![point(0, 0.4), point(BAR, 0.8), point(2 * BAR, 0.5)],
            visible: true,
        });
    }
    let lead = project.add_midi_track("Lead".into());
    if let Some(t) = project.track_mut(&lead) {
        t.inserts.push(slot("slot-synth", NativeTripleOsc::ID));
        let mut clip = hardwave_midi::MidiClip::new("rt-chords".into(), "Chords".into(), 2 * BAR);
        for beat in 0..8u64 {
            for pitch in [60u8, 64, 67] {
                clip.notes.push(hardwave_midi::MidiNote {
                    start_tick: beat * 960,
                    duration_ticks: 480,
                    pitch,
                    velocity: 0.8,
                    ..Default::default()
                });
            }
        }
        t.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "rt-chords".into(),
                clip,
            }),
            track_id: lead.clone(),
            position_ticks: 0,
            length_ticks: 2 * BAR,
            lane: 0,
            color: None,
        });
    }
    drop(project);
    engine.transport.looping.store(true, Ordering::Relaxed);
    engine.transport.loop_start.store(0, Ordering::Relaxed);
    // Two bars at the song's tempo, in samples.
    let two_bars = {
        let project = engine.project.lock();
        project.tempo_map.tick_to_samples(2 * BAR, SR as f64)
    };
    engine.transport.loop_end.store(two_bars, Ordering::Relaxed);
    engine
}

#[test]
fn playing_a_song_allocates_nothing_on_the_audio_thread() {
    let engine = song();
    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        match id {
            NativeEq::ID => Some(Box::new(NativeEq::new())),
            NativeCompressor::ID => Some(Box::new(NativeCompressor::new())),
            NativeReverb::ID => Some(Box::new(NativeReverb::new())),
            NativeTripleOsc::ID => Some(Box::new(NativeTripleOsc::new())),
            _ => None,
        }
    };
    let factory: OfflineInsertFactory<'_> = &factory;
    // Eight seconds, so the loop wraps a few times; the first blocks build
    // the graph and fill the plug-ins' buffers.
    TRACE_ON.store(
        std::env::var_os("HW_ALLOC_TRACE").is_some(),
        Ordering::Relaxed,
    );
    if let Ok(mut t) = TRACES.lock() {
        t.reserve(MAX_TRACES);
    }
    const WARM_UP_BLOCKS: usize = 32;
    let mut blocks = 0usize;
    let mut frames_done = 0usize;
    let mut watched = 0usize;
    let mut loudest = 0.0f32;
    engine
        .render_offline_with(
            SR,
            SR as u64 * 8,
            0,
            Some(factory),
            |_| {},
            |block| {
                blocks += 1;
                frames_done += block.len() / 2;
                if blocks == WARM_UP_BLOCKS {
                    WATCHING.with(|w| w.set(true));
                }
                // The last block: what follows is the render taking itself
                // down, not playing.
                if frames_done >= SR as usize * 8 {
                    WATCHING.with(|w| w.set(false));
                }
                if blocks > WARM_UP_BLOCKS {
                    watched += 1;
                }
                for &s in block {
                    loudest = loudest.max(s.abs());
                }
                true
            },
        )
        .unwrap();
    WATCHING.with(|w| w.set(false));
    if let Ok(t) = TRACES.lock() {
        let mut seen: Vec<(usize, usize, String)> = Vec::new();
        for (size, bt) in t.iter() {
            let text = bt.to_string();
            let key: String = text
                .lines()
                .collect::<Vec<_>>()
                .windows(2)
                .filter(|w| {
                    w[0].contains("hardwave_") && !w[0].contains("audio_thread_allocations")
                })
                .take(5)
                .map(|w| {
                    format!(
                        "{} @ {}",
                        w[0].trim(),
                        w[1].trim().trim_start_matches("at ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            if let Some(e) = seen.iter_mut().find(|e| e.2 == key) {
                e.0 += 1;
            } else {
                seen.push((1, *size, key));
            }
        }
        for (n, size, key) in seen {
            eprintln!("ALLOC x{n} ({size} bytes): {key}");
        }
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    assert!(loudest > 0.01, "the song played (peak {loudest})");
    assert!(watched > 100, "enough blocks were watched ({watched})");
    assert_eq!(
        allocations,
        0,
        "{allocations} allocations or frees on the audio thread over {watched} blocks of \
         playing, the largest {} bytes",
        LARGEST.load(Ordering::Relaxed)
    );
}
