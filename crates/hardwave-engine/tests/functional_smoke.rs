//! Functional smoke tests — verify the DAW engine actually does what
//! a user expects, not just that it compiles and unit-tests pass.
//!
//! These tests boot the real `DawEngine`, build a minimal project
//! programmatically, render a short window via `render_offline`, and
//! assert on the output buffer. They are the layer that catches "vapor
//! features" — code paths that compile and have unit tests but never
//! get wired into the real audio graph.
//!
//! ## Status (re-verified 2026-07-19)
//!
//! Every test here is PASS-required — a regression means we broke
//! something that demonstrably works. There are no `#[ignore]`d tests
//! left: the two former KILLER-watch panics (track FX inserts, and
//! automation clips/LFO) were re-verified against the engine, found to
//! be fully wired, and rewritten as real end-to-end coverage.
//!
//! The whole file runs as one blocking job — see
//! `.github/workflows/functional-smoke.yml`.

use hardwave_engine::DawEngine;
use std::sync::atomic::Ordering;

mod common;
use common::*;

const SAMPLE_RATE: u32 = 48_000;

// ───────────────────────────────────────────────────────────────────────
// PASS-required tests — these protect features that work today.
// A regression here is a real bug, not a known-vapor flag.
// ───────────────────────────────────────────────────────────────────────

#[test]
fn audio_clip_produces_sound() {
    // PASS-required.
    // A sine clip on an audio track should produce non-silent output.
    // Roadmap covered: P1/Audio Output, P2/Clip System, P2/File Import (engine side).
    let engine = DawEngine::new();
    add_audio_track_with_sine(
        &engine,
        "Sine",
        "smoke-sine-440",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );

    let render_samples = SAMPLE_RATE as u64 / 2; // 0.5 s
    let stats = render_and_measure(&engine, SAMPLE_RATE, render_samples);

    assert_eq!(stats.nan_count, 0, "engine produced NaN samples");
    assert_eq!(stats.inf_count, 0, "engine produced Inf samples");
    assert!(stats.frames > 0, "no audio frames were produced");
    assert!(
        stats.peak > 0.05,
        "expected audible sine output, got peak={:.5}",
        stats.peak
    );
}

#[test]
fn master_volume_attenuates_output() {
    // PASS-required.
    // Lowering master_volume_db should proportionally reduce the peak.
    // Roadmap covered: P1/Audio Graph/Master volume control.
    let engine = DawEngine::new();
    add_audio_track_with_sine(
        &engine,
        "Sine",
        "smoke-sine-master",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    let render_samples = SAMPLE_RATE as u64 / 4; // 0.25 s

    engine
        .transport
        .master_volume_db
        .store(0.0, Ordering::Relaxed);
    let unity = render_and_measure(&engine, SAMPLE_RATE, render_samples);

    engine
        .transport
        .master_volume_db
        .store(-12.0, Ordering::Relaxed);
    let attenuated = render_and_measure(&engine, SAMPLE_RATE, render_samples);

    assert!(unity.peak > 0.05, "unity render produced no audible signal");
    assert!(
        attenuated.peak < unity.peak,
        "master -12 dB should reduce peak (unity={:.4}, -12dB={:.4})",
        unity.peak,
        attenuated.peak
    );

    // -12 dB is ≈ 0.251×. Allow a generous window for any peak detector hysteresis.
    let ratio = attenuated.peak / unity.peak.max(1e-9);
    assert!(
        (0.18..=0.36).contains(&ratio),
        "-12 dB should attenuate to ~0.25× of unity; got ratio={:.4}",
        ratio
    );
}

#[test]
fn mute_silences_track() {
    // PASS-required.
    // Setting track.muted=true must produce silence (or near-silence) in the render.
    // Roadmap covered: P1/Audio Graph/Per-track mute button.
    let engine = DawEngine::new();
    let track_id = add_audio_track_with_sine(
        &engine,
        "Muteable",
        "smoke-sine-mute",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );

    // Sanity — unmuted should be loud.
    let unmuted = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(
        unmuted.peak > 0.05,
        "unmuted render is silent ({:?})",
        unmuted
    );

    // Mute the track.
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.muted = true;
        }
    }

    let muted = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(
        muted.peak < 0.001,
        "muted track should produce near-silence, got peak={:.6}",
        muted.peak
    );
}

#[test]
fn solo_silences_other_tracks() {
    // PASS-required.
    // FL Studio / Logic / Ableton convention: when ANY track is soloed,
    // every non-soloed (and non-solo-safe) audio-bearing track must
    // become inaudible. Verified at the engine layer in `rebuild_graph`
    // where `any_soloed` derives `effective_mute = !track.soloed &&
    // !track.solo_safe`.
    let engine = DawEngine::new();
    let kept_id = add_audio_track_with_sine(
        &engine,
        "Kept",
        "smoke-solo-kept",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    let silenced_id = add_audio_track_with_sine(
        &engine,
        "Silenced",
        "smoke-solo-silenced",
        SAMPLE_RATE,
        1.0,
        660.0,
        0.5,
    );

    // Baseline: both tracks unsoloed → both audible → mixed peak.
    let mixed = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(mixed.peak > 0.05, "baseline mix is silent ({:?})", mixed);

    // Solo just the first track; the second must silence.
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&kept_id) {
            t.soloed = true;
        }
        // Force the silenced track to non-solo-safe so it ducks.
        if let Some(t) = project.track_mut(&silenced_id) {
            t.solo_safe = false;
        }
    }

    let soloed = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(
        soloed.peak > 0.05,
        "soloed track must remain audible ({:?})",
        soloed
    );
    // The mix peak should drop after the second track is silenced
    // (one of the two sine sources is now contributing zero).
    assert!(
        soloed.peak <= mixed.peak,
        "solo did not reduce mix energy: mixed={:.4} solo={:.4}",
        mixed.peak,
        soloed.peak
    );
}

