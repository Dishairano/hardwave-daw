//! Track templates: a track kept to start new tracks from.
//!
//! A template keeps the track's settings and its plug-ins with their own
//! settings, so a vocal chain or a bass bus with its EQ, compressor and
//! sends is one click in another song. It keeps no clips or automation:
//! those belong to the song it came from. The bus and sends it fed are kept
//! by name, since ids belong to one song, and are linked again to a track
//! of that name in the song it is added to.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{Project, Track, TrackKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackTemplate {
    pub track: Track,
    /// The bus the track fed, by name.
    pub output_bus_name: Option<String>,
    /// Each send's target by name, in the order of `track.sends`.
    pub send_target_names: Vec<String>,
}

impl TrackTemplate {
    pub fn to_bytes(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(self).unwrap_or_default()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        rmp_serde::from_slice(bytes).ok()
    }
}

impl Project {
    /// The track `id` as a template. `live` holds the plug-ins' settings as
    /// they play now, by slot id; a slot not in it keeps its saved settings.
    /// None for the master and for a track that is not there.
    pub fn track_template(
        &self,
        id: &str,
        live: &HashMap<String, Vec<u8>>,
    ) -> Option<TrackTemplate> {
        let source = self.track(id)?;
        if matches!(source.kind, TrackKind::Master) {
            return None;
        }
        let name_of = |id: &str| self.track(id).map(|t| t.name.clone());
        let mut track = source.clone();
        track.clips.clear();
        track.automation_lanes.clear();
        track.automation_clips.clear();
        track.armed = false;
        track.frozen = false;
        track.soloed = false;
        track.muted = false;
        for slot in track.inserts.iter_mut() {
            let bytes = live
                .get(&slot.id)
                .cloned()
                .or_else(|| self.plugin_state(&slot.id).map(|e| e.chunk.clone()))
                .or_else(|| slot.state.clone());
            slot.state = bytes;
        }
        Some(TrackTemplate {
            output_bus_name: source.output_bus.as_deref().and_then(name_of),
            send_target_names: source
                .sends
                .iter()
                .map(|s| name_of(&s.target).unwrap_or_default())
                .collect(),
            track,
        })
    }

    /// Add a track made from `template`, right after `after` (or last), with
    /// new ids for it and its plug-ins. Its bus and sends go to the tracks of
    /// the same name here; one with no such track is left out. Returns the
    /// new track's id.
    pub fn add_track_from_template(
        &mut self,
        template: &TrackTemplate,
        after: Option<&str>,
    ) -> String {
        let fresh = || uuid::Uuid::new_v4().to_string();
        let id_named = |name: &str| {
            self.tracks
                .iter()
                .find(|t| t.name == name && !matches!(t.kind, TrackKind::Master))
                .map(|t| t.id.clone())
        };
        let mut track = template.track.clone();
        let new_id = fresh();
        track.id = new_id.clone();
        track.output_bus = template.output_bus_name.as_deref().and_then(id_named);
        let mut sends = Vec::new();
        for (send, name) in track.sends.iter().zip(template.send_target_names.iter()) {
            if let Some(target) = id_named(name) {
                let mut send = send.clone();
                send.target = target;
                sends.push(send);
            }
        }
        track.sends = sends;
        let mut states = Vec::new();
        for slot in track.inserts.iter_mut() {
            slot.id = fresh();
            slot.sidechain_source = None;
            if let Some(bytes) = slot.state.clone() {
                states.push((slot.id.clone(), bytes));
            }
        }
        for (slot_id, bytes) in states {
            self.set_plugin_state(slot_id, "unknown", bytes);
        }
        let at = after
            .and_then(|a| self.tracks.iter().position(|t| t.id == a))
            .map(|i| i + 1)
            .unwrap_or(self.tracks.len());
        self.tracks.insert(at, track);
        new_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mixer::Send;
    use crate::track::PluginSlot;

    fn song() -> Project {
        let mut p = Project::default();
        let bus = p.add_audio_track("Vocal bus".into());
        let verb = p.add_audio_track("Reverb".into());
        let lead = p.add_audio_track("Lead vocal".into());
        let t = p.track_mut(&lead).unwrap();
        t.output_bus = Some(bus);
        t.sends.push(Send {
            target: verb,
            gain_db: -9.0,
            pre_fader: false,
            enabled: true,
        });
        t.inserts.push(PluginSlot {
            id: "eq".into(),
            plugin_id: "hardwave.native.eq".into(),
            enabled: true,
            state: Some(vec![1]),
            sidechain_source: None,
            wet: 1.0,
        });
        t.armed = true;
        p
    }

    fn lead(p: &Project) -> String {
        p.tracks
            .iter()
            .find(|t| t.name == "Lead vocal")
            .unwrap()
            .id
            .clone()
    }

    /// The plug-ins keep their settings as they play, the clips stay behind,
    /// and the template comes back from its bytes the same.
    #[test]
    fn a_template_keeps_the_chain_and_its_settings() {
        let p = song();
        let live: HashMap<String, Vec<u8>> = [("eq".to_string(), vec![7, 7])].into();
        let t = p.track_template(&lead(&p), &live).unwrap();
        assert_eq!(t.track.inserts[0].state.as_deref(), Some(&[7u8, 7][..]));
        assert!(t.track.clips.is_empty());
        assert!(!t.track.armed);
        assert_eq!(t.output_bus_name.as_deref(), Some("Vocal bus"));
        assert_eq!(t.send_target_names, vec!["Reverb".to_string()]);
        let back = TrackTemplate::from_bytes(&t.to_bytes()).unwrap();
        assert_eq!(back.track.inserts[0].state, t.track.inserts[0].state);
    }

    /// In another song the track gets new ids, its plug-in settings are
    /// there to load, and the bus and sends find the tracks of that name;
    /// a send to a track the song does not have is left out.
    #[test]
    fn a_track_from_a_template_links_up_by_name() {
        let p = song();
        let t = p.track_template(&lead(&p), &HashMap::new()).unwrap();
        let mut other = Project::default();
        let bus = other.add_audio_track("Vocal bus".into());
        let id = other.add_track_from_template(&t, None);
        let added = other.track(&id).unwrap();
        assert_ne!(added.inserts[0].id, "eq", "a new slot id");
        assert_eq!(
            other
                .plugin_state(&added.inserts[0].id)
                .map(|e| e.chunk.clone()),
            Some(vec![1])
        );
        assert_eq!(added.output_bus.as_deref(), Some(bus.as_str()));
        assert!(added.sends.is_empty(), "no Reverb track here");
        assert_eq!(added.name, "Lead vocal");
    }

    /// The master is not a template.
    #[test]
    fn the_master_is_not_a_template() {
        let p = song();
        let master = p
            .tracks
            .iter()
            .find(|t| matches!(t.kind, TrackKind::Master))
            .map(|t| t.id.clone());
        if let Some(master) = master {
            assert!(p.track_template(&master, &HashMap::new()).is_none());
        }
    }
}
