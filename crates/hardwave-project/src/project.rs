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

    pub fn remove_track(&mut self, id: &str) {
        self.tracks.retain(|t| t.id != id);
    }

    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
}