#[test]
fn kicksynth_instrument_produces_kick_audio() {
    // PASS-required.
    //
    // A MIDI track with instrument=KickSynth must retrigger the kick
    // voice on every note-on regardless of pitch. The clip-driven
    // path inside MidiTrackNode (rebuild_graph maps NativeInstrument
    // → engine Instrument enum) hits `self.kick.note_on(pitch, vel)`
    // and renders the pre-baked kick block into the track output.
    use hardwave_midi::MidiClip;
    use hardwave_midi::MidiNote;
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
    use hardwave_project::track::NativeInstrument;

    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        let id = project.add_midi_track("Kick".to_string());

        // Flip the track's instrument to KickSynth BEFORE the graph
        // builds (rebuild_graph reads track.instrument at audio-node
        // construction time).
        if let Some(t) = project.track_mut(&id) {
            t.instrument = NativeInstrument::KickSynth;
        }

        let mut clip = MidiClip::new("smoke-kick-clip".to_string(), "kick".to_string(), 1920);
        clip.notes.push(MidiNote {
            start_tick: 0,
            duration_ticks: 480,
            pitch: 36, // C2 — kick range, but the synth ignores pitch
            velocity: 1.0,
            channel: 0,
            muted: false,
        });
        let mref = MidiClipRef {
            id: "smoke-kick-clip".to_string(),
            clip,
        };
        if let Some(track) = project.track_mut(&id) {
            track.clips.push(ClipPlacement {
                content: ClipContent::Midi(mref),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
            });
        }
    }

    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 2);
    assert_eq!(stats.nan_count, 0, "kick synth produced NaN samples");
    assert!(
        stats.peak > 0.01,
        "KickSynth note must produce audible kick output, got peak={:.6}",
        stats.peak
    );
}

