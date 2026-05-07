use crate::AppState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct InsertInfo {
    pub id: String,
    #[serde(rename = "pluginId")]
    pub plugin_id: String,
    #[serde(rename = "pluginName")]
    pub plugin_name: String,
    pub enabled: bool,
    pub wet: f32,
    #[serde(rename = "sidechainSource")]
    pub sidechain_source: Option<String>,
}

#[derive(Serialize)]
pub struct TrackInfo {
    id: String,
    name: String,
    kind: String,
    color: String,
    volume_db: f64,
    pan: f64,
    muted: bool,
    soloed: bool,
    solo_safe: bool,
    armed: bool,
    #[serde(rename = "monitorInput")]
    monitor_input: bool,
    #[serde(rename = "phaseInvert")]
    phase_invert: bool,
    #[serde(rename = "swapLr")]
    swap_lr: bool,
    #[serde(rename = "stereoSeparation")]
    stereo_separation: f64,
    #[serde(rename = "delaySamples")]
    delay_samples: i64,
    #[serde(rename = "pitchSemitones")]
    pitch_semitones: i32,
    #[serde(rename = "fineTuneCents")]
    fine_tune_cents: f32,
    #[serde(rename = "filterType")]
    filter_type: String,
    #[serde(rename = "filterCutoffHz")]
    filter_cutoff_hz: f32,
    #[serde(rename = "filterResonance")]
    filter_resonance: f32,
    #[serde(rename = "outputBus")]
    output_bus: Option<String>,
    insert_count: usize,
    inserts: Vec<InsertInfo>,
    /// Automation lanes attached to this track. Surfaced to the UI so
    /// the playlist can render a paarse curve strip directly under the
    /// track row. Lanes may be empty; an empty list means no automation.
    #[serde(rename = "automationLanes")]
    automation_lanes: Vec<AutomationLaneInfo>,
    /// Native instrument voicing for MIDI tracks. Snake-case to match
    /// the project's `NativeInstrument` discriminator.
    instrument: String,
    /// Per-track KickSynth patch. Only meaningful when
    /// `instrument == "kick_synth"`. Layers with `null` mean "use the
    /// engine's hardstyle default for this layer".
    #[serde(rename = "kickPatch")]
    kick_patch: KickPatchInfo,
}

#[derive(Serialize)]
pub struct KickPatchInfo {
    pub layers: [Option<KickLayerPatchInfo>; 4],
    pub drive: f32,
}

#[derive(Serialize)]
pub struct KickLayerPatchInfo {
    pub peak_gain: f32,
    pub length_secs: f32,
    pub release_secs: f32,
    pub sweep_start_hz: f32,
    pub sweep_end_hz: f32,
    pub sweep_secs: f32,
    pub waveform: String,
}

