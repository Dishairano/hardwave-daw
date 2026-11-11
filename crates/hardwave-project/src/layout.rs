//! Where things show: FL's three lists.
//!
//! An FL Studio song is shown as three lists: the channel rack (what
//! plays), the playlist (rows of clips) and the mixer (numbered inserts).
//! A clip stays on the track that plays it, so the engine, undo,
//! arrangements and rooms read clips exactly as before; what is added
//! here only says where each thing is drawn. None of it changes the
//! sound, except a row's mute and solo, which silence the clips on it.

use serde::{Deserialize, Serialize};

use crate::automation::AutomationLane;
use crate::automation_clip::AutomationClip;
use crate::clip::ClipPlacement;
use crate::track::{Track, TrackKind};

/// One row of the playlist: FL's playlist track. Rows hold no sound of
/// their own; a clip names the row it sits on (`ClipPlacement::row`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlaylistRow {
    /// Empty reads "Track N", as FL shows a row nobody named.
    #[serde(default)]
    pub name: String,
    /// `#rrggbb`, or none for the default.
    #[serde(default)]
    pub color: Option<String>,
    /// FL's track switch: every clip on the row is silent. The channels'
    /// effects and sends keep running.
    #[serde(default)]
    pub muted: bool,
    /// While any row is soloed, only soloed rows play.
    #[serde(default)]
    pub soloed: bool,
}

/// Where the playlist draws one track's things: a row for each of its
/// clips, automation clips and automation lanes, in the order they are
/// listed on the track.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShownRows {
    pub clips: Vec<u32>,
    pub automation_clips: Vec<u32>,
    pub lanes: Vec<u32>,
}

/// The row most of these clips sit on, the lowest of equals; none when no
/// clip has a row.
fn most_used_row(clips: &[ClipPlacement]) -> Option<u32> {
    let mut counts = std::collections::BTreeMap::<u32, usize>::new();
    for row in clips.iter().filter_map(|c| c.row) {
        *counts.entry(row).or_default() += 1;
    }
    let mut best: Option<(u32, usize)> = None;
    for (row, count) in counts {
        if best.is_none_or(|(_, most)| count > most) {
            best = Some((row, count));
        }
    }
    best.map(|(row, _)| row)
}

/// Each track's place in the playlist: its position among the tracks that
/// are not the master (none for the master). A song built row by row (a
/// new song, a starter, a template) has each track's clips on this row.
fn track_places(tracks: &[Track]) -> Vec<Option<u32>> {
    let mut next = 0u32;
    tracks
        .iter()
        .map(|t| {
            if matches!(t.kind, TrackKind::Master) {
                None
            } else {
                next += 1;
                Some(next - 1)
            }
        })
        .collect()
}

