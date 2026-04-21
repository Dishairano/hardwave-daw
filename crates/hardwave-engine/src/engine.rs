//! DawEngine — the top-level orchestrator that owns the audio graph, transport,
//! plugin host, and project state. Bridges between the audio callback and the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use std::sync::Arc;

use hardwave_audio_io::{AudioCallback, AudioDeviceManager};
use hardwave_metering::{ChannelMeter, MeterSnapshot};
use hardwave_plugin_host::PluginScanner;
use hardwave_project::Project;
use rtrb::RingBuffer;

use std::collections::HashMap;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioGraph, ProcessContext};
use crate::master_node::MasterNode;
use crate::track_node::{ClipRegion, TrackMeterState, TrackNode};
use crate::transport::{TransportCommand, TransportState};

/// Shared per-track meter map. Rebuilt when the audio graph is rebuilt.
pub type TrackMeterMap = Arc<Mutex<HashMap<String, Arc<TrackMeterState>>>>;

/// Maximum number of snapshots kept on each side of the history.
const HISTORY_CAP: usize = 256;

/// Snapshot-based undo/redo. A snapshot is a full `Project` clone; the struct is
/// small (track/clip metadata only — audio samples live in the AudioPool and are
/// referenced by source id), so cloning is cheap.
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}

impl History {
    pub fn new() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn push(&mut self, snapshot: Project) {
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn undo(&mut self, current: Project) -> Option<Project> {
        let prev = self.undo.pop()?;
        self.redo.push(current);
        if self.redo.len() > HISTORY_CAP {
            self.redo.remove(0);
        }
        Some(prev)
    }

    pub fn redo(&mut self, current: Project) -> Option<Project> {
        let next = self.redo.pop()?;
        self.undo.push(current);
        if self.undo.len() > HISTORY_CAP {
            self.undo.remove(0);
        }
        Some(next)
    }

    pub fn sizes(&self) -> (usize, usize) {
        (self.undo.len(), self.redo.len())
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

/// Main DAW engine.
pub struct DawEngine {
    pub transport: TransportState,
    pub project: Arc<Mutex<Project>>,
    pub plugin_scanner: Arc<Mutex<PluginScanner>>,
    pub audio_pool: AudioPool,

    audio_device: AudioDeviceManager,
    command_tx: Sender<EngineCommand>,
    command_rx: Receiver<EngineCommand>,

    // Metering: lock-free ring buffer (audio thread → UI thread)
    meter_consumer: Option<rtrb::Consumer<MeterSnapshot>>,
    meter_cache: MeterSnapshot,

    /// Per-track post-fader meter state, keyed by track id.
    pub track_meters: TrackMeterMap,

    /// Undo/redo history. Take a snapshot BEFORE mutating the project.
    pub history: Arc<Mutex<History>>,
}

/// Commands sent from UI thread to audio thread.
#[derive(Debug, Clone)]
pub enum EngineCommand {
    Transport(TransportCommand),
    /// Rebuild the audio graph from the current project state.
    RebuildGraph,
}

impl DawEngine {
    pub fn new() -> Self {
        let (tx, rx) = bounded(256);

        Self {
            transport: TransportState::default(),
            project: Arc::new(Mutex::new(Project::default())),
            plugin_scanner: Arc::new(Mutex::new(PluginScanner::new())),
            audio_pool: AudioPool::new(),
            audio_device: AudioDeviceManager::new(),
            command_tx: tx,
            command_rx: rx,
            meter_consumer: None,
            meter_cache: MeterSnapshot::default(),
            track_meters: Arc::new(Mutex::new(HashMap::new())),
            history: Arc::new(Mutex::new(History::new())),
        }
    }

    /// Snapshot the project before a mutation. Call this as the first line of any
    /// command that changes `Project`. Redo stack is cleared on new edits.
    pub fn snapshot_before_mutation(&self) {
        let snap = self.project.lock().clone();
        self.history.lock().push(snap);
    }

    /// Roll the project back to the most recent snapshot. Returns true when a snapshot
    /// was consumed. The caller is responsible for calling `rebuild_graph()` afterwards.
    pub fn undo(&self) -> bool {
        let current = self.project.lock().clone();
        let prev = self.history.lock().undo(current);
        if let Some(p) = prev {
            *self.project.lock() = p;
            true
        } else {
            false
        }
    }

    /// Re-apply the most recently undone mutation. Returns true on success.
    pub fn redo(&self) -> bool {
        let current = self.project.lock().clone();
        let next = self.history.lock().redo(current);
        if let Some(p) = next {
            *self.project.lock() = p;
            true
        } else {
            false
        }
    }

    /// Reset the history — e.g. after loading or creating a new project.
    pub fn reset_history(&self) {
        self.history.lock().clear();
    }

    /// (undoDepth, redoDepth) — exposed so the UI can enable/disable menu entries.
    pub fn history_sizes(&self) -> (usize, usize) {
        self.history.lock().sizes()
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), String> {
        let transport = self.transport.clone();
        let project = Arc::clone(&self.project);
        let command_rx = self.command_rx.clone();
        let audio_pool = self.audio_pool.clone();

        let sample_rate = self.audio_device.sample_rate;
        let buffer_size = self.audio_device.buffer_size;

        transport
            .sample_rate
            .store(sample_rate as u64, std::sync::atomic::Ordering::Relaxed);

        let (meter_producer, meter_consumer) = RingBuffer::new(16);
        self.meter_consumer = Some(meter_consumer);

        let callback = EngineCallback::new(
            transport,
            project,
            meter_producer,
            command_rx,
            audio_pool,
            Arc::clone(&self.track_meters),
            sample_rate,
            buffer_size,
        );

        self.audio_device.start(callback).map_err(|e| e.to_string())
    }

    /// Stop the audio engine.
    pub fn stop(&mut self) {
        self.audio_device.stop();
    }

    /// Send a transport command from the UI thread.
    pub fn send_command(&self, cmd: TransportCommand) {
        let _ = self.command_tx.try_send(EngineCommand::Transport(cmd));
    }

    /// Tell the audio thread to rebuild its graph from the current project state.
    pub fn rebuild_graph(&self) {
        let _ = self.command_tx.try_send(EngineCommand::RebuildGraph);
    }

    /// Load an audio file into the pool and return its source ID and info.
    pub fn load_audio_file(
        &self,
        path: &std::path::Path,
    ) -> Result<(String, hardwave_dsp::AudioFileInfo), String> {
        let target_sr = self.audio_device.sample_rate;
        let (info, channels) = hardwave_dsp::AudioFileReader::read_resampled(path, Some(target_sr))
            .map_err(|e| e.to_string())?;

        let num_frames = channels.first().map(|c| c.len()).unwrap_or(0);
        let source_id = format!("{:x}", md5_hash(path.to_string_lossy().as_bytes()));

        let buffer = AudioBuffer {
            channels,
            sample_rate: info.sample_rate,
            num_frames,
        };

        self.audio_pool.insert(source_id.clone(), buffer);

        Ok((source_id, info))
    }

    /// Ensure every non-master track in the project has an entry in the
    /// track meter map. Called from the UI thread so that `dev_dump_state`
    /// can see newly added tracks before the audio thread rebuilds.
    pub fn sync_track_meters(&self) {
        let project = self.project.lock();
        let mut meters = self.track_meters.lock();
        for track in &project.tracks {
            if matches!(track.kind, hardwave_project::track::TrackKind::Master) {
                continue;
            }
            meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()));
        }
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| !matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone())
            .collect();
        meters.retain(|id, _| live_ids.contains(id));
    }

