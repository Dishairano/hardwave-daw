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
use std::sync::atomic::Ordering;
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
            MidiEvent::NoteExpression {
                channel,
                note,
                value,
                ..
            } => Self {
                sample_pos,
                kind: "note_expression",
                channel,
                note: Some(note),
                velocity: None,
                cc: None,
                value: Some(value),
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

/// Blend-record (FL Ctrl+B) merge target: the last MIDI clip on the
/// track that overlaps the recorded window AND starts at-or-before it
/// (merging into a later-starting clip would need negative note ticks /
/// left-extension — those recordings fall through to a new clip).
/// Returns the merged clip's id, or None when no suitable clip exists.
fn merge_notes_into_overlapping_clip(
    track: &mut hardwave_project::track::Track,
    position_ticks: u64,
    length_ticks: u64,
    notes: &[hardwave_midi::MidiNote],
    controls: &[hardwave_midi::MidiControlPoint],
) -> Option<String> {
    use hardwave_project::clip::ClipContent;

    let rec_end = position_ticks + length_ticks;
    let target = track
        .clips
        .iter_mut()
        .filter(|p| matches!(p.content, ClipContent::Midi(_)))
        .filter(|p| p.position_ticks <= position_ticks)
        .filter(|p| p.position_ticks + p.length_ticks > position_ticks)
        .max_by_key(|p| p.position_ticks)?;

    let ClipContent::Midi(ref mut midi_ref) = target.content else {
        return None;
    };
    // Note ticks are clip-relative; shift by where the recording sits
    // inside the existing clip.
    let offset = position_ticks - target.position_ticks;
    for n in notes {
        let mut merged = n.clone();
        merged.start_tick += offset;
        midi_ref.clip.notes.push(merged);
    }
    for c in controls {
        let mut merged = *c;
        merged.tick += offset;
        midi_ref.clip.controls.push(merged);
    }
    midi_ref.clip.controls.sort_by_key(|c| c.tick);
    // Recording may run past the clip's end — grow, never shrink.
    let needed = rec_end.saturating_sub(target.position_ticks);
    if needed > target.length_ticks {
        target.length_ticks = needed;
        midi_ref.clip.length_ticks = midi_ref.clip.length_ticks.max(needed);
    }
    Some(midi_ref.id.clone())
}

/// Replace-record: remove the notes of the track's MIDI clips that start
/// under the take, and the controller moves of the kinds the take recorded,
/// so the take plays instead of on top of them. A take used to be laid
/// over the old part as a second clip and both played. Muted clips, and
/// clips this same take made (`skip`, earlier loop passes), are left alone.
/// Only a clip's own length is looked at: the repeats of a looped clip are
/// the same notes, and taking them out of one repeat would take them out
/// of all. Returns how many notes went.
fn clear_notes_under_take(
    track: &mut hardwave_project::track::Track,
    position_ticks: u64,
    length_ticks: u64,
    controls: &[hardwave_midi::MidiControlPoint],
    skip: &[String],
) -> usize {
    use hardwave_project::clip::ClipContent;

    let take_end = position_ticks + length_ticks;
    let mut removed = 0;
    for placement in track.clips.iter_mut() {
        let start = placement.position_ticks;
        let end = start + placement.length_ticks;
        if end <= position_ticks || start >= take_end {
            continue;
        }
        let ClipContent::Midi(ref mut midi_ref) = placement.content else {
            continue;
        };
        if midi_ref.clip.muted || skip.contains(&midi_ref.id) {
            continue;
        }
        // The take's window in the clip's own ticks.
        let from = position_ticks.saturating_sub(start);
        let to = take_end - start;
        let before = midi_ref.clip.notes.len();
        midi_ref
            .clip
            .notes
            .retain(|n| n.start_tick < from || n.start_tick >= to);
        removed += before - midi_ref.clip.notes.len();
        midi_ref.clip.controls.retain(|c| {
            !(c.tick >= from
                && c.tick < to
                && controls
                    .iter()
                    .any(|t| t.kind == c.kind && t.channel == c.channel))
        });
    }
    removed
}

/// A pause this long (seconds) ends a phrase: capture takes what was played
/// after the last one.
const PHRASE_GAP_SECONDS: f64 = 10.0;

/// Turn what was just played into a clip, without having pressed record.
///
/// The DAW keeps recent MIDI input in a rolling buffer whether or not it is
/// recording. This takes the last phrase from it (what was played since the
/// last pause of ten seconds or more) and makes it a clip on `track_id`.
/// Played while the song ran, the notes land where the playhead was. Played
/// with the song stopped, every note has the same playhead position, so
/// they are laid out by the engine's own clock from where the playhead
/// stands, at the song's tempo there; that used to be refused.
#[tauri::command]
pub fn capture_recent_midi(
    state: State<AppState>,
    track_id: String,
) -> Result<Vec<String>, String> {
    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;
    if sample_rate <= 0.0 {
        return Err("invalid sample rate for tick conversion".into());
    }
    let ring_arc = std::sync::Arc::clone(&engine.midi_capture_ring);
    let captured = {
        let Some(ring) = ring_arc.try_lock() else {
            return Err("the capture buffer is busy; try again".into());
        };
        ring.captured_in_order()
    };
    let phrase = last_phrase(&captured, (PHRASE_GAP_SECONDS * sample_rate) as u64);
    if phrase.is_empty() {
        return Err("nothing has been played recently".into());
    }
    let take = phrase_take(phrase);
    let created = commit_takes(
        &engine,
        &track_id,
        vec![take],
        sample_rate,
        None,
        false,
        false,
    )?;
    if created.is_empty() {
        return Err("nothing that makes a note or a controller move was played".into());
    }
    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(created)
}

/// A phrase as a take. Played while the song ran, its events sit where the
/// playhead was. Played with the song stopped, they all have the same
/// playhead position, so they are laid out by the engine's clock from there.
fn phrase_take(phrase: &[hardwave_midi::CapturedMidi]) -> Take {
    let (first, last) = (&phrase[0], &phrase[phrase.len() - 1]);
    let events: Vec<_> = if last.sample_pos > first.sample_pos {
        phrase.iter().map(|c| (c.sample_pos, c.event)).collect()
    } else {
        phrase
            .iter()
            .map(|c| {
                (
                    first.sample_pos + c.clock.saturating_sub(first.clock),
                    c.event,
                )
            })
            .collect()
    };
    let to = events
        .last()
        .map_or(first.sample_pos, |(p, _)| *p)
        .saturating_add(1);
    Take {
        from: first.sample_pos,
        to,
        events,
        name: "Captured".into(),
    }
}

/// The events since the last pause of `gap` frames or more on the engine's
/// clock, oldest first.
fn last_phrase(
    captured: &[hardwave_midi::CapturedMidi],
    gap: u64,
) -> &[hardwave_midi::CapturedMidi] {
    let mut start = captured.len();
    while start > 1
        && captured[start - 1]
            .clock
            .saturating_sub(captured[start - 2].clock)
            < gap
    {
        start -= 1;
    }
    &captured[start.saturating_sub(1).min(captured.len())..]
}

/// Commit a slice of the rolling capture into a MIDI clip placed on the/// Commit a slice of the rolling capture into a MIDI clip placed on the
/// target track at the given timeline position. Implements the
/// "arm + record + play" flow on top of the always-on capture ring,
/// reusing `hardwave_midi::MidiRecorder` to pair NoteOn / NoteOff
/// events into `MidiNote`s.
///
/// `start_sample` / `end_sample` are absolute transport samples
/// (typically `record_start_position` … `current_position`).
/// `quantize_ticks` is optional input quantize (e.g. 240 = 1/16 note
/// at 960 PPQ).
///
/// `blend` (FL Ctrl+B): when true, recorded notes merge into the last
/// suitable overlapping MIDI clip on the track instead of stacking a
/// new clip; falls back to a new clip when nothing overlaps.
///
/// Returns the (new or merged) clip id on success, or an error string
/// when no events fell in the recording window.
#[tauri::command]
pub fn commit_recording_to_midi_clip(
    state: State<AppState>,
    track_id: String,
    start_sample: u64,
    end_sample: u64,
    quantize_ticks: Option<u64>,
    blend: Option<bool>,
    replace: Option<bool>,
) -> Result<Vec<String>, String> {
    use hardwave_midi::MidiEvent;

    let engine = state.engine.lock();
    let sample_rate = engine.current_sample_rate() as f64;
    if sample_rate <= 0.0 {
        return Err("invalid sample rate for tick conversion".into());
    }

    // The window a pass covers. With the loop on, the playhead wraps, so the
    // stop position is behind the start: the command used to reject that
    // outright and the UI only logged it, which lost the whole take. The
    // loop bounds are used instead, and each pass is committed separately.
    let looping = engine.transport.looping.load(Ordering::Relaxed);
    let loop_start = engine.transport.loop_start.load(Ordering::Relaxed);
    let loop_end = engine.transport.loop_end.load(Ordering::Relaxed);
    let (punched, punch_in, punch_out) = engine.punch_samples();

    let captured = engine.midi_capture_ring.lock().captured_in_order();
    let passes = engine.record_passes();

    // One window per pass: the first from where record was pressed, the rest
    // from the loop start, each ending at the loop end (or where recording
    // stopped on the last pass), narrowed by the punch window.
    let window_for = |pass: u32| -> (u64, u64) {
        let is_last = pass + 1 >= passes;
        let mut from = if pass == 0 { start_sample } else { loop_start };
        let mut to = if is_last && !(looping && end_sample <= start_sample) {
            end_sample.max(from)
        } else if looping && loop_end > loop_start {
            loop_end
        } else {
            end_sample.max(from)
        };
        if punched {
            from = from.max(punch_in);
            to = to.min(punch_out);
        }
        (from, to.max(from))
    };

    let takes: Vec<Take> = (0..passes)
        .filter_map(|pass| {
            let (from, to) = window_for(pass);
            let events: Vec<(u64, MidiEvent)> = captured
                .iter()
                .filter(|c| c.pass == pass && c.sample_pos >= from && c.sample_pos < to)
                .map(|c| (c.sample_pos, c.event))
                .collect();
            (to > from && !events.is_empty()).then(|| Take {
                from,
                to,
                events,
                name: if passes > 1 {
                    format!("Recording {}", pass + 1)
                } else {
                    "Recording".to_string()
                },
            })
        })
        .collect();
    let created = commit_takes(
        &engine,
        &track_id,
        takes,
        sample_rate,
        quantize_ticks,
        blend.unwrap_or(false),
        replace.unwrap_or(false),
    )?;

    if created.is_empty() {
        return Err("nothing was captured in the recorded range".into());
    }

    drop(engine);
    state.engine.lock().rebuild_graph();
    Ok(created)
}

/// A stretch of played MIDI to turn into a clip: the timeline window it
/// covers (playhead samples), its events at their playhead positions, and
/// the clip's name.
struct Take {
    from: u64,
    to: u64,
    events: Vec<(u64, hardwave_midi::MidiEvent)>,
    name: String,
}

/// Turn takes into clips on `track_id`: notes paired through the recorder
/// (quantized when asked), controller moves kept as played, merged into an
/// overlapping clip with `blend`, or played alone over what they cover with
/// `replace`. Returns the clips made or merged into.
fn commit_takes(
    engine: &hardwave_engine::DawEngine,
    track_id: &str,
    takes: Vec<Take>,
    sample_rate: f64,
    quantize_ticks: Option<u64>,
    blend: bool,
    replace: bool,
) -> Result<Vec<String>, String> {
    use hardwave_midi::{MidiClip, MidiControlKind, MidiControlPoint, MidiEvent, MidiRecorder};
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};

    engine.snapshot_before_mutation();

    let mut created = Vec::new();
    for Take {
        from,
        to,
        events,
        name,
    } in takes
    {
        // Ticks through the project's tempo map, not one tempo: a song with a
        // tempo change used to record its notes in the wrong place.
        let (position_ticks, length_ticks, tick_of) = {
            let project = engine.project.lock();
            let map = &project.tempo_map;
            let position = map.samples_to_tick(from, sample_rate);
            let end = map.samples_to_tick(to, sample_rate);
            let of = |pos: u64| {
                map.samples_to_tick(pos, sample_rate)
                    .saturating_sub(position)
            };
            (
                position,
                end.saturating_sub(position).max(1),
                events
                    .iter()
                    .map(|(p, ev)| (of(*p), *ev))
                    .collect::<Vec<_>>(),
            )
        };

        let mut recorder = MidiRecorder::default();
        if let Some(q) = quantize_ticks {
            recorder.set_quantize(Some(q));
        }
        recorder.start();
        // Controller movements are kept as played: quantizing a mod-wheel
        // sweep to the grid would turn it into steps.
        let mut controls: Vec<MidiControlPoint> = Vec::new();
        for (tick, ev) in tick_of {
            match ev {
                MidiEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                    ..
                } => recorder.note_on(tick, note, velocity, channel),
                MidiEvent::NoteOff { note, channel, .. } => recorder.note_off(tick, note, channel),
                MidiEvent::ControlChange {
                    channel, cc, value, ..
                } => controls.push(MidiControlPoint {
                    tick,
                    channel,
                    kind: MidiControlKind::Cc(cc),
                    value: value.clamp(0.0, 1.0),
                }),
                MidiEvent::PitchBend { channel, value, .. } => controls.push(MidiControlPoint {
                    tick,
                    channel,
                    kind: MidiControlKind::PitchBend,
                    value: value.clamp(-1.0, 1.0),
                }),
                MidiEvent::ChannelPressure {
                    channel, pressure, ..
                } => controls.push(MidiControlPoint {
                    tick,
                    channel,
                    kind: MidiControlKind::ChannelPressure,
                    value: pressure.clamp(0.0, 1.0),
                }),
                // Per-note aftertouch has nowhere to live in a clip yet,
                // and note expression is made by the host, not played in.
                MidiEvent::Aftertouch { .. } | MidiEvent::NoteExpression { .. } => {}
            }
        }
        recorder.stop();
        let notes = recorder.take_notes();
        if notes.is_empty() && controls.is_empty() {
            continue;
        }

        // Replace-record: what the take covers is played by the take alone.
        if replace {
            let mut project = engine.project.lock();
            let Some(track) = project.track_mut(track_id) else {
                return Err(format!("track {track_id} not found"));
            };
            clear_notes_under_take(track, position_ticks, length_ticks, &controls, &created);
        }

        // Blend-record merges into an overlapping clip when one exists.
        if blend && !replace {
            let merged = {
                let mut project = engine.project.lock();
                let Some(track) = project.track_mut(track_id) else {
                    return Err(format!("track {track_id} not found"));
                };
                merge_notes_into_overlapping_clip(
                    track,
                    position_ticks,
                    length_ticks,
                    &notes,
                    &controls,
                )
            };
            if let Some(clip_id) = merged {
                created.push(clip_id);
                continue;
            }
        }

        let clip_id = uuid::Uuid::new_v4().to_string();
        let mut clip = MidiClip::new(clip_id.clone(), name, length_ticks);
        clip.notes = notes;
        clip.controls = controls;
        {
            let mut project = engine.project.lock();
            let Some(track) = project.track_mut(track_id) else {
                return Err(format!("track {track_id} not found"));
            };
            track.clips.push(ClipPlacement {
                content: ClipContent::Midi(MidiClipRef {
                    id: clip_id.clone(),
                    clip,
                }),
                track_id: track_id.to_string(),
                position_ticks,
                length_ticks,
                lane: 0,
                color: None,
            });
        }
        created.push(clip_id);
    }

    Ok(created)
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