/// Wire format for an automation lane. Mirrors the project shape but
/// uses camelCase + a string-tagged enum so the TS side is ergonomic.
#[derive(Serialize)]
pub struct AutomationLaneInfo {
    pub id: String,
    pub target: AutomationTargetInfo,
    pub points: Vec<AutomationPointInfo>,
    pub visible: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AutomationTargetInfo {
    TrackVolume,
    TrackPan,
    TrackMute,
    PluginParam {
        #[serde(rename = "slotId")]
        slot_id: String,
        #[serde(rename = "paramId")]
        param_id: u32,
    },
    SendLevel {
        #[serde(rename = "sendIndex")]
        send_index: usize,
    },
}

#[derive(Serialize)]
pub struct AutomationPointInfo {
    pub tick: u64,
    pub value: f64,
    pub curve: String,
    pub tension: f64,
}

fn track_to_info(
    t: &hardwave_project::Track,
    plugin_name_lookup: &dyn Fn(&str) -> String,
) -> TrackInfo {
    let inserts = t
        .inserts
        .iter()
        .map(|s| InsertInfo {
            id: s.id.clone(),
            plugin_id: s.plugin_id.clone(),
            plugin_name: plugin_name_lookup(&s.plugin_id),
            enabled: s.enabled,
            wet: s.wet,
            sidechain_source: s.sidechain_source.clone(),
        })
        .collect();
    TrackInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        kind: format!("{:?}", t.kind),
        color: t.color.clone(),
        volume_db: t.volume_db,
        pan: t.pan,
        muted: t.muted,
        soloed: t.soloed,
        solo_safe: t.solo_safe,
        armed: t.armed,
        monitor_input: t.monitor_input,
        phase_invert: t.phase_invert,
        swap_lr: t.swap_lr,
        stereo_separation: t.stereo_separation,
        delay_samples: t.delay_samples,
        pitch_semitones: t.pitch_semitones,
        fine_tune_cents: t.fine_tune_cents,
        filter_type: t.filter_type.clone(),
        filter_cutoff_hz: t.filter_cutoff_hz,
        filter_resonance: t.filter_resonance,
        output_bus: t.output_bus.clone(),
        insert_count: t.inserts.len(),
        inserts,
        automation_lanes: t
            .automation_lanes
            .iter()
            .map(|l| AutomationLaneInfo {
                id: l.id.clone(),
                target: match &l.target {
                    hardwave_project::automation::AutomationTarget::TrackVolume => {
                        AutomationTargetInfo::TrackVolume
                    }
                    hardwave_project::automation::AutomationTarget::TrackPan => {
                        AutomationTargetInfo::TrackPan
                    }
                    hardwave_project::automation::AutomationTarget::TrackMute => {
                        AutomationTargetInfo::TrackMute
                    }
                    hardwave_project::automation::AutomationTarget::PluginParam {
                        slot_id,
                        param_id,
                    } => AutomationTargetInfo::PluginParam {
                        slot_id: slot_id.clone(),
                        param_id: *param_id,
                    },
                    hardwave_project::automation::AutomationTarget::SendLevel { send_index } => {
                        AutomationTargetInfo::SendLevel {
                            send_index: *send_index,
                        }
                    }
                },
                points: l
                    .points
                    .iter()
                    .map(|p| AutomationPointInfo {
                        tick: p.tick,
                        value: p.value,
                        curve: format!("{:?}", p.curve),
                        tension: p.tension,
                    })
                    .collect(),
                visible: l.visible,
            })
            .collect(),
        instrument: match t.instrument {
            hardwave_project::track::NativeInstrument::BuiltinSine => "builtin_sine".to_string(),
            hardwave_project::track::NativeInstrument::KickSynth => "kick_synth".to_string(),
        },
        kick_patch: KickPatchInfo {
            drive: t.kick_patch.drive,
            layers: [
                t.kick_patch.layers[0].map(|l| KickLayerPatchInfo {
                    peak_gain: l.peak_gain,
                    length_secs: l.length_secs,
                    release_secs: l.release_secs,
                    sweep_start_hz: l.sweep_start_hz,
                    sweep_end_hz: l.sweep_end_hz,
                    sweep_secs: l.sweep_secs,
                    waveform: l.waveform.clone(),
                }),
                t.kick_patch.layers[1].map(|l| KickLayerPatchInfo {
                    peak_gain: l.peak_gain,
                    length_secs: l.length_secs,
                    release_secs: l.release_secs,
                    sweep_start_hz: l.sweep_start_hz,
                    sweep_end_hz: l.sweep_end_hz,
                    sweep_secs: l.sweep_secs,
                    waveform: l.waveform.clone(),
                }),
                t.kick_patch.layers[2].map(|l| KickLayerPatchInfo {
                    peak_gain: l.peak_gain,
                    length_secs: l.length_secs,
                    release_secs: l.release_secs,
                    sweep_start_hz: l.sweep_start_hz,
                    sweep_end_hz: l.sweep_end_hz,
                    sweep_secs: l.sweep_secs,
                    waveform: l.waveform.clone(),
                }),
                t.kick_patch.layers[3].map(|l| KickLayerPatchInfo {
                    peak_gain: l.peak_gain,
                    length_secs: l.length_secs,
                    release_secs: l.release_secs,
                    sweep_start_hz: l.sweep_start_hz,
                    sweep_end_hz: l.sweep_end_hz,
                    sweep_secs: l.sweep_secs,
                    waveform: l.waveform.clone(),
                }),
            ],
        },
    }
}

