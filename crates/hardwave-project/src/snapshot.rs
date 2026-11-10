//! A song kept for undo that shares what did not change with the step
//! next to it.
//!
//! Each undo step used to hold a whole copy of the song. An FL song with
//! 501 tracks and 3,166 clips weighs 2.27 MB in memory, so a full history
//! of 256 steps held 580 MB for a song whose file is 0.11 MB. A snapshot
//! keeps the song in parts behind `Arc`s: the song's own settings, the
//! arrangements, and per track its settings, clips and automation clips.
//! A part equal to the neighbouring step's is that step's part, so a step
//! costs what its edit changed plus a pointer per track (and per clip on
//! the tracks it changed).
//!
//! The parts are compared by value, not tracked by the edit commands, so
//! no command has to say what it changed and none can forget to. A part
//! that is not shared is a plain clone, so a snapshot always gives back
//! the very song it was taken from. (Equal is `==`: a number that only
//! went from 0.0 to -0.0 counts as unchanged, which sounds the same.)

use std::collections::HashMap;
use std::mem::{size_of, size_of_val};
use std::sync::Arc;

use crate::arrangement::Arrangement;
use crate::automation_clip::AutomationClip;
use crate::clip::{ClipContent, ClipPlacement};
use crate::track::Track;
use crate::Project;

/// One song as an undo step keeps it.
#[derive(Debug)]
pub struct Snapshot {
    /// The song with its tracks and arrangements taken out.
    pub(crate) rest: Arc<Project>,
    pub(crate) arrangements: Arc<Vec<Arrangement>>,
    pub(crate) tracks: Vec<Arc<TrackPart>>,
}

/// One track in a snapshot.
#[derive(Debug)]
pub(crate) struct TrackPart {
    /// The track with its clips and automation clips taken out.
    pub(crate) settings: Arc<Track>,
    pub(crate) clips: Vec<Arc<ClipPlacement>>,
    pub(crate) automation_clips: Vec<Arc<AutomationClip>>,
}

impl Snapshot {
    /// Keep `song` as it is now, sharing every part equal to `near`'s.
    /// Returns the snapshot and roughly how many bytes it added (see
    /// [`Snapshot::bytes_freed_by_drop`]).
    ///
    /// `song` is only borrowed mutably to set its tracks and arrangements
    /// aside while its other parts are compared; it is left as it was.
    pub fn take(song: &mut Project, near: Option<&Snapshot>) -> (Self, usize) {
        let mut added = size_of::<Snapshot>();
        let mut tracks = std::mem::take(&mut song.tracks);
        let arrangements = std::mem::take(&mut song.arrangements);

        let rest = match near {
            Some(near) if *near.rest == *song => Arc::clone(&near.rest),
            _ => {
                let rest = song.clone();
                added += weigh_rest(&rest);
                Arc::new(rest)
            }
        };
        let kept_arrangements = match near {
            Some(near) if *near.arrangements == arrangements => Arc::clone(&near.arrangements),
            _ => {
                added += weigh_arrangements(&arrangements);
                Arc::new(arrangements.clone())
            }
        };

        // A track is matched by its place first, then by its id, so adding
        // or removing one track does not make every track after it new.
        let mut by_id: Option<HashMap<&str, &Arc<TrackPart>>> = None;
        let mut parts = Vec::with_capacity(tracks.len());
        added += tracks.len() * size_of::<Arc<TrackPart>>();
        for (i, track) in tracks.iter_mut().enumerate() {
            let old = near.and_then(|near| {
                near.tracks
                    .get(i)
                    .filter(|part| part.settings.id == track.id)
                    .or_else(|| {
                        by_id
                            .get_or_insert_with(|| {
                                near.tracks
                                    .iter()
                                    .map(|part| (part.settings.id.as_str(), part))
                                    .collect()
                            })
                            .get(track.id.as_str())
                            .copied()
                    })
            });
            parts.push(TrackPart::take(track, old, &mut added));
        }

        song.tracks = tracks;
        song.arrangements = arrangements;
        let snapshot = Self {
            rest,
            arrangements: kept_arrangements,
            tracks: parts,
        };
        (snapshot, added)
    }

