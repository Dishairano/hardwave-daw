//! Editing the playlist's rows: FL's playlist tracks.
//!
//! A row holds clips of any channel and says nothing about how they sound
//! (see `layout`). Moving a clip to another row, renaming or colouring a
//! row, adding or taking out rows only changes where things are drawn.
//! Only a row's mute and solo change the sound: the engine leaves the
//! clips on a silent row out when it builds its graph.

use crate::clip::{ClipContent, ClipPlacement};
use crate::layout::PlaylistRow;
use crate::sanitize::MAX_ROWS;
use crate::track::{Track, TrackKind};
use crate::Project;

/// A clip's id, whichever kind of clip it is.
pub fn clip_id(clip: &ClipPlacement) -> &str {
    match &clip.content {
        ClipContent::Audio(a) => &a.id,
        ClipContent::Midi(m) => &m.id,
    }
}

/// A row number the song can hold.
pub fn check_row_number(row: u32) -> Result<(), String> {
    if row as usize >= MAX_ROWS {
        return Err(format!("row {} is past the last playlist row", row + 1));
    }
    Ok(())
}

/// `#rrggbb`.
fn is_colour(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Where a new audio clip channel plays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelInsert {
    /// Into this insert, which nothing used yet.
    Route(String),
    /// Into an insert of its own with this number: no insert was free.
    Own(u16),
}

impl Project {
    /// Rows from the top that hold anything (clips, automation clips or
    /// lanes) or have been named, coloured or switched: how far the
    /// playlist is in use.
    pub fn rows_in_use(&self) -> usize {
        let mut used = self
            .rows
            .iter()
            .rposition(|r| *r != PlaylistRow::default())
            .map_or(0, |r| r + 1);
        let mut see = |row: Option<u32>| {
            if let Some(r) = row {
                used = used.max(r as usize + 1);
            }
        };
        for track in &self.tracks {
            track.clips.iter().for_each(|c| see(c.row));
            track.automation_clips.iter().for_each(|c| see(c.row));
            track.automation_lanes.iter().for_each(|l| see(l.row));
        }
        used
    }

    /// The first row with nothing drawn on it (clips, automation clips or
    /// lanes), for a new automation clip or lane that was given no row. A
    /// clip with no row of its own takes the row the playlist draws it on.
    pub fn first_empty_row(&self) -> u32 {
        let taken: std::collections::HashSet<u32> = crate::layout::shown_rows(&self.tracks)
            .into_iter()
            .flat_map(|r| r.clips.into_iter().chain(r.automation_clips).chain(r.lanes))
            .collect();
        let mut row = 0;
        while taken.contains(&row) {
            row += 1;
        }
        row
    }

    /// Move clips to other rows: `(track, clip, row)` each. They stay on
    /// their channel, so they sound as they did unless a row's switch
    /// silences them. Returns how many moved; every clip has to exist.
    pub fn set_clip_rows(&mut self, moves: &[(String, String, u32)]) -> Result<usize, String> {
        self.check_clip_rows(moves)?;
        let mut moved = 0;
        for (track_id, id, row) in moves {
            if let Some(clip) = self
                .track_mut(track_id)
                .and_then(|t| t.clips.iter_mut().find(|c| clip_id(c) == id))
            {
                if clip.row != Some(*row) {
                    clip.row = Some(*row);
                    moved += 1;
                }
            }
        }
        Ok(moved)
    }

    /// Whether every `(track, clip, row)` names a clip that is there and a
    /// row the song can hold.
    pub fn check_clip_rows(&self, moves: &[(String, String, u32)]) -> Result<(), String> {
        for (track_id, id, row) in moves {
            check_row_number(*row)?;
            let track = self
                .track(track_id)
                .ok_or_else(|| format!("Track not found: {track_id}"))?;
            if !track.clips.iter().any(|c| clip_id(c) == id) {
                return Err(format!("Clip not found: {id}"));
            }
        }
        Ok(())
    }

    /// Move an automation clip to another row. It keeps moving the same
    /// knob wherever it sits.
    pub fn set_automation_clip_row(
        &mut self,
        track_id: &str,
        clip_id: &str,
        row: u32,
    ) -> Result<(), String> {
        check_row_number(row)?;
        let clip = self
            .track_mut(track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?
            .automation_clips
            .iter_mut()
            .find(|c| c.id == clip_id)
            .ok_or_else(|| format!("Automation clip not found: {clip_id}"))?;
        clip.row = Some(row);
        Ok(())
    }

    /// The row to change, made when the list does not reach it yet (a row
    /// past the end reads as an unnamed one with its switches on).
    fn row_mut(&mut self, row: u32) -> Result<&mut PlaylistRow, String> {
        check_row_number(row)?;
        let index = row as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, PlaylistRow::default());
        }
        Ok(&mut self.rows[index])
    }

    /// Name a row; an empty name reads "Track N" again.
    pub fn set_row_name(&mut self, row: u32, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.chars().count() > 128 {
            return Err("a row name is at most 128 characters".into());
        }
        self.row_mut(row)?.name = name.to_string();
        Ok(())
    }

    /// Rename a track (a channel or an insert). Returns false when there
    /// is no such track or the name is empty.
    ///
    /// A row is FL's playlist track and keeps a name of its own, but a row
    /// made for this track (a new song's, a starter's or an older song's,
    /// named after the track as `claim_row` and `lay_out_as_before` name
    /// them) that still carries the track's old name follows the rename,
    /// in every arrangement. Such a row is the track's place in the
    /// playlist or a row its clips are drawn on. A row named anything
    /// else, or another track's row of the same name, keeps its name.
    pub fn rename_track(&mut self, track_id: &str, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let Some(index) = self.tracks.iter().position(|t| t.id == track_id) else {
            return false;
        };
        let old = std::mem::replace(&mut self.tracks[index].name, name.to_string());
        if old == name {
            return true;
        }
        let place = crate::layout::track_places(&self.tracks)[index];
        let follow = |rows: &mut [PlaylistRow], drawn: &[u32]| {
            let mine: std::collections::BTreeSet<u32> =
                drawn.iter().copied().chain(place).collect();
            for row in mine {
                if let Some(r) = rows.get_mut(row as usize).filter(|r| r.name == old) {
                    r.name = name.to_string();
                }
            }
        };
        let shown = crate::layout::shown_rows(&self.tracks).swap_remove(index);
        let live: Vec<u32> = (shown.clips.into_iter())
            .chain(shown.automation_clips)
            .chain(shown.lanes)
            .collect();
        follow(&mut self.rows, &live);
        let active = self.active_arrangement.clone();
        for arrangement in &mut self.arrangements {
            // The open arrangement's clips are the live ones.
            let drawn: Vec<u32> = if arrangement.id == active {
                live.clone()
            } else {
                (arrangement.timelines.get(track_id).into_iter())
                    .flat_map(|tl| {
                        (tl.clips.iter().filter_map(|c| c.row))
                            .chain(tl.automation_clips.iter().filter_map(|c| c.row))
                    })
                    .collect()
            };
            follow(&mut arrangement.rows, &drawn);
        }
        true
    }

    /// Colour a row (`#rrggbb`), or none for the default.
    pub fn set_row_color(&mut self, row: u32, color: Option<&str>) -> Result<(), String> {
        if let Some(c) = color {
            if !is_colour(c) {
                return Err(format!("not a colour: {c}"));
            }
        }
        self.row_mut(row)?.color = color.map(str::to_lowercase);
        Ok(())
    }

    /// FL's track switch: every clip on the row goes quiet.
    pub fn set_row_mute(&mut self, row: u32, muted: bool) -> Result<(), String> {
        self.row_mut(row)?.muted = muted;
        Ok(())
    }

    /// While any row is soloed only soloed rows play.
    pub fn set_row_solo(&mut self, row: u32, soloed: bool) -> Result<(), String> {
        self.row_mut(row)?.soloed = soloed;
        Ok(())
    }

    /// Every row on the open arrangement's tracks, for moving them all at
    /// once: clips, automation clips and lanes.
    fn each_row_mut(&mut self, mut f: impl FnMut(&mut Option<u32>)) {
        for track in &mut self.tracks {
            track.clips.iter_mut().for_each(|c| f(&mut c.row));
            track
                .automation_clips
                .iter_mut()
                .for_each(|c| f(&mut c.row));
            track
                .automation_lanes
                .iter_mut()
                .for_each(|l| f(&mut l.row));
        }
    }

    /// Put `count` empty rows in at `at`; everything from there on moves
    /// down with its row, switches and name included.
    pub fn insert_rows(&mut self, at: u32, count: u32) -> Result<(), String> {
        if count == 0 {
            return Ok(());
        }
        let last = (self.rows_in_use() as u64).max(at as u64) + count as u64;
        if last > MAX_ROWS as u64 {
            return Err("that many rows would go past the last playlist row".into());
        }
        if (at as usize) < self.rows.len() {
            let fresh = std::iter::repeat_n(PlaylistRow::default(), count as usize);
            self.rows.splice(at as usize..at as usize, fresh);
        }
        self.each_row_mut(|row| {
            if let Some(r) = row.as_mut() {
                if *r >= at {
                    *r += count;
                }
            }
        });
        Ok(())
    }

    /// Take out `count` rows from `at`, as FL deletes a playlist track: the
    /// clips and automation clips on them go, and the rows under them
    /// move up. An automation lane is not a clip and is kept, on the row
    /// that moves into their place.
    pub fn delete_rows(&mut self, at: u32, count: u32) -> Result<(), String> {
        check_row_number(at)?;
        if count == 0 {
            return Ok(());
        }
        let end = at.saturating_add(count);
        let gone = |row: Option<u32>| row.is_some_and(|r| r >= at && r < end);
        for track in &mut self.tracks {
            track.clips.retain(|c| !gone(c.row));
            track.automation_clips.retain(|c| !gone(c.row));
        }
        if (at as usize) < self.rows.len() {
            let stop = (end as usize).min(self.rows.len());
            self.rows.drain(at as usize..stop);
        }
        self.each_row_mut(|row| {
            if let Some(r) = row.as_mut() {
                if *r >= end {
                    *r -= count;
                } else if *r >= at {
                    *r = at;
                }
            }
        });
        Ok(())
    }

    /// A copy of a clip on the same channel, at `position_ticks` on `row`,
    /// as the picker places one. Returns the copy's id.
    pub fn place_clip_copy(
        &mut self,
        track_id: &str,
        id: &str,
        position_ticks: u64,
        row: u32,
        new_id: String,
    ) -> Result<String, String> {
        check_row_number(row)?;
        let track = self
            .track_mut(track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        let mut copy = track
            .clips
            .iter()
            .find(|c| clip_id(c) == id)
            .ok_or_else(|| format!("Clip not found: {id}"))?
            .clone();
        match &mut copy.content {
            ClipContent::Audio(a) => a.id = new_id.clone(),
            ClipContent::Midi(m) => m.id = new_id.clone(),
        }
        copy.position_ticks = position_ticks;
        copy.row = Some(row);
        copy.lane = 0;
        track.clips.push(copy);
        Ok(new_id)
    }

    /// Whether anything plays into, sends to or listens to this track.
    fn is_reached(&self, id: &str) -> bool {
        self.tracks.iter().any(|t| {
            t.output_bus.as_deref() == Some(id)
                || t.input_source.as_deref() == Some(id)
                || t.sends.iter().any(|s| s.target == id)
                || t.midi_route_to.iter().any(|r| r == id)
                || t.inserts
                    .iter()
                    .any(|s| s.sidechain_source.as_deref() == Some(id))
        })
    }

    /// An insert nothing uses yet: a numbered mixer track that is not a
    /// channel, holds no clips, effects or automation in any arrangement,
    /// keeps its "Insert N" name, and nothing plays into.
    fn is_free_insert(&self, track: &Track) -> bool {
        let Some(n) = track.mixer_number.filter(|n| *n > 0) else {
            return false;
        };
        matches!(
            track.kind,
            TrackKind::Audio | TrackKind::Bus | TrackKind::Return
        ) && track.rack_group.is_none()
            && track.clips.is_empty()
            && track.automation_clips.is_empty()
            && track.automation_lanes.is_empty()
            && track.inserts.is_empty()
            && track.sends.is_empty()
            && track.output_bus.is_none()
            && !track.armed
            && !track.frozen
            && (track.name.is_empty() || track.name == format!("Insert {n}"))
            && !self.arrangements.iter().any(|a| {
                a.timelines
                    .get(&track.id)
                    .is_some_and(|tl| !tl.clips.is_empty() || !tl.automation_clips.is_empty())
            })
            && !self.is_reached(&track.id)
    }

    /// FL's "next free insert" for a new channel: the lowest numbered
    /// insert nothing uses yet, or else the lowest number no track has.
    pub fn next_free_insert(&self) -> ChannelInsert {
        let free = self
            .tracks
            .iter()
            .filter(|t| self.is_free_insert(t))
            .min_by_key(|t| t.mixer_number);
        if let Some(t) = free {
            return ChannelInsert::Route(t.id.clone());
        }
        let taken: std::collections::HashSet<u16> =
            self.tracks.iter().filter_map(|t| t.mixer_number).collect();
        let n = (1..=crate::sanitize::MAX_MIXER_NUMBER)
            .find(|n| !taken.contains(n))
            .unwrap_or_else(|| self.next_mixer_number());
        ChannelInsert::Own(n)
    }

    /// The audio clip channel for a sample file: the channel that already
    /// plays it, else a new one named after it that plays into the next
    /// free insert (which takes its name, as FL names an insert after the
    /// channel routed to it). Returns the channel's id and whether it was
    /// made now.
    pub fn sample_channel(&mut self, file: &str, name: &str, new_id: String) -> (String, bool) {
        let plays_it = |t: &Track| {
            t.clips.iter().any(|c| match &c.content {
                ClipContent::Audio(a) => a.source_file == file,
                ClipContent::Midi(_) => false,
            })
        };
        let channel = |t: &Track| {
            matches!(t.kind, TrackKind::Audio) && t.rack_group.is_some() && t.mixer_number.is_none()
        };
        if let Some(t) = self.tracks.iter().find(|t| channel(t) && plays_it(t)) {
            return (t.id.clone(), false);
        }
        // A channel the drop used to make, named after the sample and
        // holding nothing: it is this sample's channel.
        let unused_namesake = |t: &Track| {
            matches!(t.kind, TrackKind::Audio)
                && t.rack_group.is_some()
                && !t.id.starts_with("insert-")
                && t.name == name
                && t.clips.is_empty()
                && t.inserts.is_empty()
        };
        if let Some(t) = self.tracks.iter().find(|t| unused_namesake(t)) {
            return (t.id.clone(), false);
        }
        let mut track = Track::new_audio(new_id.clone(), name.to_string());
        track.rack_group = Some(0);
        if self.rack_groups.is_empty() {
            self.rack_groups
                .push(crate::layout::DEFAULT_RACK_GROUP.to_string());
        }
        match self.next_free_insert() {
            ChannelInsert::Route(insert) => {
                if let Some(t) = self.track_mut(&insert) {
                    t.name = name.to_string();
                    track.color = t.color.clone();
                }
                track.output_bus = Some(insert);
            }
            ChannelInsert::Own(n) => track.mixer_number = Some(n),
        }
        self.tracks.push(track);
        (new_id, true)
    }

    /// The automation clips on the open arrangement's tracks that have no
    /// row yet get the empty row each is drawn on, so two of them never
    /// share a row by accident. Returns how many got one.
    pub fn row_new_automation_clips(&mut self) -> usize {
        let shown = crate::layout::shown_rows(&self.tracks);
        let mut n = 0;
        for (track, rows) in self.tracks.iter_mut().zip(shown) {
            for (clip, row) in track.automation_clips.iter_mut().zip(rows.automation_clips) {
                if clip.row.is_none() {
                    clip.row = Some(row);
                    n += 1;
                }
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation::AutomationTarget;
    use crate::automation_clip::AutomationClip;
    use crate::clip::MidiClipRef;

    fn midi(track: &str, id: &str, at: u64, row: Option<u32>) -> ClipPlacement {
        ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: id.into(),
                clip: hardwave_midi::MidiClip::new(id.into(), "c".into(), 960),
            }),
            track_id: track.into(),
            position_ticks: at,
            length_ticks: 960,
            lane: 0,
            color: None,
            row,
        }
    }

    /// A master, a lead with two clips on row 0 and an automation clip on
    /// row 2, and a bass with one clip on row 1.
    fn song() -> Project {
        let mut p = Project::default();
        p.tracks.truncate(1);
        let mut lead = Track::new_midi("lead".into(), "Lead".into());
        lead.rack_group = Some(0);
        lead.mixer_number = Some(1);
        lead.clips = vec![
            midi("lead", "a", 0, Some(0)),
            midi("lead", "b", 960, Some(0)),
        ];
        let mut ac = AutomationClip::new("ac", AutomationTarget::TrackVolume, 0, 960);
        ac.row = Some(2);
        lead.automation_clips.push(ac);
        let mut bass = Track::new_midi("bass".into(), "Bass".into());
        bass.rack_group = Some(0);
        bass.mixer_number = Some(2);
        bass.clips = vec![midi("bass", "c", 0, Some(1))];
        p.tracks.push(lead);
        p.tracks.push(bass);
        p.rows = vec![
            PlaylistRow {
                name: "Lead".into(),
                ..PlaylistRow::default()
            },
            PlaylistRow {
                name: "Bass".into(),
                ..PlaylistRow::default()
            },
        ];
        p
    }

    fn row_of(p: &Project, track: &str, id: &str) -> Option<u32> {
        p.track(track)
            .unwrap()
            .clips
            .iter()
            .find(|c| clip_id(c) == id)
            .unwrap()
            .row
    }

    #[test]
    fn a_clip_moves_to_another_row_and_stays_on_its_channel() {
        let mut p = song();
        let moved = p
            .set_clip_rows(&[
                ("lead".into(), "b".into(), 7),
                ("bass".into(), "c".into(), 1),
            ])
            .unwrap();
        assert_eq!(moved, 1, "the bass clip was already on row 1");
        assert_eq!(row_of(&p, "lead", "b"), Some(7));
        assert_eq!(p.track("lead").unwrap().clips.len(), 2);
        assert!(p.set_clip_rows(&[("lead".into(), "zz".into(), 1)]).is_err());
        assert!(p
            .set_clip_rows(&[("lead".into(), "a".into(), MAX_ROWS as u32)])
            .is_err());
        assert_eq!(
            row_of(&p, "lead", "a"),
            Some(0),
            "a refused move moves nothing"
        );
    }

    #[test]
    fn an_automation_clip_moves_to_any_row() {
        let mut p = song();
        p.set_automation_clip_row("lead", "ac", 9).unwrap();
        assert_eq!(p.track("lead").unwrap().automation_clips[0].row, Some(9));
        assert!(p.set_automation_clip_row("lead", "nope", 1).is_err());
    }

    #[test]
    fn a_row_is_named_coloured_and_switched_and_made_when_past_the_end() {
        let mut p = song();
        p.set_row_name(5, "  Drums ").unwrap();
        assert_eq!(p.rows.len(), 6);
        assert_eq!(p.rows[5].name, "Drums");
        assert_eq!(p.rows[3], PlaylistRow::default());
        p.set_row_color(5, Some("#AABBCC")).unwrap();
        assert_eq!(p.rows[5].color.as_deref(), Some("#aabbcc"));
        assert!(p.set_row_color(5, Some("red")).is_err());
        p.set_row_color(5, None).unwrap();
        assert_eq!(p.rows[5].color, None);
        p.set_row_mute(1, true).unwrap();
        p.set_row_solo(0, true).unwrap();
        assert!(p.rows[1].muted && p.rows[0].soloed);
        assert!(p.set_row_name(MAX_ROWS as u32, "x").is_err());
    }

    #[test]
    fn inserted_rows_push_everything_under_them_down() {
        let mut p = song();
        p.track_mut("lead")
            .unwrap()
            .automation_lanes
            .push(crate::AutomationLane {
                id: "lane".into(),
                target: AutomationTarget::TrackPan,
                points: Vec::new(),
                visible: true,
                row: Some(3),
            });
        p.insert_rows(1, 2).unwrap();
        let names: Vec<_> = p.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Lead", "", "", "Bass"]);
        assert_eq!(row_of(&p, "lead", "a"), Some(0));
        assert_eq!(row_of(&p, "bass", "c"), Some(3));
        let lead = p.track("lead").unwrap();
        assert_eq!(lead.automation_clips[0].row, Some(4));
        assert_eq!(lead.automation_lanes[0].row, Some(5));
    }

    #[test]
    fn a_deleted_row_takes_its_clips_with_it_and_the_rows_under_it_move_up() {
        let mut p = song();
        p.rows[1].muted = true;
        p.delete_rows(1, 1).unwrap();
        assert!(p.track("bass").unwrap().clips.is_empty());
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.track("lead").unwrap().automation_clips[0].row, Some(1));
        assert!(!p.rows.iter().any(|r| r.muted), "its switch went with it");
        // Rows past the end of the list: only the clips move.
        p.delete_rows(0, 5).unwrap();
        assert!(p.tracks.iter().all(|t| t.clips.is_empty()));
    }

    #[test]
    fn a_placed_copy_stays_on_the_channel_of_the_clip_it_copies() {
        let mut p = song();
        let id = p
            .place_clip_copy("bass", "c", 3840, 6, "copy".into())
            .unwrap();
        assert_eq!(id, "copy");
        let bass = p.track("bass").unwrap();
        assert_eq!(bass.clips.len(), 2);
        assert_eq!(bass.clips[1].position_ticks, 3840);
        assert_eq!(bass.clips[1].row, Some(6));
        assert!(p.place_clip_copy("bass", "nope", 0, 0, "x".into()).is_err());
    }

    #[test]
    fn the_first_empty_row_skips_rows_with_anything_on_them() {
        let mut p = song();
        assert_eq!(p.rows_in_use(), 3);
        assert_eq!(p.first_empty_row(), 3);
        p.set_clip_rows(&[("bass".into(), "c".into(), 0)]).unwrap();
        assert_eq!(p.first_empty_row(), 1);
        // A clip with no row is drawn with its channel's other clips (row 0
        // here), and with none on its track's place (the bass's is row 1).
        let bass = p.track_mut("bass").unwrap();
        bass.clips.push(midi("bass", "d", 960, None));
        assert_eq!(p.first_empty_row(), 1);
        p.track_mut("bass").unwrap().clips.clear();
        p.track_mut("bass")
            .unwrap()
            .clips
            .push(midi("bass", "e", 0, None));
        assert_eq!(
            p.first_empty_row(),
            3,
            "the bass's clip is drawn on its place, row 1"
        );
    }

    #[test]
    fn new_automation_clips_each_get_a_row_of_their_own() {
        let mut p = song();
        let lead = p.track_mut("lead").unwrap();
        lead.automation_clips.push(AutomationClip::new(
            "n1",
            AutomationTarget::TrackPan,
            0,
            960,
        ));
        lead.automation_clips.push(AutomationClip::new(
            "n2",
            AutomationTarget::TrackMute,
            0,
            960,
        ));
        assert_eq!(p.row_new_automation_clips(), 2);
        let rows: Vec<_> = p
            .track("lead")
            .unwrap()
            .automation_clips
            .iter()
            .map(|c| c.row)
            .collect();
        assert_eq!(rows, [Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn a_dropped_sample_gets_a_channel_into_the_next_free_insert() {
        let mut p = Project::default();
        // Insert 1 is in use: it has an effect on it.
        p.track_mut("insert-001")
            .unwrap()
            .inserts
            .push(crate::track::PluginSlot {
                id: "fx".into(),
                plugin_id: "hardwave.hardclip".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        assert_eq!(
            p.next_free_insert(),
            ChannelInsert::Route("insert-002".into())
        );
        let (id, made) = p.sample_channel("C:/kick.wav", "kick", "ch1".into());
        assert!(made);
        let channel = p.track(&id).unwrap();
        assert_eq!(channel.output_bus.as_deref(), Some("insert-002"));
        assert_eq!(channel.rack_group, Some(0));
        assert_eq!(channel.mixer_number, None, "a channel, not an insert");
        assert_eq!(p.track("insert-002").unwrap().name, "kick");
        // Insert 2 now has something playing into it.
        assert_eq!(
            p.next_free_insert(),
            ChannelInsert::Route("insert-003".into())
        );
        // The same file again: the same channel, once it plays it.
        p.track_mut(&id).unwrap().clips.push(ClipPlacement {
            content: ClipContent::Audio(crate::clip::AudioClip {
                id: "k".into(),
                name: "kick".into(),
                source_path: "pool".into(),
                source_hash: String::new(),
                source_start: 0,
                source_end: 1,
                gain_db: 0.0,
                fade_in_ticks: 0,
                fade_out_ticks: 0,
                muted: false,
                reversed: false,
                pitch_semitones: 0.0,
                stretch_ratio: 1.0,
                warp_markers: Vec::new(),
                fade_in_curve: Default::default(),
                fade_out_curve: Default::default(),
                source_file: "C:/kick.wav".into(),
                treatment: Default::default(),
                tuned_notes: Vec::new(),
            }),
            track_id: id.clone(),
            position_ticks: 0,
            length_ticks: 960,
            lane: 0,
            color: None,
            row: Some(0),
        });
        assert_eq!(
            p.sample_channel("C:/kick.wav", "kick", "ch2".into()),
            (id, false)
        );
    }

    #[test]
    fn with_no_insert_free_a_channel_gets_its_own_number() {
        let mut p = song();
        assert_eq!(p.next_free_insert(), ChannelInsert::Own(3));
        let (id, _) = p.sample_channel("/s/clap.wav", "clap", "ch".into());
        let t = p.track(&id).unwrap();
        assert_eq!((t.mixer_number, t.output_bus.as_deref()), (Some(3), None));
        p.make_safe().unwrap();
        assert_eq!(
            p.track(&id).unwrap().mixer_number,
            Some(3),
            "a number used once"
        );
    }

    #[test]
    fn a_song_from_a_build_that_left_rows_behind_is_laid_out_again() {
        // A step-1 build added a track and a clip and kept no row for
        // either: the clip had none and the rows stopped at 500.
        let mut p = Project::default();
        let id = p.add_midi_track("Late".into());
        p.track_mut(&id)
            .unwrap()
            .clips
            .push(midi(&id, "late", 0, None));
        p.version = 3;
        let bytes = zstd::encode_all(rmp_serde::to_vec_named(&p).unwrap().as_slice(), 3).unwrap();
        let back = Project::from_bytes(&bytes).unwrap();
        assert_eq!(back.version, crate::project::FORMAT_VERSION);
        assert_eq!(back.rows.len(), 501);
        assert_eq!(back.rows[500].name, "Late");
        assert_eq!(back.track(&id).unwrap().clips[0].row, Some(500));
        // A song saved now is never laid out again.
        let mut now = back.clone();
        now.set_row_name(500, "Kept").unwrap();
        let again = Project::from_bytes(&now.to_bytes().unwrap()).unwrap();
        assert_eq!(again.rows[500].name, "Kept");
    }

    #[test]
    fn a_renamed_channel_takes_the_row_made_for_it_along() {
        let mut p = Project::default();
        let kick = p.claim_row(1, "Kick", true);
        assert!(p.rename_track(&kick, "  Big Kick "));
        assert_eq!(p.track(&kick).unwrap().name, "Big Kick");
        assert_eq!(p.rows[0].name, "Big Kick", "the row made for it follows");
        // An insert's row, named after it with the song, follows too.
        assert!(p.rename_track("insert-004", "Vocals"));
        assert_eq!(p.rows[3].name, "Vocals");
        assert_eq!(p.rows[4].name, "Insert 5", "the others stay");
        assert!(!p.rename_track(&kick, "   "), "an empty name is refused");
        assert!(!p.rename_track("nope", "X"));
    }

    #[test]
    fn a_row_named_by_hand_or_never_named_keeps_its_name() {
        let mut p = song();
        // Bass's clip moves to row 4, which nobody named ("Track 5"), and
        // the lead's row is given a name of its own.
        p.set_clip_rows(&[("bass".into(), "c".into(), 4)]).unwrap();
        p.set_row_name(0, "Hook").unwrap();
        p.rename_track("bass", "Sub");
        p.rename_track("lead", "Pluck");
        assert_eq!(p.rows[0].name, "Hook");
        assert_eq!(p.rows[1].name, "Sub", "still the bass's place");
        assert_eq!(p.rows.get(4).map_or("", |r| r.name.as_str()), "");
    }

    #[test]
    fn only_the_renamed_tracks_row_follows_when_two_share_a_name() {
        let mut p = song();
        p.track_mut("bass").unwrap().name = "Lead".into();
        p.rows[1].name = "Lead".into();
        p.rename_track("bass", "Bass 2");
        assert_eq!(p.rows[0].name, "Lead", "the lead's row stays");
        assert_eq!(p.rows[1].name, "Bass 2");
    }

    #[test]
    fn a_renamed_channel_takes_its_row_along_in_every_arrangement() {
        let mut p = song();
        p.ensure_arrangements();
        let first = p.active_arrangement.clone();
        p.create_arrangement("Two", true);
        p.rename_track("bass", "Sub");
        assert_eq!(p.rows[1].name, "Sub");
        p.capture_active_arrangement();
        p.apply_arrangement(&first).unwrap();
        assert_eq!(p.rows[1].name, "Sub", "the stored arrangement's row too");
    }

    #[test]
    fn claiming_a_row_names_the_row_it_shows_on() {
        let mut p = Project::default();
        let id = p.claim_row(3, "Bass", true);
        assert_eq!(p.rows[2].name, "Bass");
        assert_eq!(
            p.rows[2].color.as_deref(),
            Some(p.track(&id).unwrap().color.as_str())
        );
        assert_eq!(
            p.rows[3].name, "Insert 4",
            "the rows around it are left alone"
        );
        // A row named by hand keeps its name.
        p.set_row_name(4, "Mine").unwrap();
        p.claim_row(5, "Lead", true);
        assert_eq!(p.rows[4].name, "Mine");
    }
}
