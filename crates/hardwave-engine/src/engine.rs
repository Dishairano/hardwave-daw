//! DawEngine — the top-level orchestrator that owns the audio graph, transport,
//! plugin host, and project state. Bridges between the audio callback and the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use std::sync::Arc;

use hardwave_audio_io::{AudioCallback, AudioDeviceManager};
use hardwave_metering::{ChannelMeter, MeterSnapshot};
use hardwave_plugin_host::PluginScanner;
use hardwave_project::Project;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioGraph, ProcessContext};
use crate::master_node::MasterNode;
use crate::track_node::{ClipRegion, TrackNode};
use crate::transport::{TransportCommand, TransportState};

/// Main DAW engine.
pub struct DawEngine {
    pub transport: TransportState,
    pub project: Arc<Mutex<Project>>,
    pub plugin_scanner: Arc<Mutex<PluginScanner>>,
    pub audio_pool: AudioPool,

    audio_device: AudioDeviceManager,
    command_tx: Sender<EngineCommand>,
    command_rx: Receiver<EngineCommand>,

    // Metering (written by audio thread, read by UI)
    master_meter: Arc<Mutex<MeterSnapshot>>,
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
            master_meter: Arc::new(Mutex::new(MeterSnapshot::default())),
        }
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), String> {
        let transport = self.transport.clone();
        let project = Arc::clone(&self.project);
        let master_meter = Arc::clone(&self.master_meter);
        let command_rx = self.command_rx.clone();
        let audio_pool = self.audio_pool.clone();

        let sample_rate = self.audio_device.sample_rate;
        let buffer_size = self.audio_device.buffer_size;

        transport
            .sample_rate
            .store(sample_rate as u64, std::sync::atomic::Ordering::Relaxed);

        let callback = EngineCallback::new(
            transport,
            project,
            master_meter,
            command_rx,
            audio_pool,
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
        let (info, channels) =
            hardwave_dsp::AudioFileReader::read(path).map_err(|e| e.to_string())?;

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

    /// Get the latest master meter snapshot.
    pub fn master_meter(&self) -> MeterSnapshot {
        self.master_meter.lock().clone()
    }

    /// Scan for plugins.
    pub fn scan_plugins(&self) -> Vec<hardwave_plugin_host::PluginDescriptor> {
        let mut scanner = self.plugin_scanner.lock();
        scanner.scan().to_vec()
    }

    pub fn is_running(&self) -> bool {
        self.audio_device.is_running()
    }

    /// Get a reference to the audio device manager for device listing.
    pub fn audio_device_manager(&self) -> &AudioDeviceManager {
        &self.audio_device
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
    master_meter: Arc<Mutex<MeterSnapshot>>,
    command_rx: Receiver<EngineCommand>,
    audio_pool: AudioPool,
    graph: AudioGraph,
    meter: ChannelMeter,
    sample_rate: u32,
    needs_rebuild: bool,
}

impl EngineCallback {
    fn new(
        transport: TransportState,
        project: Arc<Mutex<Project>>,
        master_meter: Arc<Mutex<MeterSnapshot>>,
        command_rx: Receiver<EngineCommand>,
        audio_pool: AudioPool,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Self {
        let mut cb = Self {
            transport,
            project,
            master_meter,
            command_rx,
            audio_pool,
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
                self.transport
                    .playing
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

        for track in &project.tracks {
            if matches!(track.kind, hardwave_project::track::TrackKind::Master) {
                continue;
            }

            let mut node = TrackNode::new(track.name.clone(), self.audio_pool.clone());
            node.set_volume_db(track.volume_db);
            node.set_pan(track.pan);
            // If any track is soloed, mute tracks that aren't soloed (unless solo-safe)
            let effective_mute =
                track.muted || (any_soloed && !track.soloed && !track.solo_safe);
            node.set_muted(effective_mute);
            node.set_soloed(track.soloed);

            // Convert clip placements to sample-based ClipRegions
            let regions: Vec<ClipRegion> = track
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
                        Some(ClipRegion {
                            source_id: audio_clip.source_path.clone(),
                            timeline_start,
                            timeline_end,
                            source_offset: audio_clip.source_start,
                            gain,
                            muted: audio_clip.muted,
                        })
                    }
                    _ => None,
                })
                .collect();

            node.set_clips(regions);

            let node_id = self.graph.add_node(Box::new(node));
            track_node_ids.push(node_id);
        }

        // Add master node
        let master_node = MasterNode::new();
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
            time_sig: (4, 4),
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

                if let Some(mut snapshot) = self.master_meter.try_lock() {
                    snapshot.peak_db = self.meter.peak_db();
                    snapshot.peak_hold_db = self.meter.peak_hold_db();
                    snapshot.true_peak_db = self.meter.true_peak_db();
                    snapshot.rms_db = self.meter.rms_db();
                    snapshot.lufs_m = self.meter.lufs_m(num_frames);
                    snapshot.lufs_s = self.meter.lufs_s(num_frames);
                    snapshot.lufs_i = self.meter.lufs_i();
                    snapshot.clipped = self.meter.clipped();
                }
            }
        } else {
            output.fill(0.0);
        }

        // Advance transport
        self.transport.advance(num_frames as u64);
    }
}
