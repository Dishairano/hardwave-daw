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

// Fields are camelCase like the variants, since that is what the app sends
// ({ kind: 'pluginParam', trackId, slotId, paramId }). They were snake_case,
// so a plug-in parameter could never be learned (its fields were "missing")
// and a channel's fader learned as no channel. The aliases still read
// mappings saved before.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MidiMapTarget {
    MasterVolume,
    TrackVolume {
        #[serde(default, skip_serializing_if = "Option::is_none", alias = "track_id")]
        track_id: Option<String>,
    },
    TrackPan {
        #[serde(default, skip_serializing_if = "Option::is_none", alias = "track_id")]
        track_id: Option<String>,
    },
    TrackMute {
        #[serde(default, skip_serializing_if = "Option::is_none", alias = "track_id")]
        track_id: Option<String>,
    },
    /// A specific parameter of a plug-in slot inside a track's insert
    /// chain. `param_id` matches the index/id the plug-in exposes via
    /// its parameter list. CC values arrive normalised 0..1 and are
    /// mapped into the parameter's own range by the chain
    /// (InsertChain::set_parameter_normalized).
    PluginParam {
        #[serde(alias = "track_id")]
        track_id: String,
        #[serde(alias = "slot_id")]
        slot_id: String,
        #[serde(alias = "param_id")]
        param_id: u32,
    },
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
    /// Controls moved by mapped controllers since the app last asked, for
    /// automation write to record (see `drain_midi_mapped_moves`).
    pub moves: Vec<MappedMove>,
}

/// A control a mapped MIDI controller moved, as automation write records
/// it: which lane, and its value on that lane, 0 to 1.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappedMove {
    pub track_id: String,
    /// "track_volume", "track_pan", "track_mute" or "plugin_param".
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param_id: Option<u32>,
    pub value: f64,
}

/// Moves kept while nobody collects them (automation write is off): the
/// newest, enough for a few seconds of a knob turned fast.
const MOVES_KEPT: usize = 256;

/// What a controller value does to a lane. Volume lanes run from -60 to
/// +6 dB, so the -60 to 0 dB a controller covers is most of the lane, not
/// all of it. The master volume has no lane of its own here: a mapped
/// master knob moves the output level, which nothing records.
pub fn mapped_move(target: &MidiMapTarget, value: f32) -> Option<MappedMove> {
    let value = value.clamp(0.0, 1.0) as f64;
    let on_track = |track_id: &Option<String>, kind: &'static str, v: f64| {
        track_id.as_ref().map(|id| MappedMove {
            track_id: id.clone(),
            kind,
            slot_id: None,
            param_id: None,
            value: v,
        })
    };
    match target {
        MidiMapTarget::MasterVolume => None,
        MidiMapTarget::TrackVolume { track_id } => {
            let db = -60.0 + value * 60.0;
            on_track(track_id, "track_volume", (db + 60.0) / 66.0)
        }
        MidiMapTarget::TrackPan { track_id } => on_track(track_id, "track_pan", value),
        MidiMapTarget::TrackMute { track_id } => {
            on_track(track_id, "track_mute", if value > 0.5 { 1.0 } else { 0.0 })
        }
        MidiMapTarget::PluginParam {
            track_id,
            slot_id,
            param_id,
        } => Some(MappedMove {
            track_id: track_id.clone(),
            kind: "plugin_param",
            slot_id: Some(slot_id.clone()),
            param_id: Some(*param_id),
            value,
        }),
    }
}

