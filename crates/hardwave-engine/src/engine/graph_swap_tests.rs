//! The graph is built off the audio thread and swapped in: the audio
//! thread neither builds nor allocates for an edit, and the sound carries
//! on across the swap as if nothing had happened.

use super::*;
use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve, MidiClipRef};
use hardwave_project::track::NativeInstrument;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Counts allocations and frees made on a thread that asked to be watched,
/// standing in for the audio thread. Every other thread is left alone.
struct Watching;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
}

fn watched() -> bool {
    WATCHED.try_with(|w| w.get()).unwrap_or(false)
}

unsafe impl GlobalAlloc for Watching {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if watched() {
            ALLOCS.fetch_add(1, Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if watched() {
            FREES.fetch_add(1, Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if watched() {
            ALLOCS.fetch_add(1, Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static WATCHING: Watching = Watching;

/// Allocations and frees `f` makes on this thread.
fn counted(f: impl FnOnce()) -> (usize, usize) {
    let (a, b) = (ALLOCS.load(Relaxed), FREES.load(Relaxed));
    WATCHED.with(|w| w.set(true));
    f();
    WATCHED.with(|w| w.set(false));
    (ALLOCS.load(Relaxed) - a, FREES.load(Relaxed) - b)
}

const SR: u32 = 48_000;
const BLOCK: usize = 256;
const BAR: u64 = 4 * hardwave_midi::PPQ;

fn sine(frames: usize, hz: f32) -> AudioBuffer {
    let ch: Vec<f32> = (0..frames)
        .map(|n| (std::f32::consts::TAU * hz * n as f32 / SR as f32).sin() * 0.4)
        .collect();
    AudioBuffer {
        channels: vec![ch.clone(), ch],
        sample_rate: SR,
        num_frames: frames,
    }
}

fn midi_part(track_id: &str, notes: &[(u64, u64, u8)]) -> ClipPlacement {
    let mut clip = hardwave_midi::MidiClip::new("part".into(), "part".into(), 8 * BAR);
    for &(start_tick, duration_ticks, pitch) in notes {
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick,
            duration_ticks,
            pitch,
            velocity: 0.8,
            ..Default::default()
        });
    }
    ClipPlacement {
        content: ClipContent::Midi(MidiClipRef {
            id: format!("part-{track_id}"),
            clip,
        }),
        track_id: track_id.into(),
        position_ticks: 0,
        length_ticks: 8 * BAR,
        lane: 0,
        color: None,
        row: None,
    }
}

/// A song with sound that carries state from block to block: a saw lead
/// holding one long note through a filter, a kick whose tail rings, and a
/// sample on a track with a filter and a delay. Plus an empty track to
/// edit, and the name of the lead.
fn song(pool: &AudioPool) -> (Project, String, String) {
    let mut project = Project::default();
    let lead = project.add_midi_track("Lead".into());
    let kick = project.add_midi_track("Kick".into());
    let loop_id = project.add_audio_track("Loop".into());
    let spare = project.add_audio_track("Spare".into());
    pool.insert("sine".into(), sine(SR as usize * 4, 220.0));
    {
        let t = project.track_mut(&lead).unwrap();
        t.instrument = NativeInstrument::BuiltinSaw;
        t.filter_type = "lp".into();
        t.filter_cutoff_hz = 900.0;
        t.filter_resonance = 0.5;
        t.clips.push(midi_part(&lead, &[(0, 4 * BAR, 45)]));
    }
    {
        let t = project.track_mut(&kick).unwrap();
        t.instrument = NativeInstrument::KickSynth;
        t.clips.push(midi_part(&kick, &[(0, 120, 36)]));
    }
    {
        let t = project.track_mut(&loop_id).unwrap();
        t.filter_type = "lp".into();
        t.filter_cutoff_hz = 1_500.0;
        t.filter_resonance = 0.7;
        t.delay_samples = 700;
        t.clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "clip-sine".into(),
                name: "sine".into(),
                source_path: "sine".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: SR as u64 * 4,
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
                tuned_notes: Vec::new(),
            }),
            track_id: loop_id.clone(),
            position_ticks: 0,
            length_ticks: 4 * BAR,
            lane: 0,
            color: None,
            row: None,
        });
    }
    (project, lead, spare)
}

fn make_callback(
    project: Arc<Mutex<Project>>,
    pool: AudioPool,
) -> (EngineCallback, crossbeam_channel::Sender<EngineCommand>) {
    let (cb, commands, _, _) = make_callback_with_plugins(project, pool);
    (cb, commands)
}

/// The callback with its plug-in command queue and graveyard.
fn make_callback_with_plugins(
    project: Arc<Mutex<Project>>,
    pool: AudioPool,
) -> (
    EngineCallback,
    crossbeam_channel::Sender<EngineCommand>,
    crate::insert_chain::InsertCommandSender,
    crate::insert_chain::PluginGraveyardReceiver,
) {
    let transport = TransportState::default();
    transport.sample_rate.store(SR as u64, Relaxed);
    transport.playing.store(true, Relaxed);
    let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
    let (command_tx, command_rx) = bounded::<EngineCommand>(8);
    let (cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
    let (grave_tx, grave_rx) = crate::insert_chain::graveyard_channel(8);
    let cb = EngineCallback::new(
        transport,
        project,
        meter_producer,
        command_rx,
        pool,
        Arc::new(Mutex::new(HashMap::new())),
        Arc::new(Mutex::new(None)),
        crate::input_node::capture_set(),
        master_tap::new_shared(),
        Arc::new(std::sync::atomic::AtomicU32::new(0)),
        cmd_rx,
        grave_tx,
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(MidiInputManager::new())),
        Arc::new(Mutex::new(MidiCaptureRing::new(64))),
        SR,
        BLOCK as u32,
        LoadCounters::new(),
        crate::metronome::MetronomeSettings::silent(),
        crate::preview_player::PreviewRequest::new(),
        crate::audio_prefs::AudioPrefs::new(),
        Arc::new(std::sync::atomic::AtomicU32::new(0)),
        Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX)),
        Arc::new(Mutex::new(None)),
        Arc::new(Mutex::new(None)),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        Arc::new(crate::session::SessionState::new()),
    );
    (cb, command_tx, cmd_tx, grave_rx)
}

