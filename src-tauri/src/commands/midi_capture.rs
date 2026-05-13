//! Rolling MIDI capture commands. Exposes the engine's always-on
//! capture ring (`MidiCaptureRing`) to the UI so the user can dump
//! the last N seconds of input into a new pattern without ever having
//! armed a track.
//!
//! The ring lives on the engine, the audio thread pushes per-block,
//! and these commands read snapshots on the UI thread under a
//! parking_lot Mutex try_lock.

use crate::AppState;
use serde::Serialize;
use tauri::State;

/// Serialised entry from the rolling ring. `sample_pos` is the absolute
/// transport sample at which the event was observed by the audio
/// thread. The UI converts to ticks using the project's tempo map.
#[derive(Serialize)]
pub struct CapturedMidiEntry {
    pub sample_pos: u64,
    pub kind: &'static str,
    pub channel: u8,
    pub note: Option<u8>,
    pub velocity: Option<f32>,
    pub cc: Option<u8>,
    pub value: Option<f32>,
}

impl CapturedMidiEntry {
    fn from(sample_pos: u64, ev: hardwave_midi::MidiEvent) -> Self {
        use hardwave_midi::MidiEvent;
        match ev {
            MidiEvent::NoteOn {
                channel,
                note,
                velocity,
                ..
            } => Self {
                sample_pos,
                kind: "note_on",
                channel,
                note: Some(note),
                velocity: Some(velocity),
                cc: None,
                value: None,
            },
            MidiEvent::NoteOff {
                channel,
                note,
                velocity,
                ..
            } => Self {
                sample_pos,
                kind: "note_off",
                channel,
                note: Some(note),
                velocity: Some(velocity),
                cc: None,
                value: None,
            },
            MidiEvent::ControlChange {
                channel, cc, value, ..
            } => Self {
                sample_pos,
                kind: "control_change",
                channel,
                note: None,
                velocity: None,
                cc: Some(cc),
                value: Some(value),
            },
            MidiEvent::PitchBend { channel, value, .. } => Self {
                sample_pos,
                kind: "pitch_bend",
                channel,
                note: None,
                velocity: None,
                cc: None,
                value: Some(value),
            },
            MidiEvent::Aftertouch {
                channel,
                note,
                pressure,
                ..
            } => Self {
                sample_pos,
                kind: "aftertouch",
                channel,
                note: Some(note),
                velocity: Some(pressure),
                cc: None,
                value: None,
            },
            MidiEvent::ChannelPressure {
                channel, pressure, ..
            } => Self {
                sample_pos,
                kind: "channel_pressure",
                channel,
                note: None,
                velocity: Some(pressure),
                cc: None,
                value: None,
            },
        }
    }
}

/// Return every event currently in the rolling capture ring, oldest
/// first. The UI filters by `sample_pos >= now - window_samples` to
/// implement "dump last 30 seconds" / "dump everything since I sat
/// down". Returns an empty Vec if the ring is busy (audio thread
/// pushing) — the UI can retry immediately.
#[tauri::command]
pub fn dump_midi_capture(state: State<AppState>) -> Vec<CapturedMidiEntry> {
    // Same Arc-clone-then-drop-engine pattern as clear_midi_capture to
    // keep the MutexGuard's borrow disjoint from the engine binding.
    let ring_arc = {
        let engine = state.engine.lock();
        std::sync::Arc::clone(&engine.midi_capture_ring)
    };
    let Some(ring) = ring_arc.try_lock() else {
        return Vec::new();
    };
    ring.entries_in_order()
        .into_iter()
        .map(|(pos, ev)| CapturedMidiEntry::from(pos, ev))
        .collect()
}