#[cfg(test)]
mod blend_tests {
    use super::merge_notes_into_overlapping_clip;
    use hardwave_midi::{MidiClip, MidiNote};
    use hardwave_project::clip::{ClipContent, ClipPlacement, MidiClipRef};
    use hardwave_project::track::Track;

    fn note(start_tick: u64) -> MidiNote {
        MidiNote {
            start_tick,
            duration_ticks: 240,
            pitch: 60,
            velocity: 0.8,
            channel: 0,
            muted: false,
            ..Default::default()
        }
    }

    fn track_with_midi_clip(clip_pos: u64, clip_len: u64) -> Track {
        let mut t = Track::new_midi("t1".into(), "MIDI".into());
        let mut clip = MidiClip::new("existing".into(), "Take 1".into(), clip_len);
        clip.notes.push(note(0));
        t.clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "existing".into(),
                clip,
            }),
            track_id: "t1".into(),
            position_ticks: clip_pos,
            length_ticks: clip_len,
            lane: 0,
            color: None,
        });
        t
    }

    #[test]
    fn merges_into_overlapping_clip_with_tick_offset() {
        // Existing clip at tick 1000, len 4000. Recording at 2000 → the
        // merged note must land at clip-relative tick 1000 + its own 480.
        let mut t = track_with_midi_clip(1000, 4000);
        let merged = merge_notes_into_overlapping_clip(&mut t, 2000, 1000, &[note(480)], &[]);
        assert_eq!(merged.as_deref(), Some("existing"));
        let ClipContent::Midi(ref m) = t.clips[0].content else {
            panic!()
        };
        assert_eq!(m.clip.notes.len(), 2);
        assert_eq!(
            m.clip.notes[1].start_tick, 1480,
            "offset by window - clip position"
        );
        assert_eq!(t.clips.len(), 1, "no new clip stacked");
    }

    #[test]
    fn grows_clip_when_recording_runs_past_the_end() {
        let mut t = track_with_midi_clip(0, 1000);
        // Recording overlaps the tail and extends 2000 ticks beyond it.
        let merged = merge_notes_into_overlapping_clip(&mut t, 500, 2500, &[note(0)], &[]);
        assert!(merged.is_some());
        assert_eq!(
            t.clips[0].length_ticks, 3000,
            "placement grows to cover the take"
        );
        let ClipContent::Midi(ref m) = t.clips[0].content else {
            panic!()
        };
        assert_eq!(m.clip.length_ticks, 3000, "clip length grows in lockstep");
    }

    #[test]
    fn no_merge_when_nothing_overlaps_or_clip_starts_later() {
        // Recording entirely AFTER the clip → no merge.
        let mut t = track_with_midi_clip(0, 1000);
        assert!(merge_notes_into_overlapping_clip(&mut t, 5000, 1000, &[note(0)], &[]).is_none());
        // Clip starts AFTER the recording window → no merge (would need
        // negative note ticks).
        let mut t2 = track_with_midi_clip(3000, 1000);
        assert!(merge_notes_into_overlapping_clip(&mut t2, 2500, 400, &[note(0)], &[]).is_none());
    }
    #[test]
    fn blend_recording_merges_controller_moves_too() {
        use hardwave_midi::{MidiControlKind, MidiControlPoint};
        let mut t = track_with_midi_clip(1000, 2000);
        let controls = vec![MidiControlPoint {
            tick: 100,
            channel: 0,
            kind: MidiControlKind::Cc(1),
            value: 0.5,
        }];
        let merged = merge_notes_into_overlapping_clip(&mut t, 2000, 1000, &[note(0)], &controls);
        assert!(merged.is_some());
        let ClipContent::Midi(ref m) = t.clips[0].content else {
            panic!()
        };
        assert_eq!(m.clip.controls.len(), 1);
        assert_eq!(
            m.clip.controls[0].tick, 1100,
            "the move shifts by where the take sits inside the clip"
        );
    }

    /// Replace-record: the old notes under the take go, the ones before
    /// and after it stay, and the take's own clip is not touched.
    #[test]
    fn replace_takes_out_only_the_notes_under_the_take() {
        let mut t = track_with_midi_clip(0, 4000);
        let ClipContent::Midi(ref mut m) = t.clips[0].content else {
            unreachable!()
        };
        m.clip.notes = vec![note(0), note(960), note(1920), note(3000)];
        // The take covers ticks 900 to 2400 of the song; the clip starts at 0.
        let gone = super::clear_notes_under_take(&mut t, 900, 1500, &[], &[]);
        assert_eq!(gone, 2);
        let ClipContent::Midi(ref m) = t.clips[0].content else {
            unreachable!()
        };
        let left: Vec<u64> = m.clip.notes.iter().map(|n| n.start_tick).collect();
        assert_eq!(left, vec![0, 3000]);
        // A clip the take made itself is skipped.
        let id = m.id.clone();
        let ClipContent::Midi(ref mut m) = t.clips[0].content else {
            unreachable!()
        };
        m.clip.notes.push(note(1000));
        assert_eq!(
            super::clear_notes_under_take(&mut t, 900, 1500, &[], &[id]),
            0
        );
    }
}