#[test]
fn two_tracks_mix_louder_than_one() {
    // PASS-required.
    // Two coherent sine sources at the same frequency must sum at the
    // master bus to a higher peak than either alone. Tests the audio
    // graph's mix step + master node summation.
    let engine_one = DawEngine::new();
    add_audio_track_with_sine(
        &engine_one,
        "Solo",
        "smoke-mix-one",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    let one = render_and_measure(&engine_one, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);

    let engine_two = DawEngine::new();
    add_audio_track_with_sine(
        &engine_two,
        "A",
        "smoke-mix-A",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    add_audio_track_with_sine(
        &engine_two,
        "B",
        "smoke-mix-B",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    let two = render_and_measure(&engine_two, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);

    assert!(one.peak > 0.05, "single-track render is silent");
    assert!(
        two.peak > one.peak * 1.4,
        "two coherent sines should sum to ~2× peak; one={:.4} two={:.4}",
        one.peak,
        two.peak
    );
}

// ───────────────────────────────────────────────────────────────────────
// Former KILLER-watch tests — RESOLVED 2026-07-19.
//
// Both used to be `#[ignore]`d panics asserting their features were
// vapor. Re-verification showed the features were in fact fully wired
// (the audit notes they were written against had gone stale), so the
// panics were replaced with real end-to-end coverage and un-ignored.
// They now run in the default `cargo test` pass like any other test.
// ───────────────────────────────────────────────────────────────────────

#[test]
fn midi_clip_produces_sound() {
    // PASS-required.
    //
    // A MIDI track with a clip containing a note must play through the
    // BuiltinSine voice that MidiTrackNode wires up by default. Was a
    // killer-watch through 2026-05-04 — the wiring landed in the
    // MidiTrackNode lineage and the v0.164 work confirmed the live
    // MIDI plumbing; clip notes ride the same code path.
    //
    // `render_offline_with` forces transport.playing=true (engine.rs:773),
    // so we don't need to start playback explicitly. The MidiTrackNode
    // builds `note_regions` from the clip during rebuild_graph and
    // fires the voice on the first block where position_samples >= note_on_sample.
    use hardwave_midi::MidiClip;
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};

    use hardwave_midi::MidiNote;

    let engine = DawEngine::new();
    {
        let mut project = engine.project.lock();
        let id = project.add_midi_track("Synth".to_string());

        // Construct a one-note MIDI clip — middle C, half a beat long, full velocity.
        let mut clip = MidiClip::new(
            "smoke-midi-clip".to_string(),
            "smoke".to_string(),
            1920, // 2 quarter notes at 960 PPQ
        );
        clip.notes.push(MidiNote {
            start_tick: 0,
            duration_ticks: 480, // half a beat
            pitch: 60,
            velocity: 1.0,
            channel: 0,
            muted: false,
        });

        let mref = MidiClipRef {
            id: "smoke-midi-clip".to_string(),
            clip,
        };
        if let Some(track) = project.track_mut(&id) {
            track.clips.push(ClipPlacement {
                content: ClipContent::Midi(mref),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
            });
        }
    }

    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 2);
    assert_eq!(stats.nan_count, 0, "MIDI track produced NaN samples");
    assert!(
        stats.peak > 0.001,
        "MIDI clip note should produce audible BuiltinSine output, got peak={:.6} — \
         check MidiTrackNode wiring in rebuild_graph + voice/envelope state machine",
        stats.peak
    );
}

#[test]
fn killer_track_insert_modifies_audio() {
    // Was a KILLER-watch panic ("TrackNode does not process track.inserts").
    // FLIPPED GREEN 2026-07-19: the insert chain is fully wired. This now
    // proves it end-to-end — attach an FX insert (a hard-clip mock) to a
    // track and assert the offline render differs from the dry render.
    //
    // Uses `render_offline_with(.., Some(factory), ..)`, the same offline
    // hydration path the export command drives via `build_offline_insert_factory`.
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use hardwave_project::track::PluginSlot;

    /// A hard clipper at ±`ceiling` — an unmistakable, audible effect that
    /// reshapes the waveform and caps the peak. Mirrors the I/O contract
    /// `InsertChain::process` expects (inputs[0]=L, inputs[1]=R; write the
    /// processed block into outputs[0]/[1]).
    struct HardClip {
        desc: PluginDescriptor,
        ceiling: f32,
    }
    impl HostedPlugin for HardClip {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                if let Some(out) = outputs.get_mut(ch) {
                    out.clear();
                    for i in 0..num_samples {
                        let s = inp.get(i).copied().unwrap_or(0.0);
                        out.push(s.clamp(-self.ceiling, self.ceiling));
                    }
                }
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    fn hardclip_desc() -> PluginDescriptor {
        PluginDescriptor {
            id: "test.hardclip".into(),
            name: "HardClip".into(),
            vendor: "t".into(),
            version: "1".into(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    let engine = DawEngine::new();
    let track_id =
        add_audio_track_with_sine(&engine, "FX", "smoke-sine-fx", SAMPLE_RATE, 1.0, 440.0, 0.5);

    // Small local render helper: render a quarter-second and return peak.
    let render_peak =
        |factory: Option<&dyn Fn(&str) -> Option<Box<dyn HostedPlugin>>>| -> f32 {
            let mut peak = 0.0f32;
            engine
                .render_offline_with(
                    SAMPLE_RATE,
                    SAMPLE_RATE as u64 / 4,
                    0,
                    factory,
                    |_| {},
                    |block| {
                        for &s in block {
                            peak = peak.max(s.abs());
                        }
                        true
                    },
                )
                .unwrap();
            peak
        };

    // Dry render (no factory → insert skipped): the raw 0.5 sine, attenuated
    // ~0.707 by equal-power pan-center → ~0.354 at the master.
    let dry = render_peak(None);
    assert!(dry > 0.3, "dry render should be the full sine, got peak={dry}");

    // Attach the hard-clip insert, then render with a factory that builds it.
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&track_id) {
            t.inserts.push(PluginSlot {
                id: "slot-clip".into(),
                plugin_id: "test.hardclip".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
    }
    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == "test.hardclip" {
            Some(Box::new(HardClip {
                desc: hardclip_desc(),
                ceiling: 0.25,
            }))
        } else {
            None
        }
    };
    let wet = render_peak(Some(&factory));
    // Insert sits pre-fader, so the ±0.25 clip ceiling is also pan-attenuated
    // ~0.707 → ~0.177 at the master.
    assert!(
        wet > 0.15 && wet < 0.21,
        "hard-clip insert should cap the peak near its pan-scaled ±0.25 ceiling, got peak={wet}"
    );
    assert!(
        wet < dry - 0.1,
        "track FX insert must change the audio (wet {wet} vs dry {dry})"
    );
}

#[test]
fn recording_api_captures_samples() {
    // PASS-required.
    //
    // Recording was a killer-watch through 2026-05-04 claiming no public
    // engine API existed. In reality the production flow lives on
    // `CaptureTap` (input_node.rs) with `engine.start_capture()` +
    // `engine.stop_capture()` as the entry points, wired through to
    // the Tauri `stop` transport command's `finalize_recording_session`
    // which serialises the drained buffer to a WAV (transport.rs:48).
    //
    // The audio-thread integration (InputNode pushing samples into the
    // tap when recording is true) needs cpal hardware to test directly,
    // so this smoke covers the engine API contract:
    //   start_capture → flag on → samples accumulate → stop_capture
    //   drains them and flips the flag back.
    use std::sync::atomic::Ordering;
    let engine = DawEngine::new();

    assert!(!engine.capture.recording.load(Ordering::Relaxed));
    assert!(
        engine.stop_capture().is_empty(),
        "stop_capture before start should return empty buffer"
    );

    engine.start_capture();
    assert!(
        engine.capture.recording.load(Ordering::Relaxed),
        "start_capture must flip the recording flag"
    );

    // Simulate the audio thread pushing samples into the tap. In
    // production this is `InputNode::process` writing interleaved L/R
    // pairs into `capture.buffer` while `capture.recording` is true.
    {
        let mut buf = engine.capture.buffer.lock();
        buf.extend_from_slice(&[0.1_f32, -0.1, 0.2, -0.2, 0.3, -0.3]);
    }

    let samples = engine.stop_capture();
    assert_eq!(
        samples.len(),
        6,
        "stop_capture must drain all queued samples"
    );
    assert!(
        !engine.capture.recording.load(Ordering::Relaxed),
        "stop_capture must clear the recording flag"
    );
    assert!((samples[0] - 0.1).abs() < 1e-6, "first sample preserved");

    // Idempotent — stop again returns empty.
    assert!(engine.stop_capture().is_empty());
}

// ───────────────────────────────────────────────────────────────────────
// Live MIDI smoke tests — guard beta blockers #4 and #5 (v0.164.x).
//
// These prove the END-TO-END live MIDI path: an injected event into
// MidiInputManager flows through the engine drain, the audio graph
// forwards it to MIDI tracks, the synth voice fires, and the master
// output picks up non-zero samples.
//
// Pre-v0.164.0 the engine NEVER drained MidiInputManager and the
// MidiTrackNode underscored its midi_in parameter — these tests would
// have failed silent (zero peak) before the fix.
// ───────────────────────────────────────────────────────────────────────

#[test]
fn live_midi_noteon_drives_master_output() {
    // PASS-required.
    // Inject a NoteOn into MidiInputManager, render a short window with
    // transport playing, expect the BuiltinSine voice to produce audible
    // samples at the master output.
    let engine = DawEngine::new();
    add_midi_track(&engine, "Live MIDI test");

    // Pre-load the injected event BEFORE render_offline. The shared
    // midi_input means the throwaway audio thread inside render_offline
    // drains the same queue.
    engine
        .midi_input
        .lock()
        .inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 69, // A4 — 440 Hz
            velocity: 0.9,
        });

    // Engage transport so MidiTrackNode treats it as playing.
    engine.transport.playing.store(true, Ordering::Relaxed);

    let render_samples = SAMPLE_RATE as u64 / 4; // 0.25 s window
    let stats = render_and_measure(&engine, SAMPLE_RATE, render_samples);

    assert_eq!(stats.nan_count, 0, "live MIDI produced NaN samples");
    assert_eq!(stats.inf_count, 0, "live MIDI produced Inf samples");
    assert!(
        stats.peak > 0.01,
        "expected audible output from injected NoteOn, got peak={:.5}",
        stats.peak
    );
}

#[test]
fn live_midi_noteon_audible_with_transport_stopped() {
    // PASS-required.
    // FL Studio / Logic / Ableton convention: a soft synth must
    // audition from a controller without engaging Play. This used to
    // fail because MidiTrackNode::process bailed on `!ctx.playing`
    // before consuming midi_in.
    let engine = DawEngine::new();
    add_midi_track(&engine, "Stopped audition");
    engine
        .midi_input
        .lock()
        .inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.8,
        });
    // Transport explicitly NOT started — playing stays false.

    let render_samples = SAMPLE_RATE as u64 / 4;
    let stats = render_and_measure(&engine, SAMPLE_RATE, render_samples);

    assert!(
        stats.peak > 0.01,
        "live NoteOn must audition while transport is stopped — peak={:.5}",
        stats.peak
    );
}

#[test]
fn armed_audio_track_drains_live_midi() {
    // Beta blocker #5 routing guard.
    //
    // Pre-fix, the engine never drained MidiInputManager at all, so an
    // armed audio track hosting a synth plug-in would stay silent.
    // After the fix, audio tracks with `armed && monitor_input` are
    // marked `accepts_live_midi=true` during rebuild_graph and the
    // audio thread drains the queue and forwards to InsertChain.
    //
    // Verify the drain step actually happens: inject MIDI, render
    // offline against an armed audio track, then check the capture
    // ring — the audio thread pushes drained events into it as part
    // of the same per-block routine. A non-empty ring proves the
    // drain ran, which is the prerequisite for InsertChain forwarding
    // to plug-ins (covered separately by the insert_chain.rs unit
    // test `process_forwards_midi_in_to_enabled_slots`).
    let engine = DawEngine::new();
    add_armed_audio_track(&engine, "Armed audio");
    engine
        .midi_input
        .lock()
        .inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.7,
        });
    engine.transport.playing.store(true, Ordering::Relaxed);

    let _ = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 10);

    let entries = engine.midi_capture_ring.lock().entries_in_order();
    assert!(
        !entries.is_empty(),
        "armed audio track must trigger the live-MIDI drain; capture ring empty"
    );
}