    /// The song this snapshot was taken from.
    pub fn to_project(&self) -> Project {
        let mut song = (*self.rest).clone();
        song.arrangements = (*self.arrangements).clone();
        song.tracks = self
            .tracks
            .iter()
            .map(|part| {
                let mut track = (*part.settings).clone();
                track.clips = part.clips.iter().map(|c| (**c).clone()).collect();
                track.automation_clips = part
                    .automation_clips
                    .iter()
                    .map(|c| (**c).clone())
                    .collect();
                track
            })
            .collect();
        song
    }

    /// Roughly how many bytes dropping this snapshot gives back now: the
    /// parts no other snapshot shares. Each part is counted once by the
    /// snapshot that made it and once by the last one to let it go, so
    /// adding up [`Snapshot::take`] and this keeps a true running total.
    pub fn bytes_freed_by_drop(&self) -> usize {
        let mut freed = size_of::<Snapshot>() + self.tracks.len() * size_of::<Arc<TrackPart>>();
        if Arc::strong_count(&self.rest) == 1 {
            freed += weigh_rest(&self.rest);
        }
        if Arc::strong_count(&self.arrangements) == 1 {
            freed += weigh_arrangements(&self.arrangements);
        }
        for part in &self.tracks {
            if Arc::strong_count(part) == 1 {
                freed += part.bytes_freed_by_drop();
            }
        }
        freed
    }
}

impl TrackPart {
    /// Keep `track` as it is now, sharing what is equal in `old`. The
    /// whole part is `old`'s when nothing on the track changed.
    fn take(track: &mut Track, old: Option<&Arc<TrackPart>>, added: &mut usize) -> Arc<Self> {
        let clips = std::mem::take(&mut track.clips);
        let automation_clips = std::mem::take(&mut track.automation_clips);
        let part = match old {
            Some(old)
                if *old.settings == *track
                    && same_list(&old.clips, &clips)
                    && same_list(&old.automation_clips, &automation_clips) =>
            {
                Arc::clone(old)
            }
            _ => {
                let settings = match old {
                    Some(old) if *old.settings == *track => Arc::clone(&old.settings),
                    _ => {
                        *added += weigh_settings(track);
                        Arc::new(track.clone())
                    }
                };
                let part = TrackPart {
                    settings,
                    clips: share_list(
                        &clips,
                        old.map(|o| o.clips.as_slice()),
                        clip_id,
                        weigh_clip,
                        added,
                    ),
                    automation_clips: share_list(
                        &automation_clips,
                        old.map(|o| o.automation_clips.as_slice()),
                        |c| c.id.as_str(),
                        weigh_automation_clip,
                        added,
                    ),
                };
                *added += part.own_bytes();
                Arc::new(part)
            }
        };
        track.clips = clips;
        track.automation_clips = automation_clips;
        part
    }

    /// The part itself: its lists of pointers.
    fn own_bytes(&self) -> usize {
        size_of::<TrackPart>()
            + self.clips.len() * size_of::<Arc<ClipPlacement>>()
            + self.automation_clips.len() * size_of::<Arc<AutomationClip>>()
    }

    fn bytes_freed_by_drop(&self) -> usize {
        let mut freed = self.own_bytes();
        if Arc::strong_count(&self.settings) == 1 {
            freed += weigh_settings(&self.settings);
        }
        for clip in &self.clips {
            if Arc::strong_count(clip) == 1 {
                freed += weigh_clip(clip);
            }
        }
        for clip in &self.automation_clips {
            if Arc::strong_count(clip) == 1 {
                freed += weigh_automation_clip(clip);
            }
        }
        freed
    }
}

/// Whether two kept items are equal: the same item, or equal ones.
pub(crate) fn same<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

/// Whether two kept lists hold equal items, in order. Items shared by
/// the two snapshots are told equal by their pointers alone.
pub(crate) fn same_items<T: PartialEq>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
}

/// Whether two kept tracks are equal.
pub(crate) fn same_track(a: &Arc<TrackPart>, b: &Arc<TrackPart>) -> bool {
    Arc::ptr_eq(a, b)
        || (same(&a.settings, &b.settings)
            && same_items(&a.clips, &b.clips)
            && same_items(&a.automation_clips, &b.automation_clips))
}

