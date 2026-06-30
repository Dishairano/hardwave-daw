use crate::AppState;
use hardwave_midi::theory::ChordQuality;
use hardwave_midi::{
    arpeggiate, chordify, humanize, note_repeat, snap_to_scale, strum, ArpSettings,
    HumanizeSettings, MidiNote, Scale, StrumDirection,
};
use serde::Serialize;
use tauri::State;

/// Resolve the selected notes of a MIDI clip into owned copies plus the
/// indices they came from. An empty `indices` selects every note.
fn collect_selected(notes: &[MidiNote], indices: &[usize]) -> (Vec<MidiNote>, Vec<usize>) {
    if indices.is_empty() {
        return (notes.to_vec(), (0..notes.len()).collect());
    }
    let mut sel = Vec::new();
    let mut idx = Vec::new();
    for &i in indices {
        if let Some(n) = notes.get(i) {
            sel.push(n.clone());
            idx.push(i);
        }
    }
    (sel, idx)
}

/// Run `f` against the notes of clip `clip_id` on track `track_id` with
/// the standard snapshot → mutate → rebuild discipline. `f` returns the
/// command's success value.
fn with_clip_notes<T>(
    state: &State<AppState>,
    track_id: &str,
    clip_id: &str,
    f: impl FnOnce(&mut Vec<MidiNote>) -> T,
) -> Result<T, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let result = {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;
        let notes = track
            .clips
            .iter_mut()
            .find_map(|clip| match &mut clip.content {
                hardwave_project::clip::ClipContent::Midi(mc) if mc.id == clip_id => {
                    Some(&mut mc.clip.notes)
                }
                _ => None,
            })
            .ok_or_else(|| format!("MIDI clip not found: {}", clip_id))?;
        f(notes)
    };
    engine.rebuild_graph();
    Ok(result)
}

#[derive(Serialize)]
pub struct MidiNoteInfo {
    pub index: usize,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub pitch: u8,
    pub velocity: f32,
    pub channel: u8,
    pub muted: bool,
}

/// Create a new empty MIDI clip on a track.
#[tauri::command]
pub fn create_midi_clip(
    state: State<AppState>,
    track_id: String,
    name: Option<String>,
    position_ticks: Option<u64>,
    length_ticks: Option<u64>,
) -> Result<String, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let clip_id = uuid::Uuid::new_v4().to_string();
    let midi_clip_id = uuid::Uuid::new_v4().to_string();
    let len = length_ticks.unwrap_or(hardwave_midi::PPQ * 4); // Default 1 bar

    let midi_clip = hardwave_midi::MidiClip::new(
        midi_clip_id,
        name.unwrap_or_else(|| "MIDI Clip".into()),
        len,
    );

    let placement = hardwave_project::clip::ClipPlacement {
        content: hardwave_project::clip::ClipContent::Midi(hardwave_project::clip::MidiClipRef {
            id: clip_id.clone(),
            clip: midi_clip,
        }),
        track_id: track_id.clone(),
        position_ticks: position_ticks.unwrap_or(0),
        length_ticks: len,
        lane: 0,
    };

    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.clips.push(placement);
        } else {
            return Err(format!("Track not found: {}", track_id));
        }
    }

    engine.rebuild_graph();
    Ok(clip_id)
}

/// Export a MIDI clip to a Standard MIDI File (.mid) at `path`, so the
/// idea can be carried into another DAW or plug-in. Tempo is taken from
/// the transport and embedded in the file.
#[tauri::command]
pub fn export_clip_midi(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    path: String,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let engine = state.engine.lock();
    let bpm = engine.transport.bpm.load(Ordering::Relaxed);
    let bytes = {
        let project = engine.project.lock();
        let track = project
            .track(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let mc = track
            .clips
            .iter()
            .find_map(|c| match &c.content {
                hardwave_project::clip::ClipContent::Midi(mc) if mc.id == clip_id => Some(mc),
                _ => None,
            })
            .ok_or_else(|| format!("MIDI clip not found: {clip_id}"))?;
        hardwave_midi::write_smf(&mc.clip, bpm)
    };
    std::fs::write(&path, bytes).map_err(|e| format!("write {path}: {e}"))?;
    Ok(())
}

/// Get all notes in a MIDI clip.
#[tauri::command]
pub fn get_midi_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<Vec<MidiNoteInfo>, String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let track = project
        .track(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    for clip in &track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &clip.content {
            if mc.id == clip_id {
                return Ok(mc
                    .clip
                    .notes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| MidiNoteInfo {
                        index: i,
                        start_tick: n.start_tick,
                        duration_ticks: n.duration_ticks,
                        pitch: n.pitch,
                        velocity: n.velocity,
                        channel: n.channel,
                        muted: n.muted,
                    })
                    .collect());
            }
        }
    }

    Err(format!("MIDI clip not found: {}", clip_id))
}

/// Add a note to a MIDI clip.
#[tauri::command]
pub fn add_midi_note(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    pitch: u8,
    start_tick: u64,
    duration_ticks: u64,
    velocity: Option<f32>,
) -> Result<usize, String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    for clip in &mut track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &mut clip.content {
            if mc.id == clip_id {
                let note = hardwave_midi::MidiNote {
                    start_tick,
                    duration_ticks,
                    pitch,
                    velocity: velocity.unwrap_or(0.8),
                    channel: 0,
                    muted: false,
                };
                mc.clip.notes.push(note);
                let idx = mc.clip.notes.len() - 1;
                return Ok(idx);
            }
        }
    }

    Err(format!("MIDI clip not found: {}", clip_id))
}

