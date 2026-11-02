//! Where things show: FL's three lists.
//!
//! An FL Studio song is shown as three lists: the channel rack (what
//! plays), the playlist (rows of clips) and the mixer (numbered inserts).
//! A clip stays on the track that plays it, so the engine, undo,
//! arrangements and rooms read clips exactly as before; what is added
//! here only says where each thing is drawn. None of it changes the
//! sound, except a row's mute and solo, which silence the clips on it.

use serde::{Deserialize, Serialize};

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
}