#[tauri::command]
pub fn get_tracks(state: State<AppState>) -> Vec<TrackInfo> {
    let engine = state.engine.lock();
    let project = engine.project.lock();
    let scanner = engine.plugin_scanner.lock();
    // Build an id → name map once; fall back to the plugin id itself when the
    // plugin is missing from the current scan (uninstalled, or scan not yet
    // run) so the mixer never shows empty slot labels.
    let name_of = |id: &str| -> String {
        scanner
            .find(id)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    project
        .tracks
        .iter()
        .map(|t| track_to_info(t, &name_of))
        .collect()
}

#[tauri::command]
pub fn add_audio_track(state: State<AppState>, name: String) -> String {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.add_audio_track(name)
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    id
}

#[tauri::command]
pub fn add_midi_track(state: State<AppState>, name: String) -> String {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.add_midi_track(name)
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    id
}

#[tauri::command]
pub fn add_automation_track(state: State<AppState>, name: String) -> String {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    let id = {
        let mut project = engine.project.lock();
        project.add_automation_track(name)
    };
    engine.sync_track_meters();
    engine.rebuild_graph();
    id
}

#[tauri::command]
pub fn remove_track(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        project.remove_track(&track_id);
    }
    engine.sync_track_meters();
    engine.rebuild_graph();
}

