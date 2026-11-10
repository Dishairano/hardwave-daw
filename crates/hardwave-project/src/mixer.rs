use serde::{Deserialize, Serialize};

use crate::track::TrackId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

/// Whether sound leaving track `from` arrives at track `to`: through its
/// output, an enabled send, or as the sidechain key of a plug-in, and on
/// from there. Routing `to` into `from` in any of those ways would close a
/// loop. Sends used to be checked only against sends, and outputs only
/// against outputs, so a bus could send back into a track playing into it.
pub fn feeds(tracks: &[crate::track::Track], from: &str, to: &str) -> bool {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut stack: Vec<&str> = vec![from];
    while let Some(id) = stack.pop() {
        if id == to {
            return true;
        }
        if !seen.insert(id) {
            continue;
        }
        let Some(track) = tracks.iter().find(|t| t.id == id) else {
            continue;
        };
        stack.extend(track.output_bus.as_deref());
        stack.extend(
            track
                .sends
                .iter()
                .filter(|s| s.enabled)
                .map(|s| s.target.as_str()),
        );
        // A track with a plug-in keyed from this one takes its sound too.
        stack.extend(
            tracks
                .iter()
                .filter(|t| {
                    t.inserts
                        .iter()
                        .any(|slot| slot.sidechain_source.as_deref() == Some(id))
                })
                .map(|t| t.id.as_str()),
        );
    }
    false
}

/// Solo one track alone (ctrl-click on Solo): it is soloed and no other
/// is. On a track that is already the only one soloed it clears the solo,
/// so the same click goes back. With several soloed, it used to clear them
/// all, the one clicked included, instead of keeping just that one.
pub fn solo_exclusively(tracks: &mut [crate::track::Track], id: &str) {
    use crate::track::TrackKind;
    let only_this = tracks.iter().any(|t| t.id == id && t.soloed)
        && tracks
            .iter()
            .filter(|t| !matches!(t.kind, TrackKind::Master))
            .all(|t| !t.soloed || t.id == id);
    for t in tracks.iter_mut() {
        if matches!(t.kind, TrackKind::Master) {
            continue;
        }
        t.soloed = t.id == id && !only_this;
    }
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
    // What plays into a soloed track, all the way back: through its output
    // or through a send. A soloed reverb return was silent, because the
    // tracks sending to it were muted with everything else.
    loop {
        let feeders: Vec<TrackId> = tracks
            .iter()
            .filter(audible_kind)
            .filter(|t| !heard.contains(&t.id))
            .filter(|t| {
                t.output_bus.as_ref().is_some_and(|bus| heard.contains(bus))
                    || t.sends
                        .iter()
                        .any(|s| s.enabled && heard.contains(&s.target))
            })
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

/// Every track one track's sound passes through after it leaves: the bus
/// it plays into, that bus's bus, the tracks it sends to and where those
/// go, on to the master. Not the track itself, unless its routing comes
/// back round to it.
pub fn way_out(tracks: &[crate::track::Track], id: &str) -> std::collections::HashSet<TrackId> {
    let mut passed: std::collections::HashSet<TrackId> = std::collections::HashSet::new();
    let mut queue: Vec<&str> = vec![id];
    while let Some(id) = queue.pop() {
        let Some(track) = tracks.iter().find(|t| t.id == id) else {
            continue;
        };
        let onward = track
            .output_bus
            .iter()
            .chain(track.sends.iter().filter(|s| s.enabled).map(|s| &s.target));
        for next in onward {
            if passed.insert(next.clone()) {
                queue.push(next);
            }
        }
    }
    passed
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

    /// A bus a track plays into, through its output, a send or a sidechain
    /// key, is fed by that track; a track elsewhere is not.
    #[test]
    fn feeds_follows_outputs_sends_and_sidechain_keys() {
        let mut vox = track("vox", Some("bus"), false);
        vox.inserts.push(crate::track::PluginSlot {
            id: "comp".into(),
            plugin_id: "compressor".into(),
            enabled: true,
            state: None,
            sidechain_source: Some("kick".into()),
            wet: 1.0,
        });
        let mut bus = track("bus", None, false);
        bus.sends.push(Send {
            target: "reverb".into(),
            gain_db: 0.0,
            pre_fader: false,
            enabled: true,
        });
        let tracks = [
            vox,
            bus,
            track("reverb", None, false),
            track("kick", None, false),
            track("lead", None, false),
        ];
        assert!(feeds(&tracks, "vox", "bus"), "by its output");
        assert!(feeds(&tracks, "vox", "reverb"), "on through the bus's send");
        assert!(feeds(&tracks, "kick", "reverb"), "as a key, then onward");
        assert!(!feeds(&tracks, "reverb", "vox"));
        assert!(!feeds(&tracks, "lead", "bus"));
    }

    fn soloed(tracks: &[Track]) -> Vec<&str> {
        tracks
            .iter()
            .filter(|t| t.soloed)
            .map(|t| t.id.as_str())
            .collect()
    }

    /// Ctrl-click on Solo with two tracks soloed keeps the one clicked and
    /// clears the other; a second ctrl-click clears it too.
    #[test]
    fn solo_exclusively_keeps_only_the_track_clicked() {
        let mut tracks = [
            track("kick", None, true),
            track("bass", None, true),
            track("lead", None, false),
        ];
        solo_exclusively(&mut tracks, "kick");
        assert_eq!(soloed(&tracks), vec!["kick"]);
        solo_exclusively(&mut tracks, "lead");
        assert_eq!(soloed(&tracks), vec!["lead"]);
        solo_exclusively(&mut tracks, "lead");
        assert!(soloed(&tracks).is_empty());
    }

    #[test]
    fn a_track_s_way_out_is_its_buses_and_sends_and_where_they_go() {
        let mut lead = track("lead", Some("group"), false);
        lead.sends.push(Send {
            target: "reverb".into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: true,
        });
        lead.sends.push(Send {
            target: "delay".into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: false,
        });
        let tracks = [
            lead,
            track("chords", Some("group"), false),
            track("group", Some("fx"), false),
            track("fx", None, false),
            track("reverb", Some("fx"), false),
            track("delay", None, false),
        ];
        let mut ids: Vec<String> = way_out(&tracks, "lead").into_iter().collect();
        ids.sort();
        assert_eq!(ids, vec!["fx", "group", "reverb"]);
        assert!(way_out(&tracks, "fx").is_empty(), "straight to the master");
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

    /// Soloing a reverb return hears it with the tracks that send to it.
    /// It was silent: the senders were muted by the solo, so nothing
    /// reached it.
    #[test]
    fn a_soloed_return_is_heard_with_the_tracks_that_send_to_it() {
        let mut lead = track("lead", None, false);
        lead.sends.push(Send {
            target: "reverb".into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: true,
        });
        let mut pad = track("pad", None, false);
        pad.sends.push(Send {
            target: "reverb".into(),
            gain_db: -6.0,
            pre_fader: false,
            enabled: false,
        });
        let tracks = [
            lead,
            pad,
            track("reverb", None, true),
            track("kick", None, false),
        ];
        assert_eq!(heard(&tracks), vec!["lead", "reverb"]);
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
