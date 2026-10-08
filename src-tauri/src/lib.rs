use parking_lot::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tauri::{Emitter, Manager};

mod collab;
mod commands;
mod control_surface;
mod diagnostics;
mod endpoints;
mod frontend_updater;
mod midi_clock;
mod midi_map;
mod midi_sync;
mod midi_timecode;
mod osc_control;
mod plugin_describe;
mod plugin_probe;
mod plugin_sandbox;
mod plugin_window_host;
mod prefs;
mod process_memory;
mod scripting;
#[cfg(test)]
mod security_tests;
mod stems_client;
mod window_guard;
mod workspace_cloud;

use hardwave_engine::DawEngine;
pub use midi_clock::MidiClockState;
pub use midi_map::MidiMappings;
pub use midi_sync::MidiClockSyncState;
pub use midi_timecode::MidiTimecodeState;
pub use prefs::AudioPrefs;

/// Shared engine state accessible from Tauri commands.
pub struct AppState {
    pub engine: Arc<Mutex<DawEngine>>,
    /// Flipped by `cancel_export` to halt an in-progress offline render.
    /// The export command clears it on entry and checks it each block.
    pub export_cancel: Arc<AtomicBool>,
    pub midi_mappings: Arc<Mutex<MidiMappings>>,
    /// Live automation recording sessions. The recorder module existed with
    /// tests and no caller, so moving a control during playback wrote nothing.
    pub automation_write: Arc<commands::automation_write::AutomationWriteSessions>,
    pub midi_clock: Arc<MidiClockState>,
    pub midi_sync: Arc<MidiClockSyncState>,
    pub midi_timecode: Arc<MidiTimecodeState>,
    /// Live plugin editor instances, keyed by the Tauri window label
    /// they're parented to. Dropping an entry closes the editor view.
    pub plugin_editors: Arc<
        Mutex<
            std::collections::HashMap<String, Box<dyn hardwave_plugin_host::types::HostedPlugin>>,
        >,
    >,
    /// Side-table mapping `(track_id, slot_id)` to the chain plug-in's
    /// `pending_params` queue. Populated when `add_plugin_to_track`
    /// builds a Vst3PluginInstance and is about to ship it to the audio
    /// thread; consumed by `open_plugin_editor` to wire a freshly-built
    /// editor instance to the same queue. Without this table, the
    /// editor and chain hold separate queues and GUI knob movements
    /// never reach the audio thread.
    #[allow(clippy::type_complexity)]
    pub slot_param_queues:
        Arc<Mutex<std::collections::HashMap<(String, String), Arc<Mutex<Vec<(u32, f64)>>>>>>,
    /// Where each insert logs the knob moves made in its own window.
    ///
    /// The queue above is drained by the plug-in on the audio path, so
    /// the app never saw those edits: a knob inside a plug-in window
    /// could be heard but not recorded. This one is drained by the app,
    /// which is what lets automation write follow it.
    #[allow(clippy::type_complexity)]
    /// The control surface: whether a desk is being listened to, and
    /// which eight tracks its strips are on.
    pub control_surface: crate::control_surface::SharedSurface,
    /// The reference track's measured loudness and its name, which the
    /// engine itself has no reason to know.
    pub reference_meta: Arc<Mutex<(f32, String)>>,
    /// Plug-ins the user has asked to run in a process of their own.
    pub sandboxed_plugins: Arc<Mutex<std::collections::HashSet<String>>>,
    /// Which live slots are running sandboxed, and whether their
    /// process has gone, so the mixer can say which plug-in it was.
    #[allow(clippy::type_complexity)]
    pub sandbox_health: Arc<Mutex<std::collections::HashMap<(String, String), Option<String>>>>,
    /// Whether the OSC listener is running. Its thread watches this, so
    /// switching OSC off stops it without waiting for a packet.
    /// Working on a song with someone else: the room, and what has
    /// crossed it.
    pub collab: Arc<crate::collab::Collab>,
    pub osc_enabled: Arc<std::sync::atomic::AtomicBool>,
    pub osc_port: Arc<Mutex<u16>>,
    #[allow(clippy::type_complexity)]
    pub slot_gui_edit_logs:
        Arc<Mutex<std::collections::HashMap<(String, String), Arc<Mutex<Vec<(u32, f64)>>>>>>,
    /// Where each insert publishes its gain reduction, keyed by track and
    /// slot. Compressors and limiters worked this out every sample and
    /// Peak levels in and out of each slot, for the plug-in window's meters.
    #[allow(clippy::type_complexity)]
    pub slot_levels: Arc<
        Mutex<
            std::collections::HashMap<
                (String, String),
                Arc<hardwave_engine::insert_chain::SlotLevels>,
            >,
        >,
    >,
    /// threw it away; the mixer can show it now.
    #[allow(clippy::type_complexity)]
    pub slot_gain_reduction: Arc<
        Mutex<
            std::collections::HashMap<
                (String, String),
                Arc<hardwave_engine::atomic_float::AtomicF32>,
            >,
        >,
    >,
    /// Cached launch-time decision from `resolve_launch_plan`. Populated
    /// by the splash-driven `frontend_update_check_and_apply` and READ by
    /// the follow-up `version_contract_state` command so both fronts of
    /// App.tsx see the same plan even if the manifest CDN flips between
    /// the two calls (staged-rollout race). The cache also records the
    /// resolved-at timestamp and source manifest version for the 24h
    /// staleness check, plus an `applied` flag set when the splash
    /// finishes a HotSwapReady so a subsequent state query won't keep
    /// telling App.tsx the same bundle still needs applying.
    pub frontend_launch_plan: Arc<Mutex<Option<frontend_updater::LaunchPlanCacheEntry>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Our own plug-ins are free inside this DAW and paid in other
    // hosts, so they have to be able to tell where they are. A
    // plug-in loads into this process and reads this, and a sandboxed
    // one inherits it from here, which is the whole mechanism.
    //
    // Safety: set before any thread of ours exists, which is the one
    // time setting an environment variable is sound.
    unsafe {
        std::env::set_var("HARDWAVE_HOST", "hardwave-daw");
    }