/// Commit a slice of the rolling capture into a new MIDI clip placed
/// on the target track at the given timeline position. Implements the
/// "arm + record + play" flow on top of the always-on capture ring,
/// reusing `hardwave_midi::MidiRecorder` to pair NoteOn / NoteOff
/// events into `MidiNote`s.
///
/// `start_sample` / `end_sample` are absolute transport samples
/// (typically `record_start_position` … `current_position`).
/// `quantize_ticks` is optional input quantize (e.g. 240 = 1/16 note
/// at 960 PPQ).
///
/// Returns the new clip id on success, or an error string when no
/// events fell in the recording window.
#[tauri::command]
pub fn commit_recording_to_midi_clip(
    state: State<AppState>,
    track_id: String,
    start_sample: u64,
    end_sample: u64,
    quantize_ticks: Option<u64>,
) -> Result<String, String> {
    use hardwave_midi::{MidiClip, MidiEvent, MidiRecorder};
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};

    if end_sample <= start_sample {
        return Err("end_sample must be > start_sample".into());
    }

    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;
    let bpm = engine
        .transport
        .bpm
        .load(std::sync::atomic::Ordering::Relaxed);
    if bpm <= 0.0 || sample_rate <= 0.0 {
        return Err("invalid tempo / sample rate for tick conversion".into());
    }
    // 960 PPQ matches `hardwave_midi::PPQ` and is the project default.
    let samples_per_tick = sample_rate * 60.0 / (bpm * 960.0);
    if samples_per_tick <= 0.0 {
        return Err("computed samples_per_tick is non-positive".into());
    }

    // Snapshot entries in range under the ring lock. try_lock would
    // race against the audio thread's push; the command path can wait.
    let entries: Vec<(u64, MidiEvent)> = engine
        .midi_capture_ring
        .lock()
        .entries_in_order()
        .into_iter()
        .filter(|(pos, _)| *pos >= start_sample && *pos < end_sample)
        .collect();

    let mut recorder = MidiRecorder::default();
    if let Some(q) = quantize_ticks {
        recorder.set_quantize(Some(q));
    }
    recorder.start();
    for (sample_pos, ev) in entries {
        let rel = sample_pos.saturating_sub(start_sample);
        let tick = (rel as f64 / samples_per_tick).round() as u64;
        match ev {
            MidiEvent::NoteOn {
                note,
                velocity,
                channel,
                ..
            } => recorder.note_on(tick, note, velocity, channel),
            MidiEvent::NoteOff { note, channel, .. } => recorder.note_off(tick, note, channel),
            _ => {} // CC / pitch bend skipped — captured separately by automation
        }
    }
    recorder.stop();

    let notes = recorder.take_notes();
    if notes.is_empty() {
        return Err("no MIDI notes captured in the recording window".into());
    }

    let length_ticks = ((end_sample - start_sample) as f64 / samples_per_tick).ceil() as u64;
    let position_ticks = (start_sample as f64 / samples_per_tick).round() as u64;
    let clip_id = uuid::Uuid::new_v4().to_string();
    let mut clip = MidiClip::new(clip_id.clone(), "Recording".into(), length_ticks);
    clip.notes = notes;

    {
        let mut project = engine.project.lock();
        let Some(track) = project.track_mut(&track_id) else {
            return Err(format!("track {track_id} not found"));
        };
        track.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: clip_id.clone(),
                clip,
            }),
            track_id: track_id.clone(),
            position_ticks,
            length_ticks,
            lane: 0,
        });
    }
    drop(engine);

    // Rebuild the audio graph so the new clip is picked up on the
    // next audio block. Caller doesn't have to do this manually.
    state.engine.lock().rebuild_graph();

    Ok(clip_id)
}

/// Wipe the capture ring. Used by the UI on project switch so the
/// next "dump last N seconds" doesn't smuggle events from the
/// previously-loaded session into the new one.
#[tauri::command]
pub fn clear_midi_capture(state: State<AppState>) {
    let ring_arc = {
        let engine = state.engine.lock();
        std::sync::Arc::clone(&engine.midi_capture_ring)
    };
    // Bind the Option<MutexGuard> to a NAMED local. With a let-binding
    // it's a proper variable rather than a tail-expression temporary,
    // so drop order is declaration-reverse: opt drops before ring_arc.
    let opt = ring_arc.try_lock();
    if let Some(mut ring) = opt {
        ring.clear();
    }
}
