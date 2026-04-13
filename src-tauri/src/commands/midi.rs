use crate::AppState;
use serde::Serialize;
use tauri::State;

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