/// Play `blocks` blocks, calling `edit` before block `at`. Returns the
/// interleaved output.
fn play(
    cb: &mut EngineCallback,
    blocks: usize,
    mut edit: impl FnMut(&mut EngineCallback, usize),
) -> Vec<f32> {
    let mut all = Vec::with_capacity(blocks * BLOCK * 2);
    let mut out = vec![0.0f32; BLOCK * 2];
    for block in 0..blocks {
        edit(cb, block);
        cb.process(&mut out, BLOCK, 2);
        all.extend_from_slice(&out);
    }
    all
}

/// The first sample where two renders differ.
fn first_difference(a: &[f32], b: &[f32]) -> Option<(usize, f32, f32)> {
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
        .map(|i| (i, a[i], b[i]))
}

/// An edit in the middle of playback that changes nothing heard (a note
/// added past the end, a track renamed) swaps in a new graph, and the
/// sound goes on sample for sample as if there had been no swap: the lead's
/// note, the kick's tail, the filters and the delay all carry on. Each
/// rebuild started them over: the held note stopped, the kick's tail was
/// cut and every filter and delay started from silence.
#[test]
fn a_swap_while_playing_carries_on_sample_for_sample() {
    let pool = AudioPool::new();
    let (project, lead, spare) = song(&pool);
    let blocks = 120;

    let (mut reference_cb, _) = make_callback(Arc::new(Mutex::new(project.clone())), pool.clone());
    let reference = play(&mut reference_cb, blocks, |_, _| {});
    assert!(
        reference.iter().any(|s| s.abs() > 0.05),
        "the song makes a sound"
    );

    let project_arc = Arc::new(Mutex::new(project));
    let (mut cb, _) = make_callback(Arc::clone(&project_arc), pool);
    let edited = play(&mut cb, blocks, |cb, block| {
        if block == 37 || block == 80 {
            let mut p = project_arc.lock();
            p.track_mut(&spare).unwrap().name = format!("Spare {block}");
            if let ClipContent::Midi(part) = &mut p.track_mut(&lead).unwrap().clips[0].content {
                part.clip.notes.push(hardwave_midi::MidiNote {
                    start_tick: 7 * BAR + block as u64,
                    duration_ticks: 120,
                    pitch: 60,
                    velocity: 0.8,
                    ..Default::default()
                });
            }
            drop(p);
            cb.needs_rebuild = true;
        }
    });
    assert_eq!(cb.graph_seq, 3, "two edits, two new graphs");
    if let Some((i, was, is)) = first_difference(&reference, &edited) {
        panic!(
            "the sound changed at frame {} (block {}): {was} became {is}",
            i / 2,
            i / 2 / BLOCK
        );
    }
}

