//! The sample window's NOTES view: the notes a take holds, tuned by hand.
//!
//! Finding the notes follows the pitch of the whole take, which is a few
//! seconds for a long one, so it runs off the window's thread and says how
//! far along it is. The notes as moved are kept with the clip (every clip
//! of the same file, as every setting in the sample window is); the engine
//! renders the take with them off the audio thread and plays that, and an
//! export waits for it. The file itself is never changed.

use crate::AppState;
use hardwave_project::clip::{ClipContent, TunedNote};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

/// One note as the view shows and sends it. Seconds from the start of the
/// file; pitches are MIDI notes with the cents in the fraction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteInfo {
    pub start: f64,
    pub end: f64,
    pub sung: f64,
    pub pitch: f64,
    pub drift: f64,
    pub new_start: f64,
    pub new_end: f64,
    pub formants: bool,
}

impl From<&TunedNote> for NoteInfo {
    fn from(n: &TunedNote) -> Self {
        Self {
            start: n.start,
            end: n.end,
            sung: n.sung,
            pitch: n.pitch,
            drift: n.drift,
            new_start: n.new_start,
            new_end: n.new_end,
            formants: n.formants,
        }
    }
}

impl From<&NoteInfo> for TunedNote {
    fn from(n: &NoteInfo) -> Self {
        Self {
            start: n.start,
            end: n.end,
            sung: n.sung,
            pitch: n.pitch,
            drift: n.drift,
            new_start: n.new_start,
            new_end: n.new_end,
            formants: n.formants,
        }
    }
}

/// What the NOTES view draws.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipNotes {
    /// The file's length, in seconds.
    pub seconds: f64,
    /// The notes in time order: as tuned, or as found when the take has
    /// not been tuned yet.
    pub notes: Vec<NoteInfo>,
    /// The pitch as sung, the line through each note: frame i is at
    /// `curve_start + i * curve_step` seconds, a MIDI note with the cents
    /// in the fraction, or null where nothing is sung.
    pub curve_start: f64,
    pub curve_step: f64,
    pub curve: Vec<Option<f32>>,
    /// The key the notes seem to be in: its root as a pitch class (0 is C)
    /// and whether it is minor. None for a take of fewer than three notes.
    pub key_root: Option<u8>,
    pub key_minor: bool,
    /// The fingerprint of the notes as kept (`ClipInfo::tune_key`).
    pub tune_key: String,
}

/// How far along finding the notes is, for the view's bar.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress<'a> {
    source_id: &'a str,
    done: f32,
}

/// The notes of an audio clip's take, found the first time the take is
/// opened in the NOTES view (with progress as `tune-notes-progress`
/// events) and kept for every move after.
#[tauri::command]
pub async fn find_clip_notes(
    app: tauri::AppHandle,
    track_id: String,
    clip_id: String,
) -> Result<ClipNotes, String> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let (source, stored, pool) = {
            let engine = state.engine.lock();
            let project = engine.project.lock();
            let clip = find_audio_clip(&project, &track_id, &clip_id)?;
            (
                clip.source_path.clone(),
                clip.tuned_notes.clone(),
                engine.audio_pool.clone(),
            )
        };
        let mut last = -1.0f32;
        let take = pool
            .take_analysis(&source, &mut |done| {
                if done - last >= 0.02 || done >= 1.0 {
                    last = done;
                    let _ = app.emit(
                        "tune-notes-progress",
                        Progress {
                            source_id: &source,
                            done,
                        },
                    );
                }
            })
            .ok_or_else(|| "that clip's audio is not loaded".to_string())?;
        Ok(clip_notes(&take, &stored))
    })
    .await
    .map_err(|e| format!("finding the notes stopped: {e}"))?
}