/// Where the playlist draws everything on these tracks (the open
/// arrangement's), one entry per track.
///
/// The one rule for a thing with no row of its own (`row: None`), shared
/// by the playlist, the engine's row mute and solo, and `settle_rows`: a
/// clip is drawn on the row most of its channel's other clips sit on, else
/// on its track's place in the playlist; an automation clip or lane, or a
/// clip of the master, on the first row nothing else is on, in track
/// order. The playlist's row index (`rowIndex.ts`, `buildRowIndex`) gets
/// these rows from the app and follows the same rule for anything else,
/// so the two cannot disagree.
pub fn shown_rows(tracks: &[Track]) -> Vec<ShownRows> {
    if !any_unplaced(tracks) {
        // Every song once its rows are settled: each on its own row.
        return tracks
            .iter()
            .map(|t| ShownRows {
                clips: t.clips.iter().filter_map(|c| c.row).collect(),
                automation_clips: t.automation_clips.iter().filter_map(|c| c.row).collect(),
                lanes: t.automation_lanes.iter().filter_map(|l| l.row).collect(),
            })
            .collect();
    }
    #[derive(Clone, Copy)]
    enum Part {
        Clip,
        AutomationClip,
        Lane,
    }
    let places = track_places(tracks);
    let mut taken = std::collections::HashSet::<u32>::new();
    // What still needs the first empty row once everything with a place
    // of its own is in, in the order the playlist hands those rows out.
    let mut homeless: Vec<(usize, Part, usize)> = Vec::new();
    let mut out: Vec<ShownRows> = Vec::with_capacity(tracks.len());
    for (t, track) in tracks.iter().enumerate() {
        let home = most_used_row(&track.clips).or(places[t]);
        let mut place = |row: Option<u32>, part: Part, i: usize| match row {
            Some(r) => {
                taken.insert(r);
                r
            }
            None => {
                homeless.push((t, part, i));
                0
            }
        };
        let shown = ShownRows {
            clips: (track.clips.iter().enumerate())
                .map(|(i, c)| place(c.row.or(home), Part::Clip, i))
                .collect(),
            automation_clips: (track.automation_clips.iter().enumerate())
                .map(|(i, c)| place(c.row, Part::AutomationClip, i))
                .collect(),
            lanes: (track.automation_lanes.iter().enumerate())
                .map(|(i, l)| place(l.row, Part::Lane, i))
                .collect(),
        };
        out.push(shown);
    }
    let mut next = 0u32;
    for (t, part, i) in homeless {
        while taken.contains(&next) {
            next += 1;
        }
        taken.insert(next);
        let slot = match part {
            Part::Clip => &mut out[t].clips[i],
            Part::AutomationClip => &mut out[t].automation_clips[i],
            Part::Lane => &mut out[t].lanes[i],
        };
        *slot = next;
    }
    out
}

/// Whether anything on these tracks has no row of its own.
fn any_unplaced(tracks: &[Track]) -> bool {
    tracks.iter().any(|t| {
        t.clips.iter().any(|c| c.row.is_none())
            || t.automation_clips.iter().any(|c| c.row.is_none())
            || t.automation_lanes.iter().any(|l| l.row.is_none())
    })
}

/// Which playlist rows are silent while the song plays: the muted ones,
/// and while any row is soloed every row that is not. Worked out once
/// when the engine builds its graph. Something with no row of its own is
/// judged by the row the playlist draws it on (`shown_rows`).
#[derive(Debug, Default)]
pub struct SilentRows {
    /// Per row, from the top; empty when no row is muted or soloed.
    silent: Vec<bool>,
    /// A row past the end of the list is silent while any row is soloed.
    any_solo: bool,
    /// The rows things without one of their own are drawn on, by id:
    /// clips, automation clips and lanes. Filled only while a row is
    /// muted or soloed.
    unplaced_clips: std::collections::HashMap<String, u32>,
    unplaced_automation_clips: std::collections::HashMap<String, u32>,
    unplaced_lanes: std::collections::HashMap<String, u32>,
}

impl SilentRows {
    /// Nothing is silent (and nothing is allocated) unless a row is muted
    /// or soloed.
    pub fn of(project: &crate::Project) -> Self {
        let rows = &project.rows;
        let any_solo = rows.iter().any(|r| r.soloed);
        if !any_solo && !rows.iter().any(|r| r.muted) {
            return Self::default();
        }
        let mut silent = Self {
            silent: rows
                .iter()
                .map(|r| r.muted || (any_solo && !r.soloed))
                .collect(),
            any_solo,
            ..Self::default()
        };
        // A song made in a build that kept no rows for its clips (every
        // clip of a starter, an import or a recording): each is silent or
        // not by the row it is drawn on, as the playlist shows it.
        if any_unplaced(&project.tracks) {
            let shown = shown_rows(&project.tracks);
            for (track, rows) in project.tracks.iter().zip(&shown) {
                for (clip, row) in track.clips.iter().zip(&rows.clips) {
                    if clip.row.is_none() {
                        let id = crate::row_edit::clip_id(clip).to_string();
                        silent.unplaced_clips.insert(id, *row);
                    }
                }
                for (clip, row) in track.automation_clips.iter().zip(&rows.automation_clips) {
                    if clip.row.is_none() {
                        silent
                            .unplaced_automation_clips
                            .insert(clip.id.clone(), *row);
                    }
                }
                for (lane, row) in track.automation_lanes.iter().zip(&rows.lanes) {
                    if lane.row.is_none() {
                        silent.unplaced_lanes.insert(lane.id.clone(), *row);
                    }
                }
            }
        }
        silent
    }

