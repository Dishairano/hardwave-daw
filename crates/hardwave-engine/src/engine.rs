//! DawEngine — the top-level orchestrator that owns the audio graph, transport,
//! plugin host, and project state. Bridges between the audio callback and the UI.

use crossbeam_channel::{bounded, Receiver, Sender};
use parking_lot::Mutex;
use std::sync::Arc;

use hardwave_audio_io::{AudioCallback, AudioDeviceManager};
use hardwave_metering::{ChannelMeter, MeterSnapshot};
use hardwave_midi::{MidiCaptureRing, MidiInputManager};
use hardwave_plugin_host::PluginScanner;
use hardwave_project::Project;

/// Factory that instantiates a plug-in by its scanner id, used to
/// hydrate insert chains for offline render (so exports apply the same FX
/// as live playback). The closure returning `None` skips that insert.
/// Implemented by the command layer, which owns the native/VST3/CLAP
/// factory + scanner.
pub type OfflineInsertFactory<'a> =
    &'a dyn Fn(&str) -> Option<Box<dyn hardwave_plugin_host::types::HostedPlugin>>;
use rtrb::RingBuffer;

use std::collections::HashMap;

use crate::audio_pool::{AudioBuffer, AudioPool};
use crate::graph::{AudioGraph, AudioNode, ProcessContext};
use crate::input_node::{CaptureTap, InputNode, SharedInputConsumer};
use crate::master_node::MasterNode;
use crate::master_tap::{self, SharedMasterTap};
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

/// Channel sender the audio thread uses to ship its end-of-block
/// plug-in state snapshot map back to the UI thread. Keyed by
/// `(track_id, slot_id)` so the project save path can serialise each
/// slot's opaque state blob.
pub type PluginStateSnapshotSender =
    std::sync::mpsc::SyncSender<HashMap<(String, String), Vec<u8>>>;

/// UI-thread → audio-thread parking slot for `PluginStateSnapshotSender`.
/// The UI places a sender here before `save_project`, the audio thread
/// services exactly one snapshot request per audio block.
pub type PluginStateSnapshotSlot = Arc<Mutex<Option<PluginStateSnapshotSender>>>;

/// Main DAW engine.
pub struct DawEngine {
    pub transport: TransportState,
    pub project: Arc<Mutex<Project>>,
    pub plugin_scanner: Arc<Mutex<PluginScanner>>,
    pub midi_input: Arc<Mutex<MidiInputManager>>,
    pub audio_pool: AudioPool,
    /// Critical-path latency in samples, published by the audio thread after
    /// each graph rebuild. Coarse proxy for "total project latency" until
    /// full PDC lands.
    pub graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,

    audio_device: AudioDeviceManager,
    command_tx: Sender<EngineCommand>,
    command_rx: Receiver<EngineCommand>,

    // Metering: lock-free ring buffer (audio thread → UI thread)
    meter_consumer: Option<rtrb::Consumer<MeterSnapshot>>,
    meter_cache: MeterSnapshot,

    /// Per-track post-fader meter state, keyed by track id.
    pub track_meters: TrackMeterMap,

    /// Shared consumer side of the input monitor ring buffer. The audio-io
    /// input callback pushes interleaved stereo samples into the matching
    /// producer; the graph's InputNode drains from this consumer. Held in a
    /// Mutex<Option<_>> so we can swap it when the input stream is stopped
    /// and restarted without rebuilding the whole engine.
    input_consumer: SharedInputConsumer,

    /// Recording capture target. The audio thread's InputNode appends
    /// interleaved L/R samples here when [`CaptureTap::recording`] is
    /// true. The UI thread can drain this on stop to write a WAV file
    /// and place a clip on the armed track.
    pub capture: Arc<CaptureTap>,

    /// Undo/redo history. Take a snapshot BEFORE mutating the project.
    pub history: Arc<Mutex<History>>,

    /// Circular buffer of recent master-bus output samples. Used by the UI
    /// for oscilloscope / spectrum / correlation visualizations.
    pub master_tap: SharedMasterTap,

    /// UI-side handle for queueing per-track plug-in commands toward
    /// the audio thread. Populated when `start()` builds the audio
    /// callback; `None` before start or after stop. Wrapped in a Mutex
    /// because Tauri command handlers fire concurrently — the
    /// underlying rtrb Producer is single-producer.
    pub insert_command_sender: Arc<Mutex<Option<crate::insert_chain::InsertCommandSender>>>,

    /// Receiver side of the audio→drop graveyard. The UI thread (or
    /// engine shutdown) drains and drops vacated plug-in instances so
    /// the audio thread never frees memory directly.
    pub insert_graveyard: Arc<Mutex<Option<crate::insert_chain::PluginGraveyardReceiver>>>,

