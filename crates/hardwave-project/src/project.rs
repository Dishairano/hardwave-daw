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
    /// Display title for export metadata + the "Show on Open" splash.
    /// Distinct from `name` (which is the on-disk filename stem).
    #[serde(default)]
    pub title: String,
    /// Free-text genre tag. Round-trips into rendered WAV/MP3 metadata
    /// for distribution / DAW interop. ID3v1 has a fixed list but we
    /// accept any string — exporters can map to the closest ID3 slot.
    #[serde(default)]
    pub genre: String,
    /// Long-form description. Shown in the Project Info splash and
    /// embedded in rendered audio file comments. Free-form text.
    #[serde(default)]
    pub info: String,
    /// Homepage / contact URL. Embedded in rendered file metadata as a
    /// link the listener can click.
    #[serde(default)]
    pub url: String,
    /// When true, the Project Info dialog auto-opens whenever this
    /// project is loaded — useful for sharing a project with notes for
    /// the next collaborator.
    #[serde(default)]
    pub show_on_open: bool,
    /// Accumulated seconds the user has had the project open and
    /// active. UI ticks this periodically; reset via the Project Info
    /// dialog's "Reset working time" button.
    #[serde(default)]
    pub working_time_seconds: u64,
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
    /// Switchable arrangements (FL Studio-style). Empty on legacy
    /// projects; `ensure_arrangements()` lazily seeds "Arrangement 1".
    /// Only the active arrangement's timeline is live on the tracks at
    /// any moment — see `arrangement.rs`.
    #[serde(default)]
    pub arrangements: Vec<crate::arrangement::Arrangement>,
    /// Id of the arrangement currently applied to the live tracks.
    #[serde(default)]
    pub active_arrangement: String,
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

/// How many empty audio inserts a fresh project starts with.
///
/// Per the playlist/mixer decoupling design (mockup at
/// `playlist-mixer-decoupling-mockup`): a new project ships with 500
/// pre-allocated playlist inserts so the user never has to "Add new
/// track" — they just drop on Insert 27 if they want it there. Inserts
/// stay zero-cost when empty (no clips, no chain) so memory + audio
/// graph overhead is negligible.
pub const DEFAULT_INSERT_COUNT: usize = 500;

impl Default for Project {
    fn default() -> Self {
        let mut tracks = Vec::with_capacity(DEFAULT_INSERT_COUNT + 1);
        tracks.push(Track::new_master("master".into()));
        for i in 1..=DEFAULT_INSERT_COUNT {
            let id = format!("insert-{:03}", i);
            tracks.push(Track::new_audio(id, format!("Insert {}", i)));
        }

        Self {
            version: 1,
            metadata: ProjectMetadata {
                name: "Untitled".into(),
                author: String::new(),
                sample_rate: 48000,
                created_at: chrono::Utc::now().to_rfc3339(),
                modified_at: chrono::Utc::now().to_rfc3339(),
                title: String::new(),
                genre: String::new(),
                info: String::new(),
                url: String::new(),
                show_on_open: false,
                working_time_seconds: 0,
            },
            tempo_map: TempoMap::default(),
            tracks,
            channel_rack_state: None,
            midi_mappings: None,
            plugin_states: Vec::new(),
            arrangements: Vec::new(),
            active_arrangement: String::new(),
        }
    }
}

