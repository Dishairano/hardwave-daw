//! The mixer by number, as FL shows it.
//!
//! The mixer is the master (0) and the inserts 1, 2, 3 and on, each a
//! track with that `mixer_number`. A number no track has is drawn as an
//! empty insert and costs nothing: there is no track behind it until
//! something is routed to it or put on it, and then one is made with that
//! number. None of this changes where sound goes except `route_to_insert`,
//! which sets a track's `output_bus` as picking an output always did.

use crate::project::Project;
use crate::sanitize::MAX_MIXER_NUMBER;
use crate::track::{Track, TrackKind};

impl Project {
    /// The track that is insert `number` (0 is the master), if one is.
    pub fn insert_numbered(&self, number: u16) -> Option<&Track> {
        self.tracks.iter().find(|t| t.mixer_number == Some(number))
    }

    /// Make insert `number` a track, or find the one it is. A new one is an
    /// empty bus playing into the master, named `name` or "Insert N" as FL
    /// calls an insert nobody named; an empty bus nothing plays into costs
    /// nothing in the engine. Returns its id and whether it was made now.
    pub fn make_insert(
        &mut self,
        number: u16,
        name: Option<&str>,
    ) -> Result<(String, bool), String> {
        if number == 0 {
            return self
                .insert_numbered(0)
                .map(|t| (t.id.clone(), false))
                .ok_or_else(|| "this song has no master".to_string());
        }
        if number > MAX_MIXER_NUMBER {
            return Err(format!("the mixer goes up to {MAX_MIXER_NUMBER}"));
        }
        if let Some(track) = self.insert_numbered(number) {
            return Ok((track.id.clone(), false));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map_or_else(|| format!("Insert {number}"), str::to_string);
        let mut track = Track::new_bus(id.clone(), name);
        track.mixer_number = Some(number);
        self.tracks.push(track);
        Ok((id, true))
    }

    /// Send a track's sound to insert `number`, as FL's insert box does: 0
    /// is the master, and so is the track's own number (a channel with an
    /// insert of its own plays through it into the master). An empty number
    /// becomes an insert now. Refused, with nothing changed, when the track
    /// is not there, is the master, or the route would play back into
    /// itself. Returns the id of an insert made for it.
    pub fn route_to_insert(
        &mut self,
        track_id: &str,
        number: u16,
    ) -> Result<Option<String>, String> {
        self.check_route_to_insert(track_id, number)?;
        let own = self.track(track_id).and_then(|t| t.mixer_number);
        if number == 0 || own == Some(number) {
            self.track_mut(track_id).expect("checked").output_bus = None;
            return Ok(None);
        }
        let (target, made) = self.make_insert(number, None)?;
        self.track_mut(track_id).expect("found above").output_bus = Some(target.clone());
        Ok(made.then_some(target))
    }

    /// Whether `route_to_insert` would take this route, without making or
    /// changing anything, so a command can refuse before its undo step.
    pub fn check_route_to_insert(&self, track_id: &str, number: u16) -> Result<(), String> {
        let track = self.track(track_id).ok_or("track not found")?;
        if matches!(track.kind, TrackKind::Master) {
            return Err("the master plays into your audio output".into());
        }
        if number > MAX_MIXER_NUMBER {
            return Err(format!("the mixer goes up to {MAX_MIXER_NUMBER}"));
        }
        if number == 0 || track.mixer_number == Some(number) {
            return Ok(());
        }
        // An insert that is there could already play into this track; a new
        // one plays only into the master, so it cannot close a loop.
        match self.insert_numbered(number) {
            Some(target) if crate::mixer::feeds(&self.tracks, &target.id, track_id) => {
                Err("routing would form a cycle".into())
            }
            _ => Ok(()),
        }
    }

    /// A track just added to the song takes its place in the lists the way
    /// a track of its kind always showed: a channel in the rack with an
    /// insert of its own after the highest number, as a track added to the
    /// mixer went to its end. An automation track is only a channel, a bus
    /// or a return only an insert. A number it brings along (from a
    /// template) is not kept: it would be taken twice.
    pub fn list_new_track(&mut self, track_id: &str) {
        let Some(index) = self.tracks.iter().position(|t| t.id == track_id) else {
            return;
        };
        let kind = self.tracks[index].kind.clone();
        if matches!(kind, TrackKind::Master) {
            return;
        }
        self.tracks[index].mixer_number = None;
        let number = match kind {
            TrackKind::Automation => None,
            _ => Some(self.next_mixer_number()),
        };
        let groups = self.rack_groups.len();
        let track = &mut self.tracks[index];
        track.mixer_number = number;
        track.rack_group = match kind {
            TrackKind::Bus | TrackKind::Return => None,
            _ => Some(
                track
                    .rack_group
                    .filter(|g| (*g as usize) < groups)
                    .unwrap_or(0),
            ),
        };
        if track.rack_group.is_some() && groups == 0 {
            self.rack_groups
                .push(crate::layout::DEFAULT_RACK_GROUP.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song as FL's import makes it: the master, insert 4 with a channel
    /// routed into it, and a channel straight into the master.
    fn fl_song() -> (Project, String, String) {
        let mut p = Project::default();
        p.tracks.retain(|t| matches!(t.kind, TrackKind::Master));
        let mut four = Track::new_bus("ins-4".into(), "Insert 4".into());
        four.mixer_number = Some(4);
        let mut kick = Track::new_audio("kick".into(), "Kick".into());
        kick.rack_group = Some(0);
        kick.output_bus = Some("ins-4".into());
        let mut hat = Track::new_audio("hat".into(), "Hat".into());
        hat.rack_group = Some(0);
        p.tracks.extend([four, kick, hat]);
        p.make_safe().unwrap();
        (p, "kick".into(), "hat".into())
    }

    #[test]
    fn an_empty_number_becomes_an_insert_when_something_is_routed_to_it() {
        let (mut p, _, hat) = fl_song();
        assert!(p.insert_numbered(9).is_none(), "9 is empty");
        let made = p.route_to_insert(&hat, 9).unwrap().expect("made now");
        let nine = p.insert_numbered(9).unwrap();
        assert_eq!(nine.id, made);
        assert_eq!(nine.name, "Insert 9");
        assert!(matches!(nine.kind, TrackKind::Bus));
        assert_eq!(nine.rack_group, None, "an insert, not a channel");
        assert_eq!(nine.output_bus, None, "into the master");
        assert_eq!(
            p.track(&hat).unwrap().output_bus.as_deref(),
            Some(made.as_str())
        );
        let before = p.tracks.len();
        assert_eq!(p.route_to_insert(&hat, 9).unwrap(), None, "made once");
        assert_eq!(p.tracks.len(), before);
    }

    #[test]
    fn an_insert_that_is_there_is_used_and_0_is_the_master() {
        let (mut p, kick, hat) = fl_song();
        let before = p.tracks.len();
        assert_eq!(p.route_to_insert(&hat, 4).unwrap(), None);
        assert_eq!(p.track(&hat).unwrap().output_bus.as_deref(), Some("ins-4"));
        assert_eq!(p.tracks.len(), before, "no second insert 4");
        p.route_to_insert(&kick, 0).unwrap();
        assert_eq!(p.track(&kick).unwrap().output_bus, None);
    }

    #[test]
    fn a_channel_with_its_own_insert_picking_its_own_number_plays_through_it() {
        let mut p = Project::default();
        let lead = p.add_midi_track("Lead".into());
        let own = p.track(&lead).unwrap().mixer_number.unwrap();
        p.route_to_insert(&lead, 3).unwrap();
        assert_eq!(
            p.track(&lead).unwrap().output_bus.as_deref(),
            Some("insert-003")
        );
        let before = p.tracks.len();
        p.route_to_insert(&lead, own).unwrap();
        assert_eq!(p.track(&lead).unwrap().output_bus, None);
        assert_eq!(p.tracks.len(), before, "its own number is not made again");
    }

    #[test]
    fn a_route_that_would_play_back_into_itself_changes_nothing() {
        let (mut p, kick, _) = fl_song();
        let four = p.insert_numbered(4).unwrap().id.clone();
        let kick_number = p.track(&kick).unwrap().mixer_number;
        assert_eq!(kick_number, None, "a channel without an insert");
        // Insert 4 into insert 5, then 5 into 4: a loop.
        p.route_to_insert(&four, 5).unwrap();
        let five = p.insert_numbered(5).unwrap().id.clone();
        let before = p.clone();
        assert!(p.route_to_insert(&five, 4).is_err());
        assert_eq!(p.tracks, before.tracks);
        assert!(
            p.route_to_insert("master", 3).is_err(),
            "the master goes out"
        );
        assert!(p.route_to_insert("nobody", 3).is_err());
        assert!(p.route_to_insert(&kick, MAX_MIXER_NUMBER + 1).is_err());
        assert_eq!(p.tracks, before.tracks);
    }

    #[test]
    fn making_an_insert_by_name_and_finding_one_that_is_there() {
        let (mut p, _, _) = fl_song();
        let (id, made) = p.make_insert(14, Some("  KICK VERB ")).unwrap();
        assert!(made);
        assert_eq!(p.track(&id).unwrap().name, "KICK VERB");
        assert_eq!(p.make_insert(14, Some("other")).unwrap(), (id, false));
        assert_eq!(
            p.make_insert(0, None).unwrap(),
            ("master".to_string(), false)
        );
        let mut song = p.clone();
        song.make_safe().unwrap();
        assert_eq!(song.tracks, p.tracks, "what it makes is already safe");
    }

    #[test]
    fn a_track_added_now_is_a_channel_with_the_next_insert() {
        let (mut p, _, _) = fl_song();
        let vox = p.add_audio_track("Vox".into());
        let lead = p.add_midi_track("Lead".into());
        let auto = p.add_automation_track("Auto".into());
        let t = |id: &str| p.track(id).unwrap().clone();
        assert_eq!(t(&vox).mixer_number, Some(5), "after insert 4");
        assert_eq!(t(&vox).rack_group, Some(0));
        assert_eq!(t(&lead).mixer_number, Some(6));
        assert_eq!(t(&lead).rack_group, Some(0));
        assert_eq!(t(&auto).mixer_number, None, "the mixer has no automation");
        assert_eq!(t(&auto).rack_group, Some(0));
        let mut song = p.clone();
        song.make_safe().unwrap();
        assert_eq!(song.tracks, p.tracks, "nothing left to put right");
    }

    #[test]
    fn a_new_song_and_an_fl_import_put_inserts_at_0_db_and_older_songs_keep_the_drop() {
        let new = Project::default();
        assert!(new.inserts_unity_at_centre, "a new song, as in FL");
        let back = Project::from_bytes(&new.to_bytes().unwrap()).unwrap();
        assert!(back.inserts_unity_at_centre, "kept when saved and opened");

        // A song saved by a build from before format 3, and one saved by a
        // format 3 build from before the setting: neither has the field.
        let old = Project::from_bytes(&new.older_format_bytes()).unwrap();
        assert!(!old.inserts_unity_at_centre, "format 2 keeps the old law");
        let mut named = serde_json::to_value(&new).unwrap();
        named
            .as_object_mut()
            .unwrap()
            .remove("inserts_unity_at_centre");
        let bytes =
            zstd::encode_all(rmp_serde::to_vec_named(&named).unwrap().as_slice(), 3).unwrap();
        let three = Project::from_bytes(&bytes).unwrap();
        assert!(
            !three.inserts_unity_at_centre,
            "format 3 without it keeps the old law"
        );

        let fl =
            crate::fl_build::build_project(&crate::fl_import::FlProject::default(), "FL", &|_| {
                None
            });
        assert!(fl.project.inserts_unity_at_centre, "an FL import, as in FL");
    }

    #[test]
    fn a_track_added_to_a_song_without_groups_gets_the_unsorted_one() {
        let mut p = Project::default();
        p.rack_groups.clear();
        let lead = p.add_midi_track("Lead".into());
        assert_eq!(p.track(&lead).unwrap().rack_group, Some(0));
        assert_eq!(p.rack_groups, ["Unsorted"]);
    }
}