/// A note a plug-in instrument is holding that an edit deletes is let go
/// of at once; one the edit leaves alone keeps sounding with no second
/// note-on.
#[test]
fn a_held_note_the_edit_deletes_is_let_go() {
    let pool = AudioPool::new();
    let mut project = Project::default();
    let lead = project.add_midi_track("Lead".into());
    project
        .track_mut(&lead)
        .unwrap()
        .clips
        .push(midi_part(&lead, &[(0, 4 * BAR, 45), (0, 4 * BAR, 52)]));
    let project_arc = Arc::new(Mutex::new(project));
    let (mut cb, _) = make_callback(Arc::clone(&project_arc), pool);
    let mut out = vec![0.0f32; BLOCK * 2];
    for _ in 0..10 {
        cb.process(&mut out, BLOCK, 2);
    }
    if let ClipContent::Midi(part) =
        &mut project_arc.lock().track_mut(&lead).unwrap().clips[0].content
    {
        part.clip.notes.retain(|n| n.pitch != 52);
    }
    cb.needs_rebuild = true;
    cb.process(&mut out, BLOCK, 2);
    let node_id = cb.track_id_to_node[&lead];
    let node = cb
        .graph
        .node_mut(node_id)
        .unwrap()
        .as_any_mut()
        .unwrap()
        .downcast_mut::<crate::midi_track_node::MidiTrackNode>()
        .unwrap();
    assert_eq!(
        node.offs_sent_last_block(),
        vec![(0, 52)],
        "the deleted note, and only it, is let go"
    );
}