    /// Whether what sits on `row` stays quiet.
    fn row(&self, row: Option<u32>) -> bool {
        match row {
            Some(r) => self
                .silent
                .get(r as usize)
                .copied()
                .unwrap_or(self.any_solo),
            None => false,
        }
    }

    /// Whether this clip of the song's tracks stays quiet. A clip that is
    /// not one of them (the rack's pattern in pattern mode) never does.
    pub fn clip(&self, clip: &ClipPlacement) -> bool {
        self.row(clip.row.or_else(|| {
            self.unplaced_clips
                .get(crate::row_edit::clip_id(clip))
                .copied()
        }))
    }

    /// Whether this automation clip stays still.
    pub fn automation_clip(&self, clip: &AutomationClip) -> bool {
        self.row(
            clip.row
                .or_else(|| self.unplaced_automation_clips.get(&clip.id).copied()),
        )
    }

    /// Whether this automation lane stays still.
    pub fn lane(&self, lane: &AutomationLane) -> bool {
        self.row(
            lane.row
                .or_else(|| self.unplaced_lanes.get(&lane.id).copied()),
        )
    }
}

/// What a track is in FL's three lists, from its kind and its two display
/// fields (`mixer_number`, `rack_group`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRole {
    /// Something that plays, listed in the channel rack, playing into an
    /// insert or the master.
    Channel,
    /// A numbered insert in the mixer: effects, a fader and sends.
    Insert,
    /// The master, insert 0.
    Master,
    /// A channel with an insert of its own: one track in both lists. Every
    /// track of a song made before the three lists is one of these.
    ChannelWithInsert,
}

impl Track {
    /// What this track is in FL's three lists.
    pub fn role(&self) -> TrackRole {
        if matches!(self.kind, TrackKind::Master) {
            return TrackRole::Master;
        }
        match (self.rack_group.is_some(), self.mixer_number.is_some()) {
            (true, true) => TrackRole::ChannelWithInsert,
            (true, false) => TrackRole::Channel,
            (false, _) => TrackRole::Insert,
        }
    }
}

/// The rack's one group in a song made here, as FL names its own.
pub const DEFAULT_RACK_GROUP: &str = "Unsorted";

/// Today's playlist rows for one arrangement: every track but the master
/// gets a row with its name and colour, then one row per automation lane
/// and one per automation clip under it. `automation_clips` says how many
/// automation clips a track has in this arrangement. Returns the rows and
/// each track's own row (none for the master).
fn rows_as_before(
    tracks: &[Track],
    automation_clips: impl Fn(&Track) -> usize,
) -> (Vec<PlaylistRow>, Vec<Option<u32>>) {
    let mut rows = Vec::new();
    let mut row_of = Vec::with_capacity(tracks.len());
    for track in tracks {
        if matches!(track.kind, TrackKind::Master) {
            row_of.push(None);
            continue;
        }
        row_of.push(Some(rows.len() as u32));
        rows.push(PlaylistRow {
            name: track.name.clone(),
            color: Some(track.color.clone()),
            ..PlaylistRow::default()
        });
        let under = track.automation_lanes.len() + automation_clips(track);
        rows.extend(std::iter::repeat_n(PlaylistRow::default(), under));
    }
    (rows, row_of)
}

