//! VCA groups: one fader that rides several others.
//!
//! Pulling a whole drum bus down meant either routing every drum track
//! into a bus, which changes where the effects sit and what the sidechain
//! keys off, or dragging six faders and hoping they stayed in proportion.
//! A VCA moves nothing in the signal path. It adds its own level to each
//! member's fader, so the balance between them is kept and the routing is
//! exactly what it was.

use serde::{Deserialize, Serialize};

use crate::track::TrackId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vca {
    pub id: String,
    pub name: String,
    /// Added to each member's own fader, in decibels. 0 changes nothing.
    pub gain_db: f64,
    /// Silences every member without touching their own mute buttons, so
    /// unmuting the group gives back exactly what was there.
    pub muted: bool,
    pub members: Vec<TrackId>,
}

impl Vca {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            gain_db: 0.0,
            muted: false,
            members: Vec::new(),
        }
    }
}

/// What the VCAs add to one track's fader, and whether they silence it.
///
/// A track can sit in more than one group, in which case the offsets add
/// up, the same way two faders in series would.
pub fn offset_for(vcas: &[Vca], track_id: &str) -> (f64, bool) {
    let mut gain_db = 0.0;
    let mut muted = false;
    for vca in vcas {
        if vca.members.iter().any(|m| m == track_id) {
            gain_db += vca.gain_db;
            muted = muted || vca.muted;
        }
    }
    (gain_db, muted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(name: &str, gain_db: f64, members: &[&str]) -> Vca {
        Vca {
            id: name.to_string(),
            name: name.to_string(),
            gain_db,
            muted: false,
            members: members.iter().map(|m| m.to_string()).collect(),
        }
    }

    #[test]
    fn a_track_in_no_group_is_left_alone() {
        let vcas = vec![group("Drums", -6.0, &["kick"])];
        assert_eq!(offset_for(&vcas, "lead"), (0.0, false));
    }

    #[test]
    fn a_member_gets_the_group_level_on_top_of_its_own() {
        let vcas = vec![group("Drums", -6.0, &["kick", "snare"])];
        assert_eq!(offset_for(&vcas, "kick"), (-6.0, false));
    }

    #[test]
    fn two_groups_add_up_like_two_faders_in_series() {
        let vcas = vec![
            group("Drums", -6.0, &["kick"]),
            group("Everything", -3.0, &["kick"]),
        ];
        assert_eq!(offset_for(&vcas, "kick"), (-9.0, false));
    }

    #[test]
    fn a_muted_group_silences_its_members() {
        let mut vcas = vec![group("Drums", 0.0, &["kick"])];
        vcas[0].muted = true;
        assert_eq!(offset_for(&vcas, "kick"), (0.0, true));
    }
}