/// The view's picture of a take: the notes kept with the clip, or the
/// ones found when there are none yet.
pub(crate) fn clip_notes(
    take: &hardwave_dsp::note_tune::TakeAnalysis,
    stored: &[TunedNote],
) -> ClipNotes {
    let rate = take.sample_rate.max(1.0);
    let notes: Vec<NoteInfo> = if stored.is_empty() {
        take.notes
            .iter()
            .map(|n| {
                let (start, end) = (n.start as f64 / rate, n.end as f64 / rate);
                NoteInfo {
                    start,
                    end,
                    sung: n.pitch,
                    pitch: n.pitch,
                    drift: 1.0,
                    new_start: start,
                    new_end: end,
                    formants: true,
                }
            })
            .collect()
    } else {
        stored.iter().map(NoteInfo::from).collect()
    };
    let key = hardwave_dsp::note_tune::guess_key(&take.notes);
    ClipNotes {
        seconds: take.len as f64 / rate,
        notes,
        curve_start: (take.window / 2) as f64 / rate,
        curve_step: take.hop as f64 / rate,
        curve: take
            .pitch
            .iter()
            .map(|p| (!p.is_nan()).then_some(*p))
            .collect(),
        key_root: key.map(|k| k.0),
        key_minor: key.is_some_and(|k| k.1),
        tune_key: tune_key(stored),
    }
}

/// The fingerprint the view compares to see that the notes changed under
/// it (an undo, a room partner): empty for a take never tuned.
pub(crate) fn tune_key(notes: &[TunedNote]) -> String {
    if notes.is_empty() {
        String::new()
    } else {
        format!("{:016x}", hardwave_project::clip::tuned_notes_hash(notes))
    }
}

fn find_audio_clip<'a>(
    project: &'a hardwave_project::Project,
    track_id: &str,
    clip_id: &str,
) -> Result<&'a hardwave_project::clip::AudioClip, String> {
    project
        .track(track_id)
        .ok_or_else(|| format!("Track not found: {track_id}"))?
        .clips
        .iter()
        .find_map(|c| match &c.content {
            ClipContent::Audio(ac) if ac.id == clip_id => Some(ac),
            _ => None,
        })
        .ok_or_else(|| "that clip is not an audio clip".to_string())
}

/// Keep a take's notes as the view moved them: one undo step, heard at
/// once. They go to every clip of the same file, as everything in the
/// sample window does. Returns how many clips changed.
#[tauri::command]
pub fn set_clip_notes(
    state: State<AppState>,
    track_id: String,
    clip_id: String,
    notes: Vec<NoteInfo>,
) -> Result<usize, String> {
    let engine = state.engine.lock();
    let seconds = {
        let project = engine.project.lock();
        let clip = find_audio_clip(&project, &track_id, &clip_id)?;
        engine
            .audio_pool
            .get(&clip.source_path)
            .map(|b| b.num_frames as f64 / b.sample_rate.max(1) as f64)
    };
    engine.snapshot_before_mutation();
    let changed = {
        let mut project = engine.project.lock();
        apply_clip_notes(&mut project, &track_id, &clip_id, &notes, seconds)?
    };
    engine.rebuild_graph();
    Ok(changed)
}

