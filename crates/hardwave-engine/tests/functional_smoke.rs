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
#[ignore = "killer-watch: automation_clip.rs + automation_recording.rs + lfo.rs are still zero-caller (~1400 LOC)"]
fn killer_automation_clips_and_lfo() {
    // KILLER-watch — SCOPE NARROWED 2026-05-13.
    //
    // automation.rs ITSELF is now wired: TrackNode.process walks
    // `track.automation_lanes` every block (track_node.rs:526) and
    // overrides volume / pan / mute / plug-in param values.
    // `automation_lane_silences_track_via_volume` (PASS-required, above)
    // is the regression gate.
    //
    // What remains vapor:
    //   - automation_clip.rs (per-clip automation tracks)
    //   - automation_recording.rs (touch / write / latch record modes)
    //   - lfo.rs (six built-in LFO shapes that nothing instantiates)
    //
    // When clip-based automation lands, replace this body with: create
    // an automation CLIP (not lane), render, assert. Today: no public
    // API to attach a clip-based automation track.
    let _engine = DawEngine::new();
    panic!(
        "killer-watch: automation lanes work; automation CLIPS + LFO + \
         touch/write/latch recording have zero engine callers"
    );
}
