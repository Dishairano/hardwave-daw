use crate::AppState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use tauri::State;

#[derive(Serialize, Clone, Default)]
pub struct MidiActivitySnapshot {
    pub open_ports: Vec<String>,
    pub ms_since_last_event: Option<u64>,
    /// The last key played, for the first run's Controller step.
    pub last_note: Option<LastNote>,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastNote {
    pub note: u8,
    /// 1 to 127.
    pub velocity: u8,
}

#[derive(Serialize)]
pub struct MidiClockSyncStatus {
    pub enabled: bool,
    pub ticks_seen: bool,
    pub last_bpm: Option<f64>,
}

#[tauri::command]
pub fn list_midi_inputs(_state: State<AppState>) -> Vec<String> {
    // No lock: listing can take seconds on a busy machine.
    hardwave_midi::MidiInputManager::available_ports()
}

/// The MIDI inputs there now, each with what it is by its name (a keyboard,
/// a control surface or a virtual port), for the first run and for a
/// controller plugged in while the DAW was closed. Off the window's thread:
/// listing the inputs can take seconds on a busy machine, and this is asked
/// at launch.
#[tauri::command]
pub async fn list_midi_input_kinds() -> Vec<crate::midi_hotplug::InputSeen> {
    crate::midi_hotplug::kinds(&hardwave_midi::MidiInputManager::available_ports())
}

/// The plain sine voice the first run's Controller step plays the keys on,
/// on while the step shows. A new song has no instrument, so without it the
/// keys played nothing there.
#[tauri::command]
pub fn set_check_voice(state: State<AppState>, enabled: bool) {
    state.engine.lock().check_voice().set(enabled);
}

#[tauri::command]
pub fn open_midi_input(state: State<AppState>, port_name: String) -> Result<(), String> {
    let mut manager = state.midi_input.lock();
    manager.open(&port_name)
}

#[tauri::command]
pub fn close_midi_input(state: State<AppState>, port_name: String) {
    let mut manager = state.midi_input.lock();
    manager.close(&port_name);
}

#[tauri::command]
pub fn close_all_midi_inputs(state: State<AppState>) {
    let mut manager = state.midi_input.lock();
    manager.close_all();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiDriverStatus {
    pub ok: bool,
    /// Why not, when it is not ok. Empty when it is.
    pub detail: String,
}

/// Whether the MIDI subsystem started.
///
/// The setup wizard showed a green "MIDI driver healthy" tick that was drawn
/// without asking anything, so a machine with no working MIDI was told its
/// MIDI was fine.
#[tauri::command]
pub fn midi_driver_status() -> MidiDriverStatus {
    match hardwave_midi::MidiInputManager::probe_subsystem() {
        Ok(()) => MidiDriverStatus {
            ok: true,
            detail: String::new(),
        },
        Err(detail) => MidiDriverStatus { ok: false, detail },
    }
}

/// Turn MIDI input on or off as a whole.
///
/// The wizard's master switch said that with it off no MIDI input reaches the
/// audio thread, and nothing read it. Ports stay open while it is off and
/// their messages are dropped on arrival, so turning it back on is instant.
#[tauri::command]
pub fn set_midi_master_enabled(state: State<AppState>, enabled: bool) {
    state.midi_input.lock().set_enabled(enabled);
}

#[tauri::command]
pub fn get_midi_master_enabled(state: State<AppState>) -> bool {
    let enabled = state.midi_input.lock().is_enabled();
    enabled
}

/// Set the velocity curve for one MIDI input port.
///
/// The setup wizard has asked for this per controller since it was written
/// and nothing read the answer, so every curve behaved as linear. The curve
/// is applied to incoming note-ons on that port.
#[tauri::command]
pub fn set_midi_velocity_curve(state: State<AppState>, port_name: String, curve: String) {
    let mut manager = state.midi_input.lock();
    manager.set_velocity_curve(
        &port_name,
        hardwave_midi::velocity::VelocityCurve::from_id(&curve),
    );
}

/// The velocity curve in force for one port, as the id the UI uses.
#[tauri::command]
pub fn get_midi_velocity_curve(state: State<AppState>, port_name: String) -> String {
    let manager = state.midi_input.lock();
    manager.velocity_curve(&port_name).id().to_string()
}

/// The last activity reading, answered while the engine is busy.
static LAST_MIDI_ACTIVITY: parking_lot::Mutex<Option<MidiActivitySnapshot>> =
    parking_lot::const_mutex(None);

#[tauri::command]
pub fn get_midi_activity(state: State<AppState>) -> MidiActivitySnapshot {
    // Five times a second for the MIDI light, from the window's own thread:
    // never wait for it. Busy means the last reading.
    let reading = state
        .midi_input
        .try_lock_for(crate::commands::engine::METER_POLL_WAIT)
        .map(|manager| MidiActivitySnapshot {
            open_ports: manager.open_port_names(),
            ms_since_last_event: manager.ms_since_last_event(),
            last_note: manager
                .last_note_on()
                .map(|(note, velocity)| LastNote { note, velocity }),
        });
    match reading {
        Some(reading) => {
            *LAST_MIDI_ACTIVITY.lock() = Some(reading.clone());
            reading
        }
        None => LAST_MIDI_ACTIVITY.lock().clone().unwrap_or_default(),
    }
}

#[tauri::command]
pub fn get_midi_desired_ports(state: State<AppState>) -> Vec<String> {
    let manager = state.midi_input.lock();
    manager.desired_port_names()
}

#[tauri::command]
pub fn set_midi_clock_sync_enabled(state: State<AppState>, enabled: bool) {
    state.midi_sync.enabled.store(enabled, Ordering::Relaxed);
}

#[tauri::command]
pub fn get_midi_clock_sync_status(state: State<AppState>) -> MidiClockSyncStatus {
    MidiClockSyncStatus {
        enabled: state.midi_sync.enabled.load(Ordering::Relaxed),
        ticks_seen: state.midi_sync.ticks_seen.load(Ordering::Relaxed),
        last_bpm: *state.midi_sync.last_bpm.lock(),
    }
}

/// Following MIDI timecode: on or off, the master's timecode at the top of
/// the song, and what the master last sent.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtcStatus {
    pub enabled: bool,
    pub offset_seconds: f64,
    /// The master's last position in seconds, and its frame rate.
    pub seen_seconds: Option<f64>,
    pub fps: Option<f64>,
}

#[tauri::command]
pub fn set_mtc_follow(state: State<AppState>, enabled: bool, offset_seconds: f64) {
    state
        .midi_sync
        .mtc_enabled
        .store(enabled, Ordering::Relaxed);
    *state.midi_sync.mtc_offset.lock() = offset_seconds.clamp(0.0, 24.0 * 3600.0);
}

#[tauri::command]
pub fn get_mtc_status(state: State<AppState>) -> MtcStatus {
    let seen = *state.midi_sync.mtc_seen.lock();
    MtcStatus {
        enabled: state.midi_sync.mtc_enabled.load(Ordering::Relaxed),
        offset_seconds: *state.midi_sync.mtc_offset.lock(),
        seen_seconds: seen.map(|(s, _)| s),
        fps: seen.map(|(_, f)| f),
    }
}

/// Wire-format DTO for `inject_midi_event`. Mirrors the variant set of
/// `hardwave_midi::MidiEvent` 1:1 so the frontend can build a typed
/// payload without needing access to the Rust enum. Timing is in
/// per-block samples; on-screen/computer-keyboard injection always
/// passes 0 because the event reaches the audio thread as soon as the
/// drain loop runs.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MidiEventDto {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: f32,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        cc: u8,
        value: f32,
    },
    PitchBend {
        channel: u8,
        value: f32,
    },
}

