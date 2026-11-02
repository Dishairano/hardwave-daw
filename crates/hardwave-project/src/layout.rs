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

    #[test]
    fn a_row_saved_without_its_switches_reads_as_on() {
        let json = serde_json::json!({ "name": "Drums" });
        let row: PlaylistRow = serde_json::from_value(json).unwrap();
        assert_eq!(row.name, "Drums");
        assert!(!row.muted && !row.soloed);
        assert_eq!(row.color, None);
    }
}
