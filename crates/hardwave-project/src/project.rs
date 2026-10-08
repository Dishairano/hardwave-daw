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

/// The project file format this build writes.
///
/// 1: MessagePack, struct written as a positional array.
/// 2: MessagePack with field names, so the order of fields in the code no
///    longer decides whether an older song opens correctly.
///
/// A file claiming a higher number than this is refused on load instead of
/// being read as whatever happens to line up.
pub const FORMAT_VERSION: u32 = 2;

/// A film or a game capture, played in time with the song.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoTrack {
    pub path: String,
    /// Where the first frame sits in the song, in ticks.
    #[serde(default)]
    pub offset_ticks: u64,
    #[serde(default)]
    pub muted: bool,
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
    /// `channel_rack_state` read into the step sequencer's notes. Not
    /// saved: it is made from the rack state when that is set or loaded.
    #[serde(skip)]
    pub step_rack: Option<crate::step_rack::StepRack>,
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
    /// Opaque JSON for timeline state the UI owns: markers and the punch
    /// range. Same ferry pattern as `channel_rack_state`. These lived in the
    /// browser's localStorage, so they followed the machine instead of the
    /// song: move or share a .hwp and the markers were gone, while the next
    /// project you opened inherited them.
    ///
    /// MUST STAY LAST. `save` writes MessagePack through `rmp_serde::to_vec`,
    /// which encodes a struct as a positional array, so a new field anywhere
    /// but the end shifts every field after it and misreads every existing
    /// project. Appending leaves older files simply shorter, and serde fills
    /// the gap from `default`.
    #[serde(default)]
    pub timeline_state: Option<String>,
    /// Macros: one knob that moves several parameters across several
    /// plug-ins. Appended after `timeline_state` for the same reason that
    /// field carries its warning: a positional file written by version 1
    /// simply stops early and serde fills this from `default`.
    #[serde(default)]
    pub macros: Vec<crate::macros::Macro>,
    /// VCA groups: faders that ride several tracks without routing them
    /// anywhere. Appended after `macros` for the same reason that field
    /// sits where it does.
    #[serde(default)]
    pub vcas: Vec<crate::vca::Vca>,
    /// Modulation routes: a source that keeps running wired to a knob.
    #[serde(default)]
    pub modulations: Vec<crate::modulation::ModRoute>,
    /// The rows of the launcher grid, by name. A row is a section:
    /// launch it and every track with a clip in that row starts
    /// together.
    #[serde(default)]
    pub scenes: Vec<String>,
    /// A video to score to: the file, and where in the song its first
    /// frame sits. Stored at the end of the struct, like every field
    /// added after a release, because the save format reads a struct
    /// by position.
    #[serde(default)]
    pub video: Option<VideoTrack>,
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
            version: FORMAT_VERSION,
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
            step_rack: None,
            midi_mappings: None,
            plugin_states: Vec::new(),
            arrangements: Vec::new(),
            active_arrangement: String::new(),
            timeline_state: None,
            macros: Vec::new(),
            vcas: Vec::new(),
            modulations: Vec::new(),
            video: None,
            scenes: vec![
                "Scene 1".to_string(),
                "Scene 2".to_string(),
                "Scene 3".to_string(),
                "Scene 4".to_string(),
            ],
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
        // Written with FIELD NAMES (`to_vec_named`), not as a positional
        // array. Until format 2 the file was an array in struct order, so a
        // field added anywhere but the end shifted every field after it and
        // misread every existing project: the whole save format rested on
        // nobody ever inserting a line in the wrong place. Named fields cost
        // a little size and end that class of accident. Reading still takes
        // either shape, so files written before this keep opening.
        let mut data = rmp_serde::to_vec_named(self)?;
        // `version` is a field like any other, and a project loaded from an
        // older file carries the version it was migrated to, so the value
        // written here is already FORMAT_VERSION. This guards the one case
        // where it is not: a Project built by hand in code.
        if self.version != FORMAT_VERSION {
            let mut copy = self.clone();
            copy.version = FORMAT_VERSION;
            data = rmp_serde::to_vec_named(&copy)?;
        }
        let compressed = zstd::encode_all(data.as_slice(), 3)?;
        write_atomic(path, &compressed)
    }

    /// The same bytes `save` writes, without a file.
    ///
    /// Working on a song together means the other person needs the
    /// song, and sending it is the one moment where the whole project
    /// crosses the network rather than one edit at a time.
    pub fn to_bytes(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut copy = self.clone();
        copy.version = FORMAT_VERSION;
        let data = rmp_serde::to_vec_named(&copy)?;
        Ok(zstd::encode_all(data.as_slice(), 3)?)
    }

    /// Read what `to_bytes` wrote.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        let data = decompress_capped(bytes)?;
        let mut project: Project = rmp_serde::from_slice(&data)?;
        if project.version > FORMAT_VERSION {
            return Err(format!(
                "That song was made by a newer version of Hardwave DAW \
                 (project format {}, this build reads up to {}).",
                project.version, FORMAT_VERSION
            )
            .into());
        }
        project.migrate();
        project.make_safe()?;
        project.refresh_step_rack();
        Ok(project)
    }

    /// Load project from a .hwp file.
    ///
    /// Accepts both shapes: the named-field format written from format 2 on,
    /// and the positional one written before it. A file from a newer DAW is
    /// refused by name rather than decoded into something that looks right
    /// and is not.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        if std::fs::metadata(path)?.len() > MAX_FILE_BYTES {
            return Err("That file is far larger than any song; it was not opened.".into());
        }
        let compressed = std::fs::read(path)?;
        let data = decompress_capped(&compressed)?;
        let mut project: Project = rmp_serde::from_slice(&data)?;
        if project.version > FORMAT_VERSION {
            return Err(format!(
                "This song was saved by a newer version of Hardwave DAW \
                 (project format {}, this build reads up to {}). \
                 Update the DAW and open it again.",
                project.version, FORMAT_VERSION
            )
            .into());
        }
        project.migrate();
        project.make_safe()?;
        project.refresh_step_rack();
        Ok(project)
    }

    /// Bring a project loaded from an older format up to the current one.
    ///
    /// Format 1 to 2 changed how the file is written, not what it holds, so
    /// there is nothing to move: reading it already produced the right
    /// values. Later steps go here, each one keyed on the version it starts
    /// from, and the version is stamped at the end so the next save writes
    /// the current format.
    fn migrate(&mut self) {
        self.version = FORMAT_VERSION;
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

    /// Clone a track right after itself (FL's "Clone selected"): settings,
    /// clips, sends, automation and plug-ins. `live_states` holds the
    /// plug-ins' state as they play now, by slot id; a slot not in it keeps
    /// its saved state. Slots, clips, lanes and automation clips get new
    /// ids, and automation that drove a plug-in of the original drives the
    /// same plug-in of the clone. Returns the clone's id.
    pub fn duplicate_track(
        &mut self,
        id: &str,
        live_states: &std::collections::HashMap<String, Vec<u8>>,
    ) -> Option<String> {
        use crate::automation::AutomationTarget;
        use crate::clip::ClipContent;
        let fresh = || uuid::Uuid::new_v4().to_string();
        let index = self.tracks.iter().position(|t| t.id == id)?;
        let mut copy = self.tracks[index].clone();
        let new_id = fresh();
        copy.id = new_id.clone();
        copy.name = format!("{} #2", copy.name);
        copy.armed = false;
        copy.frozen = false;
        let mut slot_ids = std::collections::HashMap::new();
        let mut states = Vec::new();
        for slot in copy.inserts.iter_mut() {
            let old = std::mem::replace(&mut slot.id, fresh());
            let bytes = live_states
                .get(&old)
                .cloned()
                .or_else(|| self.plugin_state(&old).map(|e| e.chunk.clone()))
                .or_else(|| slot.state.clone());
            if let Some(bytes) = bytes {
                slot.state = Some(bytes.clone());
                states.push((slot.id.clone(), bytes));
            }
            slot_ids.insert(old, slot.id.clone());
        }
        let remap = |target: &mut AutomationTarget| {
            if let AutomationTarget::PluginParam { slot_id, .. } = target {
                if let Some(new) = slot_ids.get(slot_id) {
                    *slot_id = new.clone();
                }
            }
        };
        for clip in copy.clips.iter_mut() {
            clip.track_id = new_id.clone();
            match &mut clip.content {
                ClipContent::Audio(a) => a.id = fresh(),
                ClipContent::Midi(m) => m.id = fresh(),
            }
        }
        for lane in copy.automation_lanes.iter_mut() {
            lane.id = fresh();
            remap(&mut lane.target);
        }
        for clip in copy.automation_clips.iter_mut() {
            clip.id = fresh();
            remap(&mut clip.target);
            remap(&mut clip.lane.target);
        }
        for (slot_id, bytes) in states {
            self.set_plugin_state(slot_id, "unknown", bytes);
        }
        self.tracks.insert(index + 1, copy);
        Some(new_id)
    }

    /// Set the channel rack's state (the UI's JSON) and read its steps.
    pub fn set_channel_rack_state(&mut self, payload: Option<String>) {
        self.channel_rack_state = payload;
        self.refresh_step_rack();
    }

    /// Put the rack's active pattern on the playlist at `tick`: one MIDI
    /// clip per channel that has steps in it, on that channel, as long as
    /// the pattern. In song mode a pattern only plays from the playlist, so
    /// this is how steps get into a song. Returns how many clips it placed.
    pub fn place_pattern(&mut self, tick: u64) -> usize {
        let Some(rack) = self.step_rack.clone() else {
            return 0;
        };
        let length = rack.length_ticks();
        let mut placed = 0;
        for track in self.tracks.iter_mut() {
            let notes = rack.notes_for(&track.id);
            if notes.is_empty() {
                continue;
            }
            let id = uuid::Uuid::new_v4().to_string();
            let mut clip = hardwave_midi::MidiClip::new(id.clone(), "Pattern".into(), length);
            clip.notes = notes
                .into_iter()
                .map(|n| hardwave_midi::MidiNote {
                    start_tick: n.tick,
                    duration_ticks: n.duration_ticks,
                    pitch: n.pitch,
                    velocity: n.velocity,
                    pan: n.pan,
                    fine_cents: n.fine_cents,
                    release_velocity: n.release_velocity,
                    ..Default::default()
                })
                .collect();
            track.clips.push(crate::clip::ClipPlacement {
                content: crate::clip::ClipContent::Midi(crate::clip::MidiClipRef { id, clip }),
                track_id: track.id.clone(),
                position_ticks: tick,
                length_ticks: length,
                lane: 0,
            });
            placed += 1;
        }
        placed
    }

    /// Read the steps from the saved rack state (after a load).
    pub fn refresh_step_rack(&mut self) {
        self.step_rack = self
            .channel_rack_state
            .as_deref()
            .and_then(crate::step_rack::StepRack::parse);
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

/// The largest .hwp opened at all.
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// The most a project may expand to when it is unpacked. A few
/// kilobytes of zstd can claim to hold gigabytes; past this it is not
/// a song.
pub const MAX_DECODED_BYTES: u64 = 512 * 1024 * 1024;

/// Unpack a project, stopping at `MAX_DECODED_BYTES` rather than
/// trusting what the data claims to expand to.
fn decompress_capped(bytes: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    decompress_within(bytes, MAX_DECODED_BYTES)
}

fn decompress_within(bytes: &[u8], cap: u64) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let decoder = zstd::stream::read::Decoder::new(bytes)?;
    let mut data = Vec::new();
    decoder.take(cap + 1).read_to_end(&mut data)?;
    if data.len() as u64 > cap {
        return Err(
            "That song unpacks to more than any song could hold; it was not opened.".into(),
        );
    }
    Ok(data)
}

#[cfg(test)]
mod decompress_tests {
    use super::*;

    #[test]
    fn a_small_file_that_unpacks_to_far_too_much_is_refused() {
        // Four megabytes of nothing packs into a few hundred bytes.
        let bomb = zstd::encode_all(vec![0u8; 4 * 1024 * 1024].as_slice(), 19).unwrap();
        assert!(bomb.len() < 4096, "{}", bomb.len());
        assert!(decompress_within(&bomb, 1024 * 1024).is_err());
        assert_eq!(
            decompress_within(&bomb, 8 * 1024 * 1024).unwrap().len(),
            4 * 1024 * 1024
        );
    }
}

#[cfg(test)]
mod tests {
    /// Placing a pattern puts one clip per channel with steps, where asked.
    #[test]
    fn a_pattern_goes_on_the_playlist_per_channel() {
        let mut p = Project::default();
        let a = p.add_midi_track("Kick".into());
        let b = p.add_midi_track("Clap".into());
        let _c = p.add_midi_track("Empty".into());
        p.set_channel_rack_state(Some(format!(
            r#"{{"activeId":"p","patterns":[{{"id":"p","steps":{{"{a}":[1,0,0,0,1],"{b}":[0,0,0,0,1]}}}}]}}"#
        )));
        assert_eq!(p.place_pattern(3840), 2);
        let kick = p.track(&a).unwrap();
        assert_eq!(kick.clips.len(), 1);
        assert_eq!(kick.clips[0].position_ticks, 3840);
        let crate::clip::ClipContent::Midi(m) = &kick.clips[0].content else {
            panic!("a MIDI clip")
        };
        assert_eq!(m.clip.notes.len(), 2);
        assert_eq!(p.track(&b).unwrap().clips.len(), 1);
    }

    /// A clone has its own ids everywhere, keeps the plug-ins' live state,
    /// and its automation drives its own plug-in, not the original's.
    #[test]
    fn a_cloned_track_owns_its_plugins_and_automation() {
        use crate::automation::{AutomationLane, AutomationTarget};
        let mut p = Project::default();
        let id = p.add_midi_track("Lead".into());
        {
            let t = p.track_mut(&id).unwrap();
            t.inserts.push(crate::track::PluginSlot {
                id: "slot-a".into(),
                plugin_id: "hardwave.native.filter".into(),
                enabled: true,
                state: Some(vec![1]),
                sidechain_source: None,
                wet: 1.0,
            });
            t.automation_lanes.push(AutomationLane {
                id: "lane-a".into(),
                target: AutomationTarget::PluginParam {
                    slot_id: "slot-a".into(),
                    param_id: 2,
                },
                points: Vec::new(),
                visible: true,
            });
        }
        let mut live = std::collections::HashMap::new();
        live.insert("slot-a".to_string(), vec![9, 9]);
        let clone = p.duplicate_track(&id, &live).unwrap();
        let idx = p.tracks.iter().position(|t| t.id == clone).unwrap();
        assert_eq!(p.tracks[idx - 1].id, id, "right after the original");
        let c = p.track(&clone).unwrap();
        assert_eq!(c.name, "Lead #2");
        let slot = &c.inserts[0];
        assert_ne!(slot.id, "slot-a");
        assert_eq!(
            slot.state.as_deref(),
            Some(&[9u8, 9][..]),
            "the live state, not the saved one"
        );
        assert_eq!(
            p.plugin_state(&slot.id).map(|e| e.chunk.clone()),
            Some(vec![9, 9])
        );
        assert_ne!(c.automation_lanes[0].id, "lane-a");
        assert!(matches!(&c.automation_lanes[0].target,
            AutomationTarget::PluginParam { slot_id, param_id: 2 } if *slot_id == slot.id));
        let orig = p.track(&id).unwrap();
        assert_eq!(orig.inserts[0].id, "slot-a", "the original is untouched");
    }

    use super::*;

    #[test]
    fn a_song_survives_being_sent_to_the_other_person() {
        let mut project = Project::default();
        let track = project.add_midi_track("Kick".into());
        project.track_mut(&track).unwrap().volume_db = -4.5;
        project.scenes.push("Drop".into());

        let bytes = project.to_bytes().expect("to bytes");
        let back = Project::from_bytes(&bytes).expect("and back");

        assert_eq!(back.tracks.len(), project.tracks.len());
        assert_eq!(back.track(&track).unwrap().volume_db, -4.5);
        assert!(back.scenes.contains(&"Drop".to_string()));
        assert_eq!(back.version, FORMAT_VERSION);
    }

    #[test]
    fn timeline_state_round_trips_and_defaults_on_legacy_projects() {
        let dir = std::env::temp_dir().join(format!("hwp-timeline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("timeline.hwp");

        let mut p = Project::default();
        assert_eq!(
            p.timeline_state, None,
            "a new project has no timeline state"
        );
        p.timeline_state = Some(r#"{"markers":[{"id":"mk_1","tick":1920}]}"#.to_string());
        p.save(&path).expect("save");

        let back = Project::load(&path).expect("load");
        assert_eq!(back.timeline_state.as_deref(), p.timeline_state.as_deref());

        // A project written before this field existed must still load. The
        // format is MessagePack via rmp_serde::to_vec, which writes a struct
        // as a positional array, so this mirrors the exact field order of the
        // previous layout: one element short, with no timeline_state.
        #[derive(Serialize)]
        struct LegacyProject<'a> {
            version: u32,
            metadata: &'a ProjectMetadata,
            tempo_map: &'a TempoMap,
            tracks: Vec<Track>,
            channel_rack_state: Option<String>,
            midi_mappings: Option<String>,
            plugin_states: Vec<PluginStateEntry>,
            arrangements: Vec<crate::arrangement::Arrangement>,
            active_arrangement: String,
        }
        let legacy = LegacyProject {
            version: 1,
            metadata: &p.metadata,
            tempo_map: &p.tempo_map,
            tracks: Vec::new(),
            channel_rack_state: Some("{}".into()),
            midi_mappings: None,
            plugin_states: Vec::new(),
            arrangements: Vec::new(),
            active_arrangement: "arr-1".into(),
        };
        let legacy_path = dir.join("legacy.hwp");
        let packed = rmp_serde::to_vec(&legacy).unwrap();
        std::fs::write(
            &legacy_path,
            zstd::encode_all(packed.as_slice(), 3).unwrap(),
        )
        .unwrap();

        let loaded = Project::load(&legacy_path).expect("legacy load");
        assert_eq!(loaded.timeline_state, None, "missing field defaults");
        // Everything that came before it still lands in the right field,
        // which is what a mid-struct insertion would have broken.
        assert_eq!(loaded.channel_rack_state.as_deref(), Some("{}"));
        assert_eq!(loaded.active_arrangement, "arr-1");

        std::fs::remove_dir_all(&dir).unwrap();
    }

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
    /// A clip's controller moves have to survive a save, like its notes.
    /// They are the last field in the clip for the same reason the mute
    /// flag was: the format is positional.
    #[test]
    fn clip_controllers_round_trip_through_a_save() {
        use crate::clip::{ClipContent, ClipPlacement, MidiClipRef};
        use hardwave_midi::{MidiClip, MidiControlKind, MidiControlPoint};

        let dir = std::env::temp_dir().join(format!("hwp-controls-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("controls.hwp");

        let mut p = Project::default();
        let track_id = p.add_midi_track("Lead".to_string());
        let mut clip = MidiClip::new("c1".into(), "Take".into(), 1920);
        clip.controls = vec![
            MidiControlPoint {
                tick: 0,
                channel: 0,
                kind: MidiControlKind::Cc(1),
                value: 0.25,
            },
            MidiControlPoint {
                tick: 960,
                channel: 0,
                kind: MidiControlKind::PitchBend,
                value: -1.0,
            },
        ];
        p.track_mut(&track_id).unwrap().clips.push(ClipPlacement {
            content: ClipContent::Midi(MidiClipRef {
                id: "c1".into(),
                clip,
            }),
            track_id: track_id.clone(),
            position_ticks: 0,
            length_ticks: 1920,
            lane: 0,
        });
        p.save(&path).expect("save");

        let back = Project::load(&path).expect("load");
        let ClipContent::Midi(ref m) = back.track(&track_id).unwrap().clips[0].content else {
            panic!("the clip came back as something else")
        };
        assert_eq!(m.clip.controls.len(), 2);
        assert_eq!(m.clip.controls[0].kind, MidiControlKind::Cc(1));
        assert!((m.clip.controls[0].value - 0.25).abs() < 1e-6);
        assert_eq!(m.clip.controls[1].kind, MidiControlKind::PitchBend);
        assert!((m.clip.controls[1].value + 1.0).abs() < 1e-6);

        let _ = std::fs::remove_file(&path);
    }
    /// The point of format 2: the order fields sit in the file no longer
    /// decides whether a song opens correctly. Before it, this file would
    /// have been read as version = whatever landed in slot one.
    #[test]
    fn a_file_whose_fields_are_in_another_order_still_opens() {
        let dir = std::env::temp_dir().join(format!("hwp-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("order.hwp");

        // The same field names, declared in a deliberately different order,
        // and missing several: what a file written by another build with a
        // different field layout looks like.
        #[derive(Serialize)]
        struct Reordered<'a> {
            timeline_state: Option<String>,
            metadata: &'a ProjectMetadata,
            active_arrangement: String,
            version: u32,
            tracks: Vec<Track>,
            tempo_map: &'a TempoMap,
        }
        let p = Project::default();
        let reordered = Reordered {
            timeline_state: Some(r#"{"markers":[]}"#.into()),
            metadata: &p.metadata,
            active_arrangement: "arr-1".into(),
            version: FORMAT_VERSION,
            tracks: Vec::new(),
            tempo_map: &p.tempo_map,
        };
        let data = rmp_serde::to_vec_named(&reordered).unwrap();
        std::fs::write(&path, zstd::encode_all(data.as_slice(), 3).unwrap()).unwrap();

        let back = Project::load(&path).expect("load");
        assert_eq!(back.version, FORMAT_VERSION);
        assert_eq!(back.active_arrangement, "arr-1");
        assert_eq!(back.timeline_state.as_deref(), Some(r#"{"markers":[]}"#));
        assert!(back.tracks.is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_saved_project_carries_the_current_format_version() {
        let dir = std::env::temp_dir().join(format!("hwp-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("version.hwp");

        // As an older file would have it in memory.
        let p = Project {
            version: 1,
            ..Project::default()
        };
        p.save(&path).expect("save");
        let back = Project::load(&path).expect("load");
        assert_eq!(back.version, FORMAT_VERSION);
        let _ = std::fs::remove_file(&path);
    }

    /// Opening a song from a newer DAW must say so, not decode it into
    /// something that looks plausible.
    #[test]
    fn a_file_from_a_newer_daw_is_refused_by_name() {
        let dir = std::env::temp_dir().join(format!("hwp-newer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("newer.hwp");

        let p = Project {
            version: FORMAT_VERSION + 5,
            ..Project::default()
        };
        // Write it without going through `save`, which would stamp the
        // current version.
        let data = rmp_serde::to_vec_named(&p).unwrap();
        std::fs::write(&path, zstd::encode_all(data.as_slice(), 3).unwrap()).unwrap();

        let err = Project::load(&path).expect_err("a newer file must not load");
        let text = err.to_string();
        assert!(text.contains("newer version"), "unhelpful message: {text}");
        assert!(text.contains(&FORMAT_VERSION.to_string()));
        let _ = std::fs::remove_file(&path);
    }
}