/// Whether `kept` holds exactly the items of `live`, in order.
fn same_list<T: PartialEq>(kept: &[Arc<T>], live: &[T]) -> bool {
    kept.len() == live.len() && kept.iter().zip(live).all(|(k, l)| **k == *l)
}

/// `live` as shared items: each one equal to an item of `old` (at the same
/// place, or else with the same id) is that item.
fn share_list<T: PartialEq + Clone>(
    live: &[T],
    old: Option<&[Arc<T>]>,
    id: fn(&T) -> &str,
    weigh: fn(&T) -> usize,
    added: &mut usize,
) -> Vec<Arc<T>> {
    let old = old.unwrap_or(&[]);
    let mut by_id: Option<HashMap<&str, &Arc<T>>> = None;
    live.iter()
        .enumerate()
        .map(|(i, item)| {
            if let Some(same) = old.get(i).filter(|o| ***o == *item) {
                return Arc::clone(same);
            }
            let by_id = by_id.get_or_insert_with(|| old.iter().map(|o| (id(&**o), o)).collect());
            match by_id.get(id(item)) {
                Some(same) if ***same == *item => Arc::clone(same),
                _ => {
                    *added += weigh(item);
                    Arc::new(item.clone())
                }
            }
        })
        .collect()
}

pub(crate) fn clip_id(clip: &ClipPlacement) -> &str {
    match &clip.content {
        ClipContent::Audio(audio) => &audio.id,
        ClipContent::Midi(midi) => &midi.id,
    }
}

// Rough weights in bytes: the size of each value plus what grows with the
// song (notes, points, names, plug-in state). They only have to be close
// enough to keep the history under its budget; the measurement in
// hardwave-engine's fl_song_cost test checks them against an allocator.

/// What an `Arc` adds around its value: the two counts.
const ARC: usize = 2 * size_of::<usize>();

fn weigh_clip(clip: &ClipPlacement) -> usize {
    let content = match &clip.content {
        ClipContent::Audio(audio) => {
            audio.id.len()
                + audio.name.len()
                + audio.source_path.len()
                + audio.source_hash.len()
                + audio.source_file.len()
                + std::mem::size_of_val(audio.warp_markers.as_slice())
                + std::mem::size_of_val(audio.tuned_notes.as_slice())
        }
        ClipContent::Midi(midi) => {
            midi.id.len()
                + midi.clip.id.len()
                + midi.clip.name.len()
                + std::mem::size_of_val(midi.clip.notes.as_slice())
                + std::mem::size_of_val(midi.clip.controls.as_slice())
        }
    };
    ARC + size_of::<ClipPlacement>()
        + clip.track_id.len()
        + clip.color.as_ref().map_or(0, String::len)
        + content
}

fn weigh_automation_clip(clip: &AutomationClip) -> usize {
    ARC + size_of::<AutomationClip>()
        + clip.id.len()
        + clip.lane.id.len()
        + std::mem::size_of_val(clip.lane.points.as_slice())
}

/// A track without its clips.
fn weigh_settings(track: &Track) -> usize {
    let inserts: usize = track
        .inserts
        .iter()
        .map(|slot| {
            size_of_val(slot)
                + slot.id.len()
                + slot.plugin_id.len()
                + slot.state.as_ref().map_or(0, Vec::len)
        })
        .sum();
    let lanes: usize = track
        .automation_lanes
        .iter()
        .map(|lane| size_of_val(lane) + lane.id.len() + size_of_val(lane.points.as_slice()))
        .sum();
    let slots: usize = track
        .session_slots
        .iter()
        .map(|slot| {
            size_of_val(slot)
                + slot.as_ref().map_or(0, |s| {
                    s.name.len()
                        + s.source_path.len()
                        + s.notes.as_ref().map_or(0, |n| size_of_val(n.as_slice()))
                })
        })
        .sum();
    ARC + size_of::<Track>()
        + track.id.len()
        + track.name.len()
        + track.color.len()
        + track.filter_type.len()
        + inserts
        + lanes
        + slots
        + size_of_val(track.sends.as_slice())
        + size_of_val(track.midi_fx.as_slice())
        + track
            .midi_route_to
            .iter()
            .map(|id| 24 + id.len())
            .sum::<usize>()
}

