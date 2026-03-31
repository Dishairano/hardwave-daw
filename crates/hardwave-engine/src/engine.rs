//! DawEngine — the top-level orchestrator that owns the audio graph, transport,
//! plugin host, and project state. Bridges between the audio callback and the UI.

use std::sync::Arc;
use parking_lot::Mutex;
use crossbeam_channel::{Receiver, Sender, bounded};

use hardwave_audio_io::{AudioCallback, AudioDeviceManager};
use hardwave_project::Project;
use hardwave_metering::{ChannelMeter, MeterSnapshot};
use hardwave_plugin_host::PluginScanner;

use crate::graph::{AudioGraph, ProcessContext};
use crate::transport::{TransportState, TransportCommand};

/// Main DAW engine.
pub struct DawEngine {
    pub transport: TransportState,
    pub project: Arc<Mutex<Project>>,
    pub plugin_scanner: Arc<Mutex<PluginScanner>>,

    audio_device: AudioDeviceManager,
    command_tx: Sender<TransportCommand>,
    command_rx: Receiver<TransportCommand>,

    // Metering (written by audio thread, read by UI)
    master_meter: Arc<Mutex<MeterSnapshot>>,
}

impl DawEngine {
    pub fn new() -> Self {
        let (tx, rx) = bounded(64);

        Self {
            transport: TransportState::default(),
            project: Arc::new(Mutex::new(Project::default())),
            plugin_scanner: Arc::new(Mutex::new(PluginScanner::new())),
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

        let sample_rate = self.audio_device.sample_rate;
        let buffer_size = self.audio_device.buffer_size;

        transport.sample_rate.store(sample_rate as u64, std::sync::atomic::Ordering::Relaxed);

        let callback = EngineCallback::new(
            transport,
            project,
            master_meter,
            command_rx,
            sample_rate,
            buffer_size,
        );

        self.audio_device.start(callback)
            .map_err(|e| e.to_string())
    }

    /// Stop the audio engine.
    pub fn stop(&mut self) {
        self.audio_device.stop();
    }

    /// Send a transport command from the UI thread.
    pub fn send_command(&self, cmd: TransportCommand) {
        let _ = self.command_tx.try_send(cmd);
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
}

impl Default for DawEngine {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Audio callback (runs on the real-time audio thread)
// ---------------------------------------------------------------------------

struct EngineCallback {
    transport: TransportState,
    project: Arc<Mutex<Project>>,
    master_meter: Arc<Mutex<MeterSnapshot>>,
    command_rx: Receiver<TransportCommand>,
    graph: AudioGraph,
    meter: ChannelMeter,
    sample_rate: u32,
}

impl EngineCallback {
    fn new(
        transport: TransportState,
        project: Arc<Mutex<Project>>,
        master_meter: Arc<Mutex<MeterSnapshot>>,
        command_rx: Receiver<TransportCommand>,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Self {
        Self {
            transport,
            project,
            master_meter,
            command_rx,
            graph: AudioGraph::new(buffer_size as usize),
            meter: ChannelMeter::new(sample_rate as f64),
            sample_rate,
        }
    }

    fn process_commands(&mut self) {
        while let Ok(cmd) = self.command_rx.try_recv() {
            match cmd {
                TransportCommand::Play => {
                    self.transport.playing.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                TransportCommand::Stop => {
                    self.transport.playing.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                TransportCommand::Record => {
                    self.transport.recording.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.transport.playing.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                TransportCommand::SetPosition(pos) => {
                    self.transport.set_position(pos);
                }
                TransportCommand::SetBpm(bpm) => {
                    self.transport.bpm.store(bpm, std::sync::atomic::Ordering::Relaxed);
                }
                TransportCommand::SetLoop(start, end) => {
                    self.transport.loop_start.store(start, std::sync::atomic::Ordering::Relaxed);
                    self.transport.loop_end.store(end, std::sync::atomic::Ordering::Relaxed);
                }
                TransportCommand::ToggleLoop => {
                    let current = self.transport.looping.load(std::sync::atomic::Ordering::Relaxed);
                    self.transport.looping.store(!current, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }
}

impl AudioCallback for EngineCallback {
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        self.process_commands();

        if !self.transport.is_playing() {
            output.fill(0.0);
            return;
        }

        let ctx = ProcessContext {
            sample_rate: self.sample_rate as f64,
            buffer_size: num_frames as u32,
            tempo: self.transport.bpm.load(std::sync::atomic::Ordering::Relaxed),
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
                    let l = master_out.get(0).and_then(|ch| ch.get(frame)).copied().unwrap_or(0.0);
                    let r = master_out.get(1).and_then(|ch| ch.get(frame)).copied().unwrap_or(0.0);
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
