//! Scripting API — contract types for the DAW's Lua scripting
//! engine, macro recording, REST + WebSocket + OSC bridges, and
//! MIDI controller scripting mappings.
//!
//! The actual Lua VM, HTTP server, and WebSocket runtime live in
//! separate tiers; this module defines the commands, events, and
//! catalogs that each bridge speaks so the UI can exercise macros,
//! shortcuts, and external controllers independently.

use serde::{Deserialize, Serialize};

/// A Lua script entry. Scripts are user-authored or generated via
/// macro recording; each has a stable id so shortcut bindings
/// survive renames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: String,
    pub name: String,
    pub body: String,
    pub kind: ScriptKind,
    pub keyboard_shortcut: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptKind {
    UserAuthored,
    MacroRecording,
    Generated,
}

impl Script {
    pub fn new(id: impl Into<String>, name: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            body: body.into(),
            kind: ScriptKind::UserAuthored,
            keyboard_shortcut: None,
        }
    }
}

/// Library of scripts — stored on disk, loaded into the scripting
/// engine on startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptLibrary {
    pub scripts: Vec<Script>,
}

impl ScriptLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, script: Script) -> bool {
        if self.scripts.iter().any(|s| s.id == script.id) {
            return false;
        }
        self.scripts.push(script);
        true
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.scripts.len();
        self.scripts.retain(|s| s.id != id);
        self.scripts.len() != before
    }

    pub fn by_shortcut(&self, chord: &str) -> Option<&Script> {
        self.scripts
            .iter()
            .find(|s| s.keyboard_shortcut.as_deref() == Some(chord))
    }

    pub fn bind_shortcut(&mut self, id: &str, chord: impl Into<String>) -> bool {
        // Clear any other binding on the same chord so shortcuts are
        // exclusive.
        let chord = chord.into();
        for s in self.scripts.iter_mut() {
            if s.keyboard_shortcut.as_deref() == Some(chord.as_str()) && s.id != id {
                s.keyboard_shortcut = None;
            }
        }
        if let Some(target) = self.scripts.iter_mut().find(|s| s.id == id) {
            target.keyboard_shortcut = Some(chord);
            return true;
        }
        false
    }
}

/// Host command — what scripts invoke into the DAW. The Lua VM
/// binds each variant as a standalone function; the REST and
/// WebSocket bridges tunnel the same commands via JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ScriptCommand {
    /// Transport — play / stop / seek.
    TransportPlay,
    TransportStop,
    TransportSeek {
        tick: u64,
    },
    /// Track state.
    SetTrackVolume {
        track_id: String,
        db: f32,
    },
    SetTrackPan {
        track_id: String,
        pan: f32,
    },
    SetTrackMuted {
        track_id: String,
        muted: bool,
    },
    /// Mixer state.
    SetMasterVolume {
        db: f32,
    },
    /// Clip operations.
    CreateClip {
        track_id: String,
        start_tick: u64,
        length_ticks: u64,
    },
    MoveClip {
        clip_id: String,
        new_start_tick: u64,
    },
    DeleteClip {
        clip_id: String,
    },
    /// Piano roll.
    InsertNote {
        clip_id: String,
        tick: u64,
        pitch: u8,
        velocity: u8,
        length_ticks: u64,
    },
    DeleteNote {
        clip_id: String,
        tick: u64,
        pitch: u8,
    },
    /// UI automation — open a panel by name.
    OpenPanel {
        panel_id: String,
    },
    /// Run another script from a script.
    RunScript {
        script_id: String,
    },
}

/// Macro recorder — captures a sequence of `ScriptCommand`s so the
/// user can replay them or export as a Lua script. Recording is
/// toggled on / off via `start()` + `stop()`; the returned sequence
/// is what gets written to the script library.
#[derive(Debug, Clone, Default)]
pub struct MacroRecorder {
    recording: bool,
    commands: Vec<ScriptCommand>,
}

impl MacroRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self) {
        self.recording = true;
        self.commands.clear();
    }

    pub fn stop(&mut self) -> Vec<ScriptCommand> {
        self.recording = false;
        std::mem::take(&mut self.commands)
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Observe an action by the user. If recording, appends to the
    /// sequence; if not, discards.
    pub fn observe(&mut self, command: ScriptCommand) {
        if self.recording {
            self.commands.push(command);
        }
    }

    pub fn recorded(&self) -> &[ScriptCommand] {
        &self.commands
    }
}