impl crate::Project {
    /// Fill in where everything shows so the playlist, the mixer and the
    /// rack look exactly as they did before FL's three lists: for a song
    /// saved before them (format 2 and older), and for a new song.
    ///
    /// Only display fields are written. Volumes, routes, sends, plug-ins,
    /// clip positions and audio are not touched, so the song plays as it
    /// did, sample for sample.
    ///
    /// - The mixer counts as it did: the master is 0, then every track
    ///   that is not an automation track 1, 2, 3 in list order.
    /// - The rack has one group, "Unsorted", and lists every audio, MIDI
    ///   or automation track it listed before (anything but the numbered
    ///   inserts, and MIDI tracks wherever they sit) or that holds clips
    ///   in any arrangement. Buses, returns and the master are not
    ///   channels.
    /// - The rows are the playlist's, per arrangement: each track's own
    ///   row with all its clips on it, then a row per automation lane and
    ///   per automation clip.
    pub fn lay_out_as_before(&mut self) {
        let mut next = 1u16;
        let mut master_seen = false;
        for track in &mut self.tracks {
            track.mixer_number = match track.kind {
                TrackKind::Master if !master_seen => {
                    master_seen = true;
                    Some(0)
                }
                TrackKind::Master | TrackKind::Automation => None,
                _ => {
                    let n = next;
                    next = next.saturating_add(1);
                    Some(n)
                }
            };
        }

        let mut holds_clips: std::collections::HashSet<String> = self
            .tracks
            .iter()
            .filter(|t| !t.clips.is_empty())
            .map(|t| t.id.clone())
            .collect();
        for arrangement in &self.arrangements {
            for (id, timeline) in &arrangement.timelines {
                if !timeline.clips.is_empty() {
                    holds_clips.insert(id.clone());
                }
            }
        }
        self.rack_groups = vec![DEFAULT_RACK_GROUP.to_string()];
        for track in &mut self.tracks {
            let listed = !track.id.starts_with("insert-") || matches!(track.kind, TrackKind::Midi);
            track.rack_group = match track.kind {
                TrackKind::Audio | TrackKind::Midi | TrackKind::Automation
                    if listed || holds_clips.contains(&track.id) =>
                {
                    Some(0)
                }
                _ => None,
            };
        }

        // The open arrangement, on the live tracks.
        let (rows, row_of) = rows_as_before(&self.tracks, |t| t.automation_clips.len());
        for (track, row) in self.tracks.iter_mut().zip(&row_of) {
            let Some(row) = *row else { continue };
            for clip in &mut track.clips {
                clip.row = Some(row);
            }
            let mut under = row + 1;
            for lane in &mut track.automation_lanes {
                lane.row = Some(under);
                under += 1;
            }
            for clip in &mut track.automation_clips {
                clip.row = Some(under);
                under += 1;
            }
        }
        self.rows = rows;

        // Every stored arrangement, from its own timelines. A track's
        // automation lanes belong to the track, not to an arrangement, so
        // they keep the open arrangement's rows; each arrangement still
        // leaves room for them, as the playlist did.
        let place: std::collections::HashMap<&str, usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), i))
            .collect();
        for arrangement in &mut self.arrangements {
            let (rows, row_of) = rows_as_before(&self.tracks, |t| {
                arrangement
                    .timelines
                    .get(&t.id)
                    .map_or(0, |tl| tl.automation_clips.len())
            });
            for (id, timeline) in arrangement.timelines.iter_mut() {
                let Some(&index) = place.get(id.as_str()) else {
                    continue;
                };
                let Some(row) = row_of[index] else { continue };
                for clip in &mut timeline.clips {
                    clip.row = Some(row);
                }
                let first = row + 1 + self.tracks[index].automation_lanes.len() as u32;
                for (under, clip) in (first..).zip(timeline.automation_clips.iter_mut()) {
                    clip.row = Some(under);
                }
            }
            arrangement.rows = rows;
        }
    }

    /// Put everything on the open arrangement's tracks that has no row of
    /// its own on the row the playlist draws it on (`shown_rows`), as FL
    /// gives every clip the row it was put on. Nothing moves on screen and
    /// nothing sounds different; from then on the clip stays on its row
    /// when its channel's other clips move, and row edits (insert, delete,
    /// mute, solo) treat it like any other. Returns how many got a row.
    pub fn settle_rows(&mut self) -> usize {
        if !any_unplaced(&self.tracks) {
            return 0;
        }
        let shown = shown_rows(&self.tracks);
        let mut n = 0;
        let mut settle = |slot: &mut Option<u32>, row: u32| {
            if slot.is_none() {
                *slot = Some(row);
                n += 1;
            }
        };
        for (track, rows) in self.tracks.iter_mut().zip(shown) {
            for (clip, row) in track.clips.iter_mut().zip(rows.clips) {
                settle(&mut clip.row, row);
            }
            for (clip, row) in track.automation_clips.iter_mut().zip(rows.automation_clips) {
                settle(&mut clip.row, row);
            }
            for (lane, row) in track.automation_lanes.iter_mut().zip(rows.lanes) {
                settle(&mut lane.row, row);
            }
        }
        n
    }

    /// Clear every field FL's three lists added: what a song read from
    /// format 2 holds.
    pub(crate) fn forget_layout(&mut self) {
        self.rows.clear();
        self.rack_groups.clear();
        let forget =
            |clips: &mut [crate::clip::ClipPlacement],
             automation: &mut [crate::automation_clip::AutomationClip]| {
                for clip in clips {
                    clip.row = None;
                }
                for clip in automation {
                    clip.row = None;
                    clip.target_track = None;
                }
            };
        for track in &mut self.tracks {
            track.mixer_number = None;
            track.rack_group = None;
            for lane in &mut track.automation_lanes {
                lane.row = None;
            }
            forget(&mut track.clips, &mut track.automation_clips);
        }
        for arrangement in &mut self.arrangements {
            arrangement.rows.clear();
            for timeline in arrangement.timelines.values_mut() {
                forget(&mut timeline.clips, &mut timeline.automation_clips);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_is_one_of_four_parts() {
        let mut t = Track::new_audio("a".into(), "A".into());
        t.rack_group = Some(0);
        assert_eq!(t.role(), TrackRole::Channel);
        t.mixer_number = Some(4);
        assert_eq!(t.role(), TrackRole::ChannelWithInsert);
        t.rack_group = None;
        assert_eq!(t.role(), TrackRole::Insert);
        let mut m = Track::new_master("m".into());
        m.mixer_number = Some(0);
        assert_eq!(m.role(), TrackRole::Master);
    }

    use crate::arrangement::{Arrangement, TrackTimeline};
    use crate::automation::{AutomationLane, AutomationTarget};
    use crate::automation_clip::AutomationClip;
    use crate::clip::{ClipContent, ClipPlacement, MidiClipRef};
    use crate::Project;

    fn clip(track: &str, at: u64) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: format!("{track}-{at}"),
                clip: hardwave_midi::MidiClip::new(format!("{track}-{at}"), "c".into(), 960),
            }),
            track_id: track.into(),
            position_ticks: at,
            length_ticks: 960,
            lane: 0,
            color: None,
            row: None,
        }
    }

    fn lane(id: &str) -> AutomationLane {
        AutomationLane {
            id: id.into(),
            target: AutomationTarget::TrackVolume,
            points: Vec::new(),
            visible: true,
            row: None,
        }
    }

    /// A song as format 2 saved it: a master, an instrument with two clips
    /// and an automation lane, an empty numbered insert, an audio track
    /// with an automation clip, an automation track and a bus.
    fn older_song() -> Project {
        let mut p = Project::default();
        p.tracks.truncate(1);
        let mut lead = Track::new_midi("lead".into(), "Lead".into());
        lead.color = "#123456".into();
        lead.clips = vec![clip("lead", 0), clip("lead", 3840)];
        lead.automation_lanes.push(lane("lead-vol"));
        p.tracks.push(lead);
        p.tracks
            .push(Track::new_audio("insert-002".into(), "Insert 2".into()));
        let mut vox = Track::new_audio("insert-003".into(), "Vox".into());
        vox.clips.push(clip("insert-003", 0));
        vox.automation_clips.push(AutomationClip::new(
            "vox-ac",
            AutomationTarget::TrackPan,
            0,
            960,
        ));
        p.tracks.push(vox);
        p.tracks
            .push(Track::new_automation("auto".into(), "Auto".into()));
        p.tracks.push(Track::new_bus("bus".into(), "Bus".into()));
        p.forget_layout();
        p.version = 2;
        p
    }

    #[test]
    fn an_older_song_is_laid_out_as_the_playlist_mixer_and_rack_showed_it() {
        let p = Project::from_bytes(&older_song().older_format_bytes()).unwrap();
        assert_eq!(p.version, crate::project::FORMAT_VERSION);
        let numbers: Vec<_> = p.tracks.iter().map(|t| t.mixer_number).collect();
        assert_eq!(
            numbers,
            [Some(0), Some(1), Some(2), Some(3), None, Some(4)],
            "the mixer counted every track but the master and automation"
        );
        let groups: Vec<_> = p.tracks.iter().map(|t| t.rack_group).collect();
        assert_eq!(
            groups,
            [None, Some(0), None, Some(0), Some(0), None],
            "the rack listed instruments, named tracks and tracks with clips"
        );
        assert_eq!(p.rack_groups, ["Unsorted"]);
        // Lead, its lane, Insert 2, Vox, its automation clip, Auto, Bus.
        let names: Vec<_> = p.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Lead", "", "Insert 2", "Vox", "", "Auto", "Bus"]);
        assert_eq!(p.rows[0].color.as_deref(), Some("#123456"));
        let lead = p.track("lead").unwrap();
        assert!(lead.clips.iter().all(|c| c.row == Some(0)));
        assert_eq!(lead.automation_lanes[0].row, Some(1));
        let vox = p.track("insert-003").unwrap();
        assert_eq!(vox.clips[0].row, Some(3));
        assert_eq!(vox.automation_clips[0].row, Some(4));
        assert_eq!(vox.automation_clips[0].target_track, None);
        assert!(p.rows.iter().all(|r| !r.muted && !r.soloed));
    }

    #[test]
    fn each_stored_arrangement_is_laid_out_from_its_own_timelines() {
        let mut song = older_song();
        // In the other arrangement Lead has an automation clip too, which
        // took one more row under it and moved every row after it down.
        let mut lead = TrackTimeline::default();
        lead.clips.push(clip("lead", 7680));
        lead.automation_clips.push(AutomationClip::new(
            "ac2",
            AutomationTarget::TrackVolume,
            0,
            960,
        ));
        let mut vox = TrackTimeline::default();
        vox.clips.push(clip("insert-003", 960));
        song.arrangements.push(Arrangement {
            id: "b".into(),
            name: "B".into(),
            timelines: [("lead".to_string(), lead), ("insert-003".to_string(), vox)]
                .into_iter()
                .collect(),
            rows: Vec::new(),
        });
        let p = Project::from_bytes(&song.older_format_bytes()).unwrap();
        let b = &p.arrangements[0];
        assert_eq!(
            b.rows.len(),
            7,
            "Lead, lane, clip, Insert 2, Vox, Auto, Bus"
        );
        assert_eq!(b.rows[4].name, "Vox");
        assert_eq!(b.timelines["lead"].clips[0].row, Some(0));
        assert_eq!(b.timelines["lead"].automation_clips[0].row, Some(2));
        assert_eq!(b.timelines["insert-003"].clips[0].row, Some(4));
        // The open one is the live tracks' layout.
        assert_eq!(p.track("insert-003").unwrap().clips[0].row, Some(3));
    }

    #[test]
    fn a_new_song_shows_its_inserts_numbered_on_their_rows() {
        let p = Project::default();
        assert_eq!(p.rows.len(), crate::project::DEFAULT_INSERT_COUNT);
        assert_eq!(p.rows[0].name, "Insert 1");
        assert_eq!(p.tracks[0].mixer_number, Some(0));
        assert_eq!(p.tracks[1].mixer_number, Some(1));
        assert_eq!(p.tracks[500].mixer_number, Some(500));
        assert!(
            p.tracks.iter().all(|t| t.rack_group.is_none()),
            "the numbered inserts are not channels"
        );
    }

    #[test]
    fn a_saved_song_keeps_its_layout_and_saves_the_same_bytes_again() {
        let mut p = Project::from_bytes(&older_song().older_format_bytes()).unwrap();
        // Laid out differently than an older song would be: a format 3
        // song is never laid out again.
        p.rows[2].name = "Pads".into();
        p.rows[2].muted = true;
        p.tracks[2].mixer_number = Some(9);
        let once = p.to_bytes().unwrap();
        let back = Project::from_bytes(&once).unwrap();
        assert_eq!(back.rows[2].name, "Pads");
        assert!(back.rows[2].muted);
        assert_eq!(back.tracks[2].mixer_number, Some(9));
        assert_eq!(
            back.to_bytes().unwrap(),
            once,
            "save, open, save: the same bytes"
        );
    }

    #[test]
    fn claiming_a_row_keeps_the_inserts_number() {
        let mut p = Project::default();
        let id = p.claim_row(3, "Bass", true);
        let t = p.track(&id).unwrap();
        assert_eq!(t.mixer_number, Some(3));
        assert_eq!(t.rack_group, Some(0), "an instrument is a channel");
        let id = p.claim_row(4, "Vocals", false);
        assert_eq!(p.track(&id).unwrap().mixer_number, Some(4));
        assert_eq!(p.track(&id).unwrap().rack_group, None);
    }

    #[test]
    fn a_row_saved_without_its_switches_reads_as_on() {
        let json = serde_json::json!({ "name": "Drums" });
        let row: PlaylistRow = serde_json::from_value(json).unwrap();
        assert_eq!(row.name, "Drums");
        assert!(!row.muted && !row.soloed);
        assert_eq!(row.color, None);
    }

    /// A new song whose playlist rows are `rows`.
    fn song_with_rows(rows: Vec<PlaylistRow>) -> Project {
        Project {
            rows,
            ..Project::default()
        }
    }

    #[test]
    fn a_muted_row_is_silent_and_a_solo_silences_every_other_row() {
        let row = |muted, soloed| PlaylistRow {
            muted,
            soloed,
            ..PlaylistRow::default()
        };
        let on = |r: Option<u32>| {
            let mut c = clip("x", 0);
            c.row = r;
            c
        };
        let none = SilentRows::of(&song_with_rows(vec![row(false, false), row(false, false)]));
        assert!(!none.clip(&on(Some(0))) && !none.clip(&on(Some(7))));
        let muted = SilentRows::of(&song_with_rows(vec![row(false, false), row(true, false)]));
        assert!(!muted.clip(&on(Some(0))));
        assert!(muted.clip(&on(Some(1))));
        assert!(!muted.clip(&on(Some(9))), "a row past the list is on");
        let solo = SilentRows::of(&song_with_rows(vec![
            row(false, true),
            row(false, false),
            row(true, true),
        ]));
        assert!(!solo.clip(&on(Some(0))));
        assert!(solo.clip(&on(Some(1))));
        assert!(solo.clip(&on(Some(2))), "muted wins over soloed, as in FL");
        assert!(solo.clip(&on(Some(9))), "a row past the list is not soloed");
        assert!(
            !solo.clip(&on(None)),
            "a clip that is not on the song's tracks (the rack's pattern) always plays"
        );
    }

    /// The tracks of `shownRows.cases.json`, the cases the playlist's row
    /// index is tested on too, and the rows it says each thing is drawn on.
    fn shared_cases() -> (Vec<Track>, std::collections::HashMap<String, u32>) {
        let json: serde_json::Value = serde_json::from_str(include_str!(
            "../../../packages/daw-ui/src/components/arrangement/shownRows.cases.json"
        ))
        .unwrap();
        let row = |v: &serde_json::Value| v["row"].as_u64().map(|r| r as u32);
        let id = |v: &serde_json::Value| v["id"].as_str().unwrap().to_string();
        let tracks = json["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                let tid = id(t);
                let mut track = match t["kind"].as_str().unwrap() {
                    "Master" => Track::new_master(tid.clone()),
                    "Midi" => Track::new_midi(tid.clone(), tid.clone()),
                    _ => Track::new_audio(tid.clone(), tid.clone()),
                };
                for c in t["clips"].as_array().unwrap() {
                    let mut placed = clip(&tid, 0);
                    if let ClipContent::Midi(m) = &mut placed.content {
                        m.id = id(c);
                    }
                    placed.row = row(c);
                    track.clips.push(placed);
                }
                for c in t["automationClips"].as_array().unwrap() {
                    let mut ac = AutomationClip::new(id(c), AutomationTarget::TrackVolume, 0, 960);
                    ac.row = row(c);
                    track.automation_clips.push(ac);
                }
                for l in t["automationLanes"].as_array().unwrap() {
                    let mut al = lane(&id(l));
                    al.row = row(l);
                    track.automation_lanes.push(al);
                }
                track
            })
            .collect();
        let shown = json["shown"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as u32))
            .collect();
        (tracks, shown)
    }

    /// Every id on these tracks with the row `shown_rows` gives it.
    fn rows_by_id(tracks: &[Track]) -> std::collections::HashMap<String, u32> {
        let mut out = std::collections::HashMap::new();
        for (track, rows) in tracks.iter().zip(shown_rows(tracks)) {
            for (c, r) in track.clips.iter().zip(rows.clips) {
                out.insert(crate::row_edit::clip_id(c).to_string(), r);
            }
            for (c, r) in track.automation_clips.iter().zip(rows.automation_clips) {
                out.insert(c.id.clone(), r);
            }
            for (l, r) in track.automation_lanes.iter().zip(rows.lanes) {
                out.insert(l.id.clone(), r);
            }
        }
        out
    }

    #[test]
    fn things_without_a_row_are_placed_where_the_playlist_draws_them() {
        let (tracks, shown) = shared_cases();
        assert_eq!(rows_by_id(&tracks), shown);
    }

    #[test]
    fn settling_writes_down_the_rows_things_are_drawn_on() {
        let (tracks, shown) = shared_cases();
        let mut p = Project {
            tracks,
            ..Project::default()
        };
        assert_eq!(p.settle_rows(), 7, "the seven without a row");
        assert_eq!(rows_by_id(&p.tracks), shown, "nothing moved");
        assert!(!any_unplaced(&p.tracks));
        let before = p.clone();
        assert_eq!(p.settle_rows(), 0);
        assert_eq!(p, before, "settling twice changes nothing");
    }

    #[test]
    fn a_muted_row_silences_the_clips_drawn_on_it_that_have_no_row() {
        let (tracks, _) = shared_cases();
        let mut p = Project {
            tracks,
            rows: vec![PlaylistRow::default(); 10],
            ..Project::default()
        };
        let find = |p: &Project, id: &str| {
            p.tracks
                .iter()
                .flat_map(|t| &t.clips)
                .find(|c| crate::row_edit::clip_id(c) == id)
                .unwrap()
                .clone()
        };
        // b's clips have no row and are drawn on its place, row 1.
        p.rows[1].muted = true;
        let silent = SilentRows::of(&p);
        assert!(silent.clip(&find(&p, "b1")) && silent.clip(&find(&p, "b2")));
        assert!(!silent.clip(&find(&p, "a4")), "drawn on row 3");
        // a4 has none and is drawn with most of a's clips, on row 3.
        p.rows[1].muted = false;
        p.rows[3].soloed = true;
        let silent = SilentRows::of(&p);
        assert!(!silent.clip(&find(&p, "a4")) && !silent.clip(&find(&p, "a1")));
        assert!(silent.clip(&find(&p, "a3")) && silent.clip(&find(&p, "b1")));
        let lane_al = p.tracks[1].automation_lanes[0].clone();
        assert!(silent.lane(&lane_al), "the lane is drawn on row 6");
        let bac = p.tracks[2].automation_clips[0].clone();
        assert!(silent.automation_clip(&bac), "drawn on row 8");
        p.rows[3].soloed = false;
        p.rows[8].muted = true;
        assert!(SilentRows::of(&p).automation_clip(&bac));
        assert!(!SilentRows::of(&p).lane(&lane_al));
    }
}