    // Crash probe: this same binary, re-run with --probe-plugin, loads one
    // plug-in and exits. Handled before anything else starts, so the process
    // that a bad plug-in kills is one holding nothing: no window, no engine,
    // no project. See plugin_probe.rs.
    // Take the folder the DAW was started from out of the places Windows
    // looks for DLLs, for this process and the children it starts, so a
    // DLL left next to a downloaded song cannot be loaded in place of the
    // real one.
    #[cfg(windows)]
    // Safety: a documented call with an empty string, before any thread
    // of ours exists.
    unsafe {
        let empty: [u16; 1] = [0];
        windows_sys::Win32::System::LibraryLoader::SetDllDirectoryW(empty.as_ptr());
    }

    let args: Vec<String> = std::env::args().collect();
    // The child modes are only ever the first argument, which is how
    // the DAW itself starts them; a switch anywhere else is a file name
    // or a mistake, not a request to load a library.
    let child_mode = args.get(1).map(String::as_str);
    // The sandbox child: this same binary, hosting one plug-in and
    // answering frames down a pipe. Handled first, for the same reason
    // the probe is: the process a bad plug-in kills holds nothing else.
    if child_mode == Some("--host-plugin") {
        let code = match (args.get(2), args.get(3)) {
            (Some(path), Some(id)) => plugin_sandbox::run_host_child(path, id),
            _ => {
                eprintln!("--host-plugin needs a path and an id");
                2
            }
        };
        std::process::exit(code);
    }

    // Reading what a CLAP contains, which means running it: in a child.
    if child_mode == Some("--describe-clap") {
        let code = match args.get(2) {
            Some(path) => plugin_describe::run_describe_child(path),
            None => 2,
        };
        std::process::exit(code);
    }

    // The same for a VST3 that does not list its classes in a file.
    if child_mode == Some("--describe-vst3") {
        let code = match args.get(2) {
            Some(path) => plugin_describe::run_describe_vst3_child(path),
            None => 2,
        };
        std::process::exit(code);
    }

    if child_mode == Some("--probe-plugin") {
        let code = match args.get(2) {
            Some(path) => plugin_probe::run_probe_child(path),
            None => {
                eprintln!("--probe-plugin needs a path");
                2
            }
        };
        std::process::exit(code);
    }

    // Session log to ~/.hardwave-daw/logs/ + panic hook. Replaces the
    // bare env_logger::init() — logging still reaches stderr too.
    diagnostics::init(env!("CARGO_PKG_VERSION"));

