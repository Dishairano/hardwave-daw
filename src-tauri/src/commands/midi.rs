use crate::AppState;
use hardwave_midi::theory::{Chord, ChordQuality};
use hardwave_midi::{
    arpeggiate, chordify, generate_melody_line, generate_progression, humanize, legato,
    note_repeat, snap_to_scale, strum, ArpSettings, HumanizeSettings, MidiNote, Scale,
    StrumDirection,
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
    /// Per-note pan, -1 left to 1 right.
    pub pan: f32,
    /// Per-note detune in cents.
    pub fine_cents: f32,
    /// How hard the key was let go, 0 to 1.
    pub release_velocity: f32,
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

/// One controller lane's points, as the piano roll draws them.
#[derive(Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlPointInfo {
    pub tick: u64,
    /// 0..=1 for a control change and channel pressure, -1..=1 for bend.
    pub value: f32,
}

fn lane_kind(kind: &str, cc: Option<u8>) -> Result<hardwave_midi::MidiControlKind, String> {
    match kind {
        "cc" => Ok(hardwave_midi::MidiControlKind::Cc(
            cc.ok_or("a cc lane needs a cc number")?,
        )),
        "pitchBend" => Ok(hardwave_midi::MidiControlKind::PitchBend),
        "channelPressure" => Ok(hardwave_midi::MidiControlKind::ChannelPressure),
        other => Err(format!("unknown controller lane: {other}")),
    }
}

/// The points of one controller lane in a clip: mod wheel, sustain, bend.
#[tauri::command]
pub fn get_clip_controls(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    kind: String,
    cc: Option<u8>,
) -> Result<Vec<ControlPointInfo>, String> {
    let want = lane_kind(&kind, cc)?;
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let track = project
        .track(&track_id)
        .ok_or_else(|| format!("Track not found: {}", track_id))?;
    for clip in &track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &clip.content {
            if mc.id == clip_id {
                let mut points: Vec<ControlPointInfo> = mc
                    .clip
                    .controls
                    .iter()
                    .filter(|c| c.kind == want)
                    .map(|c| ControlPointInfo {
                        tick: c.tick,
                        value: c.value,
                    })
                    .collect();
                points.sort_by_key(|p| p.tick);
                return Ok(points);
            }
        }
    }
    Err(format!("MIDI clip not found: {}", clip_id))
}

/// Replace one controller lane in a clip. The other lanes are untouched,
/// so drawing the mod wheel cannot wipe a recorded bend.
#[tauri::command]
pub fn set_clip_controls(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    kind: String,
    cc: Option<u8>,
    points: Vec<ControlPointInfo>,
) -> Result<(), String> {
    let lane = lane_kind(&kind, cc)?;
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {}", track_id))?;
        let clip = track
            .clips
            .iter_mut()
            .find_map(|clip| match &mut clip.content {
                hardwave_project::clip::ClipContent::Midi(mc) if mc.id == clip_id => {
                    Some(&mut mc.clip)
                }
                _ => None,
            })
            .ok_or_else(|| format!("MIDI clip not found: {}", clip_id))?;
        clip.controls.retain(|c| c.kind != lane);
        let bend = matches!(lane, hardwave_midi::MidiControlKind::PitchBend);
        for p in points {
            clip.controls.push(hardwave_midi::MidiControlPoint {
                tick: p.tick,
                channel: 0,
                kind: lane,
                value: if bend {
                    p.value.clamp(-1.0, 1.0)
                } else {
                    p.value.clamp(0.0, 1.0)
                },
            });
        }
        clip.controls.sort_by_key(|c| c.tick);
    }
    engine.rebuild_graph();
    Ok(())
}

/// Grooves are kept with the song, beside the markers and the mixer
/// snapshots, so a feel taken from one part is still there tomorrow.
const GROOVE_KEY: &str = "grooves";