/// Set one layer of a track's KickSynth patch. The frontend sends
/// the full layer (peak/length/release/sweep_start/sweep_end/sweep_secs);
/// we replace `track.kick_patch.layers[idx]` and trigger a rebuild
/// so the audio thread picks up the new params on the next block.
/// Indices: 0=Transient, 1=Punch, 2=Bass, 3=Tail.
#[tauri::command]
pub fn set_kick_layer(
    state: State<AppState>,
    track_id: String,
    layer_index: usize,
    peak_gain: f32,
    length_secs: f32,
    release_secs: f32,
    sweep_start_hz: f32,
    sweep_end_hz: f32,
    sweep_secs: f32,
    /// `"sine" | "saw" | "square" | "triangle"`. Optional; defaults
    /// to "sine" if the caller omits it.
    waveform: Option<String>,
) -> Result<(), String> {
    if layer_index >= 4 {
        return Err(format!("Layer index out of range (0..=3): {layer_index}"));
    }
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.kick_patch.layers[layer_index] =
            Some(hardwave_project::track::KickLayerPatch {
                peak_gain,
                length_secs,
                release_secs,
                sweep_start_hz,
                sweep_end_hz,
                sweep_secs,
                waveform: waveform.unwrap_or_else(|| "sine".to_string()),
            });
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Apply a named KickSynth preset. The named layer set replaces the
/// track's `kick_patch` wholesale. Names match
/// `hardwave_dsp::kick_synth::PRESET_NAMES`. Returns `Ok(())` even
/// when the preset name is unknown — the engine falls back to the
/// default preset, matching `preset_layers`'s soft-fail.
#[tauri::command]
pub fn apply_kick_preset(
    state: State<AppState>,
    track_id: String,
    preset_name: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let layers = hardwave_dsp::kick_synth::preset_layers(&preset_name);
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.kick_patch.layers = [
            Some(layer_to_patch(&layers[0])),
            Some(layer_to_patch(&layers[1])),
            Some(layer_to_patch(&layers[2])),
            Some(layer_to_patch(&layers[3])),
        ];
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Returns the static list of available preset names so the UI can
/// populate its dropdown without hard-coding the values.
#[tauri::command]
pub fn list_kick_presets() -> Vec<String> {
    hardwave_dsp::kick_synth::PRESET_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn layer_to_patch(l: &hardwave_dsp::kick_synth::Layer) -> hardwave_project::track::KickLayerPatch {
    use hardwave_dsp::kick_synth::LayerWaveform;
    let waveform = match l.waveform {
        LayerWaveform::Sine => "sine",
        LayerWaveform::Saw => "saw",
        LayerWaveform::Square => "square",
        LayerWaveform::Triangle => "triangle",
    }
    .to_string();
    hardwave_project::track::KickLayerPatch {
        peak_gain: l.envelope.peak_gain,
        length_secs: l.envelope.length_secs,
        release_secs: l.envelope.release_secs,
        sweep_start_hz: l.sweep.start_hz,
        sweep_end_hz: l.sweep.end_hz,
        sweep_secs: l.sweep.sweep_secs,
        waveform,
    }
}

/// Set the post-mix drive amount for a track's KickSynth (0..=1).
/// Drive runs after the four layers are summed and sharpens the
/// soft-clipper for hardstyle / raw / uptempo bite.
#[tauri::command]
pub fn set_kick_drive(
    state: State<AppState>,
    track_id: String,
    drive: f32,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.kick_patch.drive = drive.clamp(0.0, 1.0);
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Reset a track's KickSynth patch back to engine defaults.
#[tauri::command]
pub fn reset_kick_patch(
    state: State<AppState>,
    track_id: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.kick_patch = hardwave_project::track::KickPatch::default();
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

/// Switch the native instrument that voices a MIDI track. Audio /
/// Master / Bus tracks ignore this — the call is still safe (no-op).
/// `kind` is the snake-case name of one of `NativeInstrument`'s
/// variants: `"builtin_sine"` or `"kick_synth"`.
#[tauri::command]
pub fn set_track_instrument(
    state: State<AppState>,
    track_id: String,
    kind: String,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    {
        let engine = state.engine.lock();
        let mut project = engine.project.lock();
        let track = project
            .track_mut(&track_id)
            .ok_or_else(|| format!("Track not found: {track_id}"))?;
        track.instrument = match kind.as_str() {
            "kick_synth" | "kicksynth" => {
                hardwave_project::track::NativeInstrument::KickSynth
            }
            "builtin_sine" | "sine" | _ => {
                hardwave_project::track::NativeInstrument::BuiltinSine
            }
        };
    }
    state.engine.lock().rebuild_graph();
    Ok(())
}

#[tauri::command]
pub fn set_track_volume(state: State<AppState>, track_id: String, volume_db: f64) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.volume_db = volume_db;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_pan(state: State<AppState>, track_id: String, pan: f64) {
    if !pan.is_finite() {
        return;
    }
    let pan = pan.clamp(-1.0, 1.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.pan = pan;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_mute(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.muted = !track.muted;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_solo(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.soloed = !track.soloed;
        }
    }
    engine.rebuild_graph();
}

/// Exclusive solo: solo only this track, unsolo all others.
#[tauri::command]
pub fn set_exclusive_solo(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        let target_currently_soloed = project.track(&track_id).map(|t| t.soloed).unwrap_or(false);

        for track in &mut project.tracks {
            if matches!(track.kind, hardwave_project::track::TrackKind::Master) {
                continue;
            }
            if track.id == track_id {
                // If already the only soloed track, unsolo it (toggle off).
                track.soloed = !target_currently_soloed;
            } else {
                track.soloed = false;
            }
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn toggle_arm(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.armed = !track.armed;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn reorder_track(state: State<AppState>, track_id: String, new_index: usize) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        // Master must stay last; clamp new_index to non-master range.
        let old_idx = match project.tracks.iter().position(|t| t.id == track_id) {
            Some(i) => i,
            None => return,
        };
        if matches!(
            project.tracks[old_idx].kind,
            hardwave_project::track::TrackKind::Master
        ) {
            return;
        }
        let master_count = project
            .tracks
            .iter()
            .filter(|t| matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .count();
        let max_idx = project.tracks.len().saturating_sub(1 + master_count);
        let target = new_index.min(max_idx);
        let track = project.tracks.remove(old_idx);
        project.tracks.insert(target, track);
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_name(state: State<AppState>, track_id: String, name: String) {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return;
    }
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.name = trimmed.to_string();
        }
    }
}

#[tauri::command]
pub fn set_track_color(state: State<AppState>, track_id: String, color: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.color = color;
        }
    }
}

#[tauri::command]
pub fn toggle_solo_safe(state: State<AppState>, track_id: String) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.solo_safe = !track.solo_safe;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_phase_invert(state: State<AppState>, track_id: String, invert: bool) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.phase_invert = invert;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_swap_lr(state: State<AppState>, track_id: String, swap: bool) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.swap_lr = swap;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_stereo_separation(
    state: State<AppState>,
    track_id: String,
    separation: Option<f64>,
) {
    let Some(separation) = separation else { return };
    if !separation.is_finite() {
        return;
    }
    let separation = separation.clamp(0.0, 2.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.stereo_separation = separation;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_monitor_input(state: State<AppState>, track_id: String, enabled: bool) {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.monitor_input = enabled;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_delay_samples(state: State<AppState>, track_id: String, samples: i64) {
    // Allow positive (latency compensation) and negative (look-ahead) values.
    // Clamp to ±10 minutes at 96 kHz to prevent runaway storage values.
    let samples = samples.clamp(-57_600_000, 57_600_000);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.delay_samples = samples;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_pitch_semitones(state: State<AppState>, track_id: String, semitones: i32) {
    let semitones = semitones.clamp(-24, 24);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.pitch_semitones = semitones;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_fine_tune_cents(state: State<AppState>, track_id: String, cents: Option<f32>) {
    let Some(cents) = cents else { return };
    if !cents.is_finite() {
        return;
    }
    let cents = cents.clamp(-100.0, 100.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.fine_tune_cents = cents;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_filter_type(state: State<AppState>, track_id: String, filter_type: String) {
    let normalized = match filter_type.as_str() {
        "lp" | "hp" | "bp" | "off" => filter_type,
        _ => "off".to_string(),
    };
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.filter_type = normalized;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_filter_cutoff(state: State<AppState>, track_id: String, cutoff_hz: f32) {
    if !cutoff_hz.is_finite() {
        return;
    }
    let cutoff = cutoff_hz.clamp(20.0, 20_000.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.filter_cutoff_hz = cutoff;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_filter_resonance(state: State<AppState>, track_id: String, resonance: f32) {
    if !resonance.is_finite() {
        return;
    }
    // Allow self-oscillation territory (Q up to 10) used by acid-style filters.
    let r = resonance.clamp(0.0, 10.0);
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        if let Some(track) = project.track_mut(&track_id) {
            track.filter_resonance = r;
        }
    }
    engine.rebuild_graph();
}

#[tauri::command]
pub fn set_track_output_bus(
    state: State<AppState>,
    track_id: String,
    output_bus: Option<String>,
) -> Result<(), String> {
    state.engine.lock().snapshot_before_mutation();
    let engine = state.engine.lock();
    {
        let mut project = engine.project.lock();
        // Reject self-routing. Reject routing to tracks that don't exist.
        if let Some(ref bus_id) = output_bus {
            if bus_id == &track_id {
                return Err("cannot route a track to itself".into());
            }
            if !project.tracks.iter().any(|t| &t.id == bus_id) {
                return Err("target track not found".into());
            }
            // Reject obvious cycles: if the target already routes (directly or
            // transitively) back to this track, the routing would form a loop.
            let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut cursor = Some(bus_id.clone());
            while let Some(next) = cursor {
                if next == track_id {
                    return Err("routing would form a cycle".into());
                }
                if !visited.insert(next.clone()) {
                    break;
                }
                cursor = project
                    .tracks
                    .iter()
                    .find(|t| t.id == next)
                    .and_then(|t| t.output_bus.clone());
            }
        }
        if let Some(track) = project.track_mut(&track_id) {
            track.output_bus = output_bus;
        } else {
            return Err("track not found".into());
        }
    }
    engine.rebuild_graph();
    Ok(())
}