#[cfg(test)]
mod capture_tests {
    use super::{last_phrase, phrase_take};
    use hardwave_midi::{CapturedMidi, MidiEvent};

    fn at(sample_pos: u64, clock: u64, on: bool) -> CapturedMidi {
        CapturedMidi {
            sample_pos,
            pass: 0,
            clock,
            event: if on {
                MidiEvent::NoteOn {
                    timing: 0,
                    channel: 0,
                    note: 60,
                    velocity: 0.8,
                }
            } else {
                MidiEvent::NoteOff {
                    timing: 0,
                    channel: 0,
                    note: 60,
                    velocity: 0.0,
                }
            },
        }
    }

    #[test]
    fn the_last_phrase_starts_after_the_last_long_pause() {
        let gap = 480_000;
        let played = [
            at(0, 0, true),
            at(0, 1_000, false),
            at(0, 900_000, true),
            at(0, 901_000, false),
        ];
        assert_eq!(last_phrase(&played, gap).len(), 2);
        assert_eq!(last_phrase(&played[..2], gap).len(), 2);
        assert!(last_phrase(&[], gap).is_empty());
    }

    /// With the song stopped every event has the same playhead position;
    /// the take lays them out by the clock from there.
    #[test]
    fn played_while_stopped_is_laid_out_by_the_clock() {
        let phrase = [
            at(96_000, 5_000, true),
            at(96_000, 29_000, false),
            at(96_000, 53_000, true),
        ];
        let take = phrase_take(&phrase);
        let positions: Vec<u64> = take.events.iter().map(|(p, _)| *p).collect();
        assert_eq!(positions, [96_000, 120_000, 144_000]);
        assert_eq!((take.from, take.to), (96_000, 144_001));
    }

    /// Played while the song ran, the playhead positions are used as they are.
    #[test]
    fn played_while_running_keeps_the_playhead_positions() {
        let phrase = [at(10_000, 70_000, true), at(34_000, 94_000, false)];
        let take = phrase_take(&phrase);
        let positions: Vec<u64> = take.events.iter().map(|(p, _)| *p).collect();
        assert_eq!(positions, [10_000, 34_000]);
    }
}