    let mut engine = DawEngine::new();
    // Apply persisted audio preferences before the engine starts so the first
    // stream honors the user's last device choice instead of the default.
    let prefs = AudioPrefs::load();
    if let Err(e) = engine.set_audio_config(
        prefs.output_device.clone(),
        prefs.sample_rate,
        prefs.buffer_size,
    ) {
        log::warn!("Failed to apply saved audio output prefs: {e}");
    }
    engine.set_input_config(prefs.input_device.clone(), prefs.input_channels);
    // Which pair of the interface is recorded from and played out of.
    engine.set_channel_offsets(prefs.input_channel_offset, prefs.output_channel_offset);
    // Sharing the audio work across cores, if the setting asks for it.
    engine.set_trusted_servers(prefs.trusted_sample_servers.clone());
    // CLAP libraries are read in a child process, never in this one.
    engine
        .plugin_scanner
        .lock()
        .set_clap_describer(std::sync::Arc::new(
            plugin_describe::describe_out_of_process,
        ));
    engine
        .plugin_scanner
        .lock()
        .set_vst3_describer(std::sync::Arc::new(
            plugin_describe::describe_vst3_out_of_process,
        ));
    if prefs.worker_threads > 0 {
        engine.set_worker_threads(prefs.worker_threads);
    }
    if prefs.link_enabled {
        engine.set_link_enabled(true);
    }
    #[cfg(target_os = "windows")]
    if prefs.wasapi_exclusive {
        if let Err(e) = engine.set_wasapi_exclusive(true) {
            log::warn!("Failed to apply saved WASAPI exclusive pref: {e}");
        }
    }
    let midi_mappings = Arc::new(Mutex::new(MidiMappings::load()));
    let midi_clock = Arc::new(MidiClockState::new());
    let midi_sync = Arc::new(MidiClockSyncState::new());
    let midi_timecode = Arc::new(MidiTimecodeState::with_output(Arc::clone(
        &midi_clock.output,
    )));
    let state = AppState {
        engine: Arc::new(Mutex::new(engine)),
        export_cancel: Arc::new(AtomicBool::new(false)),
        plugin_editors: Arc::new(Mutex::new(std::collections::HashMap::new())),
        slot_param_queues: Arc::new(Mutex::new(std::collections::HashMap::new())),
        control_surface: Arc::new(crate::control_surface::ControlSurface::new()),
        reference_meta: Arc::new(Mutex::new((f32::NEG_INFINITY, String::new()))),
        sandboxed_plugins: commands::plugins::sandboxed_set(
            prefs.sandboxed_plugins.iter().cloned().collect(),
        ),
        sandbox_health: Arc::new(Mutex::new(std::collections::HashMap::new())),
        collab: Arc::new(crate::collab::Collab::default()),
        osc_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        osc_port: Arc::new(Mutex::new(if prefs.osc_port == 0 {
            osc_control::DEFAULT_PORT
        } else {
            prefs.osc_port
        })),
        slot_gui_edit_logs: Arc::new(Mutex::new(std::collections::HashMap::new())),
        slot_gain_reduction: Arc::new(Mutex::new(std::collections::HashMap::new())),
        slot_levels: Arc::new(Mutex::new(std::collections::HashMap::new())),
        midi_mappings: Arc::clone(&midi_mappings),
        automation_write: Arc::new(commands::automation_write::AutomationWriteSessions::new()),
        midi_clock: Arc::clone(&midi_clock),
        midi_sync: Arc::clone(&midi_sync),
        midi_timecode: Arc::clone(&midi_timecode),
        frontend_launch_plan: Arc::new(Mutex::new(None)),
    };

