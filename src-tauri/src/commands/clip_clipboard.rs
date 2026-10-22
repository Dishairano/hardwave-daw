//! Cut, copy and paste of playlist clips.
//!
//! The interface kept a list of the copied clips' ids and pasted by
//! duplicating each id on the target track. A clip only exists on its own
//! track, so pasting onto any other track failed without a word, and cut
//! deleted the clips before the paste could find them, so cut and paste
//! pasted nothing at all. The clipboard now holds copies of the clips
//! themselves, taken when they are copied.

use hardwave_project::clip::{ClipContent, ClipPlacement};
use hardwave_project::track::TrackKind;
use hardwave_project::Project;
use tauri::State;

use crate::AppState;

/// A copied clip and the playlist row it came from, so clips copied from
/// several tracks land on as many tracks, the same distance apart.
#[derive(Clone)]
struct Copied {
    row: usize,
    placement: ClipPlacement,
}

static CLIPBOARD: parking_lot::Mutex<Vec<Copied>> = parking_lot::const_mutex(Vec::new());

#[derive(serde::Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PasteOutcome {
    /// Ids of the new clips, to select them.
    pub pasted: Vec<String>,
    /// Clips that had no fitting track there: an audio clip goes on an
    /// audio track, a MIDI clip on a MIDI track, and nothing goes past the
    /// last track.
    pub skipped: usize,
}

/// The tracks the playlist shows, in its order.
fn rows(project: &Project) -> Vec<usize> {
    project
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| !matches!(t.kind, TrackKind::Master))
        .map(|(i, _)| i)
        .collect()
}

fn id_of(placement: &ClipPlacement) -> &str {
    match &placement.content {
        ClipContent::Audio(a) => &a.id,
        ClipContent::Midi(m) => &m.id,
    }
}

fn copy_from(project: &Project, clip_ids: &[String]) -> Vec<Copied> {
    let rows = rows(project);
    let mut out = Vec::new();
    for (row, &index) in rows.iter().enumerate() {
        for placement in &project.tracks[index].clips {
            if clip_ids.iter().any(|id| id == id_of(placement)) {
                out.push(Copied {
                    row,
                    placement: placement.clone(),
                });
            }
        }
    }
    out
}

fn fits(kind: &TrackKind, content: &ClipContent) -> bool {
    match content {
        ClipContent::Audio(_) => matches!(kind, TrackKind::Audio),
        ClipContent::Midi(_) => matches!(kind, TrackKind::Midi),
    }
}

/// Put copies of `copied` into the song: the earliest at `position_ticks`
/// and the rest as far after it as they were, the top one on
/// `target_track_id` (or on its own track when there is none) and the rest
/// as many rows below it as they were.
fn paste_into(
    project: &mut Project,
    copied: &[Copied],
    target_track_id: Option<&str>,
    position_ticks: u64,
    mut new_id: impl FnMut() -> String,
) -> PasteOutcome {
    let mut outcome = PasteOutcome {
        pasted: Vec::new(),
        skipped: 0,
    };
    let Some(earliest) = copied.iter().map(|c| c.placement.position_ticks).min() else {
        return outcome;
    };
    let top = copied.iter().map(|c| c.row).min().unwrap_or(0);
    let rows = rows(project);
    let target_row = target_track_id
        .and_then(|id| rows.iter().position(|&i| project.tracks[i].id == id))
        .unwrap_or(top);
    for c in copied {
        let row = target_row + (c.row - top);
        let Some(&index) = rows.get(row) else {
            outcome.skipped += 1;
            continue;
        };
        let track = &mut project.tracks[index];
        if !fits(&track.kind, &c.placement.content) {
            outcome.skipped += 1;
            continue;
        }
        let mut placement = c.placement.clone();
        let id = new_id();
        match &mut placement.content {
            ClipContent::Audio(a) => a.id = id.clone(),
            ClipContent::Midi(m) => m.id = id.clone(),
        }
        placement.track_id = track.id.clone();
        placement.position_ticks = position_ticks + (c.placement.position_ticks - earliest);
        track.clips.push(placement);
        outcome.pasted.push(id);
    }
    outcome
}

/// Copy these clips. Returns how many were found.
#[tauri::command]
pub fn copy_clips(state: State<AppState>, clip_ids: Vec<String>) -> usize {
    let engine = state.engine.lock();
    let copied = copy_from(&engine.project.lock(), &clip_ids);
    let n = copied.len();
    *CLIPBOARD.lock() = copied;
    n
}

/// Copy these clips and take them out of the song, as one undo step.
#[tauri::command]
pub fn cut_clips(state: State<AppState>, clip_ids: Vec<String>) -> usize {
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let n = {
        let mut project = engine.project.lock();
        let copied = copy_from(&project, &clip_ids);
        for track in project.tracks.iter_mut() {
            track
                .clips
                .retain(|p| !clip_ids.iter().any(|id| id == id_of(p)));
        }
        let n = copied.len();
        *CLIPBOARD.lock() = copied;
        n
    };
    engine.rebuild_graph();
    n
}