    /// One-shot plug-in state snapshot request channel. The UI thread
    /// places a SyncSender here just before `save_project`, then waits
    /// on the matching Receiver. The audio thread polls this slot on
    /// every block and, if it finds a Sender, walks all chains, calls
    /// `plugin.get_state()` per slot, and sends the resulting map back.
    /// Used to capture plug-in state into `Project.plugin_states` so
    /// save/load round-trips preserve the user's knob tweaks.
    pub pending_state_snapshot: PluginStateSnapshotSlot,
    /// Rolling 3-minute capture of every live MIDI event seen by the
    /// engine. Audio thread pushes per-block; UI thread can dump the
    /// last N seconds into a clip without arming a track. The ring is
    /// always recording — there is no start/stop, just "dump from now
    /// minus N seconds". Capacity sized for ~8k events which covers
    /// dense input over the 3-minute window.
    pub midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
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
            midi_input: Arc::new(Mutex::new(MidiInputManager::new())),
            audio_pool: AudioPool::new(),
            audio_device: AudioDeviceManager::new(),
            command_tx: tx,
            command_rx: rx,
            meter_consumer: None,
            meter_cache: MeterSnapshot::default(),
            track_meters: Arc::new(Mutex::new(HashMap::new())),
            input_consumer: Arc::new(Mutex::new(None)),
            capture: Arc::new(CaptureTap::default()),
            history: Arc::new(Mutex::new(History::new())),
            master_tap: master_tap::new_shared(),
            graph_latency_samples: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            insert_command_sender: Arc::new(Mutex::new(None)),
            insert_graveyard: Arc::new(Mutex::new(None)),
            pending_state_snapshot: Arc::new(Mutex::new(None)),
            midi_capture_ring: Arc::new(Mutex::new(MidiCaptureRing::new(8192))),
        }
    }

    /// Block the calling thread (UI) up to `timeout` while the audio
    /// thread snapshots the current state of every loaded plug-in
    /// instance into a `(track_id, slot_id) -> state_bytes` map. Returns
    /// `None` on timeout or if the engine hasn't been started yet.
    pub fn snapshot_plugin_states(
        &self,
        timeout: std::time::Duration,
    ) -> Option<std::collections::HashMap<(String, String), Vec<u8>>> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        {
            let mut slot = self.pending_state_snapshot.lock();
            // If a previous request is still pending, just bail — caller
            // can retry. We don't queue requests because the audio thread
            // only services one per block anyway.
            if slot.is_some() {
                return None;
            }
            *slot = Some(tx);
        }
        rx.recv_timeout(timeout).ok()
    }

    /// Try to push a per-track plug-in command onto the audio-thread
    /// queue. Returns `Err(cmd)` if the engine is not started yet or
    /// the queue is full so the caller can retry on the next UI tick.
    pub fn try_send_insert_command(
        &self,
        cmd: crate::insert_chain::InsertCommand,
    ) -> Result<(), crate::insert_chain::InsertCommand> {
        let mut sender = self.insert_command_sender.lock();
        match sender.as_mut() {
            Some(s) => s.try_send(cmd),
            None => Err(cmd),
        }
    }

    /// Drop everything pending in the plug-in graveyard. Call on a
    /// non-RT cadence (UI tick, on quit, etc) so vacated plug-in
    /// instances actually run their destructors. Returns the number
    /// dropped.
    pub fn drain_insert_graveyard(&self) -> usize {
        let mut graveyard = self.insert_graveyard.lock();
        match graveyard.as_mut() {
            Some(g) => g.drain_and_drop(),
            None => 0,
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

    /// Begin a recording capture. Clears any previously buffered samples
    /// and flips the `recording` flag so the audio thread starts pushing
    /// input blocks into the capture buffer immediately. Idempotent — a
    /// second call while already recording is a no-op.
    pub fn start_capture(&self) {
        use std::sync::atomic::Ordering;
        if self.capture.recording.load(Ordering::Relaxed) {
            return;
        }
        self.capture.buffer.lock().clear();
        self.capture.recording.store(true, Ordering::Relaxed);
    }

    /// Finish the active capture and return the interleaved L/R samples
    /// captured since `start_capture()`. The buffer is left empty so a
    /// follow-up start_capture begins from scratch.
    pub fn stop_capture(&self) -> Vec<f32> {
        use std::sync::atomic::Ordering;
        self.capture.recording.store(false, Ordering::Relaxed);
        std::mem::take(&mut *self.capture.buffer.lock())
    }

    /// Sample rate of the running audio device, or the offline default
    /// when the engine isn't started yet. Needed by the recording
    /// pipeline to write the right WAV header.
    pub fn current_sample_rate(&self) -> u32 {
        let sr = self.audio_device.sample_rate;
        if sr > 0 {
            sr
        } else {
            48_000
        }
    }

    /// Start the audio engine.
    pub fn start(&mut self) -> Result<(), String> {
        let transport = self.transport.clone();
        let project = Arc::clone(&self.project);
        let command_rx = self.command_rx.clone();
        let audio_pool = self.audio_pool.clone();

        // Negotiate the actual stream rate BEFORE building the callback.
        // cpal can silently fall back from the requested rate to the device's
        // default if the device doesn't accept the request — doing the
        // negotiation up front guarantees the callback, transport, and any
        // already-loaded audio buffers all agree on the same rate.
        self.audio_device
            .negotiate_output_rate()
            .map_err(|e| e.to_string())?;

        let sample_rate = self.audio_device.sample_rate;
        let buffer_size = self.audio_device.buffer_size;

        // Any audio buffer previously loaded at a different rate would play
        // back at the wrong pitch after this restart — re-resample them to
        // the negotiated rate before the audio thread sees the new pool.
        let resampled = self.audio_pool.resample_all(sample_rate, |chs, src, dst| {
            hardwave_dsp::resample_channels(chs, src, dst).ok()
        });
        if resampled > 0 {
            log::info!(
                "Re-resampled {} cached audio buffer(s) to {} Hz",
                resampled,
                sample_rate
            );
        }

        transport
            .sample_rate
            .store(sample_rate as u64, std::sync::atomic::Ordering::Relaxed);

        let (meter_producer, meter_consumer) = RingBuffer::new(16);
        self.meter_consumer = Some(meter_consumer);

        // Fresh insert-chain channels per audio session. UI-side handles
        // are exposed through `insert_command_sender` and `insert_graveyard`
        // so Tauri command handlers can route plug-in mutations to the
        // audio thread, and drain freed instances on a non-RT cadence.
        // Each TrackNode owns its own chain; the engine routes commands
        // by track_id_to_node lookup, so we build the channels directly
        // and don't use the bundled InsertRouter helper.
        let (cmd_tx, cmd_rx) = crate::insert_chain::command_channel(256);
        let (grave_tx, grave_rx) = crate::insert_chain::graveyard_channel(64);
        *self.insert_command_sender.lock() = Some(cmd_tx);
        *self.insert_graveyard.lock() = Some(grave_rx);

        let callback = EngineCallback::new(
            transport,
            project,
            meter_producer,
            command_rx,
            audio_pool,
            Arc::clone(&self.track_meters),
            Arc::clone(&self.input_consumer),
            Arc::clone(&self.capture),
            Arc::clone(&self.master_tap),
            Arc::clone(&self.graph_latency_samples),
            cmd_rx,
            grave_tx,
            Arc::clone(&self.pending_state_snapshot),
            Arc::clone(&self.midi_input),
            Arc::clone(&self.midi_capture_ring),
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
        let source_id = source_id_for_path(&path.to_string_lossy());

        let buffer = AudioBuffer {
            channels,
            sample_rate: info.sample_rate,
            num_frames,
        };

        self.audio_pool.insert(source_id.clone(), buffer);

        Ok((source_id, info))
    }

    /// Re-load every audio source referenced by the current project into the
    /// audio pool. The pool only ever fills at import time (`load_audio_file`),
    /// so without this every project reopened after an app restart has SILENT
    /// audio clips — the clips reference pool ids that no longer exist.
    /// Covers the live tracks AND every stored arrangement timeline. Returns
    /// the source paths that failed to load (missing/unreadable files) so the
    /// UI can surface them like missing plugins; the project still opens.
    pub fn rehydrate_audio_pool(&self) -> Vec<String> {
        use hardwave_project::clip::ClipContent;
        let paths: Vec<String> = {
            let project = self.project.lock();
            let mut set = std::collections::BTreeSet::new();
            let mut collect = |content: &ClipContent| {
                if let ClipContent::Audio(ac) = content {
                    set.insert(ac.source_path.clone());
                }
            };
            for track in &project.tracks {
                for clip in &track.clips {
                    collect(&clip.content);
                }
            }
            for arrangement in &project.arrangements {
                for timeline in arrangement.timelines.values() {
                    for clip in &timeline.clips {
                        collect(&clip.content);
                    }
                }
            }
            set.into_iter().collect()
        };
        let mut missing = Vec::new();
        for p in paths {
            // Skip sources already resident (e.g. loading a project into a
            // session that imported the same file) — insert would clone-churn.
            if self.audio_pool.get(&source_id_for_path(&p)).is_some() {
                continue;
            }
            if let Err(e) = self.load_audio_file(std::path::Path::new(&p)) {
                log::warn!("rehydrate_audio_pool: '{p}' failed to load: {e}");
                missing.push(p);
            }
        }
        missing
    }

    /// Ensure every non-master track in the project has an entry in the
    /// track meter map. Called from the UI thread so that `dev_dump_state`
    /// can see newly added tracks before the audio thread rebuilds.
    pub fn sync_track_meters(&self) {
        let project = self.project.lock();
        let mut meters = self.track_meters.lock();
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()));
        }
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| t.kind.is_audio_bearing())
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

    /// Snapshot the most recent `n_frames` stereo frames from the master
    /// tap, interleaved. Returns fewer samples if the tap hasn't filled.
    pub fn master_tap_snapshot(&self, n_frames: usize) -> Vec<f32> {
        self.master_tap.lock().snapshot_interleaved(n_frames)
    }

    /// Clear the master-tap circular buffer (e.g. after transport reset).
    pub fn master_tap_reset(&self) {
        self.master_tap.lock().reset();
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
    /// so the change takes effect immediately. If the restart fails (e.g. the
    /// device does not support exclusive at the current sample rate/format),
    /// the flag is rolled back and the stream is restarted in the previous
    /// mode so the engine is never left in a stopped+committed state.
    pub fn set_wasapi_exclusive(&mut self, enabled: bool) -> Result<(), String> {
        if self.audio_device.wasapi_exclusive == enabled {
            return Ok(());
        }
        let previous = self.audio_device.wasapi_exclusive;
        let was_running = self.audio_device.is_running();
        if was_running {
            self.audio_device.stop();
        }
        self.audio_device.wasapi_exclusive = enabled;
        if was_running {
            if let Err(e) = self.start() {
                self.audio_device.wasapi_exclusive = previous;
                let _ = self.start();
                return Err(e);
            }
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

    /// Open a cpal input stream that feeds the pre-record peak meter AND
    /// streams live samples into a ring buffer the graph's InputNode drains.
    /// Armed tracks with `monitor_input` enabled then hear live input.
    pub fn start_input_monitoring(&mut self) -> Result<(), String> {
        // Create a fresh ring buffer pair every time we (re)start — this
        // guarantees no stale samples from a previous session survive into
        // the new stream. Capacity is a few output blocks worth of stereo
        // samples so small drift between input/output buffer sizes absorbs
        // without audible glitching.
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(16384);
        self.audio_device.set_input_monitor_producer(Some(producer));
        *self.input_consumer.lock() = Some(consumer);
        self.audio_device
            .start_input_stream()
            .map_err(|e| e.to_string())
    }

    /// Stop the input monitor stream and detach the ring buffer.
    pub fn stop_input_monitoring(&mut self) {
        self.audio_device.stop_input_stream();
        self.audio_device.set_input_monitor_producer(None);
        *self.input_consumer.lock() = None;
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

    /// End of the project timeline in samples — the latest clip end across all
    /// tracks. Returns 0 when the project has no clips.
    pub fn project_end_samples(&self, sample_rate: u32) -> u64 {
        let project = self.project.lock();
        let sr = sample_rate as f64;
        let mut max_tick: u64 = 0;
        for track in &project.tracks {
            for clip in &track.clips {
                let end = clip.position_ticks + clip.length_ticks;
                if end > max_tick {
                    max_tick = end;
                }
            }
        }
        if max_tick == 0 {
            return 0;
        }
        project.tempo_map.tick_to_samples(max_tick, sr)
    }

    /// Offline-render the current project to an interleaved-stereo output
    /// stream. `on_block` is invoked with each rendered block (length =
    /// frames * 2). Rendering stops after `total_samples` frames.
    ///
    /// The render uses a private project snapshot and a fresh transport so
    /// the live audio thread is unaffected. `on_block` is called on the UI
    /// thread — typically it streams samples to disk.
    pub fn render_offline(
        &self,
        sample_rate: u32,
        total_samples: u64,
        on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        self.render_offline_with(sample_rate, total_samples, 0, None, |_| {}, on_block)
    }

    /// Like [`Self::render_offline`] but allows the caller to mutate the
    /// project snapshot before rendering — used for stems (mute all tracks
    /// except one) and similar isolation renders. The `on_block` callback
    /// returns `false` to halt rendering early (used for user cancellation).
    pub fn render_offline_with(
        &self,
        sample_rate: u32,
        total_samples: u64,
        start_samples: u64,
        instantiate: Option<OfflineInsertFactory<'_>>,
        prepare: impl FnOnce(&mut Project),
        mut on_block: impl FnMut(&[f32]) -> bool,
    ) -> Result<(), String> {
        if total_samples == 0 {
            return Ok(());
        }

        let mut project_snapshot = self.project.lock().clone();
        prepare(&mut project_snapshot);
        let initial_bpm = project_snapshot
            .tempo_map
            .entries
            .first()
            .map(|e| e.bpm)
            .unwrap_or(140.0);
        let project_arc = Arc::new(Mutex::new(project_snapshot));

        let transport = TransportState::default();
        use std::sync::atomic::Ordering;
        transport
            .sample_rate
            .store(sample_rate as u64, Ordering::Relaxed);
        transport.bpm.store(initial_bpm, Ordering::Relaxed);
        transport.master_volume_db.store(
            self.transport.master_volume_db.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        transport.set_position(start_samples);
        transport.playing.store(true, Ordering::Relaxed);

        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (_command_tx, command_rx) = bounded::<EngineCommand>(4);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));

        let buffer_size: usize = 1024;
        // Offline render — use a private, discardable tap so we don't mix
        // offline samples into the live UI visualization stream.
        let offline_tap = master_tap::new_shared();
        let offline_latency = Arc::new(std::sync::atomic::AtomicU32::new(0));
        // Offline render gets its own throwaway insert-chain channels.
        // Plug-in commands aren't fired during offline render anyway —
        // the snapshot project is frozen — but EngineCallback's
        // signature requires them. Capacities are tiny because nothing
        // ever pushes here during offline.
        let (_offline_cmd_tx, offline_cmd_rx) = crate::insert_chain::command_channel(8);
        let (offline_grave_tx, _offline_grave_rx) = crate::insert_chain::graveyard_channel(8);

        let mut callback = EngineCallback::new(
            transport,
            project_arc,
            meter_producer,
            command_rx,
            self.audio_pool.clone(),
            track_meters,
            input_consumer,
            // Offline render doesn't capture anything — the recording tap
            // only matters for live audio. Pass a fresh detached tap so the
            // signature stays uniform without polluting the live capture.
            Arc::new(CaptureTap::default()),
            offline_tap,
            offline_latency,
            offline_cmd_rx,
            offline_grave_tx,
            // Offline render doesn't need snapshot capability — the
            // export pipeline never asks for it. Pass a parked Arc that
            // can never be filled.
            Arc::new(Mutex::new(None)),
            // Share the LIVE midi_input and capture ring so tests (and
            // any caller who pre-injects MIDI events) can drive the
            // offline render with synthesised input. Production export
            // workflows stop live playback before bouncing, so cross-
            // thread drain contention is not a real concern.
            Arc::clone(&self.midi_input),
            Arc::clone(&self.midi_capture_ring),
            sample_rate,
            buffer_size as u32,
        );

        // Populate insert chains from the project metadata so the export
        // applies the same FX as live playback. No-op when no factory is
        // supplied (e.g. engine unit tests that don't host plug-ins).
        if let Some(inst) = instantiate {
            callback.hydrate_offline_inserts(inst);
        }

        let mut buf = vec![0.0_f32; buffer_size * 2];
        let mut remaining = total_samples;
        while remaining > 0 {
            let frames = remaining.min(buffer_size as u64) as usize;
            let slice = &mut buf[..frames * 2];
            slice.fill(0.0);
            callback.process(slice, frames, 2);
            if !on_block(slice) {
                break;
            }
            remaining -= frames as u64;
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

/// The pool key `load_audio_file` derives for a path — kept in one place so
/// `rehydrate_audio_pool`'s already-resident check can never drift from it.
fn source_id_for_path(path: &str) -> String {
    format!("{:x}", md5_hash(path.as_bytes()))
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
    input_consumer: SharedInputConsumer,
    master_tap: SharedMasterTap,
    graph: AudioGraph,
    meter: ChannelMeter,
    /// Pre-sized deinterleave scratch for the meter path. Reused every
    /// block so the audio thread never allocates to feed the meter.
    meter_left: Vec<f32>,
    meter_right: Vec<f32>,
    sample_rate: u32,
    needs_rebuild: bool,
    /// Explicit id of the master node so we don't depend on its position in
    /// the node vector — the input node is added after master when armed
    /// tracks exist, so `node_count - 1` no longer identifies the master.
    master_id: Option<crate::graph::NodeId>,
    /// True when at least one track is armed with input monitoring enabled.
    /// When false AND the transport isn't playing, we can short-circuit the
    /// graph and emit silence to save CPU.
    has_monitored_input: bool,
    /// Shared with `DawEngine.capture`. Forwarded into the InputNode each
    /// rebuild so the recording tap survives graph rebuilds.
    capture: Arc<CaptureTap>,
    graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,
    /// Per-track plug-in command receiver. Drained at the start of every
    /// audio block; commands route to TrackNodes by track_id.
    insert_command_rx: crate::insert_chain::InsertCommandReceiver,
    /// Audio→drop-thread channel for vacated plug-in instances. The
    /// audio thread never frees plug-ins directly; it pushes them here
    /// and the UI thread (or the engine on shutdown) drops them.
    insert_graveyard_tx: crate::insert_chain::PluginGraveyardSender,
    /// Stable track-id → graph-NodeId map maintained across rebuilds so
    /// per-track commands resolve to the right TrackNode in O(1).
    track_id_to_node: HashMap<String, crate::graph::NodeId>,
    /// UI-thread snapshot request slot. Cloned from `DawEngine`. When a
    /// `SyncSender` lands here the audio thread harvests `get_state()`
    /// from every loaded plug-in and ships the resulting map back.
    pending_state_snapshot: PluginStateSnapshotSlot,
    /// Shared handle to the live MIDI input manager. Drained at the
    /// start of every audio block via `try_drain_events_into` (non-
    /// blocking — if the mutex is contended this block we just skip
    /// drain and pick up the events on the next block, which adds at
    /// most one block of latency). Cloned from
    /// [`DawEngine::midi_input`] so injected events (from the on-screen
    /// keyboard and Tauri `inject_midi_event` command) and external
    /// hardware events flow through the exact same path.
    midi_input: Arc<Mutex<MidiInputManager>>,
    /// Reusable per-block scratch for live MIDI events. Pre-sized so
    /// the audio thread doesn't allocate when the controller is busy.
    midi_event_scratch: Vec<hardwave_midi::MidiEvent>,
    /// Shared rolling capture for "dump last N seconds to pattern".
    /// Pushed-into per block on the audio thread.
    midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
    /// Insert commands whose target TrackNode doesn't exist yet because
    /// a graph rebuild was deferred by lock contention (see
    /// `rebuild_graph`'s try_lock). Retried at the start of the next
    /// dispatch pass instead of being dropped — otherwise a plug-in
    /// added to a just-created track could silently vanish. Pre-sized
    /// so pushes within capacity never allocate on the audio thread.
    deferred_insert_commands: Vec<crate::insert_chain::InsertCommand>,
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
        input_consumer: SharedInputConsumer,
        capture: Arc<CaptureTap>,
        master_tap: SharedMasterTap,
        graph_latency_samples: Arc<std::sync::atomic::AtomicU32>,
        insert_command_rx: crate::insert_chain::InsertCommandReceiver,
        insert_graveyard_tx: crate::insert_chain::PluginGraveyardSender,
        pending_state_snapshot: PluginStateSnapshotSlot,
        midi_input: Arc<Mutex<MidiInputManager>>,
        midi_capture_ring: Arc<Mutex<MidiCaptureRing>>,
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
            input_consumer,
            master_tap,
            graph: AudioGraph::new(buffer_size as usize),
            meter: ChannelMeter::new(sample_rate as f64),
            meter_left: Vec::with_capacity(buffer_size as usize),
            meter_right: Vec::with_capacity(buffer_size as usize),
            sample_rate,
            needs_rebuild: true,
            master_id: None,
            has_monitored_input: false,
            capture,
            graph_latency_samples,
            insert_command_rx,
            insert_graveyard_tx,
            track_id_to_node: HashMap::new(),
            pending_state_snapshot,
            midi_input,
            // 256 fits well above the typical few-events-per-block load
            // even from a chord-spamming controller (~32 events). Sized
            // up-front so the audio thread doesn't grow the Vec on its
            // first busy block.
            midi_event_scratch: Vec::with_capacity(256),
            midi_capture_ring,
            // 64 comfortably exceeds the commands one UI tick can emit
            // for tracks that don't have nodes yet (a track add plus a
            // handful of chain edits).
            deferred_insert_commands: Vec::with_capacity(64),
        };
        cb.rebuild_graph();
        cb
    }

    /// Honour a one-shot plug-in state snapshot request from the UI
    /// thread, if any. Called once per audio block. If the UI placed a
    /// SyncSender into `pending_state_snapshot`, we drain it here, walk
    /// every track in `track_id_to_node`, call `get_state()` on each
    /// loaded plug-in slot, and ship the map back via the channel.
    ///
    /// `get_state` is allowed on the audio thread for the formats we
    /// host today: native plug-ins return tiny JSON blobs (single-digit
    /// kilobytes), CLAP / VST3 chunks are also small and the
    /// implementations don't allocate beyond a `Vec<u8>`. A future
    /// commit can move this to a worker thread if a third-party plug-in
    /// turns out to misbehave.
    fn service_snapshot_request(&mut self) {
        // RT-safety: try_lock — if the UI thread is placing a request
        // into the slot right now, we simply pick it up next block
        // instead of blocking the audio callback on its mutex.
        let tx = {
            let Some(mut slot) = self.pending_state_snapshot.try_lock() else {
                return;
            };
            slot.take()
        };
        let Some(tx) = tx else { return };

        let mut map: std::collections::HashMap<(String, String), Vec<u8>> =
            std::collections::HashMap::new();
        // Clone the (track_id, node_id) pairs first so the iteration
        // borrow doesn't conflict with the mutable graph access below.
        let pairs: Vec<(String, crate::graph::NodeId)> = self
            .track_id_to_node
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        for (track_id, node_id) in pairs {
            if let Some(node) = self.graph.node_mut(node_id) {
                for (slot_id, bytes) in node.snapshot_plugin_states() {
                    map.insert((track_id.clone(), slot_id), bytes);
                }
            }
        }
        // Best-effort send — if the UI thread dropped the receiver
        // (e.g. on save abort) we silently discard.
        let _ = tx.try_send(map);
    }

    /// Drain pending insert-chain commands and dispatch each to the
    /// TrackNode whose `track_id` matches. Called on the audio thread
    /// at the start of every block, BEFORE the graph runs, so the
    /// chain reflects the latest UI state for this block.
    fn dispatch_insert_commands(&mut self) {
        // Retry commands parked by an earlier block whose target node
        // didn't exist yet (rebuild deferred by lock contention). Only
        // replay once the rebuild has actually landed — while it is still
        // pending the targets can't exist, and replaying would re-park
        // into the freshly-taken (capacity-0) Vec, allocating on the
        // audio thread. With the rebuild done nothing re-parks, so the
        // original allocation is retained via the put-back below.
        if !self.needs_rebuild && !self.deferred_insert_commands.is_empty() {
            let mut parked = std::mem::take(&mut self.deferred_insert_commands);
            for cmd in parked.drain(..) {
                self.dispatch_one_insert_command(cmd);
            }
            // Put the (now empty) original allocation back if nothing
            // re-parked in the meantime; otherwise keep the new pushes.
            if self.deferred_insert_commands.is_empty() {
                self.deferred_insert_commands = parked;
            }
        }
        while let Some(cmd) = self.insert_command_rx.try_recv() {
            self.dispatch_one_insert_command(cmd);
        }
    }

    fn dispatch_one_insert_command(&mut self, cmd: crate::insert_chain::InsertCommand) {
        let sample_rate = self.sample_rate as f64;
        let buffer_size = self.graph.buffer_size_hint();
        // Borrow the target id just long enough to look it up; after
        // `.copied()` the NodeId is owned and the borrow on `cmd`
        // ends, freeing it to move into apply_insert_command below.
        let node_id = self.track_id_to_node.get(cmd.target_track_id()).copied();
        let Some(node_id) = node_id else {
            // No node for this track yet. If a rebuild is still pending
            // (deferred by try_lock contention) the node will appear in
            // a few blocks — park the command instead of dropping it.
            // Past capacity we fall back to the old drop-with-warning
            // behaviour rather than allocating on the audio thread.
            if self.needs_rebuild
                && self.deferred_insert_commands.len() < self.deferred_insert_commands.capacity()
            {
                self.deferred_insert_commands.push(cmd);
            } else {
                log::warn!(
                    "insert chain: no TrackNode for track_id {}; dropping command",
                    cmd.target_track_id()
                );
            }
            return;
        };
        if let Some(node) = self.graph.node_mut(node_id) {
            node.apply_insert_command(cmd, &mut self.insert_graveyard_tx, sample_rate, buffer_size);
        }
    }

    /// Populate every track's insert chain from the project's saved
    /// `inserts` metadata, for offline render. The live path builds chains
    /// via the `InsertCommand` queue / `hydrate_chains_from_project`, but a
    /// fresh offline callback fires no commands — without this, exports
    /// would render dry (no EQ/comp/reverb/synth FX). `instantiate` is the
    /// command layer's native/VST3/CLAP factory keyed by plugin id; saved
    /// plug-in state is replayed so the bounce matches the mix.
    fn hydrate_offline_inserts(&mut self, instantiate: OfflineInsertFactory<'_>) {
        let buffer_size = self.graph.buffer_size_hint();
        let sr = self.sample_rate as f64;
        // Snapshot the insert plan under the project lock, then instantiate
        // outside it (plug-in loads can be slow and must not hold the lock).
        // (track_id, slot_id, plugin_id, enabled, wet, saved_state)
        let plan = {
            let project = self.project.lock();
            let mut acc = Vec::new();
            for t in &project.tracks {
                for s in &t.inserts {
                    let state = project.plugin_state(&s.id).map(|e| e.chunk.clone());
                    acc.push((
                        t.id.clone(),
                        s.id.clone(),
                        s.plugin_id.clone(),
                        s.enabled,
                        s.wet,
                        state,
                    ));
                }
            }
            acc
        };
        for (track_id, slot_id, plugin_id, enabled, wet, state) in plan {
            let Some(node_id) = self.track_id_to_node.get(&track_id).copied() else {
                continue;
            };
            let Some(mut plugin) = instantiate(&plugin_id) else {
                continue;
            };
            if let Some(bytes) = state {
                let _ = plugin.set_state(&bytes);
            }
            if let Some(node) = self.graph.node_mut(node_id) {
                let slot = crate::insert_chain::LiveSlot {
                    slot_id,
                    plugin,
                    enabled,
                    wet,
                    // Offline render does not yet wire the sidechain bus
                    // (export ducking is a follow-up); live playback does.
                    sidechain_active: false,
                };
                node.push_offline_slot(slot, sr, buffer_size);
            }
        }
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
        use std::sync::atomic::Ordering::Relaxed;
        match cmd {
            TransportCommand::Play => {
                if self.transport.wait_for_input.load(Relaxed)
                    && !self.transport.playing.load(Relaxed)
                {
                    // Park: process() starts playback on the first MIDI event.
                    self.transport.wait_pending.store(true, Relaxed);
                } else {
                    self.transport.playing.store(true, Relaxed);
                }
            }
            TransportCommand::SetWaitForInput(on) => {
                self.transport.wait_for_input.store(on, Relaxed);
                // Disabling while parked honours the earlier Play press.
                if !on && self.transport.wait_pending.swap(false, Relaxed) {
                    self.transport.playing.store(true, Relaxed);
                }
            }
            TransportCommand::Stop => {
                self.transport.wait_pending.store(false, Relaxed);
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
                if self.transport.wait_for_input.load(Relaxed)
                    && !self.transport.playing.load(Relaxed)
                {
                    // Armed-and-parked: recording light on, playhead waits
                    // for the performer's first note.
                    self.transport.wait_pending.store(true, Relaxed);
                } else {
                    self.transport.playing.store(true, Relaxed);
                }
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
        // RT-safety: this runs on the audio thread. If the UI currently
        // holds the project or meter-map lock we must NOT block — leave
        // `needs_rebuild` set and retry on the next block. One extra
        // block on the old graph (~5ms) is inaudible; blocking the audio
        // callback behind a UI mutation (priority inversion) is an
        // audible dropout. Locks are taken up-front, before the old
        // graph is torn down, so a deferred rebuild leaves the current
        // graph fully intact.
        let Some(project) = self.project.try_lock() else {
            self.needs_rebuild = true;
            return;
        };
        let Some(mut meters) = self.track_meters.try_lock() else {
            self.needs_rebuild = true;
            return;
        };

        // Step 1: extract plug-in chains from the *outgoing* TrackNodes
        // before the graph is cleared. Without this, every rebuild would
        // drop every Box<dyn HostedPlugin> on the audio thread (calls
        // free()) and silently destroy the user's plug-in instances on
        // every property tweak. Stashing them by track_id lets us
        // re-attach to the freshly-built TrackNode below — or, if a
        // track was deleted, ship its chain to the graveyard for an
        // off-RT thread to drop.
        let mut stashed_chains: HashMap<String, crate::insert_chain::InsertChain> = HashMap::new();
        for node in self.graph.iter_nodes_mut() {
            if let Some(track_id) = node.track_id().map(|s| s.to_string()) {
                if let Some(chain) = node.take_chain() {
                    stashed_chains.insert(track_id, chain);
                }
            }
        }

        self.graph.clear();

        let sample_rate = self.sample_rate as f64;
        let tempo_map = &project.tempo_map;

        // Parallel map from project track id to graph node id, needed to
        // connect send edges once every track node has been inserted.
        let mut track_id_to_node: HashMap<String, crate::graph::NodeId> = HashMap::new();

        // Determine if any track is soloed — if so, mute all non-soloed tracks
        let any_soloed = project
            .tracks
            .iter()
            .any(|t| t.soloed && t.kind.is_audio_bearing());

        // Reconcile the per-track meter map with the current tracks: reuse existing
        // Arcs where possible, create new ones, drop meters for removed tracks.
        // (`meters` guard acquired via try_lock at the top of this fn.)
        let live_ids: std::collections::HashSet<String> = project
            .tracks
            .iter()
            .filter(|t| t.kind.is_audio_bearing())
            .map(|t| t.id.clone())
            .collect();
        meters.retain(|id, _| live_ids.contains(id));

        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }

            let meter = meters
                .entry(track.id.clone())
                .or_insert_with(|| Arc::new(TrackMeterState::default()))
                .clone();

            // MIDI tracks get an entirely separate node type — a built-in
            // monosynth that converts pre-resolved note schedules into
            // audio. The TrackNode path below is for audio-bearing tracks
            // that play sample clips, which can't service MIDI content.
            if matches!(track.kind, hardwave_project::TrackKind::Midi) {
                let mut midi_node = crate::midi_track_node::MidiTrackNode::new(
                    track.id.clone(),
                    track.name.clone(),
                    meter.clone(),
                );
                midi_node.set_volume_db(track.volume_db);
                midi_node.set_pan(track.pan);
                let effective_mute_midi =
                    track.muted || (any_soloed && !track.soloed && !track.solo_safe);
                midi_node.set_muted(effective_mute_midi);
                midi_node.set_soloed(track.soloed);
                // Map the project's NativeInstrument enum to the
                // engine's Instrument enum (they're separate so the
                // engine doesn't take a project-crate dep cycle).
                use crate::midi_track_node::Instrument as EngInstr;
                use crate::midi_track_node::Waveform;
                use hardwave_project::track::NativeInstrument as ProjInstr;
                let (eng_instr, waveform) = match track.instrument {
                    ProjInstr::BuiltinSine => (EngInstr::BuiltinSine, Waveform::Sine),
                    ProjInstr::BuiltinSaw => (EngInstr::BuiltinSine, Waveform::Saw),
                    ProjInstr::BuiltinSquare => (EngInstr::BuiltinSine, Waveform::Square),
                    ProjInstr::BuiltinTriangle => (EngInstr::BuiltinSine, Waveform::Triangle),
                    ProjInstr::KickSynth => (EngInstr::KickSynth, Waveform::Sine),
                };
                midi_node.set_instrument(eng_instr, self.sample_rate as f32);
                midi_node.set_waveform(waveform);
                if matches!(track.instrument, ProjInstr::KickSynth)
                    && track.kick_patch.has_overrides()
                {
                    midi_node.apply_kick_patch(&track.kick_patch);
                }

                // Walk the clip placements and turn each MIDI note into a
                // sample-positioned MidiNoteRegion. Tempo-map lookups happen
                // here on the UI thread so the audio thread sees a flat
                // sorted list.
                let mut note_regions: Vec<crate::midi_track_node::MidiNoteRegion> = Vec::new();
                for clip in &track.clips {
                    let hardwave_project::clip::ClipContent::Midi(midi_ref) = &clip.content else {
                        continue;
                    };
                    for note in &midi_ref.clip.notes {
                        let on_tick = clip.position_ticks + note.start_tick;
                        let off_tick = on_tick + note.duration_ticks.max(1);
                        let note_on = tempo_map.tick_to_samples(on_tick, sample_rate);
                        let note_off = tempo_map.tick_to_samples(off_tick, sample_rate);
                        note_regions.push(crate::midi_track_node::MidiNoteRegion {
                            note_on_sample: note_on,
                            note_off_sample: note_off,
                            pitch: note.pitch,
                            velocity: note.velocity,
                            muted: note.muted,
                        });
                    }
                }
                midi_node.set_notes(note_regions);

                let node_id = self.graph.add_node(Box::new(midi_node));
                // Every MIDI track accepts live input by default. This
                // makes a freshly-created MIDI track immediately respond
                // to a controller without needing the user to find the
                // "arm" toggle. `armed && monitor_input` still narrows
                // routing when the user explicitly sets it (the OR keeps
                // the default-on behaviour while honouring the flags
                // once set).
                let accepts = matches!(track.kind, hardwave_project::TrackKind::Midi)
                    || (track.armed && track.monitor_input);
                self.graph.set_accepts_live_midi(node_id, accepts);
                track_id_to_node.insert(track.id.clone(), node_id);
                continue;
            }

            let mut node = TrackNode::new(
                track.id.clone(),
                track.name.clone(),
                self.audio_pool.clone(),
                meter,
            );
            // Snapshot the project's automation lanes for this track
            // onto the audio-thread node. Cloned here on the UI thread
            // so process() never has to walk the project tree under a
            // lock. Empty list is the no-automation steady state.
            node.set_automation_lanes(track.automation_lanes.clone());
            node.set_automation_clips(track.automation_clips.clone());
            // Reattach the plug-in chain stashed at the top of this
            // rebuild. Tracks that didn't exist before fall through to
            // the default-empty chain that TrackNode::new gave them.
            if let Some(chain) = stashed_chains.remove(&track.id) {
                node.restore_chain(chain);
            }
            node.set_volume_db(track.volume_db);
            node.set_pan(track.pan);
            let effective_mute = track.muted || (any_soloed && !track.soloed && !track.solo_safe);
            node.set_muted(effective_mute);
            node.set_soloed(track.soloed);
            node.set_phase_invert(track.phase_invert);
            node.set_swap_lr(track.swap_lr);
            node.set_stereo_separation(track.stereo_separation);
            node.set_delay_samples(track.delay_samples);
            let filter_kind = crate::track_node::TrackFilterType::parse(&track.filter_type);
            node.set_filter(
                filter_kind,
                track.filter_cutoff_hz,
                track.filter_resonance,
                sample_rate as f32,
            );

            // Per-track coarse + fine pitch offset. Combined as a single resample
            // factor that's folded into each clip's source_step below.
            let track_pitch_offset =
                (track.pitch_semitones as f64) / 12.0 + (track.fine_tune_cents as f64) / 1200.0;
            let track_pitch_factor = 2.0_f64.powf(track_pitch_offset);

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
                        // The track-level pitch/fine-tune offset is folded in as an
                        // additional resample factor.
                        let clip_pitch_factor = 2.0_f64.powf(audio_clip.pitch_semitones / 12.0);
                        let pitch_factor = clip_pitch_factor * track_pitch_factor;
                        let stretch = if audio_clip.stretch_ratio <= 0.01 {
                            1.0
                        } else {
                            audio_clip.stretch_ratio
                        };
                        let source_step = pitch_factor / stretch;
                        // Warp markers → precomputed segments. Marker
                        // ticks are clip-relative; going through the
                        // tempo map (absolute tick → samples, minus the
                        // clip start) keeps anchors honest under tempo
                        // automation.
                        let warp = if audio_clip.warp_markers.is_empty() {
                            Vec::new()
                        } else {
                            let anchors: Vec<(u64, f64)> = audio_clip
                                .warp_markers
                                .iter()
                                .map(|m| {
                                    let abs = tempo_map.tick_to_samples(
                                        clip.position_ticks + m.clip_tick,
                                        sample_rate,
                                    );
                                    (abs.saturating_sub(timeline_start), m.source_sample as f64)
                                })
                                .collect();
                            crate::track_node::build_warp_segments(
                                &anchors,
                                source_step,
                                audio_clip.source_start as f64,
                            )
                        };
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
                            warp,
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
            // Audio tracks accept live MIDI only when explicitly armed
            // for input — common pattern for routing a controller into
            // a synth plug-in loaded on an audio-bearing track. Hardware
            // events flow through the insert chain to that plug-in.
            self.graph
                .set_accepts_live_midi(node_id, track.armed && track.monitor_input);
            track_id_to_node.insert(track.id.clone(), node_id);
        }

        // Add master node (reads volume from shared transport atomic — no rebuild on change)
        let master_node = MasterNode::new(Arc::clone(&self.transport.master_volume_db));
        let master_id = self.graph.add_node(Box::new(master_node));
        self.master_id = Some(master_id);

        // Connect each track either to its configured output_bus (another
        // track) or to master. Invalid targets (self-routing, unknown id,
        // cycles) silently fall back to master — the command layer already
        // rejects bad values, but the engine is defensive to keep audio
        // flowing even if a legacy project carries stale routing.
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            let Some(&src_node) = track_id_to_node.get(&track.id) else {
                continue;
            };
            let dst_node = match track.output_bus.as_ref() {
                Some(bus_id) if bus_id != &track.id => {
                    // Walk the chain from the bus target; if the chain comes
                    // back to the current track it's a cycle, fall through to
                    // master. Otherwise use the configured bus when it maps to
                    // a real graph node.
                    let mut visited: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    let mut cursor: Option<&str> = Some(bus_id.as_str());
                    let mut cycle = false;
                    while let Some(next) = cursor {
                        if next == track.id {
                            cycle = true;
                            break;
                        }
                        if !visited.insert(next.to_string()) {
                            break;
                        }
                        cursor = project
                            .tracks
                            .iter()
                            .find(|t| t.id == next)
                            .and_then(|t| t.output_bus.as_deref());
                    }
                    if cycle {
                        master_id
                    } else {
                        track_id_to_node.get(bus_id).copied().unwrap_or(master_id)
                    }
                }
                _ => master_id,
            };
            self.graph.connect(src_node, 0, dst_node, 0);
            self.graph.connect(src_node, 1, dst_node, 1);
        }

        // Wire armed-and-monitoring tracks to a single InputNode that drains
        // the live-input ring buffer. Only build the node when at least one
        // track actually needs it — unused, it would still drain the ring on
        // every audio block and waste work.
        let mut monitor_routes: Vec<crate::graph::NodeId> = Vec::new();
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            if track.armed && track.monitor_input {
                if let Some(&node_id) = track_id_to_node.get(&track.id) {
                    monitor_routes.push(node_id);
                }
            }
        }
        self.has_monitored_input = !monitor_routes.is_empty();
        if self.has_monitored_input {
            let direct = self
                .transport
                .direct_monitoring
                .load(std::sync::atomic::Ordering::Relaxed);
            let mut input_node = InputNode::new(Arc::clone(&self.input_consumer));
            // Recording capture survives graph rebuilds because it's the same
            // CaptureTap Arc handed to every InputNode we ever build.
            input_node.set_capture(Some(Arc::clone(&self.capture)));
            let input_id = self.graph.add_node(Box::new(input_node));
            if direct {
                // Direct monitoring: bypass the track FX chain and route live
                // input straight to master for minimum latency.
                self.graph.connect(input_id, 0, master_id, 0);
                self.graph.connect(input_id, 1, master_id, 1);
            } else {
                for track_node_id in monitor_routes {
                    self.graph.connect(input_id, 0, track_node_id, 0);
                    self.graph.connect(input_id, 1, track_node_id, 1);
                }
            }
        }

        // Wire send routing. Each enabled send contributes an extra edge from
        // the source track's pre-fader tap (ports 2/3) or post-fader output
        // (ports 0/1) into the target track's input (ports 0/1), with the
        // send amount applied as per-edge gain.
        for track in &project.tracks {
            let src_node = match track_id_to_node.get(&track.id) {
                Some(&id) => id,
                None => continue,
            };
            for send in &track.sends {
                if !send.enabled {
                    continue;
                }
                let dst_node = match track_id_to_node.get(&send.target) {
                    Some(&id) => id,
                    None => continue,
                };
                if dst_node == src_node {
                    continue;
                }
                let gain = if send.gain_db <= -100.0 {
                    0.0
                } else {
                    10.0_f64.powf(send.gain_db / 20.0) as f32
                };
                let (src_l, src_r) = if send.pre_fader { (2, 3) } else { (0, 1) };
                self.graph
                    .connect_with_gain(src_node, src_l, dst_node, 0, gain);
                self.graph
                    .connect_with_gain(src_node, src_r, dst_node, 1, gain);
            }
        }

        // Sidechain routing. A plug-in slot with a `sidechain_source` set
        // gets that source track's post-fader output mixed into THIS
        // track node's input ports 2/3 — the sidechain bus, kept separate
        // from the main 0/1 signal. The node's insert chain forwards
        // those channels to the keyed slot so e.g. a compressor ducks the
        // bass against the kick. The per-slot flag is always synced (so
        // clearing a source disables it); multiple slots on one track
        // share the single bus, and distinct sources sum into it.
        for track in &project.tracks {
            let dst_node = match track_id_to_node.get(&track.id) {
                Some(&id) => id,
                None => continue,
            };
            let mut sources = Vec::new();
            for slot in &track.inserts {
                let active = slot.sidechain_source.is_some();
                if let Some(node) = self.graph.node_mut(dst_node) {
                    node.set_slot_sidechain(&slot.id, active);
                }
                if let Some(src_id) = &slot.sidechain_source {
                    if let Some(&src_node) = track_id_to_node.get(src_id) {
                        if src_node != dst_node && !sources.contains(&src_node) {
                            sources.push(src_node);
                        }
                    }
                }
            }
            for src_node in sources {
                self.graph.connect_with_gain(src_node, 0, dst_node, 2, 1.0);
                self.graph.connect_with_gain(src_node, 1, dst_node, 3, 1.0);
            }
        }

        // Persist the track_id → NodeId map so per-track plug-in
        // commands can resolve in O(1) on the audio thread.
        self.track_id_to_node = track_id_to_node;

        // Any chains left in `stashed_chains` belong to tracks that
        // disappeared during this rebuild. Ship them to the graveyard
        // so an off-RT thread drops them — never on the audio thread.
        for (track_id, mut chain) in stashed_chains.drain() {
            for slot in chain.slots.drain(..) {
                if self.insert_graveyard_tx.try_bury(slot).is_err() {
                    log::warn!(
                        "rebuild graveyard saturated dropping track {track_id} chain; remaining slots will free on engine shutdown"
                    );
                    break;
                }
            }
        }

        self.needs_rebuild = false;
        // Finalize PDC: compute per-edge compensation delays so parallel
        // paths arrive sample-aligned against the slowest branch before the
        // total-latency number is published.
        self.graph.finalize_pdc();
        self.graph_latency_samples.store(
            self.graph.total_latency_samples(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl AudioCallback for EngineCallback {
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        self.process_commands();

        if self.needs_rebuild {
            self.rebuild_graph();
        }

        // Drain pending plug-in chain commands (Add/Remove/SetParameter
        // /etc) and dispatch each to its owning TrackNode. Done AFTER
        // any pending rebuild so newly-created TrackNodes are reachable
        // and BEFORE graph.process so the chain reflects the latest
        // requested state for this block.
        self.dispatch_insert_commands();

        // Honour any pending plug-in state snapshot request from the UI
        // thread. Done AFTER dispatch so newly-added plug-ins from the
        // same UI tick are captured, and BEFORE graph.process so the
        // returned bytes reflect end-of-prior-block state instead of
        // a half-written block.
        self.service_snapshot_request();

        // Wait-for-input: while parked we must still drain MIDI (the
        // early-silence return below would otherwise starve the check
        // forever). The first event un-parks, starts playback THIS block,
        // and the drained events flow to the graph below so the note that
        // started the transport is heard.
        let parked = self
            .transport
            .wait_pending
            .load(std::sync::atomic::Ordering::Relaxed);
        let mut drained_while_parked = false;
        if parked {
            self.midi_event_scratch.clear();
            if let Some(mgr) = self.midi_input.try_lock() {
                mgr.try_drain_events_into(&mut self.midi_event_scratch);
            }
            drained_while_parked = true;
            if !self.midi_event_scratch.is_empty() {
                self.transport
                    .wait_pending
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                self.transport
                    .playing
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let playing = self.transport.is_playing();
        // When transport is stopped AND nothing is monitoring live input,
        // the graph can only produce silence — skip processing to save CPU.
        if !playing && !self.has_monitored_input {
            output.fill(0.0);
            return;
        }

        // Tempo-map following: when the project has multi-entry tempo automation,
        // look up the current BPM for the playhead and push it into the transport
        // atomic so plugins see the right tempo in ProcessContext. try_lock so the
        // audio thread never blocks on a mutating UI command — stale BPM for one
        // block is fine.
        if playing {
            if let Some(project) = self.project.try_lock() {
                if project.tempo_map.entries.len() > 1 {
                    let sr = self.sample_rate as f64;
                    let pos_samples = self.transport.position();
                    let cur_tick = project.tempo_map.samples_to_tick(pos_samples, sr);
                    let cur_bpm = project.tempo_map.bpm_at(cur_tick);
                    self.transport
                        .bpm
                        .store(cur_bpm, std::sync::atomic::Ordering::Relaxed);
                }
            }
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
            playing,
        };

        // Drain external/injected MIDI events into the per-block scratch.
        // try_lock keeps the audio thread non-blocking — on contention we
        // simply pick up the events on the next block (~5ms later, well
        // below human perception). The graph then forwards this slice
        // only to nodes whose `accepts_live_midi` flag is set during
        // rebuild (armed+monitor_input tracks + MIDI tracks).
        // (Skipped when the wait-for-input check above already drained —
        // clearing here would eat the very note that started playback.)
        if !drained_while_parked {
            self.midi_event_scratch.clear();
            if let Some(mgr) = self.midi_input.try_lock() {
                mgr.try_drain_events_into(&mut self.midi_event_scratch);
            }
        }

        // Push drained events into the rolling capture ring. try_lock
        // keeps the audio thread non-blocking — if the UI is mid-dump
        // we skip this block (events still feed the graph below).
        if !self.midi_event_scratch.is_empty() {
            if let Some(mut ring) = self.midi_capture_ring.try_lock() {
                let abs = self.transport.position();
                for ev in &self.midi_event_scratch {
                    ring.push(abs, *ev);
                }
            }
        }

        // Process the audio graph
        self.graph.process(&ctx, &self.midi_event_scratch);

        // Pre-zero the output so any early-return / missing master silences
        // the speakers instead of leaking last block's samples.
        output.fill(0.0);

        // Get master output
        if let Some(master_id) = self.master_id {
            if let Some(master_out) = self.graph.node_output(master_id) {
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

                // Update meters. Deinterleave into pre-sized scratch —
                // clear() keeps capacity, so no allocation after block 1.
                self.meter_left.clear();
                self.meter_right.clear();
                for i in 0..num_frames {
                    self.meter_left.push(output[i * 2]);
                    self.meter_right.push(output[i * 2 + 1]);
                }
                self.meter
                    .process_block(&self.meter_left, &self.meter_right);

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

                // Best-effort push of master samples for UI visualizations.
                // try_lock keeps the audio thread non-blocking — if the UI
                // is mid-snapshot we just drop this block.
                if let Some(mut tap) = self.master_tap.try_lock() {
                    tap.push_block(&output[..num_frames * 2]);
                }
            }
        }

        // Advance transport only when playing — input-monitoring alone must
        // not move the playhead.
        if playing {
            self.transport.advance(num_frames as u64);
        }
    }
}

#[cfg(test)]
mod offline_insert_tests {
    //! Proves the export path applies a track's insert FX. Before the
    //! offline-chain-hydration fix, `render_offline_with` rendered dry
    //! (no inserts). The A/B here: a track that makes sound (KickSynth)
    //! plus a "silencer" insert renders SILENT when a plugin factory is
    //! supplied, and AUDIBLE when it isn't.
    use super::*;
    use hardwave_midi::MidiEvent;
    use hardwave_plugin_host::types::{
        HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
    };
    use hardwave_project::clip::{AudioClip, ClipContent, ClipPlacement, FadeCurve};
    use hardwave_project::track::PluginSlot;

    /// Minimal insert that zeroes its output.
    struct Silencer {
        desc: PluginDescriptor,
    }
    impl HostedPlugin for Silencer {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _m: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            num_samples: usize,
        ) {
            for o in outputs.iter_mut() {
                o.clear();
                o.resize(num_samples, 0.0);
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _i: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _i: u32) -> f64 {
            0.0
        }
        fn set_parameter_value(&mut self, _i: u32, _v: f64) {}
        fn get_state(&self) -> Vec<u8> {
            Vec::new()
        }
        fn set_state(&mut self, _b: &[u8]) -> Result<(), String> {
            Ok(())
        }
        fn latency_samples(&self) -> u32 {
            0
        }
        fn open_editor(&mut self, _h: raw_window_handle::RawWindowHandle) -> bool {
            false
        }
        fn close_editor(&mut self) {}
        fn has_editor(&self) -> bool {
            false
        }
    }

    fn silencer_desc() -> PluginDescriptor {
        PluginDescriptor {
            id: "test.silencer".into(),
            name: "Silencer".into(),
            vendor: "t".into(),
            version: "1".into(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    fn build_engine_with_silenced_sine() -> DawEngine {
        let engine = DawEngine::new();
        let sr = 48_000u32;
        // A 1s stereo sine in the audio pool.
        let frames = sr as usize;
        let mut ch = Vec::with_capacity(frames);
        for n in 0..frames {
            let t = n as f32 / sr as f32;
            ch.push((std::f32::consts::TAU * 220.0 * t).sin() * 0.5);
        }
        let buffer = crate::AudioBuffer {
            channels: vec![ch.clone(), ch],
            sample_rate: sr,
            num_frames: frames,
        };
        engine.audio_pool.insert("sine".to_string(), buffer);

        let mut project = engine.project.lock();
        let id = project.add_audio_track("Sine".into());
        if let Some(t) = project.track_mut(&id) {
            t.clips.push(ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-sine".into(),
                    name: "sine".into(),
                    source_path: "sine".into(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: frames as u64,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                }),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
            });
            t.inserts.push(PluginSlot {
                id: "slot1".into(),
                plugin_id: "test.silencer".into(),
                enabled: true,
                state: None,
                sidechain_source: None,
                wet: 1.0,
            });
        }
        drop(project);
        engine
    }

    /// Regression: `load_project` swaps the Project in but the audio pool
    /// only fills at import time — before `rehydrate_audio_pool`, every
    /// project reopened after an app restart had SILENT audio clips.
    #[test]
    fn rehydrate_audio_pool_reloads_project_sources() {
        // A real WAV on disk, like a user's sample.
        let dir = std::env::temp_dir().join(format!("hw-rehydrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("kick.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..4_800 {
            w.write_sample(((i as f32 / 10.0).sin() * 8_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();
        let path_str = wav.to_string_lossy().to_string();

        // A project referencing that file — one clip on a live track, one in
        // a stored arrangement timeline (both walks must find sources).
        let engine = DawEngine::new();
        {
            let mut project = engine.project.lock();
            let id = project.add_audio_track("Kick".into());
            let clip = |cid: &str| ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: cid.into(),
                    name: "kick".into(),
                    source_path: path_str.clone(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: 4_800,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                }),
                track_id: id.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
            };
            if let Some(t) = project.track_mut(&id) {
                t.clips.push(clip("clip-live"));
            }
            let mut arrangement = hardwave_project::arrangement::Arrangement {
                id: "arr-b".into(),
                name: "B".into(),
                timelines: Default::default(),
            };
            arrangement.timelines.insert(
                id.clone(),
                hardwave_project::arrangement::TrackTimeline {
                    clips: vec![clip("clip-arr")],
                    automation_clips: vec![],
                },
            );
            project.arrangements.push(arrangement);
        }

        // Pool is empty (fresh engine) — exactly the post-restart state.
        let source_id = source_id_for_path(&path_str);
        assert!(engine.audio_pool.get(&source_id).is_none());

        let missing = engine.rehydrate_audio_pool();
        assert!(
            missing.is_empty(),
            "unexpected missing sources: {missing:?}"
        );
        let buf = engine
            .audio_pool
            .get(&source_id)
            .expect("source should be back in the pool after rehydrate");
        assert!(buf.num_frames > 0);

        // Missing files are reported, not fatal.
        {
            let mut project = engine.project.lock();
            let id2 = project.add_audio_track("Ghost".into());
            let mut ghost = ClipPlacement {
                content: ClipContent::Audio(AudioClip {
                    id: "clip-ghost".into(),
                    name: "ghost".into(),
                    source_path: dir.join("does-not-exist.wav").to_string_lossy().to_string(),
                    source_hash: String::new(),
                    source_start: 0,
                    source_end: 1,
                    gain_db: 0.0,
                    fade_in_ticks: 0,
                    fade_out_ticks: 0,
                    muted: false,
                    reversed: false,
                    pitch_semitones: 0.0,
                    stretch_ratio: 1.0,
                    warp_markers: Vec::new(),
                    fade_in_curve: FadeCurve::Linear,
                    fade_out_curve: FadeCurve::Linear,
                }),
                track_id: id2.clone(),
                position_ticks: 0,
                length_ticks: 1920,
                lane: 0,
            };
            ghost.track_id = id2.clone();
            if let Some(t) = project.track_mut(&id2) {
                t.clips.push(ghost);
            }
        }
        let missing = engine.rehydrate_audio_pool();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("does-not-exist"));

        std::fs::remove_dir_all(&dir).ok();
    }

    type TestPluginFactory<'a> = &'a dyn Fn(&str) -> Option<Box<dyn HostedPlugin>>;

    fn render_peak(engine: &DawEngine, factory: Option<TestPluginFactory<'_>>) -> f32 {
        let sr = 48_000u32;
        let mut peak = 0.0f32;
        engine
            .render_offline_with(
                sr,
                sr as u64,
                0,
                factory,
                |_| {},
                |block| {
                    for &s in block {
                        peak = peak.max(s.abs());
                    }
                    true
                },
            )
            .unwrap();
        peak
    }

    #[test]
    fn offline_render_applies_track_inserts() {
        let engine = build_engine_with_silenced_sine();
        let factory = |id: &str| -> Option<Box<dyn HostedPlugin>> {
            if id == "test.silencer" {
                Some(Box::new(Silencer {
                    desc: silencer_desc(),
                }))
            } else {
                None
            }
        };
        // With the factory, the silencer insert zeroes the kick.
        let peak_with = render_peak(&engine, Some(&factory));
        assert!(
            peak_with < 1e-6,
            "insert chain must apply offline; silencer should zero output, got peak={peak_with}"
        );
        // Without a factory (old behavior), the insert is skipped and the
        // kick is audible — confirming the difference is the hydration.
        let peak_without = render_peak(&engine, None);
        assert!(
            peak_without > 1e-3,
            "without the factory the source should still sound, got peak={peak_without}"
        );
    }
}

#[cfg(test)]
mod rt_safety_tests {
    //! Proves the audio callback never blocks on UI-held locks.
    //!
    //! Before the try_lock fix, `rebuild_graph()` took a blocking
    //! `project.lock()` on the audio thread — any UI mutation holding
    //! that mutex stalled the callback (audible dropout). Now a
    //! contended rebuild defers to the next block, and insert commands
    //! that target not-yet-built nodes are parked instead of dropped.
    use super::*;

    /// Build a minimal EngineCallback around the given shared project.
    fn make_callback(
        project_arc: Arc<Mutex<Project>>,
    ) -> (EngineCallback, crate::insert_chain::InsertCommandSender) {
        let transport = TransportState::default();
        transport
            .sample_rate
            .store(48_000, std::sync::atomic::Ordering::Relaxed);
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (_command_tx, command_rx) = bounded::<EngineCommand>(4);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));
        let (cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let cb = EngineCallback::new(
            transport,
            project_arc,
            meter_producer,
            command_rx,
            AudioPool::new(),
            track_meters,
            input_consumer,
            Arc::new(CaptureTap::default()),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            Arc::new(Mutex::new(MidiInputManager::new())),
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            48_000,
            256,
        );
        (cb, cmd_tx)
    }

    #[test]
    fn contended_rebuild_defers_instead_of_blocking() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        assert!(!cb.needs_rebuild, "constructor rebuild should have run");

        // UI thread holds the project lock across a callback.
        let guard = project_arc.lock();
        cb.needs_rebuild = true;
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(
            cb.needs_rebuild,
            "rebuild must defer while the project lock is contended"
        );
        drop(guard);

        // Lock released — next block completes the rebuild.
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild, "rebuild should run once the lock frees");
    }

    #[test]
    fn contended_meter_map_also_defers() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        let meters = Arc::clone(&cb.track_meters);
        let guard = meters.lock();
        cb.needs_rebuild = true;
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        assert!(cb.needs_rebuild, "meter-map contention must defer rebuild");
        drop(guard);
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild);
    }

    #[test]
    fn snapshot_request_survives_slot_contention() {
        let project_arc = Arc::new(Mutex::new(Project::default()));
        let (mut cb, _cmd_tx) = make_callback(Arc::clone(&project_arc));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let mut out = vec![0.0f32; 512];

        // Block 1: UI holds the slot mutex mid-placement — callback must
        // skip, not block, and the request must NOT be consumed.
        {
            let slot = Arc::clone(&cb.pending_state_snapshot);
            let guard = slot.lock();
            cb.process(&mut out, 256, 2);
            drop(guard);
        }
        *cb.pending_state_snapshot.lock() = Some(tx);
        // Block 2: uncontended — request is serviced.
        cb.process(&mut out, 256, 2);
        assert!(
            rx.try_recv().is_ok(),
            "snapshot request must be serviced on the first uncontended block"
        );
    }

    #[test]
    fn insert_command_for_pending_track_is_parked_not_dropped() {
        let mut project = Project::default();
        let track_id = project.add_audio_track("new track".into());
        let project_arc = Arc::new(Mutex::new(project));
        let (mut cb, mut cmd_tx) = make_callback(Arc::clone(&project_arc));

        // Simulate "track just added, rebuild deferred": wipe the node
        // map the command would resolve against and mark rebuild pending
        // while the UI holds the project lock.
        cb.track_id_to_node.clear();
        cb.needs_rebuild = true;

        assert!(
            cmd_tx
                .try_send(crate::insert_chain::InsertCommand::SetWet {
                    track_id: track_id.clone(),
                    slot_id: "slot-x".into(),
                    wet: 0.5,
                })
                .is_ok(),
            "queue insert command"
        );

        let guard = project_arc.lock();
        let mut out = vec![0.0f32; 512];
        cb.process(&mut out, 256, 2);
        drop(guard);
        assert_eq!(
            cb.deferred_insert_commands.len(),
            1,
            "command for a not-yet-built node must be parked while rebuild is pending"
        );

        // Rebuild completes on the next block; the parked command now
        // resolves to the rebuilt TrackNode and the parking lot drains.
        cb.process(&mut out, 256, 2);
        assert!(!cb.needs_rebuild);
        assert!(
            cb.deferred_insert_commands.is_empty(),
            "parked commands must be retried after the rebuild lands"
        );
        assert!(
            cb.track_id_to_node.contains_key(&track_id),
            "rebuilt graph must contain the new track's node"
        );
    }
}

#[cfg(test)]
mod wait_for_input_tests {
    //! FL-style "wait for input": Play/Record park the transport; the
    //! first MIDI event starts playback and is NOT swallowed (it must
    //! reach the graph in the same block so the triggering note sounds).
    use super::*;
    use std::sync::atomic::Ordering::Relaxed;

    fn make_callback(
        midi: Arc<Mutex<MidiInputManager>>,
    ) -> (EngineCallback, crossbeam_channel::Sender<EngineCommand>) {
        let transport = TransportState::default();
        transport.sample_rate.store(48_000, Relaxed);
        let (meter_producer, _meter_consumer) = RingBuffer::<MeterSnapshot>::new(4);
        let (command_tx, command_rx) = bounded::<EngineCommand>(8);
        let track_meters: TrackMeterMap = Arc::new(Mutex::new(HashMap::new()));
        let input_consumer: SharedInputConsumer = Arc::new(Mutex::new(None));
        let (_cmd_tx, cmd_rx) = crate::insert_chain::command_channel(8);
        let (grave_tx, _grave_rx) = crate::insert_chain::graveyard_channel(8);
        let cb = EngineCallback::new(
            transport,
            Arc::new(Mutex::new(Project::default())),
            meter_producer,
            command_rx,
            AudioPool::new(),
            track_meters,
            input_consumer,
            Arc::new(CaptureTap::default()),
            master_tap::new_shared(),
            Arc::new(std::sync::atomic::AtomicU32::new(0)),
            cmd_rx,
            grave_tx,
            Arc::new(Mutex::new(None)),
            midi,
            Arc::new(Mutex::new(MidiCaptureRing::new(64))),
            48_000,
            256,
        );
        (cb, command_tx)
    }

    #[test]
    fn play_parks_until_first_midi_event() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();

        // Blocks pass with no MIDI: parked, position frozen.
        for _ in 0..3 {
            cb.process(&mut out, 256, 2);
        }
        assert!(!cb.transport.is_playing(), "must stay parked with no input");
        assert!(cb.transport.wait_pending.load(Relaxed));
        assert_eq!(
            cb.transport.position(),
            0,
            "playhead must not advance while parked"
        );

        // First MIDI event: playback starts, and the event stays in the
        // scratch so the triggering note reaches the graph THIS block.
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        cb.process(&mut out, 256, 2);
        assert!(
            cb.transport.is_playing(),
            "first MIDI event starts playback"
        );
        assert!(!cb.transport.wait_pending.load(Relaxed));
        assert!(
            !cb.midi_event_scratch.is_empty(),
            "the triggering event must not be swallowed"
        );
    }

    #[test]
    fn stop_cancels_a_parked_transport() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(cb.transport.wait_pending.load(Relaxed));

        tx.send(EngineCommand::Transport(TransportCommand::Stop))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(
            !cb.transport.wait_pending.load(Relaxed),
            "Stop clears the park"
        );
        assert!(!cb.transport.is_playing());

        // Late MIDI after Stop must NOT start playback.
        midi.lock().inject(hardwave_midi::MidiEvent::NoteOn {
            timing: 0,
            channel: 0,
            note: 60,
            velocity: 0.9,
        });
        cb.process(&mut out, 256, 2);
        assert!(
            !cb.transport.is_playing(),
            "events after Stop must not un-park"
        );
    }

    #[test]
    fn disabling_wait_honours_pending_play() {
        let midi = Arc::new(Mutex::new(MidiInputManager::new()));
        let (mut cb, tx) = make_callback(Arc::clone(&midi));
        let mut out = vec![0.0f32; 512];

        cb.transport.wait_for_input.store(true, Relaxed);
        tx.send(EngineCommand::Transport(TransportCommand::Play))
            .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(cb.transport.wait_pending.load(Relaxed));

        tx.send(EngineCommand::Transport(TransportCommand::SetWaitForInput(
            false,
        )))
        .unwrap();
        cb.process(&mut out, 256, 2);
        assert!(
            cb.transport.is_playing(),
            "turning the pref off honours the earlier Play press"
        );
    }
}