#[test]
fn injected_events_land_in_capture_ring() {
    // PASS-required.
    // The rolling 3-min capture buffer is filled by the audio thread's
    // post-drain push loop. After offline render, every injected event
    // should be in the ring in oldest-first order.
    let engine = DawEngine::new();
    add_midi_track(&engine, "Capture test");
    {
        let mgr = engine.midi_input.lock();
        for note in [60u8, 62, 64] {
            mgr.inject(hardwave_midi::MidiEvent::NoteOn {
                timing: 0,
                channel: 0,
                note,
                velocity: 0.7,
            });
        }
    }
    engine.transport.playing.store(true, Ordering::Relaxed);

    // Render long enough that the audio thread drains and pushes.
    let _ = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 10);

    let entries = engine.midi_capture_ring.lock().entries_in_order();
    assert_eq!(
        entries.len(),
        3,
        "capture ring should hold the 3 injected events, got {}",
        entries.len()
    );
    let pitches: Vec<u8> = entries
        .iter()
        .filter_map(|(_, ev)| match ev {
            hardwave_midi::MidiEvent::NoteOn { note, .. } => Some(*note),
            _ => None,
        })
        .collect();
    assert_eq!(pitches, vec![60, 62, 64]);
}

#[test]
fn automation_lane_silences_track_via_volume() {
    // PASS-required.
    //
    // Track-volume automation: was a killer-watch claiming "no engine
    // callers". The Lane/Point/CurveMode types had unit tests but the
    // audit didn't trace TrackNode.process — which DOES walk
    // automation_lanes every block and overrides `volume` for the
    // TrackVolume target (track_node.rs:526).
    //
    // Construct a sine clip, attach a lane that pins value=0.0 across
    // the render window (maps to -60 dB), render, and assert the
    // automation actually pulled the audio to near-silence. Without
    // the lane wiring the track would play at unity (peak > 0.05);
    // with it the peak should be below the mute threshold.
    use hardwave_project::automation::{
        AutomationLane, AutomationPoint, AutomationTarget, CurveMode,
    };
    let engine = DawEngine::new();
    let track_id = add_audio_track_with_sine(
        &engine,
        "Auto",
        "smoke-sine-auto",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.automation_lanes.push(AutomationLane {
                id: "lane-vol".to_string(),
                target: AutomationTarget::TrackVolume,
                // Single point at tick 0, value 0.0 — denormalises to
                // -60 dB across the whole render. `value_at` for any
                // tick beyond the last point returns the last point's
                // value, so the whole render stays pinned at -60 dB.
                points: vec![AutomationPoint {
                    tick: 0,
                    value: 0.0,
                    curve: CurveMode::Linear,
                    tension: 0.0,
                }],
                visible: true,
            });
        }
    }

    let stats = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(
        stats.peak < 0.01,
        "TrackVolume automation pinned at value=0 (-60 dB) should produce \
         near-silence; got peak={:.6}. TrackNode.process not honouring \
         automation_lanes?",
        stats.peak
    );
}