fn keep_move(mappings: &Mutex<MidiMappings>, target: &MidiMapTarget, value: f32) {
    if let Some(mv) = mapped_move(target, value) {
        let mut m = mappings.lock();
        if m.moves.len() >= MOVES_KEPT {
            m.moves.remove(0);
        }
        m.moves.push(mv);
    }
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
            ..Self::default()
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
pub fn spawn_dispatcher(
    engine: Arc<Mutex<DawEngine>>,
    mappings: Arc<Mutex<MidiMappings>>,
    surface: crate::control_surface::SharedSurface,
    midi_out: Arc<Mutex<hardwave_midi::output::MidiOutputManager>>,
    midi_input: Arc<Mutex<hardwave_midi::MidiInputManager>>,
) {
    let mut last_frame_at = std::time::Instant::now();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(8));
        // The desk shows the song as it is now, about thirty times a second,
        // sending only what changed: a fader moved with the mouse, by
        // automation or by undo moves the motor too.
        if surface.is_enabled() && last_frame_at.elapsed() >= std::time::Duration::from_millis(33) {
            last_frame_at = std::time::Instant::now();
            let inputs = midi_input.lock().open_port_names();
            send_desk_frame(&engine, &surface, &midi_out, &inputs);
        }
        // The input manager's own lock only: holding the engine's while waiting for it kept
        // everything else off the engine during a MIDI port scan.
        let events = midi_input.lock().drain_events();
        if events.is_empty() {
            continue;
        }
        for ev in events {
            // A control surface speaks a fixed language rather than
            // whatever a knob was learned to, so it is read first and
            // what it claims never reaches the learn path.
            if surface.is_enabled() {
                if let Some(action) = crate::control_surface::interpret(&ev) {
                    apply_surface(&engine, &surface, &midi_out, action);
                    continue;
                }
            }
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
                keep_move(&mappings, &mapping.target, value);
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
                keep_move(&mappings, &target, value);
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
            let Some(track_id) = track_id else { return };
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
            let Some(track_id) = track_id else { return };
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
            let Some(track_id) = track_id else { return };
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
        MidiMapTarget::PluginParam {
            track_id,
            slot_id,
            param_id,
        } => {
            // Plug-in parameters live on the live `Box<dyn HostedPlugin>`
            // instance, which only the audio thread can mutate. Ship the
            // change via the shared insert-command queue so the audio
            // thread applies it at the start of its next block — exactly
            // the same path the mixer UI uses for set_parameter clicks.
            // CC values arrive 0..1; the chain maps them into the
            // parameter's own range (a built-in's Hz, dB or ms).
            let cmd = hardwave_engine::insert_chain::InsertCommand::SetParameterNormalized {
                track_id: track_id.clone(),
                slot_id: slot_id.clone(),
                param_id: *param_id,
                value01: value as f64,
            };
            let eng = engine.lock();
            let _ = eng.try_send_insert_command(cmd);
        }
    }
}

/// Act on one message from a control surface.
/// The same dispatch, for anything else that speaks a desk's language.
///
/// OSC from a phone and MIDI from a desk end up here rather than in two
/// copies of the same mixer code.
pub(crate) fn apply_surface_action(
    engine: &Arc<Mutex<DawEngine>>,
    surface: &crate::control_surface::SharedSurface,
    midi_out: &Arc<Mutex<hardwave_midi::output::MidiOutputManager>>,
    action: crate::control_surface::SurfaceAction,
) {
    apply_surface(engine, surface, midi_out, action)
}