/// The song without its tracks and arrangements. The channel rack's JSON
/// is counted twice: the step rack read from it holds about as much.
fn weigh_rest(song: &Project) -> usize {
    let text = |s: &Option<String>| s.as_ref().map_or(0, String::len);
    ARC + size_of::<Project>()
        + 2 * text(&song.channel_rack_state)
        + text(&song.midi_mappings)
        + text(&song.timeline_state)
        + song
            .plugin_states
            .iter()
            .map(|p| size_of_val(p) + p.plugin_instance_id.len() + p.chunk.len())
            .sum::<usize>()
        + song
            .rows
            .iter()
            .map(|r| size_of_val(r) + r.name.len() + r.color.as_ref().map_or(0, String::len))
            .sum::<usize>()
        + size_of_val(song.tempo_map.entries.as_slice())
        + size_of_val(song.macros.as_slice())
        + size_of_val(song.vcas.as_slice())
        + size_of_val(song.modulations.as_slice())
        + song.scenes.iter().map(|s| 24 + s.len()).sum::<usize>()
        + song.rack_groups.iter().map(|s| 24 + s.len()).sum::<usize>()
}

fn weigh_arrangements(arrangements: &[Arrangement]) -> usize {
    ARC + arrangements
        .iter()
        .map(|a| {
            size_of_val(a)
                + a.id.len()
                + a.name.len()
                + size_of_val(a.rows.as_slice())
                + a.timelines
                    .iter()
                    .map(|(id, timeline)| {
                        // A map entry: the key, the lists and the tree's own share.
                        64 + id.len()
                            + timeline.clips.iter().map(weigh_clip).sum::<usize>()
                            + timeline
                                .automation_clips
                                .iter()
                                .map(weigh_automation_clip)
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        })
        .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{AudioClip, MidiClipRef};

    fn midi_clip(id: &str, track: &str, at: u64) -> ClipPlacement {
        let mut clip = hardwave_midi::MidiClip::new(format!("m-{id}"), "Lead".into(), 3840);
        clip.notes.push(hardwave_midi::MidiNote::default());
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: id.into(),
                clip,
            }),
            track_id: track.into(),
            position_ticks: at,
            length_ticks: 3840,
            lane: 0,
            color: None,
            row: Some(0),
        }
    }

    /// A song like an imported one: a few tracks with many clips among
    /// the empty inserts.
    fn song() -> Project {
        let mut song = Project::default();
        for t in 1..=4 {
            let id = song.tracks[t].id.clone();
            for c in 0..50 {
                let clip = midi_clip(&format!("c{t}-{c}"), &id, c as u64 * 3840);
                song.tracks[t].clips.push(clip);
            }
        }
        song
    }

    fn same(a: &Project, b: &Project) -> bool {
        a.to_bytes().unwrap() == b.to_bytes().unwrap() && a == b
    }

    #[test]
    fn a_snapshot_gives_back_the_song_it_was_taken_from() {
        let mut song = song();
        song.tracks[2].automation_clips.push(AutomationClip::new(
            "a1",
            crate::automation::AutomationTarget::TrackVolume,
            0,
            3840,
        ));
        song.channel_rack_state = Some("{\"patterns\":[]}".into());
        song.ensure_arrangements();
        let before = song.clone();
        let (snap, _) = Snapshot::take(&mut song, None);
        assert!(
            same(&song, &before),
            "taking a snapshot leaves the song as it was"
        );
        assert!(same(&snap.to_project(), &before));
    }

    #[test]
    fn an_unchanged_song_shares_every_part() {
        let mut song = song();
        let (first, whole) = Snapshot::take(&mut song, None);
        let (second, added) = Snapshot::take(&mut song, Some(&first));
        assert!(Arc::ptr_eq(&first.rest, &second.rest));
        assert!(first
            .tracks
            .iter()
            .zip(&second.tracks)
            .all(|(a, b)| Arc::ptr_eq(a, b)));
        // Only the list of tracks is new.
        assert!(added < 8 * 1024, "{added} bytes");
        assert!(whole > 20 * added, "{whole} against {added}");
    }

    #[test]
    fn a_moved_clip_is_the_only_new_clip() {
        let mut song = song();
        let (first, _) = Snapshot::take(&mut song, None);
        song.tracks[3].clips[10].position_ticks += 96;
        let (second, _) = Snapshot::take(&mut song, Some(&first));
        let (a, b) = (&first.tracks[3], &second.tracks[3]);
        assert!(!Arc::ptr_eq(a, b), "the track changed");
        assert!(
            Arc::ptr_eq(&a.settings, &b.settings),
            "its settings did not"
        );
        let new: Vec<usize> = (0..50)
            .filter(|&i| !Arc::ptr_eq(&a.clips[i], &b.clips[i]))
            .collect();
        assert_eq!(new, vec![10]);
        assert!(Arc::ptr_eq(&first.tracks[2], &second.tracks[2]));
        assert!(same(&second.to_project(), &song));
        assert_eq!(
            first.to_project().tracks[3].clips[10].position_ticks,
            10 * 3840
        );
    }

    #[test]
    fn deleted_clips_and_a_removed_track_leave_the_rest_shared() {
        let mut song = song();
        let (first, _) = Snapshot::take(&mut song, None);
        song.tracks[1].clips.drain(0..20);
        let removed = song.tracks.remove(2);
        let (second, _) = Snapshot::take(&mut song, Some(&first));
        // Every clip left on track 1 is the one kept before, though each
        // moved up twenty places.
        let kept = &first.tracks[1].clips;
        assert!(second.tracks[1]
            .clips
            .iter()
            .zip(&kept[20..])
            .all(|(a, b)| Arc::ptr_eq(a, b)));
        // Tracks after the removed one are found by id.
        assert!(Arc::ptr_eq(&first.tracks[3], &second.tracks[2]));
        assert!(Arc::ptr_eq(&first.tracks[400], &second.tracks[399]));
        assert!(same(&second.to_project(), &song));
        let mut back = first.to_project();
        assert!(back.tracks.iter().any(|t| t.id == removed.id));
        back.tracks.remove(2);
        back.tracks[1].clips.drain(0..20);
        assert!(same(&back, &song));
    }

    #[test]
    fn a_changed_clip_with_the_same_id_is_kept_as_it_now_is() {
        let mut song = song();
        let track_id = song.tracks[1].id.clone();
        song.tracks[1].clips.push(ClipPlacement {
            content: ClipContent::Audio(AudioClip {
                id: "audio".into(),
                name: "Kick".into(),
                source_path: "kick.wav".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 44_100,
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
            }),
            track_id,
            position_ticks: 0,
            length_ticks: 960,
            lane: 0,
            color: None,
            row: None,
        });
        let (first, _) = Snapshot::take(&mut song, None);
        // Moved to the front and changed: found by id, but not equal.
        let mut clip = song.tracks[1].clips.pop().unwrap();
        if let ClipContent::Audio(audio) = &mut clip.content {
            audio.gain_db = -3.0;
        }
        song.tracks[1].clips.insert(0, clip);
        let (second, _) = Snapshot::take(&mut song, Some(&first));
        assert!(same(&second.to_project(), &song));
        let gain = |clip: &ClipPlacement| match &clip.content {
            ClipContent::Audio(audio) => audio.gain_db,
            ClipContent::Midi(_) => unreachable!(),
        };
        assert_eq!(gain(&second.tracks[1].clips[0]), -3.0);
        assert_eq!(gain(&first.tracks[1].clips[50]), 0.0);
        assert!(Arc::ptr_eq(
            &first.tracks[1].clips[0],
            &second.tracks[1].clips[1]
        ));
    }

    #[test]
    fn the_bytes_given_back_add_up_to_the_bytes_taken() {
        let mut song = song();
        let mut steps = Vec::new();
        let mut total = 0usize;
        for n in 0..10u64 {
            let (snap, added) = Snapshot::take(&mut song, steps.last());
            total += added;
            steps.push(snap);
            song.tracks[1 + (n as usize % 3)].clips[n as usize].position_ticks += 1;
            if n == 5 {
                song.tracks[7].volume_db = -6.0;
                song.rows.push(Default::default());
            }
        }
        // Dropped in any order, every part is given back exactly once.
        for i in [3, 0, 7, 1, 6, 2, 5, 4, 1, 0] {
            let snap = steps.remove(i.min(steps.len() - 1));
            total -= snap.bytes_freed_by_drop();
            drop(snap);
        }
        assert!(steps.is_empty());
        assert_eq!(total, 0);
    }
}