fn read_grooves(project: &hardwave_project::Project) -> Vec<hardwave_midi::groove::Groove> {
    let Some(raw) = project.timeline_state.as_deref() else {
        return Vec::new();
    };
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get(GROOVE_KEY).cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn write_grooves(
    project: &mut hardwave_project::Project,
    grooves: &[hardwave_midi::groove::Groove],
) {
    let mut value: serde_json::Value = project
        .timeline_state
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !value.is_object() {
        value = serde_json::json!({});
    }
    value[GROOVE_KEY] = serde_json::to_value(grooves).unwrap_or(serde_json::Value::Null);
    project.timeline_state = Some(value.to_string());
}

/// Take the timing and accents of a clip and keep them under a name.
#[tauri::command]
pub fn extract_groove(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    grid_ticks: u64,
    name: String,
) -> Result<usize, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("give the groove a name".into());
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let mut project = engine.project.lock();
    let (notes, length) = {
        let track = project
            .track(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let clip = track
            .clips
            .iter()
            .find_map(|c| match &c.content {
                hardwave_project::clip::ClipContent::Midi(mc) if mc.id == clip_id => Some(&mc.clip),
                _ => None,
            })
            .ok_or_else(|| format!("MIDI clip not found: {clip_id}"))?;
        (clip.notes.clone(), clip.length_ticks)
    };
    if notes.is_empty() {
        return Err("that clip has no notes to take a groove from".into());
    }
    let groove = hardwave_midi::groove::extract(&notes, grid_ticks, length, name.clone());
    let steps = groove.steps.len();
    let mut grooves = read_grooves(&project);
    grooves.retain(|g| g.name != name);
    grooves.push(groove);
    write_grooves(&mut project, &grooves);
    Ok(steps)
}

#[tauri::command]
pub fn list_grooves(state: State<AppState>) -> Vec<String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    read_grooves(&project).into_iter().map(|g| g.name).collect()
}

/// Put a saved groove on a clip. `strength` runs 0 to 1.
#[tauri::command]
pub fn apply_groove(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    name: String,
    strength: f32,
) -> Result<usize, String> {
    let groove = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        read_grooves(&project)
            .into_iter()
            .find(|g| g.name == name)
            .ok_or_else(|| format!("no groove called {name}"))?
    };
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        hardwave_midi::groove::apply(notes, &groove, strength);
        notes.len()
    })
}

/// Get all notes in a MIDI clip./// Get all notes in a MIDI clip./// Get all notes in a MIDI clip.
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
                        pan: n.pan,
                        fine_cents: n.fine_cents,
                        release_velocity: n.release_velocity,
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
                    ..Default::default()
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
    pan: Option<f32>,
    fine_cents: Option<f32>,
    release_velocity: Option<f32>,
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
                // Clamped here rather than trusted: the piano roll drags
                // these, and a drag that runs past the end of its strip
                // should stop at the edge, not detune a note by an octave.
                if let Some(p) = pan {
                    note.pan = p.clamp(-1.0, 1.0);
                }
                if let Some(c) = fine_cents {
                    note.fine_cents = c.clamp(-100.0, 100.0);
                }
                if let Some(r) = release_velocity {
                    note.release_velocity = r.clamp(0.0, 1.0);
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

/// Generate a diatonic chord progression into a clip, appended after any
/// existing notes. `bars` chords are walked from the start chord (root
/// pitch-class + quality) using common-practice harmony rules.
#[tauri::command]
pub fn generate_progression_in_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    key_root: u8,
    quality: String,
    bars: usize,
) -> Result<usize, String> {
    let quality = parse_chord_quality(&quality)?;
    let start = Chord::new(key_root, quality);
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let generated = generate_progression(
            key_root,
            start,
            bars.clamp(1, 64),
            60,
            hardwave_midi::PPQ * 4,
        );
        let count = generated.len();
        notes.extend(generated);
        count
    })
}

/// Generate a diatonic MELODY line into a clip, appended after any existing
/// notes. Builds a `bars`-long chord progression from the start chord (root
/// pitch-class + quality) and lays `notes_per_bar` melody notes over each bar.
#[tauri::command]
pub fn generate_melody_in_clip(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    key_root: u8,
    quality: String,
    bars: usize,
    notes_per_bar: usize,
) -> Result<usize, String> {
    let quality = parse_chord_quality(&quality)?;
    let start = Chord::new(key_root, quality);
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let generated = generate_melody_line(
            key_root,
            start,
            bars.clamp(1, 64),
            60,
            hardwave_midi::PPQ * 4,
            notes_per_bar.clamp(1, 16),
        );
        let count = generated.len();
        notes.extend(generated);
        count
    })
}