fn apply_surface(
    engine: &Arc<Mutex<DawEngine>>,
    surface: &crate::control_surface::SharedSurface,
    midi_out: &Arc<Mutex<hardwave_midi::output::MidiOutputManager>>,
    action: crate::control_surface::SurfaceAction,
) {
    use crate::control_surface as cs;
    use std::sync::atomic::Ordering;

    let bank = surface.bank.load(Ordering::Relaxed);
    let track_of = |strip: usize| -> Option<String> {
        let eng = engine.lock();
        let project = eng.project.lock();
        cs::bank_tracks(&project, bank).get(strip).cloned()
    };

    match action {
        cs::SurfaceAction::Touch { strip, on } => {
            surface.set_touched(strip, on);
            // One undo step per move of the hand, not one per message the
            // fader sends while it moves.
            if on {
                engine.lock().snapshot_before_mutation();
            }
        }
        cs::SurfaceAction::VPot { strip, clicks } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            let eng = engine.lock();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.pan = (track.pan + clicks as f64 * 0.02).clamp(-1.0, 1.0);
                }
            }
            eng.apply_track_mix(&track_id);
        }
        cs::SurfaceAction::VPotPush { strip } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            let eng = engine.lock();
            eng.snapshot_before_mutation();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.pan = 0.0;
                }
            }
            eng.apply_track_mix(&track_id);
        }
        cs::SurfaceAction::Fader { strip, value } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            let eng = engine.lock();
            {
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.volume_db = cs::fader_to_db(value);
                }
            }
            // Straight to the node, as the mixer's own fader does: a desk
            // fader sends dozens of messages a second, and each rebuilt
            // the whole graph.
            eng.apply_track_mix(&track_id);
        }
        cs::SurfaceAction::MasterFader { value } => {
            let eng = engine.lock();
            eng.transport
                .master_volume_db
                .store(cs::fader_to_db(value), Ordering::Relaxed);
        }
        cs::SurfaceAction::Mute { strip } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            let muted = {
                let eng = engine.lock();
                let mut project = eng.project.lock();
                let Some(track) = project.track_mut(&track_id) else {
                    return;
                };
                track.muted = !track.muted;
                track.muted
            };
            engine.lock().rebuild_graph();
            // The button's light follows in the next desk frame.
            let _ = muted;
        }
        cs::SurfaceAction::Solo { strip } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            {
                let eng = engine.lock();
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.soloed = !track.soloed;
                }
            }
            engine.lock().rebuild_graph();
        }
        cs::SurfaceAction::Arm { strip } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            // Not while recording: which input goes into which take is
            // fixed when recording starts.
            if engine.lock().transport.recording.load(Ordering::Relaxed) {
                return;
            }
            {
                let eng = engine.lock();
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.armed = !track.armed;
                }
            }
            engine.lock().rebuild_graph();
            let _ = engine.lock().follow_armed_inputs();
        }
        cs::SurfaceAction::BankLeft | cs::SurfaceAction::BankRight => {
            let count = {
                let eng = engine.lock();
                let project = eng.project.lock();
                cs::bank_count(&project)
            };
            let next = match action {
                cs::SurfaceAction::BankLeft => bank.saturating_sub(1),
                _ => (bank + 1).min(count.saturating_sub(1)),
            };
            surface.bank.store(next, Ordering::Relaxed);
            send_bank_state(engine, surface, midi_out);
        }
        cs::SurfaceAction::Play => {
            let eng = engine.lock();
            eng.transport.playing.store(true, Ordering::Relaxed);
        }
        cs::SurfaceAction::Stop => {
            let eng = engine.lock();
            eng.transport.playing.store(false, Ordering::Relaxed);
        }
        cs::SurfaceAction::Record => {
            let eng = engine.lock();
            eng.transport.recording.store(true, Ordering::Relaxed);
        }
        cs::SurfaceAction::Rewind => {
            let eng = engine.lock();
            eng.transport.set_position(0);
        }
        cs::SurfaceAction::Forward => {
            let eng = engine.lock();
            let sample_rate = eng.current_sample_rate() as u64;
            let position = eng.transport.position();
            eng.transport.set_position(position + sample_rate * 4);
        }
        cs::SurfaceAction::Tempo { bpm } => {
            let bpm = bpm.clamp(20.0, 999.0);
            let eng = engine.lock();
            eng.transport.bpm.store(bpm, Ordering::Relaxed);
            eng.send_command(hardwave_engine::transport::TransportCommand::SetBpm(bpm));
            let mut project = eng.project.lock();
            if let Some(entry) = project.tempo_map.entries.get_mut(0) {
                entry.bpm = bpm;
            }
        }
        cs::SurfaceAction::Pan { strip, value } => {
            let Some(track_id) = track_of(strip) else {
                return;
            };
            {
                let eng = engine.lock();
                let mut project = eng.project.lock();
                if let Some(track) = project.track_mut(&track_id) {
                    track.pan = value.clamp(-1.0, 1.0) as f64;
                }
            }
            engine.lock().apply_track_mix(&track_id);
        }
        cs::SurfaceAction::Goto { beats } => {
            let eng = engine.lock();
            let sample_rate = eng.current_sample_rate();
            let bpm = eng.transport.bpm.load(Ordering::Relaxed);
            let samples = (beats.max(0.0) * 60.0 / bpm * sample_rate as f64) as u64;
            eng.transport.set_position(samples);
        }
    }
}

/// A new bank: the whole desk is sent again on the next frame, names and
/// all, so the strips and the mix agree about what is under the fingers.
pub fn send_bank_state(
    _engine: &Arc<Mutex<DawEngine>>,
    surface: &crate::control_surface::SharedSurface,
    _midi_out: &Arc<Mutex<hardwave_midi::output::MidiOutputManager>>,
) {
    surface.redraw();
}