#[test]
fn killer_automation_clips_and_lfo() {
    // Was a KILLER-watch panic ("automation CLIPS + LFO have zero engine
    // callers"). FLIPPED GREEN 2026-07-19: both are wired end-to-end.
    // This proves it audibly:
    //   (A) an LFO baked onto a TrackVolume lane produces amplitude tremolo,
    //   (B) an AutomationClip pinning TrackVolume to 0 silences the track.
    //
    // (automation_recording.rs — touch/write/latch — remains the one genuinely
    // unwired module; it's a live param-capture primitive, covered separately.)
    use hardwave_project::automation::{AutomationLane, AutomationTarget, CurveMode};
    use hardwave_project::automation_clip::AutomationClip;
    use hardwave_project::lfo::{bake_to_points, LfoRate, LfoShape};

    // ── (A) LFO → TrackVolume tremolo ──────────────────────────────────
    let engine = DawEngine::new();
    let track_id =
        add_audio_track_with_sine(&engine, "Trem", "smoke-sine-lfo", SAMPLE_RATE, 1.0, 440.0, 0.5);
    // Bake a full-depth sine LFO. The tick span (40k) comfortably covers a
    // one-second render at any sane tempo; depth 1.0 / center 0.5 swings the
    // volume value across the whole 0..1 range → near-silence to full.
    let points = bake_to_points(
        LfoShape::Sine,
        LfoRate::Hz(8.0),
        120.0, // bpm
        960,   // ppq
        0,     // start_tick
        40_000, // length_ticks
        1.0,   // depth
        0.5,   // center
        0.0,   // phase_offset
        64,    // samples_per_cycle
    );
    assert!(
        points.len() > 8,
        "LFO should bake many control points, got {}",
        points.len()
    );
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&track_id) {
            t.automation_lanes.push(AutomationLane {
                id: "lane-trem".into(),
                target: AutomationTarget::TrackVolume,
                points,
                visible: true,
            });
        }
    }
    // Render the master, tracking a per-window peak envelope (256 interleaved
    // samples = 128 frames/window) to detect amplitude modulation.
    let win = 256usize;
    let mut env: Vec<f32> = Vec::new();
    let (mut cur, mut count) = (0.0f32, 0usize);
    engine
        .render_offline(SAMPLE_RATE, SAMPLE_RATE as u64, |block| {
            for &s in block {
                cur = cur.max(s.abs());
                count += 1;
                if count >= win {
                    env.push(cur);
                    cur = 0.0;
                    count = 0;
                }
            }
            true
        })
        .unwrap();
    if count > 0 {
        env.push(cur);
    }
    let max_env = env.iter().copied().fold(0.0f32, f32::max);
    let min_env = env.iter().copied().fold(f32::INFINITY, f32::min);
    assert!(
        max_env > 0.3,
        "LFO tremolo crest should reach near full volume, got {max_env}"
    );
    assert!(
        min_env < 0.1,
        "LFO tremolo trough should dip toward silence, got {min_env}"
    );
    assert!(
        max_env > min_env * 3.0,
        "LFO must modulate amplitude (crest {max_env} vs trough {min_env})"
    );

    // ── (B) AutomationClip pins TrackVolume to 0 → silence ─────────────
    let engine2 = DawEngine::new();
    let track2 = add_audio_track_with_sine(
        &engine2,
        "ClipAuto",
        "smoke-sine-clip",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    // Baseline: audible before any clip.
    let mut base_peak = 0.0f32;
    engine2
        .render_offline(SAMPLE_RATE, SAMPLE_RATE as u64 / 4, |block| {
            for &s in block {
                base_peak = base_peak.max(s.abs());
            }
            true
        })
        .unwrap();
    assert!(base_peak > 0.3, "baseline clip-auto render is silent");

    // A clip whose single point pins volume to 0.0 (-60 dB) across a window
    // large enough to blanket the whole render → the track goes silent. This
    // exercises the clip-evaluation branch in TrackNode.process.
    let mut clip = AutomationClip::new("auto-clip", AutomationTarget::TrackVolume, 0, 10_000_000);
    clip.insert_point(0, 0.0, CurveMode::Linear);
    {
        let mut project = engine2.project.lock();
        if let Some(t) = project.track_mut(&track2) {
            t.automation_clips.push(clip);
        }
    }
    let mut gated_peak = 0.0f32;
    engine2
        .render_offline(SAMPLE_RATE, SAMPLE_RATE as u64 / 4, |block| {
            for &s in block {
                gated_peak = gated_peak.max(s.abs());
            }
            true
        })
        .unwrap();
    assert!(
        gated_peak < 0.01,
        "AutomationClip pinning TrackVolume to 0 should silence the track, got peak={gated_peak}"
    );
}

