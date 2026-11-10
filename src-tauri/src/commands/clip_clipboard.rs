//! Cut, copy and paste of playlist clips.
//!
//! The interface kept a list of the copied clips' ids and pasted by
//! duplicating each id on the target track. A clip only exists on its own
//! track, so pasting onto any other track failed without a word, and cut
//! deleted the clips before the paste could find them, so cut and paste
//! pasted nothing at all. The clipboard now holds copies of the clips
//! themselves, taken when they are copied.
//!
//! A pasted clip stays on the channel it was copied from, as in FL, and
//! lands on a playlist row: the rows are where clips sit, the channel is
//! what they play through. Pasting onto another track used to change the
//! sound of the copy.

use hardwave_project::clip::{ClipContent, ClipPlacement};
use hardwave_project::track::TrackKind;
use hardwave_project::Project;
use tauri::State;

use crate::AppState;

/// A copied clip and the playlist row it sat on, so clips copied from
/// several rows land on as many rows, the same distance apart.
#[derive(Clone)]
struct Copied {
    row: u32,
    placement: ClipPlacement,
}

static CLIPBOARD: parking_lot::Mutex<Vec<Copied>> = parking_lot::const_mutex(Vec::new());

#[derive(serde::Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PasteOutcome {
    /// Ids of the new clips, to select them.
    pub pasted: Vec<String>,
    /// Clips that could not go back: their channel is gone, or the row
    /// would be past the last one.
    pub skipped: usize,
}

fn id_of(placement: &ClipPlacement) -> &str {
    match &placement.content {
        ClipContent::Audio(a) => &a.id,
        ClipContent::Midi(m) => &m.id,
    }
}

/// The row a clip sits on: its own, or for a clip made before rows were
/// kept, its track's place in the playlist, where it was drawn.
fn shown_row(project: &Project, track_index: usize, placement: &ClipPlacement) -> u32 {
    placement.row.unwrap_or_else(|| {
        project.tracks[..track_index]
            .iter()
            .filter(|t| !matches!(t.kind, TrackKind::Master))
            .count() as u32
    })
}

fn copy_from(project: &Project, clip_ids: &[String]) -> Vec<Copied> {
    let mut out = Vec::new();
    for (index, track) in project.tracks.iter().enumerate() {
        for placement in &track.clips {
            if clip_ids.iter().any(|id| id == id_of(placement)) {
                let mut placement = placement.clone();
                placement.track_id = track.id.clone();
                out.push(Copied {
                    row: shown_row(project, index, &placement),
                    placement,
                });
            }
        }
    }
    out
}