    /// Snapshot per-track post-fader meters.
    /// Returns (id, peak_l, peak_r, rms, pre_fader_peak).
    pub fn track_meter_snapshots(&self) -> Vec<(String, f32, f32, f32, f32)> {
        use std::sync::atomic::Ordering;
        let meters = self.track_meters.lock();
        meters
            .iter()
            .map(|(id, m)| {
                (
                    id.clone(),
                    m.peak_db_l.load(Ordering::Relaxed),
                    m.peak_db_r.load(Ordering::Relaxed),
                    m.rms_db.load(Ordering::Relaxed),
                    m.pre_fader_peak_db.load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// Get the latest master meter snapshot (drains the lock-free ring buffer).
    pub fn master_meter(&mut self) -> MeterSnapshot {
        if let Some(ref mut consumer) = self.meter_consumer {
            while let Ok(snapshot) = consumer.pop() {
                self.meter_cache = snapshot;
            }
        }
        self.meter_cache
    }

    /// Scan for plugins.
    pub fn scan_plugins(&self) -> Vec<hardwave_plugin_host::PluginDescriptor> {
        let mut scanner = self.plugin_scanner.lock();
        scanner.scan().to_vec()
    }

    pub fn is_running(&self) -> bool {
        self.audio_device.is_running()
    }

    /// Poll for runtime device failures (e.g. USB interface disconnected) and
    /// transparently fall back to the system default device.
    ///
    /// Returns `Ok(true)` if a recovery restart happened.
    pub fn poll_audio_health(&mut self) -> Result<bool, String> {
        if self.audio_device.take_stream_error() {
            log::warn!("Audio stream failed; falling back to default output device");
            self.audio_device.stop();
            // Clear the selected-device preference so resolve_output_device()
            // picks the current system default on restart.
            self.audio_device.selected_device = None;
            self.start()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Get a reference to the audio device manager for device listing.
    pub fn audio_device_manager(&self) -> &AudioDeviceManager {
        &self.audio_device
    }

    /// Fingerprint of the current output device set — used by the UI thread
    /// to detect hot-plug events.
    pub fn output_device_fingerprint(&self) -> u64 {
        self.audio_device.output_device_fingerprint()
    }

    /// List audio host backends available in this build.
    pub fn list_audio_hosts() -> Vec<String> {
        AudioDeviceManager::list_hosts()
    }

    /// Name of the currently active audio host backend.
    pub fn audio_host_name(&self) -> String {
        self.audio_device.host_name()
    }

    /// Switch audio host backend. Restarts the stream if it was running.
    pub fn set_audio_host(&mut self, host_name: &str) -> Result<(), String> {
        let was_running = self.audio_device.is_running();
        self.audio_device
            .set_host(host_name)
            .map_err(|e| e.to_string())?;
        if was_running {
            self.start()?;
        }
        Ok(())
    }

    /// Whether WASAPI exclusive mode is currently requested.
    pub fn wasapi_exclusive(&self) -> bool {
        self.audio_device.wasapi_exclusive
    }

    /// Whether WASAPI exclusive mode is applicable on this host (Windows + WASAPI).
    pub fn wasapi_exclusive_available(&self) -> bool {
        self.audio_device.exclusive_available()
    }

    /// Enable/disable WASAPI exclusive mode. Restarts the stream if running
    /// so the change takes effect immediately.
    pub fn set_wasapi_exclusive(&mut self, enabled: bool) -> Result<(), String> {
        if self.audio_device.wasapi_exclusive == enabled {
            return Ok(());
        }
        let was_running = self.audio_device.is_running();
        if was_running {
            self.audio_device.stop();
        }
        self.audio_device.wasapi_exclusive = enabled;
        if was_running {
            self.start()?;
        }
        Ok(())
    }

    /// Get current audio config.
    pub fn audio_config(&self) -> (Option<String>, u32, u32) {
        (
            self.audio_device.selected_device.clone(),
            self.audio_device.sample_rate,
            self.audio_device.buffer_size,
        )
    }

    /// Apply new audio settings. Restarts the audio stream if running.
    pub fn set_audio_config(
        &mut self,
        device: Option<String>,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<(), String> {
        let was_running = self.audio_device.is_running();
        if was_running {
            self.audio_device.stop();
        }

        self.audio_device.selected_device = device;
        self.audio_device.sample_rate = sample_rate;
        self.audio_device.buffer_size = buffer_size;

        if was_running {
            self.start()?;
        }

        Ok(())
    }

    /// Current input config: (selected device name, channels).
    pub fn input_config(&self) -> (Option<String>, u16) {
        (
            self.audio_device.selected_input_device.clone(),
            self.audio_device.input_channels,
        )
    }

    /// Update the input device preferences. The engine does not restart any
    /// input stream yet — recording isn't live — but the choice is stored so
    /// the recording pipeline picks it up when we ship it.
    pub fn set_input_config(&mut self, device: Option<String>, channels: u16) {
        self.audio_device.selected_input_device = device;
        self.audio_device.input_channels = channels.clamp(1, 2);
        // If monitoring is running, restart it so the new device/channel
        // count takes effect immediately without the caller re-toggling.
        if self.audio_device.is_input_running() {
            let _ = self.audio_device.start_input_stream();
        }
    }

    /// Open a cpal input stream that feeds the pre-record peak meter. No
    /// audio is routed into the graph yet — this is for the Audio Settings
    /// monitor and for verifying the input device choice before recording.
    pub fn start_input_monitoring(&mut self) -> Result<(), String> {
        self.audio_device
            .start_input_stream()
            .map_err(|e| e.to_string())
    }

    /// Stop the input monitor stream.
    pub fn stop_input_monitoring(&mut self) {
        self.audio_device.stop_input_stream();
    }

    pub fn is_input_monitoring(&self) -> bool {
        self.audio_device.is_input_running()
    }

    /// Read and reset the current input peak. Returns linear (0..1+) L/R
    /// values.
    pub fn input_peak_snapshot(&self) -> (f32, f32) {
        self.audio_device.take_input_peak()
    }

    /// Sample rate / buffer size the input stream is currently running at.
    /// Both are 0 when the monitor stream isn't active.
    pub fn input_active_config(&self) -> (u32, u32) {
        (
            self.audio_device.input_active_sample_rate(),
            self.audio_device.input_active_buffer_size(),
        )
    }
}

impl Default for DawEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple hash for generating source IDs from file paths.
fn md5_hash(data: &[u8]) -> u64 {
    // FNV-1a 64-bit hash (fast, no crypto needed — just a unique key)
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Audio callback (runs on the real-time audio thread)
// ---------------------------------------------------------------------------

struct EngineCallback {
    transport: TransportState,
    project: Arc<Mutex<Project>>,
    meter_producer: rtrb::Producer<MeterSnapshot>,
    command_rx: Receiver<EngineCommand>,
    audio_pool: AudioPool,
    track_meters: TrackMeterMap,
    graph: AudioGraph,
    meter: ChannelMeter,
    sample_rate: u32,
    needs_rebuild: bool,
}

impl EngineCallback {
    #[allow(clippy::too_many_arguments)]
    fn new(
        transport: TransportState,
        project: Arc<Mutex<Project>>,
        meter_producer: rtrb::Producer<MeterSnapshot>,
        command_rx: Receiver<EngineCommand>,
        audio_pool: AudioPool,
        track_meters: TrackMeterMap,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Self {
        let mut cb = Self {
            transport,
            project,
            meter_producer,
            command_rx,
            audio_pool,
            track_meters,
            graph: AudioGraph::new(buffer_size as usize),
            meter: ChannelMeter::new(sample_rate as f64),
            sample_rate,
            needs_rebuild: true,
        };
        cb.rebuild_graph();
        cb
    }

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                EngineCommand::Transport(tcmd) => self.process_transport(tcmd),
                EngineCommand::RebuildGraph => {
                    self.needs_rebuild = true;
                }
            }
        }
    }

    fn process_transport(&mut self, cmd: TransportCommand) {
        match cmd {
            TransportCommand::Play => {
                self.transport
                    .playing
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::Stop => {
                let was_playing = self
                    .transport
                    .playing
                    .swap(false, std::sync::atomic::Ordering::Relaxed);
                // Double-stop behavior: if already stopped, reset to loop start (or 0).
                if !was_playing {
                    let loop_start = if self
                        .transport
                        .looping
                        .load(std::sync::atomic::Ordering::Relaxed)
                    {
                        self.transport
                            .loop_start
                            .load(std::sync::atomic::Ordering::Relaxed)
                    } else {
                        0
                    };
                    self.transport.set_position(loop_start);
                }
                self.transport
                    .recording
                    .store(false, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::Record => {
                self.transport
                    .recording
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .playing
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetMasterVolume(db) => {
                self.transport
                    .master_volume_db
                    .store(db, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetTimeSignature(num, den) => {
                self.transport.time_sig.store(
                    crate::transport::pack_time_sig(num, den),
                    std::sync::atomic::Ordering::Relaxed,
                );
            }
            TransportCommand::SetPatternMode(on) => {
                self.transport
                    .pattern_mode
                    .store(on, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetPosition(pos) => {
                self.transport.set_position(pos);
            }
            TransportCommand::SetBpm(bpm) => {
                self.transport
                    .bpm
                    .store(bpm, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::SetLoop(start, end) => {
                self.transport
                    .loop_start
                    .store(start, std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .loop_end
                    .store(end, std::sync::atomic::Ordering::Relaxed);
            }
            TransportCommand::ToggleLoop => {
                let current = self
                    .transport
                    .looping
                    .load(std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .looping
                    .store(!current, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Rebuild the audio graph from the project state.
    /// Creates one TrackNode per non-master track, a MasterNode, and wires them together.
    fn rebuild_graph(&mut self) {
        self.graph.clear();

        let project = self.project.lock();
        let sample_rate = self.sample_rate as f64;
        let tempo_map = &project.tempo_map;

        let mut track_node_ids = Vec::new();

        // Determine if any track is soloed — if so, mute all non-soloed tracks
        let any_soloed = project
            .tracks
            .iter()
            .any(|t| t.soloed && !matches!(t.kind, hardwave_project::track::TrackKind::Master));

        // Reconcile the per-track meter map with the current tracks: reuse existing
        // Arcs where possible, create new ones, drop meters for removed tracks.
        let mut meters = self.track_meters.lock();
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| !matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone())
            .collect();
        meters.retain(|id, _| live_ids.contains(id));

        for track in &project.tracks {
            if matches!(track.kind, hardwave_project::track::TrackKind::Master) {
                continue;
            }

            let meter = meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()))
                .clone();

            let mut node = TrackNode::new(track.name.clone(), self.audio_pool.clone(), meter);
            node.set_volume_db(track.volume_db);
            node.set_pan(track.pan);
            let effective_mute = track.muted || (any_soloed && !track.soloed && !track.solo_safe);
            node.set_muted(effective_mute);
            node.set_soloed(track.soloed);
            node.set_phase_invert(track.phase_invert);
            node.set_swap_lr(track.swap_lr);
            node.set_stereo_separation(track.stereo_separation);
            node.set_delay_samples(track.delay_samples);

            // Convert clip placements to sample-based ClipRegions.
            // Clips are visited in their project-defined order, then any overlap between
            // adjacent audio clips is promoted to an equal-length crossfade: the earlier
            // clip gets a fade-out across the overlap and the later clip gets a fade-in.
            // User-authored fades take precedence when they are already longer than the
            // auto-computed value.
            let mut regions: Vec<ClipRegion> = track
                .clips
                .iter()
                .filter_map(|clip| match &clip.content {
                    hardwave_project::clip::ClipContent::Audio(audio_clip) => {
                        let timeline_start =
                            tempo_map.tick_to_samples(clip.position_ticks, sample_rate);
                        let timeline_end = tempo_map
                            .tick_to_samples(clip.position_ticks + clip.length_ticks, sample_rate);
                        let gain = if audio_clip.gain_db <= -100.0 {
                            0.0
                        } else {
                            10.0_f64.powf(audio_clip.gain_db / 20.0) as f32
                        };
                        let fade_in_samples =
                            tempo_map.tick_to_samples(audio_clip.fade_in_ticks, sample_rate);
                        let fade_out_samples =
                            tempo_map.tick_to_samples(audio_clip.fade_out_ticks, sample_rate);
                        // source_step combines pitch and stretch via resampling.
                        // pitch +12 semitones = 2x source step; stretch_ratio 2.0 = half step.
                        let pitch_factor = 2.0_f64.powf(audio_clip.pitch_semitones / 12.0);
                        let stretch = if audio_clip.stretch_ratio <= 0.01 {
                            1.0
                        } else {
                            audio_clip.stretch_ratio
                        };
                        let source_step = pitch_factor / stretch;
                        Some(ClipRegion {
                            source_id: audio_clip.source_path.clone(),
                            timeline_start,
                            timeline_end,
                            source_offset: audio_clip.source_start,
                            gain,
                            muted: audio_clip.muted,
                            fade_in_samples,
                            fade_out_samples,
                            fade_in_curve: audio_clip.fade_in_curve,
                            fade_out_curve: audio_clip.fade_out_curve,
                            reversed: audio_clip.reversed,
                            source_step,
                        })
                    }
                    _ => None,
                })
                .collect();

            // Apply auto-crossfade across overlapping, unmuted audio clips.
            // We sort indices by timeline_start so adjacency maps to timeline order,
            // without losing the original positions inside `regions`.
            let mut order: Vec<usize> = (0..regions.len()).collect();
            order.sort_by_key(|&i| regions[i].timeline_start);
            for w in order.windows(2) {
                let (a, b) = (w[0], w[1]);
                if regions[a].muted || regions[b].muted {
                    continue;
                }
                if regions[b].timeline_start < regions[a].timeline_end {
                    let overlap = regions[a]
                        .timeline_end
                        .saturating_sub(regions[b].timeline_start);
                    if overlap > 0 {
                        if regions[a].fade_out_samples < overlap {
                            regions[a].fade_out_samples = overlap;
                        }
                        if regions[b].fade_in_samples < overlap {
                            regions[b].fade_in_samples = overlap;
                        }
                    }
                }
            }

            node.set_clips(regions);

            let node_id = self.graph.add_node(Box::new(node));
            track_node_ids.push(node_id);
        }

        // Add master node (reads volume from shared transport atomic — no rebuild on change)
        let master_node = MasterNode::new(Arc::clone(&self.transport.master_volume_db));
        let master_id = self.graph.add_node(Box::new(master_node));

        // Connect all track nodes → master (L→L, R→R)
        for &track_id in &track_node_ids {
            self.graph.connect(track_id, 0, master_id, 0); // Left
            self.graph.connect(track_id, 1, master_id, 1); // Right
        }

        self.needs_rebuild = false;
    }
}

impl AudioCallback for EngineCallback {
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        self.process_commands();

        if self.needs_rebuild {
            self.rebuild_graph();
        }

        if !self.transport.is_playing() {
            output.fill(0.0);
            return;
        }

        let ctx = ProcessContext {
            sample_rate: self.sample_rate as f64,
            buffer_size: num_frames as u32,
            tempo: self
                .transport
                .bpm
                .load(std::sync::atomic::Ordering::Relaxed),
            time_sig: crate::transport::unpack_time_sig(
                self.transport
                    .time_sig
                    .load(std::sync::atomic::Ordering::Relaxed),
            ),
            position_samples: self.transport.position(),
            playing: true,
        };

        // Process the audio graph
        self.graph.process(&ctx);

        // Get master output (last node in graph)
        let node_count = self.graph.node_count();
        if node_count > 0 {
            if let Some(master_out) = self.graph.node_output(node_count - 1) {
                // Copy to interleaved output
                for frame in 0..num_frames {
                    let l = master_out
                        .first()
                        .and_then(|ch| ch.get(frame))
                        .copied()
                        .unwrap_or(0.0);
                    let r = master_out
                        .get(1)
                        .and_then(|ch| ch.get(frame))
                        .copied()
                        .unwrap_or(0.0);
                    output[frame * 2] = l;
                    output[frame * 2 + 1] = r;
                }

                // Update meters
                let left: Vec<f32> = (0..num_frames).map(|i| output[i * 2]).collect();
                let right: Vec<f32> = (0..num_frames).map(|i| output[i * 2 + 1]).collect();
                self.meter.process_block(&left, &right);

                let snapshot = MeterSnapshot {
                    peak_db: self.meter.peak_db(),
                    peak_hold_db: self.meter.peak_hold_db(),
                    true_peak_db: self.meter.true_peak_db(),
                    rms_db: self.meter.rms_db(),
                    lufs_m: self.meter.lufs_m(num_frames),
                    lufs_s: self.meter.lufs_s(num_frames),
                    lufs_i: self.meter.lufs_i(),
                    clipped: self.meter.clipped(),
                };
                // Lock-free push; if the consumer is behind, drop the sample.
                let _ = self.meter_producer.push(snapshot);
            }
        } else {
            output.fill(0.0);
        }

        // Advance transport
        self.transport.advance(num_frames as u64);
    }
}
