//! MIDI Learn — runtime mapping store for CC-to-parameter bindings plus a
//! polling dispatcher that drains the engine's MIDI input queue and applies
//! mapped CC values to the live project state.
//!
//! Mappings persist to `midi_mappings.json` in the app config dir so they
//! survive restarts independently of the current project.

use crate::prefs::prefs_dir;
use hardwave_engine::DawEngine;
use hardwave_midi::MidiEvent;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MidiMapTarget {
    MasterVolume,
    TrackVolume { track_id: String },
    TrackPan { track_id: String },
    TrackMute { track_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiMapping {
    pub id: u32,
    pub cc: u8,
    /// `None` matches any channel. Otherwise only CCs on this channel apply.
    pub channel: Option<u8>,
    pub target: MidiMapTarget,
}

#[derive(Default)]
pub struct MidiMappings {
    pub mappings: Vec<MidiMapping>,
    pub learn: Option<MidiMapTarget>,
    pub last_learned: Option<MidiMapping>,
}

impl MidiMappings {
    fn file_path() -> Option<PathBuf> {
        prefs_dir().map(|d| d.join("midi_mappings.json"))
    }

    pub fn load() -> Self {
        let path = match Self::file_path() {
            Some(p) => p,
            None => return Self::default(),
        };
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => return Self::default(),
        };
        let mappings: Vec<MidiMapping> = serde_json::from_slice(&bytes).unwrap_or_default();
        let max_id = mappings.iter().map(|m| m.id).max().unwrap_or(0);
        NEXT_ID.store(max_id + 1, Ordering::Relaxed);
        Self {
            mappings,
            learn: None,
            last_learned: None,
        }
    }

    pub fn save(&self) {
        let path = match Self::file_path() {
            Some(p) => p,
            None => return,
        };
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = match serde_json::to_vec_pretty(&self.mappings) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("Serialize midi mappings failed: {e}");
                return;
            }
        };
        if let Err(e) = fs::write(&path, &body) {
            log::warn!("Write midi mappings failed: {e}");
        }
    }

    pub fn add(&mut self, cc: u8, channel: Option<u8>, target: MidiMapTarget) -> MidiMapping {
        self.mappings
            .retain(|m| !(m.cc == cc && m.channel == channel && m.target == target));
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let m = MidiMapping {
            id,
            cc,
            channel,
            target,
        };
        self.mappings.push(m.clone());
        m
    }

    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.mappings.len();
        self.mappings.retain(|m| m.id != id);
        self.mappings.len() != before
    }

    pub fn clear(&mut self) {
        self.mappings.clear();
    }
}

/// Background worker that drains MIDI events every ~8 ms and applies any
/// mapped CC values to the live engine state. Handles learn-mode capture in
/// the same pass so the next CC becomes the new mapping.
pub fn spawn_dispatcher(engine: Arc<Mutex<DawEngine>>, mappings: Arc<Mutex<MidiMappings>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(8));
        let events = {
            let eng = engine.lock();
            let mgr = eng.midi_input.lock();
            mgr.drain_events()
        };
        if events.is_empty() {
            continue;
        }
        for ev in events {
            let (cc, channel, value) = match ev {
                MidiEvent::ControlChange {
                    cc, channel, value, ..
                } => (cc, channel, value),
                _ => continue,
            };

            let learn_target = {
                let mut m = mappings.lock();
                m.learn.take()
            };
            if let Some(target) = learn_target {
                let mapping = {
                    let mut m = mappings.lock();
                    let mapping = m.add(cc, None, target);
                    m.last_learned = Some(mapping.clone());
                    m.save();
                    mapping
                };
                log::info!("MIDI Learn captured CC {cc} → {:?}", mapping.target);
                apply_cc(&engine, &mapping.target, value);
                continue;
            }

            let applicable: Vec<MidiMapTarget> = {
                let m = mappings.lock();
                m.mappings
                    .iter()
                    .filter(|mp| mp.cc == cc && mp.channel.is_none_or(|c| c == channel))
                    .map(|mp| mp.target.clone())
                    .collect()
            };
            for target in applicable {
                apply_cc(&engine, &target, value);
            }
        }
    });
}

fn apply_cc(engine: &Arc<Mutex<DawEngine>>, target: &MidiMapTarget, value: f32) {
    use std::sync::atomic::Ordering;
    match target {
        MidiMapTarget::MasterVolume => {
            let db = (-60.0 + value as f64 * 60.0).clamp(-60.0, 6.0);
            let eng = engine.lock();
            eng.transport.master_volume_db.store(db, Ordering::Relaxed);
        }
        MidiMapTarget::TrackVolume { track_id } => {
            let db = -60.0 + value as f64 * 60.0;
            let eng = engine.lock();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(track_id) {
                    track.volume_db = db;
                }
            }
            eng.rebuild_graph();
        }
        MidiMapTarget::TrackPan { track_id } => {
            let pan = (value as f64 * 2.0 - 1.0).clamp(-1.0, 1.0);
            let eng = engine.lock();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(track_id) {
                    track.pan = pan;
                }
            }
            eng.rebuild_graph();
        }
        MidiMapTarget::TrackMute { track_id } => {
            let muted = value > 0.5;
            let eng = engine.lock();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(track_id) {
                    track.muted = muted;
                }
            }
            eng.rebuild_graph();
        }
    }
}