/// Emit a captured command sequence as a Lua source string. Pairs
/// with `MacroRecorder::stop()` so a newly-recorded macro can be
/// inserted into `ScriptLibrary` right away.
pub fn macro_to_lua(commands: &[ScriptCommand]) -> String {
    let mut out = String::from("-- Generated macro\n");
    for cmd in commands {
        match cmd {
            ScriptCommand::TransportPlay => out.push_str("transport.play()\n"),
            ScriptCommand::TransportStop => out.push_str("transport.stop()\n"),
            ScriptCommand::TransportSeek { tick } => {
                out.push_str(&format!("transport.seek({})\n", tick));
            }
            ScriptCommand::SetTrackVolume { track_id, db } => {
                out.push_str(&format!("track.set_volume(\"{}\", {})\n", track_id, db))
            }
            ScriptCommand::SetTrackPan { track_id, pan } => {
                out.push_str(&format!("track.set_pan(\"{}\", {})\n", track_id, pan))
            }
            ScriptCommand::SetTrackMuted { track_id, muted } => {
                out.push_str(&format!("track.set_muted(\"{}\", {})\n", track_id, muted))
            }
            ScriptCommand::SetMasterVolume { db } => {
                out.push_str(&format!("mixer.set_master_volume({})\n", db));
            }
            ScriptCommand::CreateClip {
                track_id,
                start_tick,
                length_ticks,
            } => out.push_str(&format!(
                "clip.create(\"{}\", {}, {})\n",
                track_id, start_tick, length_ticks
            )),
            ScriptCommand::MoveClip {
                clip_id,
                new_start_tick,
            } => out.push_str(&format!(
                "clip.move_to(\"{}\", {})\n",
                clip_id, new_start_tick
            )),
            ScriptCommand::DeleteClip { clip_id } => {
                out.push_str(&format!("clip.delete(\"{}\")\n", clip_id));
            }
            ScriptCommand::InsertNote {
                clip_id,
                tick,
                pitch,
                velocity,
                length_ticks,
            } => out.push_str(&format!(
                "notes.insert(\"{}\", {}, {}, {}, {})\n",
                clip_id, tick, pitch, velocity, length_ticks
            )),
            ScriptCommand::DeleteNote {
                clip_id,
                tick,
                pitch,
            } => out.push_str(&format!(
                "notes.delete(\"{}\", {}, {})\n",
                clip_id, tick, pitch
            )),
            ScriptCommand::OpenPanel { panel_id } => {
                out.push_str(&format!("ui.open(\"{}\")\n", panel_id));
            }
            ScriptCommand::RunScript { script_id } => {
                out.push_str(&format!("script.run(\"{}\")\n", script_id));
            }
        }
    }
    out
}

/// REST API transport — commands inbound via `POST /v1/command`
/// + events outbound via Server-Sent-Events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestRequest {
    pub command: ScriptCommand,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestResponse {
    pub ok: bool,
    pub error: Option<String>,
}

/// MIDI controller custom-mapping — maps a CC message to a script
/// command. Drives the "MIDI controller scripting" feature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MidiMapping {
    pub cc_number: u8,
    pub cc_channel: u8,
    pub target: MidiMappingTarget,
    pub scale_min: f32,
    pub scale_max: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MidiMappingTarget {
    RunScript { script_id: String },
    SetTrackVolume { track_id: String },
    SetTrackPan { track_id: String },
    TransportToggle,
}

impl MidiMapping {
    /// Translate a raw CC value (`0..=127`) into the mapped
    /// parameter range. Returns the scaled value.
    pub fn map_cc_value(&self, raw: u8) -> f32 {
        let normalized = (raw as f32 / 127.0).clamp(0.0, 1.0);
        self.scale_min + (self.scale_max - self.scale_min) * normalized
    }
}

/// Catalog of MIDI mappings — the MIDI engine reads this to route
/// incoming CC messages to script invocations.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MidiMappingTable {
    pub mappings: Vec<MidiMapping>,
}