/// Apply legato to the selected notes (or the whole clip): stretch each
/// note so it lasts until the next one begins. Returns notes affected.
#[tauri::command]
pub fn legato_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    note_indices: Vec<usize>,
) -> Result<usize, String> {
    with_clip_notes(&state, &track_id, &clip_id, |notes| {
        let (mut selected, indices) = collect_selected(notes, &note_indices);
        legato(&mut selected);
        for (sel, &i) in selected.iter().zip(indices.iter()) {
            if let Some(n) = notes.get_mut(i) {
                n.duration_ticks = sel.duration_ticks;
            }
        }
        indices.len()
    })
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

// ---------------------------------------------------------------------------
// MIDI effects before the instrument
// ---------------------------------------------------------------------------

/// The chain on one track, in order.
#[tauri::command]
pub fn get_midi_fx(
    state: State<AppState>,
    track_id: String,
) -> Vec<hardwave_midi::midi_fx::MidiFx> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project
        .track(&track_id)
        .map(|t| t.midi_fx.clone())
        .unwrap_or_default()
}

/// Replace the chain on one track.
///
/// The whole chain at once rather than one effect at a time: the order
/// matters, and sending the list the app is showing is the only way the
/// two cannot drift apart.
#[tauri::command]
pub fn set_midi_fx(
    state: State<AppState>,
    track_id: String,
    chain: Vec<hardwave_midi::midi_fx::MidiFx>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        if !matches!(track.kind, hardwave_project::TrackKind::Midi) {
            return Err("MIDI effects belong on a MIDI track".into());
        }
        track.midi_fx = chain;
    }
    // The chain is folded into the note schedule when the graph is built,
    // so the rebuild is what makes it audible.
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// What the instrument hears for one clip, with the chain applied.
///
/// The piano roll draws this behind the written notes, so an arpeggiator
/// can be seen as well as heard instead of being a knob that changes
/// something invisible.
#[tauri::command]
pub fn preview_midi_fx(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
) -> Result<Vec<MidiNoteInfo>, String> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let track = project
        .track(&track_id)
        .ok_or_else(|| format!("Track not found: {track_id}"))?;
    for clip in &track.clips {
        if let hardwave_project::clip::ClipContent::Midi(mc) = &clip.content {
            if mc.id == clip_id {
                let heard = hardwave_midi::midi_fx::apply_chain(&mc.clip.notes, &track.midi_fx);
                return Ok(heard
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
                        pan: n.pan,
                        fine_cents: n.fine_cents,
                        release_velocity: n.release_velocity,
                    })
                    .collect());
            }
        }
    }
    Err(format!("MIDI clip not found: {clip_id}"))
}

// ---------------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------------

/// Read a Scala file and put its scale on a track.
///
/// The scale itself is stored in the song, not the path, so a project
/// opened on another machine still sounds the way it did. The built-in
/// instruments follow it; a hosted plug-in keeps its own tuning,
/// because retuning one needs MTS or note expression and the host does
/// not send either yet.
#[tauri::command]
pub fn load_tuning_file(
    state: State<AppState>,
    track_id: String,
    path: String,
    root_note: Option<u8>,
    root_hz: Option<f64>,
) -> Result<hardwave_midi::scala::Tuning, String> {
    let source = std::fs::read_to_string(&path).map_err(|e| format!("read {path}: {e}"))?;
    let fallback = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Tuning");
    let mut tuning = hardwave_midi::scala::parse_scl(&source, fallback)?;
    if let Some(note) = root_note {
        tuning.root_note = note;
    }
    if let Some(hz) = root_hz {
        if hz > 0.0 {
            tuning.root_hz = hz;
        }
    }
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.tuning = Some(tuning.clone());
    }
    state.engine.lock().rebuild_graph();
    Ok(tuning)
}

/// Back to twelve equal.
#[tauri::command]
pub fn clear_tuning(state: State<AppState>, track_id: String) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.tuning = None;
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// The tuning on a track, or nothing when it is in twelve equal.
#[tauri::command]
pub fn get_tuning(
    state: State<AppState>,
    track_id: String,
) -> Option<hardwave_midi::scala::Tuning> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    project.track(&track_id).and_then(|t| t.tuning.clone())
}

/// Move the scale's anchor: which note it starts on and what that note
/// sounds at.
#[tauri::command]
pub fn set_tuning_root(
    state: State<AppState>,
    track_id: String,
    root_note: u8,
    root_hz: f64,
) -> Result<(), String> {
    if root_hz <= 0.0 {
        return Err("a root has to have a pitch".into());
    }
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let tuning = track
            .tuning
            .as_mut()
            .ok_or_else(|| "that track is in twelve equal".to_string())?;
        tuning.root_note = root_note.min(127);
        tuning.root_hz = root_hz;
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}