    // Copied out before the setup closure, which outlives `prefs`.
    let osc_on_at_launch = prefs.osc_enabled;

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        // Before any window loads: nothing but our own pages in them.
        .plugin(window_guard::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            // Transport
            commands::transport::play,
            commands::transport::stop,
            commands::transport::start_count_in,
            commands::transport::get_count_in_state,
            commands::transport::set_position,
            commands::transport::set_bpm,
            commands::transport::toggle_loop,
            commands::transport::toggle_recording,
            commands::transport::cancel_recording,
            commands::transport::set_punch_range,
            commands::transport::get_punch_range,
            commands::transport::get_playhead_tick,
            commands::transport::set_loop,
            commands::transport::set_master_volume,
            commands::transport::set_time_signature,
            commands::transport::set_pattern_mode,
            commands::transport::set_wait_for_input,
            commands::transport::get_transport_state,
            // Automation
            commands::automation_write::set_automation_write_mode,
            commands::automation_write::get_automation_write_mode,
            commands::automation_write::set_automation_trim,
            commands::automation_write::get_automation_trim,
            commands::automation_write::automation_touch_begin,
            commands::automation_write::automation_write_sample,
            commands::automation_write::automation_touch_end,
            commands::automation::add_automation_lane,
            commands::automation::delete_automation_lane,
            commands::automation::add_automation_point,
            commands::automation::move_automation_point,
            commands::automation::delete_automation_point,
            commands::automation::set_automation_point_curve,
            commands::automation::set_automation_lane_visible,
            commands::automation::apply_lfo_to_lane,
            commands::automation_clips::create_automation_clip,
            commands::automation_clips::delete_automation_clip,
            commands::automation_clips::list_automation_clips,
            commands::automation_clips::move_automation_clip,
            commands::automation_clips::resize_automation_clip,
            commands::automation_clips::add_automation_clip_point,
            commands::automation_clips::move_automation_clip_point,
            commands::automation_clips::remove_automation_clip_point,
            commands::arrangements::list_arrangements,
            commands::arrangements::create_arrangement,
            commands::arrangements::switch_arrangement,
            commands::arrangements::rename_arrangement,
            commands::arrangements::delete_arrangement,
            // Tracks
            commands::tracks::get_tracks,
            commands::tracks::get_tracks_with_clips,
            commands::tracks::get_track_with_clips,
            commands::tracks::add_audio_track,
            commands::tracks::add_midi_track,
            commands::tracks::set_track_instrument,
            commands::tracks::set_kick_layer,
            commands::tracks::reset_kick_patch,
            commands::tracks::apply_kick_preset,
            commands::tracks::list_kick_presets,
            commands::tracks::set_kick_drive,
            commands::tracks::add_automation_track,
            commands::tracks::remove_track,
            commands::tracks::set_track_volume,
            commands::tracks::set_track_pan,
            commands::tracks::toggle_mute,
            commands::tracks::toggle_solo,
            commands::tracks::set_exclusive_solo,
            commands::timeline_edit::insert_time,
            commands::timeline_edit::delete_time,
            commands::timeline_edit::list_sections,
            commands::timeline_edit::add_section,
            commands::timeline_edit::delete_section,
            commands::timeline_edit::duplicate_section,
            commands::macros::list_macros,
            commands::macros::add_macro,
            commands::macros::rename_macro,
            commands::macros::delete_macro,
            commands::macros::set_macro_value,
            commands::macros::add_macro_link,
            commands::macros::remove_macro_link,
            commands::macros::set_macro_link_range,
            commands::macros::apply_all_macros,
            commands::plugin_presets::list_all_presets,
            commands::plugin_presets::list_factory_presets,
            commands::plugin_presets::load_factory_preset,
            commands::midi::get_midi_fx,
            commands::midi::set_midi_fx,
            commands::midi::preview_midi_fx,
            commands::midi_input::get_control_surface,
            commands::midi_input::set_control_surface_enabled,
            commands::midi_input::set_control_surface_bank,
            commands::midi::get_mpe,
            commands::midi::set_mpe,
            commands::midi::load_tuning_file,
            commands::midi::clear_tuning,
            commands::midi::get_tuning,
            commands::midi::set_tuning_root,
            commands::modulation::list_modulations,
            commands::modulation::add_modulation,
            commands::modulation::set_modulation,
            commands::modulation::delete_modulation,
            commands::starter::create_starter_song,
            commands::stems::separate_stems,
            commands::stems::stop_stems,
            commands::stems::stems_service,
            commands::load_test::run_load_test,
            commands::load_test::cancel_load_test,
            commands::load_test::open_load_test_song,
            commands::cloud::save_to_workspace,
            commands::cloud::list_workspace_songs,
            commands::cloud::open_from_workspace,
            commands::collab::start_collab,
            commands::collab::stop_collab,
            commands::collab::collab_status,
            commands::collab::request_project,
            commands::collab::answer_join,
            commands::collab::share_presence,
            commands::flp::import_flp,
            commands::transport::get_video,
            commands::transport::set_video,
            commands::transport::set_video_offset,
            commands::transport::set_video_muted,
            commands::transport::clear_video,
            commands::session::get_session_grid,
            commands::session::launch_slot,
            commands::session::stop_slot,
            commands::session::launch_scene,
            commands::session::stop_all_slots,
            commands::session::set_session_quantise,
            commands::session::set_session_slot,
            commands::session::clear_session_slot,
            commands::session::add_scene,
            commands::scripts::list_scripts,
            commands::scripts::save_script,
            commands::scripts::delete_script,
            commands::scripts::check_script,
            commands::scripts::run_script,
            commands::engine::get_osc_status,
            commands::engine::set_osc_enabled,
            commands::plugins::get_sandboxed_plugins,
            commands::plugins::take_sandbox_crashes,
            commands::plugins::set_plugin_sandboxed,
            commands::audio::align_track_to,
            commands::audio::audio_clip_to_midi,
            commands::audio::tune_audio_clip,
            commands::audio::clip_spectrogram,
            commands::audio::erase_from_clip,
            commands::reference::get_reference,
            commands::reference::load_reference,
            commands::reference::set_reference_playing,
            commands::reference::match_reference_loudness,
            commands::reference::set_reference_gain,
            commands::reference::clear_reference,
            commands::engine::get_link_status,
            commands::engine::set_link_enabled,
            commands::engine::get_worker_threads,
            commands::engine::set_worker_threads,
            commands::engine::get_channel_offsets,
            commands::engine::set_channel_offsets,
            commands::plugins::drain_plugin_knob_moves,
            commands::export::freeze_track,
            commands::export::unfreeze_track,
            commands::export::list_frozen_tracks,
            commands::comping::comp_take_range,
            commands::comping::spread_takes_to_lanes,
            commands::vcas::list_vcas,
            commands::vcas::add_vca,
            commands::vcas::rename_vca,
            commands::vcas::delete_vca,
            commands::vcas::set_vca_gain,
            commands::vcas::set_vca_muted,
            commands::vcas::set_vca_members,
            commands::tracks::get_midi_routes,
            commands::tracks::set_midi_routes,
            commands::tracks::save_mixer_snapshot,
            commands::tracks::list_mixer_snapshots,
            commands::tracks::recall_mixer_snapshot,
            commands::tracks::delete_mixer_snapshot,
            commands::tracks::toggle_solo_safe,
            commands::tracks::toggle_arm,
            commands::tracks::set_track_monitor_input,
            commands::tracks::reorder_track,
            commands::tracks::set_track_name,
            commands::tracks::set_track_color,
            commands::tracks::set_track_phase_invert,
            commands::tracks::set_track_swap_lr,
            commands::tracks::set_track_stereo_separation,
            commands::tracks::set_track_mix_live,
            commands::tracks::set_track_delay_samples,
            commands::tracks::set_track_pitch_semitones,
            commands::tracks::set_track_fine_tune_cents,
            commands::tracks::set_track_filter_type,
            commands::tracks::set_track_filter_cutoff,
            commands::tracks::set_track_filter_resonance,
            commands::tracks::set_track_output_bus,
            // Sends
            commands::sends::get_sends,
            commands::sends::list_sends,
            commands::sends::add_send,
            commands::sends::remove_send,
            commands::sends::set_send_target,
            commands::sends::set_send_gain,
            commands::sends::set_send_pre_fader,
            commands::sends::set_send_enabled,
            commands::sends::create_return_with_send,
            // Plugins
            commands::plugins::scan_plugins,
            commands::plugins::get_slot_levels,
            commands::plugins::get_slot_scope,
            commands::plugins::retry_blocked_plugin,
            commands::plugins::get_plugins,
            commands::plugins::get_last_scan_diff,
            commands::plugins::get_plugin_blocklist,
            commands::plugins::set_plugin_blocklist,
            commands::plugins::get_custom_scan_paths,
            commands::plugins::set_custom_scan_paths,
            commands::plugins::plugin_cache_path,
            commands::plugins::add_plugin_to_track,
            commands::plugins::remove_plugin_from_track,
            commands::plugin_presets::list_plugin_presets,
            commands::plugin_presets::save_plugin_preset,
            commands::plugin_presets::load_plugin_preset,
            commands::plugin_presets::delete_plugin_preset,
            commands::plugin_presets::rename_plugin_preset,
            commands::plugins::open_plugin_editor,
            commands::plugins::close_plugin_editor,
            commands::plugins::set_insert_enabled,
            commands::plugins::reorder_insert,
            commands::plugins::set_fx_chain_bypassed,
            commands::plugins::set_insert_wet,
            commands::plugins::set_plugin_parameter,
            commands::plugins::get_plugin_parameters,
            commands::plugins::load_sampler,
            commands::plugins::get_gain_reduction,
            commands::plugins::set_plugin_sidechain_source,
            commands::plugins::find_missing_plugins,
            commands::plugins::rescan_and_restore_missing_plugins,
            commands::engine::get_graph_latency,
            commands::engine::get_audio_load,
            commands::engine::get_track_load,
            // Metronome (engine-generated click)
            commands::engine::process_memory,
            commands::engine::set_reset_on_transport,
            commands::engine::set_play_truncated_notes,
            commands::engine::set_record_offset_ms,
            commands::engine::set_pan_law,
            commands::engine::get_recording_latency,
            commands::engine::set_metronome_enabled,
            commands::engine::set_metronome_volume,
            commands::engine::set_metronome_accent,
            commands::engine::set_metronome_record_only,
            commands::engine::get_pdc_enabled,
            commands::engine::set_pdc_enabled,
            commands::engine::get_audio_cache_stats,
            commands::engine::set_audio_cache_max_bytes,
            // Project
            commands::project::new_project,
            commands::project::save_project,
            commands::project::load_project,
            commands::project::get_project_info,
            commands::project::get_project_meta,
            commands::project::set_project_meta,
            commands::project::reset_project_working_time,
            commands::project::tick_project_working_time,
            commands::project::get_channel_rack_state,
            commands::project::set_channel_rack_state,
            commands::project::get_timeline_state,
            commands::project::set_timeline_state,
            commands::project::get_tempo_entries,
            commands::project::add_tempo_entry,
            commands::project::remove_tempo_entry,
            commands::project::set_tempo_entry,
            commands::project::set_tempo_entry_time_signature,
            // Autosave / crash recovery
            commands::autosave::autosave_save,
            commands::autosave::autosave_latest,
            commands::autosave::autosave_clear,
            commands::autosave::autosave_mark_alive,
            commands::autosave::autosave_clear_alive,
            commands::autosave::autosave_detect_crash,
            // Engine
            commands::engine::start_engine,
            commands::engine::stop_engine,
            commands::engine::get_meters,
            commands::engine::get_master_samples,
            commands::engine::get_audio_devices,
            commands::engine::get_audio_config,
            commands::engine::set_audio_config,
            commands::engine::get_audio_input_devices,
            commands::engine::get_audio_input_config,
            commands::engine::set_audio_input_config,
            commands::engine::start_input_monitoring,
            commands::engine::stop_input_monitoring,
            commands::engine::set_direct_monitoring,
            commands::engine::get_direct_monitoring,
            commands::engine::get_input_meter,
            commands::engine::list_audio_hosts,
            commands::engine::get_audio_host,
            commands::engine::set_audio_host,
            commands::engine::get_wasapi_exclusive,
            commands::engine::set_wasapi_exclusive,
            // Browser
            commands::browser::list_directory,
            commands::browser::trash_browser_file,
            commands::browser::search_library,
            // Bug reports
            commands::bugs::bug_report_env,
            commands::bugs::session_log_tail,
            // Missing audio + relinking
            commands::sources::list_missing_sources,
            commands::sources::relink_source,
            commands::sources::auto_relink_sources,
            commands::sources::collect_project_samples,
            // Audio
            commands::audio::import_audio_file,
            commands::audio::get_track_clips,
            commands::audio::get_waveform_peaks,
            commands::audio::get_file_peaks,
            commands::audio::move_clip,
            commands::audio::move_clip_to_track,
            commands::audio::resize_clip,
            commands::audio::delete_clip,
            commands::audio::duplicate_clip,
            commands::audio::split_clip,
            commands::audio::set_clip_muted,
            commands::audio::slip_clip,
            // Browser auditions through the engine's own output
            commands::audio::preview_audio_file,
            commands::audio::preview_audio_file_in_tempo,
            commands::audio::stop_audio_preview,
            commands::audio::set_preview_volume,
            commands::audio::set_clip_gain,
            commands::audio::set_clip_fades,
            commands::audio::set_clip_fade_curves,
            commands::audio::toggle_clip_reverse,
            commands::audio::set_clip_pitch,
            commands::audio::set_clip_stretch,
            commands::audio::set_clip_warp_markers,
            commands::audio::warp_clip_to_grid,
            commands::audio::detect_clip_transients,
            // MIDI
            commands::midi::create_midi_clip,
            commands::midi::export_clip_midi,
            commands::midi::get_midi_notes,
            commands::midi::get_clip_controls,
            commands::midi::extract_groove,
            commands::midi::list_grooves,
            commands::midi::apply_groove,
            commands::midi::set_clip_controls,
            commands::midi::add_midi_note,
            commands::midi::update_midi_note,
            commands::midi::delete_midi_note,
            commands::midi::arpeggiate_clip_notes,
            commands::midi::strum_clip_notes,
            commands::midi::snap_clip_notes_to_scale,
            commands::midi::humanize_clip_notes,
            commands::midi::note_repeat_clip_notes,
            commands::midi::chordify_clip_notes,
            commands::midi::generate_progression_in_clip,
            commands::midi::generate_melody_in_clip,
            commands::windows::open_panel_window,
            commands::windows::close_panel_window,
            commands::midi::legato_clip_notes,
            // MIDI input (live)
            commands::midi_input::list_midi_inputs,
            commands::midi_input::open_midi_input,
            commands::midi_input::close_midi_input,
            commands::midi_input::close_all_midi_inputs,
            commands::midi_input::get_midi_activity,
            commands::midi_input::midi_driver_status,
            commands::midi_input::set_midi_master_enabled,
            commands::midi_input::get_midi_master_enabled,
            commands::midi_input::set_midi_velocity_curve,
            commands::midi_input::get_midi_velocity_curve,
            commands::midi_input::get_midi_desired_ports,
            commands::midi_input::set_midi_clock_sync_enabled,
            commands::midi_input::get_midi_clock_sync_status,
            commands::midi_input::inject_midi_event,
            commands::midi_capture::dump_midi_capture,
            commands::midi_capture::capture_recent_midi,
            commands::midi_capture::clear_midi_capture,
            commands::midi_capture::commit_recording_to_midi_clip,
            // MIDI Learn
            commands::midi_learn::midi_learn_start,
            commands::midi_learn::midi_learn_cancel,
            commands::midi_learn::midi_learn_status,
            commands::midi_learn::list_midi_mappings,
            commands::midi_learn::remove_midi_mapping,
            commands::midi_learn::clear_midi_mappings,
            // MIDI Clock output
            commands::midi_output::list_midi_outputs,
            commands::midi_output::open_midi_output,
            commands::midi_output::close_midi_output,
            commands::midi_output::set_midi_clock_enabled,
            commands::midi_output::get_midi_clock_status,
            commands::midi_output::set_midi_mtc_enabled,
            commands::midi_output::set_midi_mtc_fps,
            commands::midi_output::get_midi_mtc_status,
            // Undo/redo
            commands::history::undo,
            commands::history::redo,
            commands::history::history_sizes,
            commands::history::begin_history_group,
            commands::history::end_history_group,
            // Export
            commands::export::export_project_wav,
            commands::export::cancel_export,
            commands::export::export_project_stems,
            commands::export::bounce_track_to_audio,
            commands::export::consolidate_track_range,
            // Dev panel (stripped before merge to master)
            commands::dev::dev_dump_state,
            commands::dev::dev_force_device_error,
            commands::dev::dev_resolve_test_asset,
            commands::dev::dev_list_test_assets,
            // Frontend updater — splash-driven hot-swap of the UI bundle
            frontend_updater::frontend_update_check_and_apply,
            frontend_updater::frontend_update_status,
            // Version contract resolver — single-source-of-truth decision
            // between Path A (installer modal) and Path B (hot-swap). The
            // command is cache-first: launch-time callers get the same
            // LaunchPlan the splash already rendered. App.tsx's 24h
            // recheck timer passes `force_refresh: true` to bypass the
            // cache and recompute against a fresh manifest fetch.
            frontend_updater::version_contract_state,
            // Diagnostics — session-log location for Help → Export diagnostics
            diagnostics::diagnostics_info,
        ])
        .setup(move |app| {
            log::info!("Hardwave DAW starting");

            // Lets the panic hook raise a frontend crash banner.
            diagnostics::set_app_handle(app.handle().clone());

            // NOTE (2026-07-01): the custom-scheme frontend hot-swap was
            // retired — serving a cached bundle over `hardwave-app://`
            // rendered grey on WebView2. We now ALWAYS load the bundled
            // frontend over `tauri://` (which renders correctly), and the
            // launch splash drives the built-in Tauri installer updater
            // instead (see frontend_updater::frontend_update_check_and_apply).
            // So `maybe_activate_cache` is intentionally NOT called here.

            // Start meter broadcast thread
            let state = app.state::<AppState>();
            let engine = Arc::clone(&state.engine);
            let app_handle = app.handle().clone();

            // MIDI Learn dispatcher: drains incoming CC events and applies
            // mapped values to the live engine state. Also handles learn-mode
            // capture in the same loop so there's no race with the main
            // meter/transport broadcast thread.
            midi_map::spawn_dispatcher(
                Arc::clone(&state.engine),
                Arc::clone(&state.midi_mappings),
                Arc::clone(&state.control_surface),
                Arc::clone(&state.midi_clock.output),
            );

            // MIDI Clock dispatcher: sends 24 PPQN clock ticks and
            // Start/Stop system realtime messages to every open MIDI output
            // whenever the user has enabled clock send in Audio settings.
            midi_clock::spawn_dispatcher(Arc::clone(&state.engine), Arc::clone(&state.midi_clock));

            // OSC, when it was left on. A phone running TouchOSC on the
            // same network is a remote for the transport and the mixer.
            if osc_on_at_launch {
                let port = *state.osc_port.lock();
                state
                    .osc_enabled
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = osc_control::spawn_listener(
                    Arc::clone(&state.engine),
                    Arc::clone(&state.control_surface),
                    Arc::clone(&state.midi_clock.output),
                    Arc::clone(&state.osc_enabled),
                    port,
                ) {
                    state
                        .osc_enabled
                        .store(false, std::sync::atomic::Ordering::Relaxed);
                    log::warn!("OSC was on but could not start: {e}");
                }
            }

            // MIDI Clock sync dispatcher: observes clock ticks from the
            // MIDI input manager and, when sync is enabled, slaves the
            // transport BPM and play/stop state to the external master.
            midi_sync::spawn_dispatcher(Arc::clone(&state.engine), Arc::clone(&state.midi_sync));

            // MIDI Time Code dispatcher: emits Quarter Frame messages at 4×fps
            // while playing and MTC send is enabled. Uses the same output
            // manager as MidiClockState so both stream to the same ports.
            midi_timecode::spawn_dispatcher(
                Arc::clone(&state.engine),
                Arc::clone(&state.midi_timecode),
            );

            // MIDI hot-plug reconciler: every ~2s, reopens desired ports that
            // have come back online and drops connections whose device has
            // vanished. Desired set is maintained by open/close commands so
            // the user's choice survives unplug/replug.
            {
                let engine_for_reconcile = Arc::clone(&state.engine);
                std::thread::spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(2000));
                    let mgr = {
                        let eng = engine_for_reconcile.lock();
                        Arc::clone(&eng.midi_input)
                    };
                    let _report = mgr.lock().reconcile();
                });
            }

            // Load the plugin cache from disk, then kick off a background
            // rescan so added/removed plugins are detected on startup without
            // blocking the UI. Persists the fresh cache after the scan.
            {
                let engine_for_scan = Arc::clone(&state.engine);
                std::thread::spawn(move || {
                    let cache_path = hardwave_plugin_host::PluginScanner::default_cache_path();
                    if let Some(ref path) = cache_path {
                        let eng = engine_for_scan.lock();
                        let mut scanner = eng.plugin_scanner.lock();
                        match scanner.load_cache_from_disk(path) {
                            Ok(n) => log::info!("Loaded plugin cache: {n} entries"),
                            Err(e) => log::warn!("Failed to load plugin cache: {e}"),
                        }
                    }
                    {
                        // Only the scanner is held for the scan, not the
                        // engine: playback and every other command carry
                        // on while plug-in folders are walked.
                        let scanner_arc =
                            std::sync::Arc::clone(&engine_for_scan.lock().plugin_scanner);
                        // The built-ins first, so they resolve at once;
                        // the scanner keeps them through every rescan.
                        scanner_arc
                            .lock()
                            .register_natives(hardwave_native_plugins::native_plugin_descriptors());
                        // Not under the scanner's lock: the cached list
                        // keeps answering while the folders are walked.
                        hardwave_plugin_host::PluginScanner::scan_shared(&scanner_arc, None);
                        if let Some(ref path) = cache_path {
                            if let Err(e) = scanner_arc.lock().save_cache_to_disk(path) {
                                log::warn!("Failed to save plugin cache: {e}");
                            }
                        }
                    }
                });
            }

            // Hot-plug polling removed: cpal's device enumeration takes
            // hundreds of ms and blocks the engine lock, which stalls the
            // meter/transport broadcast and every UI command on a regular
            // cadence. Device loss is still detected via the stream-error
            // recovery path in `poll_audio_health`, and the device list is
            // re-fetched on demand when the Audio Settings panel opens.

            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(33));

                    // Single lock acquisition per tick — coalesces what used to be
                    // four separate engine.lock() calls and eliminates per-tick
                    // contention with command handlers.
                    let (meters, track_payload, transport_payload) = {
                        use std::sync::atomic::Ordering;
                        let mut eng = engine.lock();
                        // Surface stream death to the user: recovery (device
                        // unplugged → fell back to system default) gets a
                        // warning toast; a failed recovery gets a sticky
                        // error banner. Previously both were log-only and
                        // audio just "moved" or died silently (P1-9).
                        match eng.poll_audio_health() {
                            Ok(true) => {
                                let (device, _, _) = eng.audio_config();
                                let _ = app_handle.emit(
                                    "audio-device-recovered",
                                    device.unwrap_or_else(|| "system default output".into()),
                                );
                            }
                            Ok(false) => {}
                            Err(e) => {
                                log::error!("Audio health check failed: {e}");
                                let _ = app_handle.emit("audio-device-error", e);
                            }
                        }
                        let meters = eng.master_meter();
                        let track_payload: Vec<_> = eng
                            .track_meter_snapshots()
                            .into_iter()
                            .map(|(id, pl, pr, rms, pre_fader)| {
                                serde_json::json!({
                                    "id": id,
                                    "peakL": pl,
                                    "peakR": pr,
                                    "rms": rms,
                                    "preFaderPeak": pre_fader,
                                })
                            })
                            .collect();
                        let pos = eng.transport.position();
                        let playing = eng.transport.is_playing();
                        let bpm = eng.transport.bpm.load(Ordering::Relaxed);
                        let master_db = eng.transport.master_volume_db.load(Ordering::Relaxed);
                        let (num, den) = hardwave_engine::transport::unpack_time_sig(
                            eng.transport.time_sig.load(Ordering::Relaxed),
                        );
                        let pattern_mode = eng.transport.pattern_mode.load(Ordering::Relaxed);
                        let looping = eng.transport.looping.load(Ordering::Relaxed);
                        let loop_start = eng.transport.loop_start.load(Ordering::Relaxed);
                        let loop_end = eng.transport.loop_end.load(Ordering::Relaxed);
                        let transport_payload = serde_json::json!({
                            "position": pos,
                            "playing": playing,
                            "bpm": bpm,
                            "masterVolumeDb": master_db,
                            "timeSig": [num, den],
                            "patternMode": pattern_mode,
                            "looping": looping,
                            "loopStart": loop_start,
                            "loopEnd": loop_end,
                        });
                        (meters, track_payload, transport_payload)
                    };

                    let _ = app_handle.emit("daw:meters", &meters);
                    let _ = app_handle.emit("daw:trackMeters", &track_payload);
                    let _ = app_handle.emit("daw:transport", &transport_payload);
                }
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { .. } => {
                    // Reliable clean-shutdown hook: clear the alive marker so
                    // the next launch does not mistake this for a crash.
                    if let Ok(dir) = window.app_handle().path().app_cache_dir() {
                        let marker = dir.join("autosaves").join("session.alive");
                        let _ = std::fs::remove_file(marker);
                    }
                }
                tauri::WindowEvent::DragDrop(drag_event) => {
                    // Native-side drag-drop bridge. On Windows + WebView2 with
                    // a custom-decoration window, the JS-level
                    // `tauri://drag-drop` event sometimes does not fire even
                    // though Tauri itself receives the OLE drop. Re-emit the
                    // payload as `daw:drag-drop` so the frontend has a
                    // reliable signal regardless of the WebView2 quirk.
                    let payload = match drag_event {
                        tauri::DragDropEvent::Enter { paths, position } => {
                            serde_json::json!({
                                "kind": "enter",
                                "paths": paths,
                                "x": position.x,
                                "y": position.y,
                            })
                        }
                        tauri::DragDropEvent::Over { position } => {
                            serde_json::json!({
                                "kind": "over",
                                "x": position.x,
                                "y": position.y,
                            })
                        }
                        tauri::DragDropEvent::Drop { paths, position } => {
                            serde_json::json!({
                                "kind": "drop",
                                "paths": paths,
                                "x": position.x,
                                "y": position.y,
                            })
                        }
                        tauri::DragDropEvent::Leave => {
                            serde_json::json!({ "kind": "leave" })
                        }
                        _ => return,
                    };
                    use tauri::Emitter;
                    let _ = window.emit("daw:drag-drop", payload);
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Hardwave DAW");
}