impl MidiMappingTable {
    pub fn resolve(&self, cc_number: u8, cc_channel: u8) -> Option<&MidiMapping> {
        self.mappings
            .iter()
            .find(|m| m.cc_number == cc_number && m.cc_channel == cc_channel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_library_add_is_idempotent() {
        let mut lib = ScriptLibrary::new();
        assert!(lib.add(Script::new("s1", "Hello", "print('hi')")));
        assert!(!lib.add(Script::new("s1", "Duplicate", "")));
        assert_eq!(lib.scripts.len(), 1);
    }

    #[test]
    fn shortcut_binding_is_exclusive() {
        let mut lib = ScriptLibrary::new();
        lib.add(Script::new("a", "A", ""));
        lib.add(Script::new("b", "B", ""));
        assert!(lib.bind_shortcut("a", "ctrl+shift+1"));
        assert_eq!(
            lib.by_shortcut("ctrl+shift+1").map(|s| s.id.as_str()),
            Some("a")
        );
        assert!(lib.bind_shortcut("b", "ctrl+shift+1"));
        assert_eq!(
            lib.by_shortcut("ctrl+shift+1").map(|s| s.id.as_str()),
            Some("b")
        );
        // Script a loses its binding.
        assert!(lib
            .scripts
            .iter()
            .find(|s| s.id == "a")
            .unwrap()
            .keyboard_shortcut
            .is_none());
    }

    #[test]
    fn macro_recorder_only_appends_while_recording() {
        let mut rec = MacroRecorder::new();
        rec.observe(ScriptCommand::TransportPlay);
        assert!(rec.recorded().is_empty());
        rec.start();
        rec.observe(ScriptCommand::TransportPlay);
        rec.observe(ScriptCommand::TransportStop);
        assert_eq!(rec.recorded().len(), 2);
        let recorded = rec.stop();
        assert_eq!(recorded.len(), 2);
        assert!(!rec.is_recording());
        // A second start clears the buffer.
        rec.start();
        assert!(rec.recorded().is_empty());
    }

    #[test]
    fn macro_to_lua_emits_every_command_variant() {
        let commands = vec![
            ScriptCommand::TransportPlay,
            ScriptCommand::TransportSeek { tick: 480 },
            ScriptCommand::SetTrackVolume {
                track_id: "t1".into(),
                db: -6.0,
            },
            ScriptCommand::InsertNote {
                clip_id: "c1".into(),
                tick: 0,
                pitch: 60,
                velocity: 100,
                length_ticks: 480,
            },
        ];
        let lua = macro_to_lua(&commands);
        assert!(lua.contains("transport.play()"));
        assert!(lua.contains("transport.seek(480)"));
        assert!(lua.contains("track.set_volume(\"t1\", -6)"));
        assert!(lua.contains("notes.insert(\"c1\", 0, 60, 100, 480)"));
    }

    #[test]
    fn midi_mapping_scales_cc_into_target_range() {
        let m = MidiMapping {
            cc_number: 74,
            cc_channel: 0,
            target: MidiMappingTarget::SetTrackVolume {
                track_id: "t1".into(),
            },
            scale_min: -60.0,
            scale_max: 0.0,
        };
        assert!((m.map_cc_value(0) - (-60.0)).abs() < 1e-3);
        assert!((m.map_cc_value(127) - 0.0).abs() < 1e-3);
        assert!((m.map_cc_value(64) - (-29.76)).abs() < 0.5);
    }

    #[test]
    fn midi_mapping_table_resolves_by_cc_and_channel() {
        let mut table = MidiMappingTable::default();
        table.mappings.push(MidiMapping {
            cc_number: 74,
            cc_channel: 0,
            target: MidiMappingTarget::TransportToggle,
            scale_min: 0.0,
            scale_max: 1.0,
        });
        assert!(table.resolve(74, 0).is_some());
        assert!(table.resolve(75, 0).is_none());
        assert!(table.resolve(74, 1).is_none());
    }

    #[test]
    fn rest_response_serializes_roundtrip() {
        let res = RestResponse {
            ok: false,
            error: Some("bad parameter".into()),
        };
        let s = serde_json::to_string(&res).unwrap();
        let back: RestResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(res, back);
    }
}
