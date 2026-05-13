//! Functional smoke tests — verify the DAW engine actually does what
//! a user expects, not just that it compiles and unit-tests pass.
//!
//! These tests boot the real `DawEngine`, build a minimal project
//! programmatically, render a short window via `render_offline`, and
//! assert on the output buffer. They are the layer that catches "vapor
//! features" — code paths that compile and have unit tests but never
//! get wired into the real audio graph.
//!
//! ## Status legend (as of 2026-05-04 audit)
//!
//! * **PASS-required** — should always be green; a regression here
//!   means we broke something that used to work.
//! * **KILLER-watch** — currently failing because the underlying
//!   feature is vapor (types and tests exist, no engine wiring). When
//!   one of these flips green, the matching roadmap entry can be
//!   moved from VAPOR / PARTIAL to WORKING.
//!
//! The CI workflow runs these in two jobs: `smoke-required` blocks
//! merges, `smoke-killer-watch` reports without blocking. See
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

// ───────────────────────────────────────────────────────────────────────
// KILLER-watch tests — these CURRENTLY FAIL because the underlying
// feature is vapor. When a fix lands, the test flips green and the
// matching killer is genuinely resolved.
//
// Marked `#[ignore]` so `cargo test` stays green by default; CI runs
// them via `cargo test -- --ignored` in a separate non-blocking job.
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
#[ignore = "killer-watch: track FX inserts (PluginSlot) are not processed by TrackNode; see audit-C"]
fn killer_track_insert_modifies_audio() {
    // KILLER-watch.
    //
    // Place a sine clip on a track, add a track-insert that should change the
    // signal (gain, EQ, anything), render, and assert the output differs from
    // a reference render without the insert. Today: TrackNode never iterates
    // a track's `inserts` / `plugin_slots` — the FX chain is metadata only.
    //
    // This test deliberately does NOT depend on any specific native plugin —
    // it only requires that *something* a plugin slot can express actually
    // changes the audio. If you wire a no-op pass-through and the test still
    // fails, the assertion can be loosened, but the wiring needs to land first.
    let engine = DawEngine::new();
    let track_id =
        add_audio_track_with_sine(&engine, "FX", "smoke-sine-fx", SAMPLE_RATE, 1.0, 440.0, 0.5);

    let baseline = render_and_measure(&engine, SAMPLE_RATE, SAMPLE_RATE as u64 / 4);
    assert!(baseline.peak > 0.05, "baseline render is silent");

    // Today there is no public API to attach a track insert from outside the
    // crate (PluginSlot construction depends on plugin-host internals not
    // surfaced for tests). Even if we could attach one, TrackNode never
    // iterates `track.inserts` per audit-C — so the audio would be unchanged.
    // When the wiring lands, replace this body with: attach a known plugin,
    // render, and assert `peak` or `rms` differs from `baseline` here.
    let _baseline = baseline;
    let _track_id = track_id;
    panic!(
        "killer-watch: no public Track API exposes FX insert wiring for end-to-end testing \
         (see audit-C-p5p6.md § FX Insert Slots — TrackNode does not process track.inserts)"
    );
}

#[test]
#[ignore = "killer-watch: recording.rs has zero callers; record button has no handler; see audit-D"]
fn killer_recording_writes_clip() {
    // KILLER-watch.
    //
    // The `recording.rs` module in hardwave-dsp has 734 lines of complete
    // ring-buffer + WAV-writer + comp-takes / punch / loop logic but no
    // caller anywhere in the engine or commands layer. There is no public
    // `engine.start_recording` / `engine.stop_recording`.
    //
    // This test fails by simply asserting that such an API exists. When the
    // wiring lands, replace the body with: arm a track, feed input samples,
    // stop, and assert a clip was created.
    let _engine = DawEngine::new();
    panic!(
        "killer-watch: no public DawEngine recording API exists \
         (recording.rs has zero callers — see project_daw_real_status.md)"
    );
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
#[ignore = "killer-watch: automation has no engine callers; ParameterContextMenu shows 'soon' placeholder"]
fn killer_automation_changes_parameter() {
    // KILLER-watch.
    //
    // automation.rs + automation_clip.rs + automation_recording.rs + lfo.rs
    // contain ~1400 lines of types with zero callers. Project struct has no
    // automation_clips field. UI shows a literal disabled "Create Automation
    // (soon)" placeholder.
    //
    // This test asserts that automation actually drives a parameter at render
    // time. Today: no API exists to define an automation point + run.
    let _engine = DawEngine::new();
    panic!(
        "killer-watch: no public DawEngine automation API exists \
         (automation.rs et al have zero callers — see project_daw_real_status.md)"
    );
}