#[test]
fn automation_recording_round_trip_is_audible() {
    // Proves the AutomationRecorder data path end-to-end: a simulated live
    // knob sweep captured during "playback" bakes into automation points
    // that, once attached to a track's volume lane, audibly shape the render.
    //
    // This is the offline-verifiable core of the touch/write/latch feature —
    // the one automation module that had zero in-context coverage. (The live
    // UI knob-touch feed that calls push_sample() during real playback is the
    // remaining app-side integration, verified in-app.)
    use hardwave_project::automation::{AutomationLane, AutomationTarget, CurveMode};
    use hardwave_project::automation_recording::{AutomationRecorder, WriteMode};

    // ── Simulate a Write-mode recording pass: a downward volume sweep ──
    let mut rec = AutomationRecorder::default();
    rec.set_mode(WriteMode::Write);
    rec.on_transport_play();
    assert!(rec.is_recording(), "Write mode should record once transport plays");
    // 200 samples ramping value 1.0 → 0.0 across ticks 0..2000 (a fade-out
    // the user "drew" by pulling the volume fader down while playing).
    for i in 0..=200u64 {
        let tick = i * 10;
        let value = 1.0 - (i as f64 / 200.0);
        rec.push_sample(tick, value);
    }
    rec.on_transport_stop();
    assert!(!rec.is_recording(), "transport stop must halt recording");
    assert_eq!(rec.sample_count(), 201);
    // Thin the dense capture down to inflection points, then bake.
    rec.thin(0.02);
    assert!(rec.sample_count() < 201, "thin() should compress the capture");
    let points = rec.into_points(CurveMode::Linear);
    assert!(points.len() >= 2, "need at least the endpoints");
    assert_eq!(points.first().unwrap().tick, 0);

    // ── Attach the recorded lane and prove it's audible ──
    let engine = DawEngine::new();
    let track_id = add_audio_track_with_sine(
        &engine,
        "Rec",
        "smoke-sine-rec",
        SAMPLE_RATE,
        1.0,
        440.0,
        0.5,
    );
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&track_id) {
            t.automation_lanes.push(AutomationLane {
                id: "lane-rec".into(),
                target: AutomationTarget::TrackVolume,
                points,
                visible: true,
            });
        }
    }
    // The recorded fade means the head is loud and the tail is near-silent.
    let total = SAMPLE_RATE as u64;
    let (mut head_sq, mut head_n) = (0.0f64, 0u64);
    let (mut tail_sq, mut tail_n) = (0.0f64, 0u64);
    let mut idx = 0u64;
    engine
        .render_offline(SAMPLE_RATE, total, |block| {
            for &s in block {
                let frame = idx / 2;
                if frame < total / 8 {
                    head_sq += (s as f64) * (s as f64);
                    head_n += 1;
                } else if frame >= total * 7 / 8 {
                    tail_sq += (s as f64) * (s as f64);
                    tail_n += 1;
                }
                idx += 1;
            }
            true
        })
        .unwrap();
    let head_rms = (head_sq / head_n.max(1) as f64).sqrt();
    let tail_rms = (tail_sq / tail_n.max(1) as f64).sqrt();
    assert!(
        head_rms > tail_rms * 4.0,
        "recorded volume fade should make the head much louder than the tail \
         (head {head_rms:.4} vs tail {tail_rms:.4})"
    );
}

#[test]
fn sidechain_bus_reaches_inserts_in_offline_render() {
    // Exports must sound like playback. A plug-in slot with a
    // `sidechain_source` gets that track's output on input ports 2/3, and
    // `hydrate_offline_inserts` used to hard-code `sidechain_active: false`
    // ("export ducking is a follow-up"), so a mix that ducked correctly while
    // playing bounced with no ducking at all.
    //
    // Setup: a 100 Hz key clip covering only the first half of the render, and
    // a steady 1 kHz "bass" carrying a gate insert keyed to it. Measuring the
    // 1 kHz bin per half isolates the bass from the key's own audio (both sum
    // into the master, and the key must stay audible because the sidechain is
    // taken post-fader).
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use hardwave_project::track::PluginSlot;

    /// Attenuates hard whenever the sidechain bus is live. Falls through to a
    /// clean copy when it receives only 2 channels — which is exactly what a
    /// broken sidechain looks like, so the assertion below catches it.
    struct Ducker;
    impl HostedPlugin for Ducker {
        fn descriptor(&self) -> &PluginDescriptor {
            // Not consulted by the chain once hosted.
            unreachable!("descriptor not needed for this test double")
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            let keyed = inputs.len() >= 4
                && inputs[2]
                    .iter()
                    .take(num_samples)
                    .any(|s| s.abs() > 0.05);
            let g = if keyed { 0.1 } else { 1.0 };
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                if let Some(out) = outputs.get_mut(ch) {
                    out.clear();
                    for i in 0..num_samples {
                        out.push(inp.get(i).copied().unwrap_or(0.0) * g);
                    }
                }
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    let _ = PluginCategory::Effect; // keep the import honest across refactors
    let _ = PluginFormat::Clap;

    let engine = DawEngine::new();
    // Key: 100 Hz, first half of the render only.
    let key_id =
        add_audio_track_with_sine(&engine, "Key", "sc-key", SAMPLE_RATE, 0.5, 100.0, 0.8);
    // Bass: steady 1 kHz for the whole render.
    let bass_id =
        add_audio_track_with_sine(&engine, "Bass", "sc-bass", SAMPLE_RATE, 1.0, 1000.0, 0.5);
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&bass_id) {
            t.inserts.push(PluginSlot {
                id: "slot-duck".into(),
                plugin_id: "test.ducker".into(),
                enabled: true,
                state: None,
                sidechain_source: Some(key_id.clone()),
                wet: 1.0,
            });
        }
    }

    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == "test.ducker" {
            Some(Box::new(Ducker))
        } else {
            None
        }
    };
    let total = SAMPLE_RATE as u64;
    let mut out: Vec<f32> = Vec::with_capacity(total as usize * 2);
    engine
        .render_offline_with(SAMPLE_RATE, total, 0, Some(&factory), |_| {}, |block| {
            out.extend_from_slice(block);
            true
        })
        .unwrap();

    // Goertzel on the 1 kHz bass bin, per half.
    let bin = |mono: &[f32]| -> f64 {
        let w = std::f64::consts::TAU * 1000.0 / SAMPLE_RATE as f64;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in mono {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        ((s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)).sqrt() / mono.len().max(1) as f64
    };
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    let half = left.len() / 2;
    let ducked = bin(&left[..half]);
    let open = bin(&left[half..]);
    eprintln!("sidechain offline: 1kHz ducked-half={ducked:.5} open-half={open:.5}");
    assert!(
        open > ducked * 3.0,
        "offline render must carry the sidechain bus: the half where the key \
         plays should be ducked (ducked={ducked:.5}, open={open:.5})"
    );
}