/// `set_clip_notes` without the engine, so it can be tested.
pub(crate) fn apply_clip_notes(
    project: &mut hardwave_project::Project,
    track_id: &str,
    clip_id: &str,
    notes: &[NoteInfo],
    seconds: Option<f64>,
) -> Result<usize, String> {
    let source = find_audio_clip(project, track_id, clip_id)?
        .source_path
        .clone();
    let mut tuned: Vec<TunedNote> = notes.iter().map(TunedNote::from).collect();
    hardwave_project::clip::sanitize_tuned_notes(&mut tuned, seconds);
    let mut changed = 0;
    for track in &mut project.tracks {
        for placement in &mut track.clips {
            if let ClipContent::Audio(ac) = &mut placement.content {
                if ac.source_path == source {
                    ac.tuned_notes = tuned.clone();
                    changed += 1;
                }
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_project::clip::{AudioClip, ClipPlacement};

    fn audio(id: &str, source: &str) -> AudioClip {
        AudioClip {
            id: id.into(),
            name: id.into(),
            source_path: source.into(),
            source_hash: String::new(),
            source_start: 0,
            source_end: 0,
            gain_db: 0.0,
            fade_in_ticks: 0,
            fade_out_ticks: 0,
            muted: false,
            reversed: false,
            pitch_semitones: 0.0,
            stretch_ratio: 1.0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            warp_markers: Vec::new(),
            source_file: String::new(),
            treatment: Default::default(),
            tuned_notes: Vec::new(),
        }
    }

    fn place(project: &mut hardwave_project::Project, track: &str, clip: AudioClip) {
        project.track_mut(track).unwrap().clips.push(ClipPlacement {
            content: ClipContent::Audio(clip),
            track_id: track.into(),
            position_ticks: 0,
            length_ticks: 3840,
            lane: 0,
            color: None,
            row: None,
        });
    }

    fn tuned(project: &hardwave_project::Project, track: &str, i: usize) -> Vec<TunedNote> {
        match &project.track(track).unwrap().clips[i].content {
            ClipContent::Audio(a) => a.tuned_notes.clone(),
            _ => unreachable!(),
        }
    }

    fn note(start: f64, end: f64, sung: f64) -> NoteInfo {
        NoteInfo {
            start,
            end,
            sung,
            pitch: sung,
            drift: 1.0,
            new_start: start,
            new_end: end,
            formants: true,
        }
    }

    #[test]
    fn moved_notes_reach_every_clip_of_the_take_and_survive_a_save() {
        let mut project = hardwave_project::Project::default();
        place(&mut project, "insert-001", audio("a", "vocal"));
        place(&mut project, "insert-002", audio("b", "vocal"));
        place(&mut project, "insert-002", audio("c", "other"));
        let mut go = note(1.0, 1.8, 63.38);
        go.pitch = 65.0;
        go.drift = 0.4;
        let notes = vec![note(0.0, 0.6, 65.08), go];
        let changed = apply_clip_notes(&mut project, "insert-001", "a", &notes, Some(4.0)).unwrap();
        assert_eq!(changed, 2, "both clips of the vocal, not the other file");
        assert_eq!(tuned(&project, "insert-002", 0).len(), 2);
        assert!(tuned(&project, "insert-002", 1).is_empty());

        // Saved and opened again, the notes are as they were.
        let bytes = project.to_bytes().unwrap();
        let back = hardwave_project::Project::from_bytes(&bytes).unwrap();
        let kept = tuned(&back, "insert-001", 0);
        assert_eq!(kept[1].pitch, 65.0);
        assert_eq!(kept[1].drift, 0.4);
        let view: Vec<NoteInfo> = kept.iter().map(NoteInfo::from).collect();
        assert_eq!(view, notes, "the view gets back what it sent");
    }

    #[test]
    fn notes_from_the_view_are_put_in_order_and_range() {
        let mut project = hardwave_project::Project::default();
        place(&mut project, "insert-001", audio("a", "vocal"));
        let mut wild = note(1.0, 1.5, 60.0);
        wild.pitch = 300.0;
        let notes = vec![wild, note(0.2, 0.5, 62.0), note(9.0, 9.5, 60.0)];
        apply_clip_notes(&mut project, "insert-001", "a", &notes, Some(4.0)).unwrap();
        let kept = tuned(&project, "insert-001", 0);
        assert_eq!(
            kept.len(),
            2,
            "the note past the end of the file is dropped"
        );
        assert_eq!(kept[0].start, 0.2);
        assert_eq!(kept[1].pitch, 84.0);
    }

    #[test]
    fn a_take_never_opened_shows_the_notes_found() {
        let sr = 48_000.0;
        let audio: Vec<f32> = (0..(sr as usize))
            .map(|i| {
                let t = i as f64 / sr;
                if (0.1..0.6).contains(&t) {
                    (std::f64::consts::TAU * 220.0 * t).sin() as f32 * 0.5
                } else {
                    0.0
                }
            })
            .collect();
        let take = hardwave_dsp::note_tune::analyse(&audio, sr, &mut |_| {});
        let view = clip_notes(&take, &[]);
        assert_eq!(view.notes.len(), 1);
        let n = view.notes[0];
        assert!((n.sung - 57.0).abs() < 0.1 && n.pitch == n.sung && n.drift == 1.0);
        assert!(
            (n.start - 0.1).abs() < 0.03 && (n.end - 0.6).abs() < 0.03,
            "{n:?}"
        );
        assert!((view.seconds - 1.0).abs() < 1e-9);
        assert!(view.tune_key.is_empty());
        // The line: a pitch where it is sung, none where it is not.
        let at = |s: f64| view.curve[((s - view.curve_start) / view.curve_step).round() as usize];
        assert!(at(0.35).is_some_and(|p| (p - 57.0).abs() < 0.1));
        assert_eq!(at(0.8), None);
    }
}
