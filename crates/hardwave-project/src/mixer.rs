use serde::{Deserialize, Serialize};

use crate::track::TrackId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Send {
    pub target: TrackId,
    pub gain_db: f64,
    pub pre_fader: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStrip {
    pub track_id: TrackId,
    pub input_gain_db: f64,
    pub fader_db: f64,
    pub pan: f64,
}

/// The tracks that stay audible while some are soloed, or None when none
/// is. A soloed track is heard through everything its sound passes on its
/// way out (its bus, that bus's bus, the tracks it sends to), and a soloed
/// bus with everything that plays into it. Soloing used to silence every
/// track not soloed itself, so a track soloed inside a group went silent
/// with its group, and a soloed group with its members.
pub fn solo_audible(tracks: &[crate::track::Track]) -> Option<std::collections::HashSet<TrackId>> {
    use std::collections::HashSet;
    let audible_kind = |t: &&crate::track::Track| t.kind.is_audio_bearing();
    let mut heard: HashSet<TrackId> = tracks
        .iter()
        .filter(audible_kind)
        .filter(|t| t.soloed)
        .map(|t| t.id.clone())
        .collect();
    if heard.is_empty() {
        return None;
    }
    // What plays into a soloed track, all the way back.
    loop {
        let feeders: Vec<TrackId> = tracks
            .iter()
            .filter(audible_kind)
            .filter(|t| !heard.contains(&t.id))
            .filter(|t| t.output_bus.as_ref().is_some_and(|bus| heard.contains(bus)))
            .map(|t| t.id.clone())
            .collect();
        if feeders.is_empty() {
            break;
        }
        heard.extend(feeders);
    }
    // Where all of that goes on its way out.
    let mut queue: Vec<TrackId> = heard.iter().cloned().collect();
    while let Some(id) = queue.pop() {
        let Some(track) = tracks.iter().find(|t| t.id == id) else {
            continue;
        };
        let onward = track
            .output_bus
            .iter()
            .chain(track.sends.iter().filter(|s| s.enabled).map(|s| &s.target));
        for next in onward {
            if heard.insert(next.clone()) {
                queue.push(next.clone());
            }
        }
    }
    Some(heard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Track;

    fn track(id: &str, bus: Option<&str>, soloed: bool) -> Track {
        let mut t = Track::new_audio(id.into(), id.into());
        t.output_bus = bus.map(String::from);
        t.soloed = soloed;
        t
    }

    fn heard(tracks: &[Track]) -> Vec<String> {
        let mut ids: Vec<String> = solo_audible(tracks)
            .map(|set| set.into_iter().collect())
            .unwrap_or_default();
        ids.sort();
        ids
    }

    #[test]
    fn nothing_soloed_is_no_solo() {
        assert!(solo_audible(&[track("a", None, false)]).is_none());
    }

    #[test]
    fn a_soloed_track_is_heard_through_its_group_and_sends() {
        let mut lead = track("lead", Some("group"), true);
        lead.sends.push(Send {
            target: "reverb".into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: true,
        });
        let tracks = [
            lead,
            track("chords", Some("group"), false),
            track("group", Some("fx"), false),
            track("fx", None, false),
            track("reverb", None, false),
            track("kick", None, false),
        ];
        assert_eq!(heard(&tracks), vec!["fx", "group", "lead", "reverb"]);
    }

    #[test]
    fn a_soloed_group_is_heard_with_what_plays_into_it() {
        let tracks = [
            track("lead", Some("group"), false),
            track("chords", Some("sub"), false),
            track("sub", Some("group"), false),
            track("group", None, true),
            track("kick", None, false),
        ];
        assert_eq!(heard(&tracks), vec!["chords", "group", "lead", "sub"]);
    }
}