/// Paste the copied clips at `position_ticks`, starting on
/// `target_track_id`, or on the tracks they came from when it is absent.
#[tauri::command]
pub fn paste_clips(
    state: State<AppState>,
    target_track_id: Option<String>,
    position_ticks: u64,
) -> PasteOutcome {
    let copied = CLIPBOARD.lock().clone();
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let outcome = paste_into(
        &mut engine.project.lock(),
        &copied,
        target_track_id.as_deref(),
        position_ticks,
        || uuid::Uuid::new_v4().to_string(),
    );
    engine.rebuild_graph();
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardwave_midi::MidiClip;
    use hardwave_project::clip::{AudioClip, FadeCurve, MidiClipRef};

    fn audio(id: &str, position: u64) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: id.into(),
                name: id.into(),
                source_path: "s".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 100,
                gain_db: -3.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
                source_file: String::new(),
                treatment: Default::default(),
            }),
            track_id: String::new(),
            position_ticks: position,
            length_ticks: 960,
            lane: 0,
            color: None,
        }
    }

    fn midi(id: &str, position: u64) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: id.into(),
                clip: MidiClip::new("inner".into(), id.into(), 960),
            }),
            track_id: String::new(),
            position_ticks: position,
            length_ticks: 960,
            lane: 0,
            color: None,
        }
    }

    /// Three audio tracks and a MIDI track, with clips "a" on the first,
    /// "b" on the second and "m" on the MIDI track.
    fn song() -> (Project, Vec<String>) {
        let mut p = Project::default();
        let ids = vec![
            p.add_audio_track("A1".into()),
            p.add_audio_track("A2".into()),
            p.add_audio_track("A3".into()),
            p.add_midi_track("M".into()),
        ];
        p.track_mut(&ids[0]).unwrap().clips.push(audio("a", 960));
        p.track_mut(&ids[1]).unwrap().clips.push(audio("b", 1920));
        p.track_mut(&ids[3]).unwrap().clips.push(midi("m", 0));
        (p, ids)
    }

    fn counter() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("new-{n}")
        }
    }

    fn clips_on(p: &Project, track: &str) -> Vec<(String, u64)> {
        p.tracks
            .iter()
            .find(|t| t.id == track)
            .unwrap()
            .clips
            .iter()
            .map(|c| (id_of(c).to_string(), c.position_ticks))
            .collect()
    }

    #[test]
    fn a_clip_pastes_onto_another_track() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["a".into()]);
        let out = paste_into(&mut p, &copied, Some(&ids[2]), 3840, counter());
        assert_eq!(
            out,
            PasteOutcome {
                pasted: vec!["new-1".into()],
                skipped: 0
            }
        );
        assert_eq!(clips_on(&p, &ids[2]), vec![("new-1".into(), 3840)]);
        let pasted = &p.tracks.iter().find(|t| t.id == ids[2]).unwrap().clips[0];
        assert_eq!(pasted.track_id, ids[2]);
        let ClipContent::Audio(a) = &pasted.content else {
            unreachable!()
        };
        assert_eq!(a.gain_db, -3.0, "a copy carries the clip's settings");
    }

    #[test]
    fn a_cut_clip_can_still_be_pasted() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["a".into()]);
        p.track_mut(&ids[0]).unwrap().clips.clear();
        let out = paste_into(&mut p, &copied, Some(&ids[1]), 0, counter());
        assert_eq!(out.pasted.len(), 1);
        assert_eq!(
            clips_on(&p, &ids[1]),
            vec![("b".into(), 1920), ("new-1".into(), 0)]
        );
    }

    #[test]
    fn clips_from_two_tracks_land_on_two_tracks_the_same_distance_apart() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["a".into(), "b".into()]);
        paste_into(&mut p, &copied, Some(&ids[1]), 7680, counter());
        // "a" was a row above "b" and a beat earlier.
        assert_eq!(clips_on(&p, &ids[1]).last(), Some(&("new-1".into(), 7680)));
        assert_eq!(clips_on(&p, &ids[2]), vec![("new-2".into(), 7680 + 960)]);
    }

    #[test]
    fn without_a_target_the_clips_go_back_on_their_own_tracks() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["b".into()]);
        paste_into(&mut p, &copied, None, 0, counter());
        assert_eq!(
            clips_on(&p, &ids[1]),
            vec![("b".into(), 1920), ("new-1".into(), 0)]
        );
    }

    #[test]
    fn a_clip_with_no_fitting_track_there_is_counted_not_forced() {
        let (mut p, ids) = song();
        // A MIDI clip onto an audio track, and audio past the last track.
        let copied = copy_from(&p, &["m".into()]);
        let out = paste_into(&mut p, &copied, Some(&ids[0]), 0, counter());
        assert_eq!(
            out,
            PasteOutcome {
                pasted: vec![],
                skipped: 1
            }
        );
        let copied = copy_from(&p, &["a".into(), "b".into()]);
        let out = paste_into(&mut p, &copied, Some(&ids[3]), 0, counter());
        assert_eq!(out.skipped, 2, "a MIDI track, then nothing below it");
    }
}