/// Bring the desk up to date with the song: what changed since the last
/// frame, or everything after a redraw.
pub fn send_desk_frame(
    engine: &Arc<Mutex<DawEngine>>,
    surface: &crate::control_surface::SharedSurface,
    midi_out: &Arc<Mutex<hardwave_midi::output::MidiOutputManager>>,
    open_inputs: &[String],
) {
    use crate::control_surface as cs;
    use std::sync::atomic::Ordering;

    let bank = surface.bank.load(Ordering::Relaxed);
    let frame = {
        // try_lock: a frame skipped while something holds the song is put
        // right by the next one, a thirtieth of a second later.
        let Some(eng) = engine.try_lock() else {
            return;
        };
        let master_db = eng.transport.master_volume_db.load(Ordering::Relaxed);
        let Some(project) = eng.project.try_lock() else {
            return;
        };
        cs::desk_frame(&project, bank, master_db)
    };
    let Ok(mut shown) = surface.shown.lock() else {
        return;
    };
    let messages = cs::frame_messages(
        shown.as_ref(),
        &frame,
        surface.touched.load(Ordering::Relaxed),
    );
    if !messages.is_empty() {
        let out = midi_out.lock();
        for port in cs::desk_outputs(&out.open_port_names(), open_inputs) {
            for m in &messages {
                let _ = out.send(&port, m);
            }
        }
    }
    *shown = Some(frame);
}

#[cfg(test)]
mod target_tests {
    use super::*;

    /// The app sends camelCase; mappings saved before were snake_case.
    #[test]
    fn targets_read_what_the_app_sends_and_what_was_saved() {
        let sent: MidiMapTarget = serde_json::from_str(
            r#"{"kind":"pluginParam","trackId":"t","slotId":"s","paramId":3}"#,
        )
        .unwrap();
        let saved: MidiMapTarget = serde_json::from_str(
            r#"{"kind":"pluginParam","track_id":"t","slot_id":"s","param_id":3}"#,
        )
        .unwrap();
        assert_eq!(sent, saved);
        let fader: MidiMapTarget =
            serde_json::from_str(r#"{"kind":"trackVolume","trackId":"t"}"#).unwrap();
        assert_eq!(
            fader,
            MidiMapTarget::TrackVolume {
                track_id: Some("t".into())
            }
        );
        assert!(serde_json::to_string(&sent)
            .unwrap()
            .contains("\"trackId\":\"t\""));
    }
}

#[cfg(test)]
mod mapped_move_tests {
    use super::{mapped_move, MidiMapTarget};

    #[test]
    fn a_learned_fader_lands_on_the_volume_lane_where_it_sounds() {
        let target = MidiMapTarget::TrackVolume {
            track_id: Some("t1".into()),
        };
        // Full controller is 0 dB, which sits at 60/66 of a -60..+6 dB lane.
        let top = mapped_move(&target, 1.0).unwrap();
        assert_eq!((top.track_id.as_str(), top.kind), ("t1", "track_volume"));
        assert!((top.value - 60.0 / 66.0).abs() < 1e-9);
        assert_eq!(mapped_move(&target, 0.0).unwrap().value, 0.0);
    }

    #[test]
    fn pan_mute_and_plug_in_knobs_keep_their_values() {
        let pan = MidiMapTarget::TrackPan {
            track_id: Some("t1".into()),
        };
        assert_eq!(mapped_move(&pan, 0.25).unwrap().value, 0.25);
        let mute = MidiMapTarget::TrackMute {
            track_id: Some("t1".into()),
        };
        assert_eq!(mapped_move(&mute, 0.7).unwrap().value, 1.0);
        let knob = MidiMapTarget::PluginParam {
            track_id: "t1".into(),
            slot_id: "s1".into(),
            param_id: 7,
        };
        let mv = mapped_move(&knob, 0.4).unwrap();
        assert_eq!((mv.slot_id.as_deref(), mv.param_id), (Some("s1"), Some(7)));
    }

    #[test]
    fn nothing_to_record_without_a_track_or_for_the_master_level() {
        assert!(mapped_move(&MidiMapTarget::TrackVolume { track_id: None }, 0.5).is_none());
        assert!(mapped_move(&MidiMapTarget::MasterVolume, 0.5).is_none());
    }
}
