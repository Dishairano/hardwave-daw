//! Opening an FL Studio project.
//!
//! Most of the people this DAW is for work in FL. "Can I open my
//! projects" is the first question any of them ask, and until now the
//! answer was no, with a stub module describing what an answer would
//! look like.
//!
//! What comes across is the arrangement: the tempo, a track per
//! channel, and the notes of every pattern where the playlist puts
//! them. What cannot come across is everything a plug-in holds: FL
//! stores a plug-in's state as its own blob, and nothing outside FL
//! can mean anything by it. The report says what was left behind
//! rather than letting it be found later.

use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlpImportReport {
    pub bpm: f32,
    pub tracks: usize,
    pub clips: usize,
    pub notes: usize,
    /// Samples the FL channels pointed at, so they can be found and
    /// loaded: the paths are whatever that machine had.
    pub samples: Vec<String>,
    /// What could not be carried, in plain words.
    pub left_behind: Vec<String>,
}

/// Read a `.flp` and build a project from it.
#[tauri::command]
pub fn import_flp(state: State<AppState>, path: String) -> Result<FlpImportReport, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("could not read that file: {e}"))?;
    let fl = hardwave_project::flp_parser::parse(&bytes).map_err(|e| e.to_string())?;

    // A playlist copies a pattern's notes into every placement, so a
    // small file can ask for billions of notes. Counted before anything
    // is made: past what any song holds, nothing is imported.
    let mut notes_per_pattern: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for (pattern, _, notes) in &fl.pattern_notes {
        *notes_per_pattern.entry(*pattern).or_default() += notes.len();
    }
    let mut would_make = 0usize;
    for clip in &fl.playlist_clips {
        if let hardwave_project::fl_import::FlClipContent::Pattern { pattern_index } = &clip.content
        {
            would_make = would_make
                .saturating_add(notes_per_pattern.get(pattern_index).copied().unwrap_or(0));
        }
    }
    if would_make > hardwave_project::sanitize::MAX_NOTES
        || fl.playlist_clips.len() > hardwave_project::sanitize::MAX_CLIPS
        || fl.channels.len() > hardwave_project::sanitize::MAX_TRACKS
    {
        return Err(
            "that FL project expands to more than any song holds; it was not imported".into(),
        );
    }

    let mut report = FlpImportReport {
        bpm: fl.bpm,
        tracks: 0,
        clips: 0,
        notes: 0,
        samples: Vec::new(),
        left_behind: Vec::new(),
    };

    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();

        // The tempo first: every tick that follows is read against it.
        if let Some(entry) = project.tempo_map.entries.get_mut(0) {
            entry.bpm = fl.bpm as f64;
        }
        engine
            .transport
            .bpm
            .store(fl.bpm as f64, std::sync::atomic::Ordering::Relaxed);

        // A track per channel that actually plays something, named as
        // it was named in FL.
        let mut track_for_channel: std::collections::HashMap<u32, String> =
            std::collections::HashMap::new();
        let plays: std::collections::BTreeSet<u32> = fl
            .pattern_notes
            .iter()
            .map(|(_, channel, _)| *channel)
            .collect();
        for channel_index in plays {
            let name = fl
                .channels
                .get(channel_index as usize)
                .map(|c| c.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Channel {channel_index}"));
            let id = project.add_midi_track(name);
            track_for_channel.insert(channel_index, id);
            report.tracks += 1;
        }

        // Every placement of a pattern brings that pattern's notes
        // with it, per channel, at the place the playlist put it.
        let name_of = |pattern: u32| -> String {
            fl.pattern_names
                .iter()
                .find(|(id, _)| *id == pattern)
                .map(|(_, name)| name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| format!("Pattern {pattern}"))
        };
        for clip in &fl.playlist_clips {
            let hardwave_project::fl_import::FlClipContent::Pattern { pattern_index } =
                &clip.content
            else {
                continue;
            };
            for (pattern, channel, notes) in &fl.pattern_notes {
                if pattern != pattern_index || notes.is_empty() {
                    continue;
                }
                let Some(track_id) = track_for_channel.get(channel) else {
                    continue;
                };
                let length = notes
                    .iter()
                    .map(|n| n.tick + n.length_ticks)
                    .max()
                    .unwrap_or(hardwave_midi::PPQ * 4)
                    .max(clip.length_ticks);
                let mut midi_clip = hardwave_midi::MidiClip::new(
                    uuid::Uuid::new_v4().to_string(),
                    name_of(*pattern),
                    length,
                );
                midi_clip.notes = notes
                    .iter()
                    .map(|note| hardwave_midi::MidiNote {
                        start_tick: note.tick,
                        duration_ticks: note.length_ticks,
                        pitch: note.pitch,
                        velocity: note.velocity as f32 / 127.0,
                        channel: 0,
                        muted: false,
                        ..Default::default()
                    })
                    .collect();
                report.notes += midi_clip.notes.len();
                let placement = hardwave_project::clip::ClipPlacement {
                    content: hardwave_project::clip::ClipContent::Midi(
                        hardwave_project::clip::MidiClipRef {
                            id: uuid::Uuid::new_v4().to_string(),
                            clip: midi_clip,
                        },
                    ),
                    track_id: track_id.clone(),
                    position_ticks: clip.start_tick,
                    length_ticks: length,
                    lane: 0,
                };
                if let Some(track) = project.track_mut(track_id) {
                    track.clips.push(placement);
                    report.clips += 1;
                }
            }
        }

        // The samples those channels pointed at. The paths are from
        // whichever machine wrote the file, so they are reported
        // rather than loaded behind the user's back.
        for channel in &fl.channels {
            if let Some(sample) = &channel.sample_path {
                report.samples.push(sample.clone());
            }
        }
    }
    engine.sync_track_meters();
    engine.rebuild_graph();

    if !report.samples.is_empty() {
        report.left_behind.push(format!(
            "{} sample {} point at files on the machine the project came from",
            report.samples.len(),
            if report.samples.len() == 1 {
                "path"
            } else {
                "paths"
            }
        ));
    }
    report
        .left_behind
        .push("plug-ins and their settings: FL keeps those in its own format".into());
    report
        .left_behind
        .push("mixer routing, effects and automation clips".into());
    if report.clips == 0 {
        report
            .left_behind
            .push("no playlist clips were found, so the notes have nowhere to sit yet".into());
    }

    Ok(report)
}