/// With the builder thread running, an edit costs the audio thread no
/// build and no allocation: the block that asks for the graph and the
/// block that swaps it in allocate and free nothing, and the graph that
/// was replaced is freed on the builder thread.
#[test]
fn the_audio_thread_neither_builds_nor_allocates_for_an_edit() {
    let pool = AudioPool::new();
    let (project, lead, spare) = song(&pool);
    let blocks = 160;

    let (mut reference_cb, _) = make_callback(Arc::new(Mutex::new(project.clone())), pool.clone());
    let reference = play(&mut reference_cb, blocks, |_, _| {});

    // A plug-in that leaves the sound as it is, on the sample's track, and
    // one on a track the second edit deletes.
    let mut project = project;
    let doomed = project.add_audio_track("Doomed".into());
    let loop_id = project
        .tracks
        .iter()
        .find(|t| t.name == "Loop")
        .unwrap()
        .id
        .clone();
    for (track, slot) in [(&loop_id, "unity"), (&doomed, "gone")] {
        project
            .track_mut(track)
            .unwrap()
            .inserts
            .push(hardwave_project::track::PluginSlot {
                id: slot.into(),
                plugin_id: "test.gain".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
    }
    let project_arc = Arc::new(Mutex::new(project));
    let (mut cb, commands, mut plugins, mut graveyard) =
        make_callback_with_plugins(Arc::clone(&project_arc), pool);
    cb.attach_builder_thread();
    assert!(matches!(cb.builder, BuildMode::Thread(_)));
    for (track, slot) in [(&loop_id, "unity"), (&doomed, "gone")] {
        let (slot, _) = crate::insert_chain::tests::make_gain_slot(slot, 1.0, true);
        plugins
            .try_send(crate::insert_chain::InsertCommand::Add {
                track_id: track.clone(),
                slot,
            })
            .ok()
            .unwrap();
    }

    let mut all = Vec::with_capacity(blocks * BLOCK * 2);
    let mut out = vec![0.0f32; BLOCK * 2];
    let mut swaps = 0;
    for block in 0..blocks {
        if block == 30 || block == 90 {
            {
                let mut p = project_arc.lock();
                p.track_mut(&spare).unwrap().name = format!("Spare {block}");
                if block == 90 {
                    p.remove_track(&doomed);
                }
                if let ClipContent::Midi(part) = &mut p.track_mut(&lead).unwrap().clips[0].content {
                    part.clip.notes.push(hardwave_midi::MidiNote {
                        start_tick: 7 * BAR,
                        duration_ticks: 120,
                        pitch: 60,
                        velocity: 0.8,
                        ..Default::default()
                    });
                }
            }
            commands.send(EngineCommand::RebuildGraph).unwrap();
            // The block that takes the request. A graph the builder thread
            // finished may already go in here (one asked for earlier, or
            // this one when the test thread was held up for a moment), so
            // what is checked is that this thread built nothing.
            let seq = cb.graph_seq;
            let built_here = super::graph_build::builds_on_this_thread();
            let (allocs, frees) = counted(|| cb.process(&mut out, BLOCK, 2));
            all.extend_from_slice(&out);
            assert_eq!((allocs, frees), (0, 0), "asking for a graph");
            assert_eq!(
                super::graph_build::builds_on_this_thread(),
                built_here,
                "nothing is built on the audio thread"
            );
            let BuildMode::Thread(link) = &cb.builder else {
                panic!("the builder thread is still running");
            };
            let wanted = link.requested();
            assert!(wanted > seq, "the edit asked for a graph");
            // Wait for the builder, as the sound card would keep calling,
            // until the graph asked for is in.
            let started = std::time::Instant::now();
            while cb.graph_seq < wanted {
                while !matches!(&cb.builder, BuildMode::Thread(link) if link.has_ready()) {
                    assert!(started.elapsed().as_secs() < 30, "the builder answers");
                    std::thread::yield_now();
                }
                let (allocs, frees) = counted(|| cb.process(&mut out, BLOCK, 2));
                all.extend_from_slice(&out);
                assert_eq!((allocs, frees), (0, 0), "swapping the graph in");
            }
            assert_eq!(
                super::graph_build::builds_on_this_thread(),
                built_here,
                "nothing is built on the audio thread"
            );
            swaps += 1;
            continue;
        }
        if all.len() >= blocks * BLOCK * 2 {
            break;
        }
        cb.process(&mut out, BLOCK, 2);
        all.extend_from_slice(&out);
    }
    all.truncate(blocks * BLOCK * 2);
    assert_eq!(swaps, 2);
    let node = cb.track_id_to_node[&loop_id];
    assert_eq!(
        cb.graph.node_mut(node).unwrap().chain_len(),
        1,
        "the plug-in came through both swaps"
    );
    assert_eq!(
        graveyard.drain_and_drop(),
        1,
        "the deleted track's plug-in went to the graveyard, to be let go off the audio thread"
    );
    if let Some((i, was, is)) = first_difference(&reference, &all) {
        panic!(
            "the sound changed at frame {} (block {}): {was} became {is}",
            i / 2,
            i / 2 / BLOCK
        );
    }
}