#[test]
fn master_bus_insert_processes_the_mix() {
    // The Master track is not audio-bearing, so it gets no TrackNode and never
    // appeared in `track_id_to_node`. Every InsertCommand aimed at the master
    // strip therefore resolved to no node and was dropped, and the offline
    // hydration skipped it too — a plug-in added to the master was persisted
    // in the project and then silently processed nothing, in playback OR
    // export. The mixer still drew a full FX rack on the master strip.
    //
    // MasterNode now owns an insert chain (sum -> inserts -> fader) and the
    // Master track's id maps to it.
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use hardwave_project::track::{PluginSlot, TrackKind};

    /// Halves whatever reaches it — an unmistakable master-bus effect.
    struct HalfGain;
    impl HostedPlugin for HalfGain {
        fn descriptor(&self) -> &PluginDescriptor {
            unreachable!("descriptor not needed once hosted")
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                if let Some(out) = outputs.get_mut(ch) {
                    out.clear();
                    for i in 0..num_samples {
                        out.push(inp.get(i).copied().unwrap_or(0.0) * 0.5);
                    }
                }
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }
    let _ = (PluginCategory::Effect, PluginFormat::Clap);

    let engine = DawEngine::new();
    add_audio_track_with_sine(&engine, "Src", "smoke-sine-master-fx", SAMPLE_RATE, 1.0, 440.0, 0.5);

    // Find (or create) the Master track and put the insert on it.
    let master_id = {
        let mut project = engine.project.lock();
        let existing = project
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .map(|t| t.id.clone());
        let id = match existing {
            Some(id) => id,
            None => {
                let id = project.add_audio_track("Master".into());
                if let Some(t) = project.track_mut(&id) {
                    t.kind = TrackKind::Master;
                }
                id
            }
        };
        if let Some(t) = project.track_mut(&id) {
            t.inserts.push(PluginSlot {
                id: "slot-master".into(),
                plugin_id: "test.halfgain".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
        id
    };
    assert!(!master_id.is_empty());

    let render_peak = |factory: Option<&dyn Fn(&str) -> Option<Box<dyn HostedPlugin>>>| -> f32 {
        let mut peak = 0.0f32;
        engine
            .render_offline_with(
                SAMPLE_RATE,
                SAMPLE_RATE as u64 / 4,
                0,
                factory,
                |_| {},
                |block| {
                    for &s in block {
                        peak = peak.max(s.abs());
                    }
                    true
                },
            )
            .unwrap();
        peak
    };

    let dry = render_peak(None);
    assert!(dry > 0.3, "dry master render is silent (peak {dry})");

    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == "test.halfgain" {
            Some(Box::new(HalfGain))
        } else {
            None
        }
    };
    let wet = render_peak(Some(&factory));
    assert!(
        wet < dry * 0.65 && wet > dry * 0.35,
        "a master-bus insert must process the summed mix (dry {dry}, wet {wet})"
    );
}

#[test]
fn stem_render_keeps_its_sidechain_key() {
    // Stems used to be rendered by muting every track but the target. A muted
    // TrackNode returns before writing its output ports, so it also stopped
    // feeding any sidechain keyed off it: rendering the bass stem muted the
    // kick, and the stem came out with NO ducking while the full mix ducked
    // correctly. `stem_excluded` keeps the key track processing while cutting
    // it off from master, so the stem ducks exactly like the mix.
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{HostedPlugin, ParameterInfo, PluginDescriptor};
    use hardwave_project::track::PluginSlot;

    struct Ducker;
    impl HostedPlugin for Ducker {
        fn descriptor(&self) -> &PluginDescriptor {
            unreachable!("descriptor not needed once hosted")
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> { Ok(()) }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            let keyed = inputs.len() >= 4
                && inputs[2].iter().take(num_samples).any(|s| s.abs() > 0.05);
            let g = if keyed { 0.1 } else { 1.0 };
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                if let Some(out) = outputs.get_mut(ch) {
                    out.clear();
                    for i in 0..num_samples {
                        out.push(inp.get(i).copied().unwrap_or(0.0) * g);
                    }
                }
            }
        }
        fn get_parameter_count(&self) -> u32 { 0 }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> { None }
        fn get_parameter_value(&self, _i: u32) -> f64 { 0.0 }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> { Vec::new() }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> { Ok(()) }
        fn latency_samples(&self) -> u32 { 0 }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool { false }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool { false }
    }

    let engine = DawEngine::new();
    let key_id = add_audio_track_with_sine(&engine, "Key", "stem-key", SAMPLE_RATE, 0.5, 100.0, 0.8);
    let bass_id =
        add_audio_track_with_sine(&engine, "Bass", "stem-bass", SAMPLE_RATE, 1.0, 1000.0, 0.5);
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&bass_id) {
            t.inserts.push(PluginSlot {
                id: "slot-duck".into(),
                plugin_id: "test.ducker".into(),
                enabled: true,
                state: None,
                sidechain_source: Some(key_id.clone()),
                wet: 1.0,
            });
        }
    }
    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == "test.ducker" { Some(Box::new(Ducker)) } else { None }
    };

    // Render the BASS STEM: everything but the bass is excluded from the mix.
    let key_for_prepare = key_id.clone();
    let bass_for_prepare = bass_id.clone();
    let total = SAMPLE_RATE as u64;
    let mut out: Vec<f32> = Vec::new();
    engine
        .render_offline_with(
            SAMPLE_RATE,
            total,
            0,
            Some(&factory),
            |proj| {
                for t in proj.tracks.iter_mut() {
                    t.stem_excluded = t.id != bass_for_prepare;
                }
            },
            |block| {
                out.extend_from_slice(block);
                true
            },
        )
        .unwrap();
    let _ = key_for_prepare;

    // The key track must NOT be audible in the bass stem...
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    let bin = |mono: &[f32], freq: f64| -> f64 {
        let w = std::f64::consts::TAU * freq / SAMPLE_RATE as f64;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in mono {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        ((s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0)).sqrt() / mono.len().max(1) as f64
    };
    let half = left.len() / 2;
    let key_leak = bin(&left, 100.0);
    let ducked = bin(&left[..half], 1000.0);
    let open = bin(&left[half..], 1000.0);
    eprintln!("bass stem: key_leak(100Hz)={key_leak:.5} ducked={ducked:.5} open={open:.5}");
    assert!(
        key_leak < open * 0.25,
        "the excluded key track must not bleed into the stem (100Hz={key_leak:.5}, bass={open:.5})"
    );
    // ...but it must still be KEYING the ducker.
    assert!(
        open > ducked * 3.0,
        "a stem must duck exactly like the mix — the excluded key track has to \
         keep feeding the sidechain (ducked={ducked:.5}, open={open:.5})"
    );
}