impl MidiEventDto {
    fn into_event(self) -> hardwave_midi::MidiEvent {
        use hardwave_midi::MidiEvent;
        match self {
            MidiEventDto::NoteOn {
                channel,
                note,
                velocity,
            } => MidiEvent::NoteOn {
                timing: 0,
                channel,
                note,
                velocity: velocity.clamp(0.0, 1.0),
            },
            MidiEventDto::NoteOff { channel, note } => MidiEvent::NoteOff {
                timing: 0,
                channel,
                note,
                velocity: 0.0,
            },
            MidiEventDto::ControlChange { channel, cc, value } => MidiEvent::ControlChange {
                timing: 0,
                channel,
                cc,
                value: value.clamp(0.0, 1.0),
            },
            MidiEventDto::PitchBend { channel, value } => MidiEvent::PitchBend {
                timing: 0,
                channel,
                value: value.clamp(-1.0, 1.0),
            },
        }
    }
}

/// Inject a synthetic MIDI event into the engine's input pipeline.
/// Feeds the same shared queue as midir hardware callbacks, so the
/// on-screen keyboard, the computer-keyboard hook, and the MIDI-learn
/// dispatcher all observe events from these calls. Fire-and-forget —
/// errors are unrecoverable from the UI side (engine not started, lock
/// contention) so we silently no-op.
///
/// With `port`, the event is taken as one that came in on that MIDI input,
/// so the control surface reads it as that input's: the desk input can be
/// checked without a desk plugged in.
#[tauri::command]
pub fn inject_midi_event(state: State<AppState>, event: MidiEventDto, port: Option<String>) {
    let manager = state.midi_input.lock();
    match port {
        Some(port) => manager.inject_from(&port, event.into_event()),
        None => manager.inject(event.into_event()),
    }
}

