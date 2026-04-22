use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::tempo::TempoMap;
use crate::track::Track;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub name: String,
    pub author: String,
    pub sample_rate: u32,
    pub created_at: String,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub metadata: ProjectMetadata,
    pub tempo_map: TempoMap,
    pub tracks: Vec<Track>,
    /// Opaque JSON state for the UI-side channel rack (patterns, per-step velocity, etc).
    /// Stored as a string so schema changes in the UI don't require project-crate updates.
    #[serde(default)]
    pub channel_rack_state: Option<String>,
    /// Opaque JSON array of MIDI Learn CC→parameter mappings. Same pattern as
    /// channel_rack_state — the src-tauri side owns the real type; the project
    /// crate just ferries the blob across save/load.
    #[serde(default)]
    pub midi_mappings: Option<String>,
    /// Per-plugin state chunks, keyed by plugin instance id. Each chunk is
    /// the opaque blob the plugin returned from `get_state()`. The project
    /// crate doesn't interpret it — save/load round-trips the bytes so the
    /// host can call `set_state(blob)` after instantiation.
    #[serde(default)]
    pub plugin_states: Vec<PluginStateEntry>,
}

/// One plugin's saved state — id + opaque chunk. `format_hint` is a
/// string tag (e.g. "vst3", "clap", "native") that lets the host
/// decide whether to even try restoring if the plugin's format
/// changed between saves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginStateEntry {
    pub plugin_instance_id: String,
    pub format_hint: String,
    pub chunk: Vec<u8>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            version: 1,
            metadata: ProjectMetadata {
                name: "Untitled".into(),
                author: String::new(),
                sample_rate: 48000,
                created_at: chrono::Utc::now().to_rfc3339(),
                modified_at: chrono::Utc::now().to_rfc3339(),
            },
            tempo_map: TempoMap::default(),
            tracks: vec![Track::new_master("master".into())],
            channel_rack_state: None,
            midi_mappings: None,
            plugin_states: Vec::new(),
        }
    }
}

impl Project {
    /// Save project to a .hwp file (MessagePack + zstd).
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let data = rmp_serde::to_vec(self)?;
        let compressed = zstd::encode_all(data.as_slice(), 3)?;
        std::fs::write(path, compressed)?;
        Ok(())
    }

    /// Load project from a .hwp file.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let compressed = std::fs::read(path)?;
        let data = zstd::decode_all(compressed.as_slice())?;
        let project: Project = rmp_serde::from_slice(&data)?;
        Ok(project)
    }

    /// Save as JSON (for debugging / interop).
    pub fn save_json(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn add_audio_track(&mut self, name: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.tracks.push(Track::new_audio(id.clone(), name));
        id
    }

    pub fn add_midi_track(&mut self, name: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.tracks.push(Track::new_midi(id.clone(), name));
        id
    }

    pub fn add_automation_track(&mut self, name: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.tracks.push(Track::new_automation(id.clone(), name));
        id
    }

    pub fn remove_track(&mut self, id: &str) {
        self.tracks.retain(|t| t.id != id);
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Upsert a plugin state chunk — replaces any prior chunk for the
    /// same `plugin_instance_id`. Called by the host when a plugin is
    /// destroyed so the next save captures its final state.
    pub fn set_plugin_state(
        &mut self,
        plugin_instance_id: impl Into<String>,
        format_hint: impl Into<String>,
        chunk: Vec<u8>,
    ) {
        let id = plugin_instance_id.into();
        let hint = format_hint.into();
        if let Some(existing) = self
            .plugin_states
            .iter_mut()
            .find(|e| e.plugin_instance_id == id)
        {
            existing.format_hint = hint;
            existing.chunk = chunk;
        } else {
            self.plugin_states.push(PluginStateEntry {
                plugin_instance_id: id,
                format_hint: hint,
                chunk,
            });
        }
    }

    pub fn plugin_state(&self, plugin_instance_id: &str) -> Option<&PluginStateEntry> {
        self.plugin_states
            .iter()
            .find(|e| e.plugin_instance_id == plugin_instance_id)
    }

    pub fn remove_plugin_state(&mut self, plugin_instance_id: &str) -> bool {
        let before = self.plugin_states.len();
        self.plugin_states
            .retain(|e| e.plugin_instance_id != plugin_instance_id);
        self.plugin_states.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_state_upsert_replaces_existing() {
        let mut p = Project::default();
        p.set_plugin_state("p1", "vst3", vec![1, 2, 3]);
        assert_eq!(p.plugin_state("p1").unwrap().chunk, vec![1, 2, 3]);
        p.set_plugin_state("p1", "vst3", vec![9, 9]);
        assert_eq!(p.plugin_state("p1").unwrap().chunk, vec![9, 9]);
        assert_eq!(p.plugin_states.len(), 1);
    }

    #[test]
    fn plugin_state_round_trips_through_json() {
        let mut p = Project::default();
        p.set_plugin_state("p1", "clap", vec![7, 8, 9]);
        p.set_plugin_state("p2", "vst3", vec![1]);
        let json = serde_json::to_string(&p).expect("serialize");
        let back: Project = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.plugin_states.len(), 2);
        assert_eq!(back.plugin_state("p1").unwrap().format_hint, "clap");
        assert_eq!(back.plugin_state("p2").unwrap().chunk, vec![1]);
    }

    #[test]
    fn remove_plugin_state_returns_true_on_hit() {
        let mut p = Project::default();
        p.set_plugin_state("p1", "vst3", vec![0]);
        assert!(p.remove_plugin_state("p1"));
        assert!(!p.remove_plugin_state("p1"));
        assert!(p.plugin_state("p1").is_none());
    }
}