/// Update a note in a MIDI clip.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn update_midi_note(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_index: usize,
    pitch: Option<u8>,
    start_tick: Option<u64>,
    duration_ticks: Option<u64>,
    velocity: Option<f32>,
    muted: Option<bool>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    for clip in &mut track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &mut clip.content {
            if mc.id == clip_id {
                let note = mc
                    .clip
                    .notes
                    .get_mut(note_index)
                    .ok_or_else(|| format!("Note index out of range: {}", note_index))?;
                if let Some(p) = pitch {
                    note.pitch = p;
                }
                if let Some(s) = start_tick {
                    note.start_tick = s;
                }
                if let Some(d) = duration_ticks {
                    note.duration_ticks = d;
                }
                if let Some(v) = velocity {
                    note.velocity = v;
                }
                if let Some(m) = muted {
                    note.muted = m;
                }
                return Ok(());
            }
        }
    }

    Err(format!("MIDI clip not found: {}", clip_id))
}

/// Delete a note from a MIDI clip.
#[tauri::command]
pub fn delete_midi_note(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_index: usize,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let track = project
        .track_mut(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;

    for clip in &mut track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &mut clip.content {
            if mc.id == clip_id {
                if note_index >= mc.clip.notes.len() {
                    return Err(format!("Note index out of range: {}", note_index));
                }
                mc.clip.notes.remove(note_index);
                return Ok(());
            }
        }
    }

    Err(format!("MIDI clip not found: {}", clip_id))
}

/// Replace the selected notes (a chord) with an arpeggio generated from
/// them. An empty `note_indices` arpeggiates the whole clip. Returns the
/// number of notes produced.
#[tauri::command]
pub fn arpeggiate_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    settings: ArpSettings,
) -> Result<usize, String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (selected, mut indices) = collect_selected(notes, &note_indices);
        let generated = arpeggiate(&selected, &settings);
        // Remove the source notes (descending so indices stay valid), then
        // append the freshly generated arp.
        indices.sort_unstable();
        for i in indices.into_iter().rev() {
            if i < notes.len() {
                notes.remove(i);
            }
        }
        let count = generated.len();
        notes.extend(generated);
        count
    })
}

/// Spread the selected chord's onsets in time (guitar-style strum).
#[tauri::command]
pub fn strum_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    spread_ticks: u64,
    direction: StrumDirection,
) -> Result<(), String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (mut selected, indices) = collect_selected(notes, &note_indices);
        strum(&mut selected, spread_ticks, direction);
        // Write the shifted onsets back to their source notes.
        for (sel, &i) in selected.iter().zip(indices.iter()) {
            if let Some(n) = notes.get_mut(i) {
                n.start_tick = sel.start_tick;
            }
        }
    })
}

/// Snap the selected notes' pitches to the nearest pitch in `scale`
/// rooted at `root` (0 = C … 11 = B).
#[tauri::command]
pub fn snap_clip_notes_to_scale(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    root: u8,
    scale: Scale,
) -> Result<(), String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (mut selected, indices) = collect_selected(notes, &note_indices);
        snap_to_scale(&mut selected, root, scale);
        for (sel, &i) in selected.iter().zip(indices.iter()) {
            if let Some(n) = notes.get_mut(i) {
                n.pitch = sel.pitch;
            }
        }
    })
}

/// Replace the selected notes with evenly-spaced retriggers (note repeat
/// / chop — hi-hat rolls, stutters). An empty selection processes all.
#[tauri::command]
pub fn note_repeat_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    repeats: u32,
    gate: f32,
    decay: f32,
) -> Result<usize, String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (selected, mut indices) = collect_selected(notes, &note_indices);
        let generated = note_repeat(&selected, repeats, gate, decay);
        indices.sort_unstable();
        for i in indices.into_iter().rev() {
            if i < notes.len() {
                notes.remove(i);
            }
        }
        let count = generated.len();
        notes.extend(generated);
        count
    })
}

/// Humanize the selected notes — subtle random start/velocity deviation
/// so a programmed part feels less mechanical. Deterministic per `seed`.
#[tauri::command]
pub fn humanize_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    timing_ticks: u64,
    velocity_amount: f32,
    seed: u64,
) -> Result<(), String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (mut selected, indices) = collect_selected(notes, &note_indices);
        humanize(
            &mut selected,
            &HumanizeSettings {
                timing_ticks,
                velocity_amount,
                seed,
            },
        );
        for (sel, &i) in selected.iter().zip(indices.iter()) {
            if let Some(n) = notes.get_mut(i) {
                n.start_tick = sel.start_tick;
                n.velocity = sel.velocity;
            }
        }
    })
}

/// Map a UI chord-quality string onto the engine's [`ChordQuality`].
fn parse_chord_quality(name: &str) -> Result<ChordQuality, String> {
    match name {
        "major" => Ok(ChordQuality::Major),
        "minor" => Ok(ChordQuality::Minor),
        "dim" | "diminished" => Ok(ChordQuality::Diminished),
        "aug" | "augmented" => Ok(ChordQuality::Augmented),
        "dom7" | "dominant7" => Ok(ChordQuality::Dominant7),
        "maj7" | "major7" => Ok(ChordQuality::Major7),
        "min7" | "minor7" => Ok(ChordQuality::Minor7),
        other => Err(format!("Unknown chord quality: {other}")),
    }
}

/// Stack a chord under each selected note (FL-style chord stamp). The
/// selected notes are replaced by the generated chord voices.
#[tauri::command]
pub fn chordify_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
    quality: String,
) -> Result<usize, String> {
    let quality = parse_chord_quality(&quality)?;
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (selected, mut indices) = collect_selected(notes, &note_indices);
        let generated = chordify(&selected, quality);
        indices.sort_unstable();
        for i in indices.into_iter().rev() {
            if i < notes.len() {
                notes.remove(i);
            }
        }
        let count = generated.len();
        notes.extend(generated);
        count
    })
}
