//! Smart disable leaves a plug-in out while its input is silent and its
//! sound has died away. An export of a song with gaps in it, built-in
//! instruments and effects that ring (reverb, delay) or run a clock of
//! their own (chorus, phaser), must sound the same with smart disable on as
//! with it off: every sample within -140 dBFS.

use hardwave_engine::engine::OfflineInsertFactory;
use hardwave_engine::DawEngine;
use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
use hardwave_project::track::{PluginSlot, Track};

const SR: u32 = 48_000;
const BAR: u64 = 4 * 960;

fn plug_in(track: &mut Track, plugin_id: &str) {
    let n = track.inserts.len();
    track.inserts.push(PluginSlot {
        allow_smart_disable: None,
        id: format!("{}-{n}", track.id),
        plugin_id: plugin_id.into(),
        enabled: true,
        state: None,
        sidechain_source: None,
        wet: 1.0,
    });
}

/// A MIDI clip on `track` at `bar`, one bar long, with a short chord.
fn chord(track: &mut Track, bar: u64, pitch: u8) {
    let mut clip = hardwave_midi::MidiClip::new(format!("{}-{bar}", track.id), "Chord".into(), BAR);
    for (k, p) in [pitch, pitch + 4, pitch + 7].into_iter().enumerate() {
        clip.notes.push(hardwave_midi::MidiNote {
            start_tick: k as u64 * 120,
            duration_ticks: 960,
            pitch: p,
            velocity: 0.8,
            ..Default::default()
        });
    }
    let id = clip.id.clone();
    track.clips.push(ClipPlacement {
        content: ClipContent::Midi(MidiClipRef { id, clip }),
        track_id: track.id.clone(),
        position_ticks: bar * BAR,
        length_ticks: BAR,
        lane: 0,
        color: None,
        row: None,
    });
}

/// Two instrument tracks that play, stop for several seconds and play
/// again, through effects that ring or move on their own, and an empty
/// track with a reverb on it.
fn song() -> DawEngine {
    use hardwave_native_plugins as hn;
    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        let a = project.add_midi_track("Pad".into());
        let t = project.track_mut(&a).unwrap();
        plug_in(t, hn::NativeTripleOsc::ID);
        plug_in(t, hn::NativeChorus::ID);
        plug_in(t, hn::NativeReverb::ID);
        chord(t, 0, 57);
        chord(t, 9, 60);
        let b = project.add_midi_track("Lead".into());
        let t = project.track_mut(&b).unwrap();
        plug_in(t, hn::NativeWavetable::ID);
        plug_in(t, hn::NativePhaser::ID);
        plug_in(t, hn::NativeDelay::ID);
        chord(t, 1, 64);
        chord(t, 10, 67);
        let c = project.add_audio_track("Empty".into());
        let t = project.track_mut(&c).unwrap();
        plug_in(t, hn::NativeReverb::ID);
        plug_in(t, hn::NativeEq::ID);
    }
    engine.rebuild_graph();
    engine
}

/// The whole export, interleaved.
fn export(engine: &DawEngine, frames: u64) -> Vec<f32> {
    let factory = hardwave_native_plugins::create_native;
    let factory: OfflineInsertFactory<'_> = &factory;
    let mut out = Vec::with_capacity(frames as usize * 2);
    engine
        .render_offline_with(
            SR,
            frames,
            0,
            Some(factory),
            |_| {},
            |b| {
                out.extend_from_slice(b);
                true
            },
        )
        .expect("it renders");
    out
}

#[test]
fn an_export_sounds_the_same_with_smart_disable_on_and_off() {
    // Twelve bars at 120 BPM: 24 s, with about 14 s of nothing between the
    // two halves for the effects to fall asleep in.
    let frames = 24 * SR as u64;
    let engine = song();
    engine.audio_prefs().set_smart_disable(false);
    let off = export(&engine, frames);
    engine.audio_prefs().set_smart_disable(true);
    let on = export(&engine, frames);
    assert_eq!(off.len(), on.len());
    let peak = off.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.05, "the song is audible ({peak})");
    let mut worst = 0.0_f32;
    let mut differ = 0usize;
    for (a, b) in off.iter().zip(&on) {
        if a.to_bits() != b.to_bits() {
            differ += 1;
        }
        worst = worst.max((a - b).abs());
    }
    eprintln!(
        "{differ} of {} samples differ; the largest difference is {worst:e} ({:.1} dBFS)",
        off.len(),
        20.0 * worst.max(1e-30).log10()
    );
    assert!(
        worst < 1e-7,
        "smart disable changed the export by {worst:e}"
    );
}