/// Put copies of `copied` into the song, each on the channel it came
/// from: the earliest at `position_ticks` and the rest as far after it as
/// they were, the top one on `target_row` (or on their own rows when there
/// is none) and the rest as many rows below it as they were.
fn paste_into(
    project: &mut Project,
    copied: &[Copied],
    target_row: Option<u32>,
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
    for c in copied {
        let row = target_row.map_or(c.row, |t| t.saturating_add(c.row - top));
        if hardwave_project::row_edit::check_row_number(row).is_err() {
            outcome.skipped += 1;
            continue;
        }
        let Some(track) = project.track_mut(&c.placement.track_id) else {
            outcome.skipped += 1;
            continue;
        };
        let mut placement = c.placement.clone();
        let id = new_id();
        match &mut placement.content {
            ClipContent::Audio(a) => a.id = id.clone(),
            ClipContent::Midi(m) => m.id = id.clone(),
        }
        placement.position_ticks = position_ticks + (c.placement.position_ticks - earliest);
        placement.row = Some(row);
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

/// Paste the copied clips at `position_ticks`, starting on `target_row`,
/// or on the rows they came from when it is absent.
#[tauri::command]
pub fn paste_clips(
    state: State<AppState>,
    target_row: Option<u32>,
    position_ticks: u64,
) -> PasteOutcome {
    let copied = CLIPBOARD.lock().clone();
    let engine = state.engine.lock();
    engine.snapshot_before_mutation();
    let outcome = paste_into(
        &mut engine.project.lock(),
        &copied,
        target_row,
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
                tuned_notes: Vec::new(),
            }),
            track_id: String::new(),
            position_ticks: position,
            length_ticks: 960,
            lane: 0,
            color: None,
            row: None,
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
            row: None,
        }
    }

    /// Three audio tracks and a MIDI track, with clips "a" on the first,
    /// "b" on the second and "m" on the MIDI track, on rows 0, 1 and 3.
    fn song() -> (Project, Vec<String>) {
        let mut p = Project::default();
        let ids = vec![
            p.add_audio_track("A1".into()),
            p.add_audio_track("A2".into()),
            p.add_audio_track("A3".into()),
            p.add_midi_track("M".into()),
        ];
        let on = |mut c: ClipPlacement, row| {
            c.row = Some(row);
            c
        };
        p.track_mut(&ids[0])
            .unwrap()
            .clips
            .push(on(audio("a", 960), 0));
        p.track_mut(&ids[1])
            .unwrap()
            .clips
            .push(on(audio("b", 1920), 1));
        p.track_mut(&ids[3])
            .unwrap()
            .clips
            .push(on(midi("m", 0), 3));
        (p, ids)
    }

    fn counter() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("new-{n}")
        }
    }

    /// Each clip on a track: its id, where it starts and its row.
    fn clips_on(p: &Project, track: &str) -> Vec<(String, u64, Option<u32>)> {
        p.tracks
            .iter()
            .find(|t| t.id == track)
            .unwrap()
            .clips
            .iter()
            .map(|c| (id_of(c).to_string(), c.position_ticks, c.row))
            .collect()
    }

    #[test]
    fn a_clip_pastes_onto_another_row_and_stays_on_its_channel() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["a".into()]);
        let out = paste_into(&mut p, &copied, Some(7), 3840, counter());
        assert_eq!(
            out,
            PasteOutcome {
                pasted: vec!["new-1".into()],
                skipped: 0
            }
        );
        assert_eq!(
            clips_on(&p, &ids[0]),
            vec![("a".into(), 960, Some(0)), ("new-1".into(), 3840, Some(7))]
        );
        let pasted = &p.track(&ids[0]).unwrap().clips[1];
        assert_eq!(pasted.track_id, ids[0]);
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
        let out = paste_into(&mut p, &copied, Some(1), 0, counter());
        assert_eq!(out.pasted.len(), 1);
        assert_eq!(clips_on(&p, &ids[0]), vec![("new-1".into(), 0, Some(1))]);
    }

    #[test]
    fn clips_from_two_rows_land_on_two_rows_the_same_distance_apart() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["a".into(), "m".into()]);
        paste_into(&mut p, &copied, Some(10), 7680, counter());
        // "a" was three rows above "m" and a beat later.
        assert_eq!(
            clips_on(&p, &ids[0]).last(),
            Some(&("new-1".into(), 7680 + 960, Some(10)))
        );
        assert_eq!(
            clips_on(&p, &ids[3]).last(),
            Some(&("new-2".into(), 7680, Some(13)))
        );
    }

    #[test]
    fn without_a_target_the_clips_go_back_on_their_own_rows() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["b".into()]);
        paste_into(&mut p, &copied, None, 0, counter());
        assert_eq!(
            clips_on(&p, &ids[1]),
            vec![("b".into(), 1920, Some(1)), ("new-1".into(), 0, Some(1))]
        );
    }

    #[test]
    fn a_clip_whose_channel_is_gone_is_counted_not_forced() {
        let (mut p, ids) = song();
        let copied = copy_from(&p, &["m".into(), "a".into()]);
        p.remove_track(&ids[3]);
        let out = paste_into(&mut p, &copied, None, 0, counter());
        assert_eq!(out.pasted.len(), 1, "the audio clip");
        assert_eq!(out.skipped, 1, "the MIDI clip had nowhere to go");
        let far = hardwave_project::sanitize::MAX_ROWS as u32;
        let out = paste_into(&mut p, &copied, Some(far), 0, counter());
        assert_eq!(out.skipped, 2, "no row past the last");
    }

    #[test]
    fn a_clip_made_before_rows_were_kept_copies_from_the_row_it_showed_on() {
        let (mut p, ids) = song();
        p.track_mut(&ids[2]).unwrap().clips.push(audio("old", 0));
        let copied = copy_from(&p, &["old".into()]);
        let place = p.tracks.iter().position(|t| t.id == ids[2]).unwrap() as u32 - 1;
        assert_eq!(copied[0].row, place, "its track's place in the playlist");
        paste_into(&mut p, &copied, None, 960, counter());
        assert_eq!(clips_on(&p, &ids[2]).last().unwrap().2, Some(place));
    }

    #[test]
    fn an_audio_clip_on_a_midi_channel_pastes_back_onto_it() {
        // A consolidated part or a sample dropped on a channel is an audio
        // clip on a MIDI track, and the engine plays it there.
        let (mut p, ids) = song();
        p.track_mut(&ids[3])
            .unwrap()
            .clips
            .push(audio("part", 3840));
        let copied = copy_from(&p, &["part".into()]);
        let out = paste_into(&mut p, &copied, None, 7680, counter());
        assert_eq!(
            out,
            PasteOutcome {
                pasted: vec!["new-1".into()],
                skipped: 0
            }
        );
        assert_eq!(clips_on(&p, &ids[3]).last().map(|c| c.1), Some(7680));
    }
}