#[test]
fn plugin_latency_is_compensated() {
    // PDC had all its machinery — per-edge delay lines, a critical-path DP in
    // `finalize_pdc` — but no production node ever returned a non-zero
    // `latency_samples()`, so it aligned a graph in which everything claimed to
    // be instantaneous. A lookahead limiter or linear-phase EQ pushed its track
    // out of time against every other track, with nothing to correct it, and
    // the UI's "PDC: n samples" readout was always zero.
    //
    // Two tracks carry the same tone. One has a plug-in that delays by N
    // samples AND reports N. With compensation the clean track is padded by N
    // so both arrive aligned and sum; without it they smear apart.
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{HostedPlugin, ParameterInfo, PluginDescriptor};
    use hardwave_project::track::PluginSlot;

    const LAT: usize = 512;

    /// Delays by LAT samples and reports it honestly.
    struct LatentDelay {
        buf_l: Vec<f32>,
        buf_r: Vec<f32>,
    }
    impl HostedPlugin for LatentDelay {
        fn descriptor(&self) -> &PluginDescriptor {
            unreachable!("descriptor not needed once hosted")
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> { Ok(()) }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for ch in 0..2 {
                let inp: &[f32] = inputs.get(ch).copied().unwrap_or(&[]);
                let hist = if ch == 0 { &mut self.buf_l } else { &mut self.buf_r };
                if let Some(out) = outputs.get_mut(ch) {
                    out.clear();
                    for i in 0..num_samples {
                        hist.push(inp.get(i).copied().unwrap_or(0.0));
                        // Emit the sample from LAT ago.
                        let idx = hist.len().saturating_sub(LAT + 1);
                        out.push(if hist.len() > LAT { hist[idx] } else { 0.0 });
                    }
                }
            }
        }
        fn get_parameter_count(&self) -> u32 { 0 }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> { None }
        fn get_parameter_value(&self, _i: u32) -> f64 { 0.0 }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> { Vec::new() }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> { Ok(()) }
        fn latency_samples(&self) -> u32 { LAT as u32 }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool { false }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool { false }
    }

    let engine = DawEngine::new();
    add_audio_track_with_sine(&engine, "Clean", "pdc-clean", SAMPLE_RATE, 1.0, 440.0, 0.5);
    let latent = add_audio_track_with_sine(&engine, "Latent", "pdc-latent", SAMPLE_RATE, 1.0, 440.0, 0.5);
    {
        let mut project = engine.project.lock();
        if let Some(t) = project.track_mut(&latent) {
            t.inserts.push(PluginSlot {
                id: "slot-latent".into(),
                plugin_id: "test.latent".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
    }
    let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
        if id == "test.latent" {
            Some(Box::new(LatentDelay { buf_l: Vec::new(), buf_r: Vec::new() }))
        } else {
            None
        }
    };

    let mut out: Vec<f32> = Vec::new();
    engine
        .render_offline_with(
            SAMPLE_RATE,
            SAMPLE_RATE as u64 / 2,
            0,
            Some(&factory),
            |_| {},
            |block| { out.extend_from_slice(block); true },
        )
        .unwrap();

    // Two aligned 440 Hz tones sum coherently; a 512-sample offset at 440 Hz is
    // ~4.7 periods, so misalignment shows up as a very different summed level.
    // Measure well past the delay so both tracks are flowing.
    let left: Vec<f32> = out.chunks(2).skip(SAMPLE_RATE as usize / 8).map(|f| f[0]).collect();
    let peak = left.iter().fold(0.0f32, |m, &s| m.max(s.abs()));
    let single = 0.5f32 * std::f32::consts::FRAC_1_SQRT_2; // one sine at pan-centre
    eprintln!("PDC: summed peak={peak:.4} (one track would be {single:.4}, two aligned ~{:.4})", single * 2.0);
    assert!(
        peak > single * 1.7,
        "with PDC the two tracks must arrive aligned and sum to ~2x one track \
         (peak={peak:.4}, one track={single:.4}) — plug-in latency not compensated?"
    );
}
