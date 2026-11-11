//! A note held in a MIDI clip sounds for its whole length in playback, as
//! it does in an export, on every built-in instrument: 3xOSC, Wavetable,
//! FM, the sampler, the built-in synth and the kick. Playback runs the way a
//! sound card drives it on Windows, 480 frames a block to a graph built for
//! 512. Playback used to let go of a plug-in's note one block after it
//! began, so a two-bar note died away in a fifth of a second while the
//! export of the same song held it.

use hardwave_engine::engine::OfflineInsertFactory;
use hardwave_engine::DawEngine;
use hardwave_native_plugins::fm::NativeFmSynth;
use hardwave_native_plugins::sampler::{self, NativeSampler};
use hardwave_native_plugins::triple_osc::NativeTripleOsc;
use hardwave_native_plugins::wavetable::NativeWavetable;
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::track::{NativeInstrument, PluginSlot, Track};

const SR: u32 = 48_000;
const BAR: u64 = 4 * 960;
/// What a Windows driver commonly asks for from a graph built for 512.
const DEVICE_FRAMES: usize = 480;
/// The level is compared in windows this long.
const WINDOW: usize = SR as usize / 20;

/// The song from the report: one MIDI track, a four-bar clip, one note
/// (pitch 57) two bars long from the start. `setup` puts the instrument on
/// the track. Returns the engine, where the note ends and how long the
/// clip is, both in samples.
fn song(setup: impl FnOnce(&mut Track)) -> (DawEngine, usize, usize) {
    let engine = DawEngine::new();
    let mut project = engine.project.lock();
    let track_id = project.add_midi_track("Synth".into());
    let track = project.track_mut(&track_id).expect("the new track");
    setup(track);
    let mut clip = hardwave_midi::MidiClip::new("held".into(), "Held".into(), 4 * BAR);
    clip.notes.push(hardwave_midi::MidiNote {
        start_tick: 0,
        duration_ticks: 2 * BAR,
        pitch: 57,
        velocity: 0.8,
        ..Default::default()
    });
    track.clips.push(ClipPlacement {
        content: ClipContent::Midi(MidiClipRef {
            id: "held".into(),
            clip,
        }),
        track_id: track_id.clone(),
        position_ticks: 0,
        length_ticks: 4 * BAR,
        lane: 0,
        color: None,
        row: None,
    });
    let note_end = project.tempo_map.tick_to_samples(2 * BAR, SR as f64) as usize;
    let clip_end = project.tempo_map.tick_to_samples(4 * BAR, SR as f64) as usize;
    drop(project);
    engine.rebuild_graph();
    (engine, note_end, clip_end)
}

fn plug_in(track: &mut Track, plugin_id: &str) {
    track.inserts.push(PluginSlot {
        id: format!("slot-{plugin_id}"),
        plugin_id: plugin_id.into(),
        enabled: true,
        state: None,
        sidechain_source: None,
        wet: 1.0,
    });
}

/// The left side of `frames` frames of playback, in device-sized blocks.
fn playback(engine: &DawEngine, frames: usize) -> Vec<f32> {
    let factory = hardwave_native_plugins::create_native;
    let factory: OfflineInsertFactory<'_> = &factory;
    let mut left = Vec::with_capacity(frames);
    engine
        .play_live_in_device_blocks(SR, frames as u64, DEVICE_FRAMES, Some(factory), |b| {
            left.extend(b.iter().step_by(2));
            true
        })
        .expect("it plays");
    left
}

/// The left side of `frames` frames of an export of the same song.
fn export(engine: &DawEngine, frames: usize) -> Vec<f32> {
    let factory = hardwave_native_plugins::create_native;
    let factory: OfflineInsertFactory<'_> = &factory;
    let mut left = Vec::with_capacity(frames);
    engine
        .render_offline_with(
            SR,
            frames as u64,
            0,
            Some(factory),
            |_| {},
            |b| {
                left.extend(b.iter().step_by(2));
                true
            },
        )
        .expect("it renders");
    left
}

fn rms_db(s: &[f32]) -> f32 {
    let mean = s.iter().map(|v| v * v).sum::<f32>() / s.len().max(1) as f32;
    10.0 * mean.max(1e-12).log10()
}

/// Playback and the export are as loud as each other in every window from
/// `from` to `to`, and audible there.
fn same_level(what: &str, live: &[f32], exported: &[f32], from: usize, to: usize) {
    let mut at = from;
    while at + WINDOW <= to {
        let (l, e) = (
            rms_db(&live[at..at + WINDOW]),
            rms_db(&exported[at..at + WINDOW]),
        );
        let secs = at as f32 / SR as f32;
        assert!(
            e > -40.0,
            "{what}: the export is audible at {secs:.2} s ({e:.1} dB)"
        );
        assert!(
            (l - e).abs() < 1.5,
            "{what}: playback is {l:.1} dB at {secs:.2} s where the export is {e:.1} dB"
        );
        at += WINDOW;
    }
}

/// A plug-in instrument holds the note to its end, as in the export.
fn holds_the_note(plugin_id: &str, state: Option<Vec<u8>>) {
    let (engine, note_end, clip_end) = song(|t| plug_in(t, plugin_id));
    if let Some(state) = state {
        // Where a song keeps a plug-in's state, as the app saves it.
        engine
            .project
            .lock()
            .set_plugin_state(format!("slot-{plugin_id}"), plugin_id, state);
    }
    let live = playback(&engine, clip_end);
    let exported = export(&engine, clip_end);
    // From a tenth of a second in (past the attack and decay) to a tenth
    // before the note's end.
    same_level(
        plugin_id,
        &live,
        &exported,
        SR as usize / 10,
        note_end - SR as usize / 10,
    );
}

#[test]
fn three_osc_holds_a_note_its_whole_length_in_playback() {
    holds_the_note(NativeTripleOsc::ID, None);
}

#[test]
fn wavetable_holds_a_note_its_whole_length_in_playback() {
    holds_the_note(NativeWavetable::ID, None);
}

#[test]
fn fm_holds_a_note_its_whole_length_in_playback() {
    holds_the_note(NativeFmSynth::ID, None);
}

#[test]
fn the_sampler_holds_a_note_its_whole_length_in_playback() {
    // A steady tone longer than the note, so the note's length is what
    // ends it.
    let frames = SR as usize * 6;
    let tone: Vec<f32> = (0..frames)
        .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / SR as f32).sin() * 0.5)
        .collect();
    let state = sampler::encode_sample_state(SR, 57, vec![tone.clone(), tone]);
    holds_the_note(NativeSampler::ID, Some(state));
}

#[test]
fn the_built_in_synth_holds_a_note_its_whole_length_in_playback() {
    let (engine, note_end, clip_end) = song(|_| {});
    let live = playback(&engine, clip_end);
    let exported = export(&engine, clip_end);
    same_level(
        "built-in synth",
        &live,
        &exported,
        SR as usize / 10,
        note_end - SR as usize / 10,
    );
}

#[test]
fn the_kick_sounds_the_same_in_playback_as_in_the_export() {
    let (engine, _, clip_end) = song(|t| t.instrument = NativeInstrument::KickSynth);
    let live = playback(&engine, clip_end);
    let exported = export(&engine, clip_end);
    // The kick's body, while it is loud enough to compare.
    same_level("kick", &live, &exported, 0, SR as usize / 5);
    let worst = live
        .iter()
        .zip(&exported)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-3, "the kick lands on the same samples: {worst}");
}