/// Whether a control surface is being listened to, and where its
/// strips sit.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSurfaceStatus {
    pub enabled: bool,
    /// Which bank of eight the strips are on, counting from zero.
    pub bank: usize,
    pub bank_count: usize,
    /// "mackie" or "hui".
    pub protocol: crate::control_surface::DeskProtocol,
    /// The MIDI input read as the desk; none reads every input.
    pub input: Option<String>,
    /// Whether that input was found by its name rather than chosen.
    pub found: bool,
}

#[tauri::command]
pub fn get_control_surface(state: State<AppState>) -> ControlSurfaceStatus {
    use std::sync::atomic::Ordering;
    let bank_count = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        crate::control_surface::bank_count(&project)
    };
    let open = state.midi_input.lock().open_port_names();
    state.control_surface.find_desk(&open);
    ControlSurfaceStatus {
        enabled: state.control_surface.is_enabled(),
        bank: state.control_surface.bank.load(Ordering::Relaxed),
        bank_count,
        protocol: state.control_surface.protocol(),
        input: state.control_surface.desk_port().map(|p| p.to_string()),
        found: state.control_surface.desk_found(),
    }
}

/// Listen to a Mackie Control or HUI desk on its MIDI input.
///
/// With it on, the desk's faders and buttons drive the mixer instead of
/// going through MIDI Learn: eight faders with mute, solo and arm, the
/// V-pots, the transport keys, and bank left and right. Switching it on
/// sends the whole mix out, so a motorised desk lines up with it
/// straight away.
#[tauri::command]
pub fn set_control_surface_enabled(state: State<AppState>, enabled: bool) {
    use std::sync::atomic::Ordering;
    state
        .control_surface
        .enabled
        .store(enabled, Ordering::Relaxed);
    if enabled {
        let open = state.midi_input.lock().open_port_names();
        state.control_surface.find_desk(&open);
        crate::midi_map::send_bank_state(
            &state.engine,
            &state.control_surface,
            &state.midi_clock.output,
        );
    }
    crate::commands::engine::persist_audio_prefs_public(&state);
}

/// Which protocol the desk speaks: "mackie" (Mackie Control, and the
/// desks that copy it) or "hui".
#[tauri::command]
pub fn set_control_surface_protocol(
    state: State<AppState>,
    protocol: crate::control_surface::DeskProtocol,
) {
    state.control_surface.set_protocol(protocol);
    crate::commands::engine::persist_audio_prefs_public(&state);
}

/// Which MIDI input is the desk: one input by name, or none for every
/// input. Only the desk's input is read as the desk; every other input
/// plays as before, so a keyboard beside the desk keeps its notes.
#[tauri::command]
pub fn set_control_surface_input(state: State<AppState>, input: Option<String>) {
    use crate::control_surface::DeskInput;
    let choice = match input {
        Some(port) if !port.is_empty() => DeskInput::Port(port),
        _ => DeskInput::Every,
    };
    let open = state.midi_input.lock().open_port_names();
    state.control_surface.set_desk_input(choice, &open);
    crate::commands::engine::persist_audio_prefs_public(&state);
}

/// Put the strips on another eight tracks.
#[tauri::command]
pub fn set_control_surface_bank(state: State<AppState>, bank: usize) {
    use std::sync::atomic::Ordering;
    let count = {
        let engine = state.engine.lock();
        let project = engine.project.lock();
        crate::control_surface::bank_count(&project)
    };
    state
        .control_surface
        .bank
        .store(bank.min(count.saturating_sub(1)), Ordering::Relaxed);
    crate::midi_map::send_bank_state(
        &state.engine,
        &state.control_surface,
        &state.midi_clock.output,
    );
}