/// Write `bytes` to `path` atomically: temp file in the same directory,
/// fsync, then rename over the target. On Unix the parent directory is
/// fsynced too so the rename itself survives power loss.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let file_name = path
        .file_name()
        .ok_or("save path has no file name")?
        .to_string_lossy();
    let tmp_name = format!(".{}.tmp-{}", file_name, std::process::id());
    let tmp_path = match dir {
        Some(d) => d.join(&tmp_name),
        None => std::path::PathBuf::from(&tmp_name),
    };

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_path, path)?;
        #[cfg(unix)]
        if let Some(d) = dir {
            if let Ok(dirf) = std::fs::File::open(d) {
                let _ = dirf.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

impl Project {
    /// Save project to a .hwp file (MessagePack + zstd).
    ///
    /// The write is atomic: bytes land in a sibling temp file which is
    /// fsynced and then renamed over the target, so a crash or power loss
    /// mid-save can never leave a truncated .hwp behind — the previous
    /// save (and the autosave, which uses this same path) stays intact.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let data = rmp_serde::to_vec(self)?;
        let compressed = zstd::encode_all(data.as_slice(), 3)?;
        write_atomic(path, &compressed)
    }

    /// Load project from a .hwp file.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let compressed = std::fs::read(path)?;
        let data = zstd::decode_all(compressed.as_slice())?;
        let project: Project = rmp_serde::from_slice(&data)?;
        Ok(project)
    }

    /// Save as JSON (for debugging / interop). Atomic like `save`.
    pub fn save_json(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        write_atomic(path, json.as_bytes())
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
    fn save_round_trips_and_leaves_no_temp_files() {
        let dir = std::env::temp_dir().join(format!("hwp-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.hwp");

        let mut p = Project::default();
        p.set_plugin_state("p1", "vst3", vec![1, 2, 3]);
        p.save(&path).expect("save");

        let back = Project::load(&path).expect("load");
        assert_eq!(back.plugin_state("p1").unwrap().chunk, vec![1, 2, 3]);

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_overwrite_preserves_old_file_until_new_one_is_complete() {
        // Overwriting an existing .hwp goes through rename, so at every
        // instant the target path holds either the old or the new complete
        // file — never a truncated hybrid. We can't crash mid-write in a
        // unit test, but we verify the overwrite path works and the result
        // is the new content.
        let dir = std::env::temp_dir().join(format!("hwp-atomic-ow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.hwp");

        let mut p1 = Project::default();
        p1.set_plugin_state("old", "vst3", vec![1]);
        p1.save(&path).expect("first save");

        let mut p2 = Project::default();
        p2.set_plugin_state("new", "clap", vec![2]);
        p2.save(&path).expect("overwrite save");

        let back = Project::load(&path).expect("load");
        assert!(back.plugin_state("old").is_none());
        assert_eq!(back.plugin_state("new").unwrap().chunk, vec![2]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_into_missing_directory_errors_cleanly() {
        let path = std::env::temp_dir()
            .join(format!("hwp-missing-{}", std::process::id()))
            .join("nope")
            .join("test.hwp");
        let p = Project::default();
        assert!(
            p.save(&path).is_err(),
            "save into missing dir must error, not panic"
        );
    }

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

    /// A full track (insert + automation clip + plugin state) plus an
    /// arrangement must survive a real `.hwp` save → load (rmp + zstd).
    /// Guards the serialization of everything added this run.
    #[test]
    fn full_track_round_trips_through_hwp() {
        use crate::arrangement::{Arrangement, TrackTimeline};
        use crate::automation::AutomationTarget;
        use crate::automation_clip::AutomationClip;
        use crate::track::PluginSlot;
        use std::collections::HashMap;

        let mut p = Project::default();
        let tid = p.tracks[1].id.clone();
        if let Some(t) = p.track_mut(&tid) {
            t.inserts.push(PluginSlot {
                id: "s1".into(),
                plugin_id: "hardwave.native.reverb".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 0.8,
            });
            t.automation_clips.push(AutomationClip::new(
                "ac1",
                AutomationTarget::TrackVolume,
                0,
                1920,
            ));
        }
        p.set_plugin_state("s1", "native", vec![1, 2, 3]);
        // Push an arrangement directly (create_arrangement would swap the
        // live timeline; here we only want to exercise serialization).
        let mut timelines = HashMap::new();
        timelines.insert(tid.clone(), TrackTimeline::default());
        p.arrangements.push(Arrangement {
            id: "arr1".into(),
            name: "A1".into(),
            timelines,
        });
        p.active_arrangement = "arr1".into();

        let path = std::env::temp_dir().join(format!("hw-rt-{}.hwp", uuid::Uuid::new_v4()));
        p.save(&path).expect("save");
        let loaded = Project::load(&path).expect("load");
        std::fs::remove_file(&path).ok();

        let lt = loaded.track(&tid).expect("track survives");
        assert_eq!(lt.inserts.len(), 1, "insert survives");
        assert!(
            (lt.inserts[0].wet - 0.8).abs() < 1e-6,
            "insert wet survives"
        );
        assert_eq!(lt.automation_clips.len(), 1, "automation clip survives");
        assert_eq!(
            loaded.plugin_state("s1").map(|e| e.chunk.clone()),
            Some(vec![1, 2, 3]),
            "plugin state survives"
        );
        assert_eq!(loaded.arrangements.len(), 1, "arrangement survives");
        assert_eq!(
            loaded.active_arrangement, "arr1",
            "active arrangement survives"
        );
    }
}
