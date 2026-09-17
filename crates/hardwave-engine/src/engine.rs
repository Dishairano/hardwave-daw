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

/// Separator that marks an audio-pool id as a derived (baked stretch /
/// pitch) variant rather than a real imported source. Real source ids are
/// file paths or import hashes and never contain it.
const STRETCH_KEY_MARKER: &str = "#hw-stretch:";

/// Pool id for a baked stretch / pitch-shift variant of a source. The
/// original id stays untouched in the project, so `rehydrate_audio_pool`
/// (which walks clips' `source_path`) never sees these derived keys.
fn stretch_cache_key(source_id: &str, stretch: f64, semitones: f64) -> String {
    format!("{source_id}{STRETCH_KEY_MARKER}{stretch:.6}:{semitones:.6}")
}

/// Drops a set of stretch-bake keys out of the in-flight set when it goes out
/// of scope, so a panicking bake can't leave them permanently claimed.
struct InflightRelease<'a> {
    set: &'a Arc<Mutex<std::collections::HashSet<String>>>,
    keys: Vec<String>,
}

impl Drop for InflightRelease<'_> {
    fn drop(&mut self) {
        let mut guard = self.set.lock();
        for key in &self.keys {
            guard.remove(key);
        }
    }
}

/// Does this clip want a pitch-preserving bake? Warped clips are excluded —
/// warp markers define their own piecewise source map and take precedence.
/// Returns the effective (stretch, semitones) when a bake applies.
fn bake_params(audio_clip: &hardwave_project::clip::AudioClip) -> Option<(f64, f64)> {
    if !audio_clip.warp_markers.is_empty() {
        return None;
    }
    let stretch = if audio_clip.stretch_ratio <= 0.01 {
        1.0
    } else {
        audio_clip.stretch_ratio
    };
    if (stretch - 1.0).abs() > 1e-4 || audio_clip.pitch_semitones.abs() > 1e-4 {
        Some((stretch, audio_clip.pitch_semitones))
    } else {
        None
    }
}
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

    /// Folder the current .hwp lives in, set on save and load.
    ///
    /// A clip's `source_file` may be relative to it, which is what lets a
    /// project folder be moved, copied to another drive or zipped and still
    /// open: an absolute path only describes where the audio was on the
    /// machine that imported it.
    project_dir: Arc<Mutex<Option<std::path::PathBuf>>>,

    /// How much of each audio block's time budget the engine actually spends,
    /// in per mille, written by the audio thread after every block.
    ///
    /// This is the number a CPU meter should show. The toolbar meter used to
    /// estimate load from WebView frame jitter, which measures how busy the
    /// UI is: it moved when a window was dragged and sat still while the
    /// audio thread was close to dropping out.
    audio_load_permille: Arc<std::sync::atomic::AtomicU32>,
    /// Blocks that overran their budget since the session started. One xrun
    /// is an audible click, so the count matters even when the average looks
    /// comfortable.
    audio_xruns: Arc<std::sync::atomic::AtomicU32>,

    /// Click settings, shared with the audio thread.
    metronome: crate::metronome::MetronomeSettings,

    /// Browser audition requests, shared with the audio thread.
    preview: crate::preview_player::PreviewRequest,

    /// Behaviour switches from the Audio settings panel, read by the audio
    /// thread every block.
    audio_prefs: crate::audio_prefs::AudioPrefs,

    audio_device: AudioDeviceManager,
    command_tx: Sender<EngineCommand>,
    command_rx: Receiver<EngineCommand>,

    /// Stretch-bake keys currently being rendered on a background thread.
    /// Dragging a stretch control fires a rebuild per frame; without this
    /// every one of them would spawn a duplicate bake of the same source.
    stretch_inflight: Arc<Mutex<std::collections::HashSet<String>>>,

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
    /// True while a gesture is being treated as one undo step.
    history_group_open: Arc<std::sync::atomic::AtomicBool>,
    /// True once the open group has taken its snapshot.
    history_group_taken: Arc<std::sync::atomic::AtomicBool>,

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
            stretch_inflight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            meter_consumer: None,
            meter_cache: MeterSnapshot::default(),
            track_meters: Arc::new(Mutex::new(HashMap::new())),
            input_consumer: Arc::new(Mutex::new(None)),
            capture: Arc::new(CaptureTap::default()),
            history: Arc::new(Mutex::new(History::new())),
            history_group_open: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            history_group_taken: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            master_tap: master_tap::new_shared(),
            graph_latency_samples: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            project_dir: Arc::new(Mutex::new(None)),
            audio_load_permille: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            audio_xruns: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            metronome: crate::metronome::MetronomeSettings::new(),
            preview: crate::preview_player::PreviewRequest::new(),
            audio_prefs: crate::audio_prefs::AudioPrefs::new(),
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
        // Inside an open group only the first mutation is snapshotted, so one
        // gesture is one undo. Painting twenty clips in a drag used to leave
        // twenty snapshots behind, and undoing the drag meant pressing undo
        // twenty times.
        use std::sync::atomic::Ordering::Relaxed;
        if self.history_group_open.load(Relaxed) && self.history_group_taken.swap(true, Relaxed) {
            return;
        }
        let snap = self.project.lock().clone();
        self.history.lock().push(snap);
    }

    /// Start treating the mutations that follow as one undo step.
    ///
    /// Balanced by `end_history_group`. Re-entrant calls are ignored rather
    /// than nested: a gesture is a gesture.
    pub fn begin_history_group(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.history_group_open.swap(true, Relaxed) {
            return;
        }
        // Nothing snapshotted for this group yet.
        self.history_group_taken.store(false, Relaxed);
    }

    /// End the group, so the next mutation snapshots again.
    pub fn end_history_group(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.history_group_open.store(false, Relaxed);
        self.history_group_taken.store(false, Relaxed);
    }

    pub fn history_group_open(&self) -> bool {
        self.history_group_open
            .load(std::sync::atomic::Ordering::Relaxed)
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
            LoadCounters {
                load_permille: Arc::clone(&self.audio_load_permille),
                xruns: Arc::clone(&self.audio_xruns),
            },
            self.metronome.clone(),
            self.preview.clone(),
            self.audio_prefs.clone(),
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
        // Kick the bake off in the background and signal the rebuild now.
        // Baking a long stem takes seconds (~16 s for 30 s of stereo), so
        // doing it inline would freeze the caller; until it lands the clip
        // simply plays varispeed, and the bake triggers its own rebuild.
        self.prebake_stretch_sources_async();
        let _ = self.command_tx.try_send(EngineCommand::RebuildGraph);
    }

    /// The (source, stretch, semitones) variants this project's clips need.
    fn stretch_bake_plan(&self) -> Vec<(String, f64, f64)> {
        use hardwave_project::clip::ClipContent;
        let project = self.project.lock();
        let mut acc: Vec<(String, f64, f64)> = Vec::new();
        for track in &project.tracks {
            for clip in &track.clips {
                if let ClipContent::Audio(audio_clip) = &clip.content {
                    if let Some((stretch, semis)) = bake_params(audio_clip) {
                        let entry = (audio_clip.source_path.clone(), stretch, semis);
                        if !acc.contains(&entry) {
                            acc.push(entry);
                        }
                    }
                }
            }
        }
        acc
    }

    /// Render one stretch variant into the pool. The stretch itself runs
    /// outside the pool lock, so the audio thread's reads are never blocked
    /// while a bake is in flight — only the final insert takes the write lock.
    fn bake_stretch_variant(pool: &AudioPool, source_id: &str, stretch: f64, semitones: f64) {
        let key = stretch_cache_key(source_id, stretch, semitones);
        if pool.contains(&key) {
            return;
        }
        let Some(src) = pool.get(source_id) else {
            return;
        };
        let req = hardwave_dsp::stretch_apply::StretchRequest {
            // Phase vocoder: the codebase's own "higher quality for tonal
            // sources" choice. The cost is paid once per
            // (source, ratio, pitch) and then cached.
            algorithm: hardwave_dsp::stretch_apply::StretchAlgorithm::PhaseVocoder,
            stretch_ratio: stretch as f32,
            pitch_semitones: semitones as f32,
            sample_rate: src.sample_rate as f32,
        };
        let channels: Vec<Vec<f32>> = src
            .channels
            .iter()
            .map(|ch| hardwave_dsp::stretch_apply::apply_stretch(ch, req))
            .collect();
        let num_frames = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if num_frames == 0 {
            return;
        }
        pool.insert(
            key,
            AudioBuffer {
                channels,
                sample_rate: src.sample_rate,
                num_frames,
            },
        );
    }

    /// Bake every stretch variant the project needs, BLOCKING until done.
    ///
    /// This is the export path: a bounce must not race the bake, or it would
    /// render varispeed and not match playback. Never call it from the audio
    /// thread — see [`Self::prebake_stretch_sources_async`] for the UI path.
    pub fn prebake_stretch_sources(&self) {
        for (source_id, stretch, semitones) in self.stretch_bake_plan() {
            Self::bake_stretch_variant(&self.audio_pool, &source_id, stretch, semitones);
        }
    }

    /// Same, but hands the work to a background thread and returns at once.
    ///
    /// Playback stays responsive: an unbaked clip falls back to the varispeed
    /// resample path, and when the bake lands this triggers another rebuild so
    /// the audio thread picks it up. Keys already in flight are skipped, so
    /// dragging a stretch control doesn't spawn a bake per frame.
    pub fn prebake_stretch_sources_async(&self) {
        let todo: Vec<(String, f64, f64)> = {
            let mut inflight = self.stretch_inflight.lock();
            self.stretch_bake_plan()
                .into_iter()
                .filter(|(s, st, se)| {
                    let key = stretch_cache_key(s, *st, *se);
                    !self.audio_pool.contains(&key) && inflight.insert(key)
                })
                .collect()
        };
        if todo.is_empty() {
            return;
        }
        let pool = self.audio_pool.clone();
        let tx = self.command_tx.clone();
        let inflight = Arc::clone(&self.stretch_inflight);
        std::thread::spawn(move || {
            // Release the in-flight keys on the way out whatever happens. If a
            // bake panicked and left them claimed, that source could never be
            // retried and the clip would stay varispeed for the rest of the
            // session with nothing to indicate why.
            let _release = InflightRelease {
                set: &inflight,
                keys: todo
                    .iter()
                    .map(|(s, st, se)| stretch_cache_key(s, *st, *se))
                    .collect(),
            };
            for (source_id, stretch, semitones) in &todo {
                Self::bake_stretch_variant(&pool, source_id, *stretch, *semitones);
            }
            // Ask the audio thread to rebuild so the fresh bakes are picked up.
            let _ = tx.try_send(EngineCommand::RebuildGraph);
        });
    }

    /// Audition a file on the engine's own output device.
    ///
    /// Decodes it only once: clicking back and forth through a folder would
    /// otherwise re-decode and re-resample the same file on every click,
    /// which on a long sample is a pause before the sound starts.
    pub fn preview_file(&self, path: &std::path::Path) -> Result<(), String> {
        let source_id = source_id_for_path(&path.to_string_lossy());
        if !self.audio_pool.contains(&source_id) {
            self.load_audio_file_as(path, &source_id)?;
        }
        self.preview.play(&source_id);
        Ok(())
    }

    /// Load an audio file into the pool and return its source ID and info.
    pub fn load_audio_file(
        &self,
        path: &std::path::Path,
    ) -> Result<(String, hardwave_dsp::AudioFileInfo), String> {
        let source_id = source_id_for_path(&path.to_string_lossy());
        let info = self.load_audio_file_as(path, &source_id)?;
        Ok((source_id, info))
    }

    /// Load an audio file into the pool under an id the caller chooses.
    ///
    /// Reopening a project needs this: clips hold the pool id they were
    /// imported with, so the file has to come back under that same id even
    /// when it now lives somewhere else. Deriving the id from the path
    /// instead would give a relinked file a new id that no clip references,
    /// and the clip would stay silent.
    pub fn load_audio_file_as(
        &self,
        path: &std::path::Path,
        source_id: &str,
    ) -> Result<hardwave_dsp::AudioFileInfo, String> {
        let target_sr = self.audio_device.sample_rate;
        let (info, channels) = hardwave_dsp::AudioFileReader::read_resampled(path, Some(target_sr))
            .map_err(|e| e.to_string())?;

        let num_frames = channels.first().map(|c| c.len()).unwrap_or(0);

        let buffer = AudioBuffer {
            channels,
            sample_rate: info.sample_rate,
            num_frames,
        };

        self.audio_pool.insert(source_id.to_string(), buffer);

        Ok(info)
    }

    /// Click settings. The UI writes these; the audio thread reads them.
    pub fn metronome(&self) -> &crate::metronome::MetronomeSettings {
        &self.metronome
    }

    /// Browser auditions. Previewing through the engine means a sample comes
    /// out of the device the DAW is using, which the WebView's own audio
    /// could not do.
    pub fn preview(&self) -> &crate::preview_player::PreviewRequest {
        &self.preview
    }

    /// The Audio settings panel's behaviour switches.
    pub fn audio_prefs(&self) -> &crate::audio_prefs::AudioPrefs {
        &self.audio_prefs
    }

    /// Audio-thread load: percentage of each block's time budget in use, and
    /// how many blocks have overrun it this session.
    ///
    /// Replaces a toolbar meter that estimated load from WebView frame
    /// timing, which measured UI business rather than audio work.
    pub fn audio_load(&self) -> (f32, u32) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.audio_load_permille.load(Relaxed) as f32 / 10.0,
            self.audio_xruns.load(Relaxed),
        )
    }

    /// Remember where the current project file lives, so sample paths stored
    /// relative to it can be resolved. Set on save and on load.
    pub fn set_project_dir(&self, dir: Option<std::path::PathBuf>) {
        *self.project_dir.lock() = dir;
    }

    pub fn project_dir(&self) -> Option<std::path::PathBuf> {
        self.project_dir.lock().clone()
    }

    /// Absolute location of a clip's audio.
    ///
    /// A collected project stores its samples relative to the .hwp, so the
    /// whole folder can be moved or zipped and still open. An imported sample
    /// that lives elsewhere on disk keeps its absolute path.
    pub fn resolve_source_file(&self, file: &str) -> std::path::PathBuf {
        let path = std::path::Path::new(file);
        if path.is_absolute() {
            return path.to_path_buf();
        }
        match self.project_dir() {
            Some(dir) => dir.join(path),
            None => path.to_path_buf(),
        }
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
        // (pool id, file on disk). The id is what clips play through; the
        // file is where the audio has to be read from. They are different
        // strings, which is exactly what this used to get wrong: it fed the
        // pool id to the filesystem and tried to open a file named after a
        // hash, so every source "went missing" on reload.
        let paths: Vec<(String, String)> = {
            let project = self.project.lock();
            let mut set = std::collections::BTreeSet::new();
            let mut collect = |content: &ClipContent| {
                if let ClipContent::Audio(ac) = content {
                    // Projects written before `source_file` existed carry the
                    // path in `source_path` (the shape the engine's own tests
                    // build), so fall back to it rather than dropping them.
                    let file = if ac.source_file.is_empty() {
                        ac.source_path.clone()
                    } else {
                        ac.source_file.clone()
                    };
                    set.insert((ac.source_path.clone(), file));
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
        for (source_id, file) in paths {
            // Skip sources already resident (e.g. loading a project into a
            // session that imported the same file) — insert would clone-churn.
            if self.audio_pool.get(&source_id).is_some() {
                continue;
            }
            let resolved = self.resolve_source_file(&file);
            if let Err(e) = self.load_audio_file_as(&resolved, &source_id) {
                log::warn!("rehydrate_audio_pool: '{file}' failed to load: {e}");
                missing.push(file);
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

        // Bake stretch variants before rendering. The offline callback's
        // rebuild only looks them up, so without this an export would fall
        // back to varispeed and not match playback. Safe here — an export
        // runs on the caller's thread, not the audio callback.
        self.prebake_stretch_sources();

        // Harvest LIVE plug-in state so a bounce reflects the knobs the user
        // is actually hearing. `hydrate_offline_inserts` replays whatever is
        // stored on the project, and that is only ever written by
        // `save_project` — so without this an unsaved tweak renders stale, and
        // a slot added since the last save renders at plug-in DEFAULTS.
        //
        // Harvested before the project lock is taken: the audio thread has to
        // service the request, and it can't while we hold that lock. Returns
        // None when the engine isn't started (offline tests, headless renders),
        // in which case the stored state is still the best available.
        let live_plugin_states = self.snapshot_plugin_states(std::time::Duration::from_millis(500));

        let mut project_snapshot = self.project.lock().clone();
        if let Some(states) = live_plugin_states {
            for ((_track_id, slot_id), bytes) in states {
                project_snapshot.set_plugin_state(slot_id, "unknown", bytes);
            }
        }
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
            LoadCounters::new(),
            // A bounce is the music, not the guide track.
            crate::metronome::MetronomeSettings::silent(),
            // Its own request slot, never wired to the UI: an export cannot
            // pick up whatever someone is auditioning in the browser.
            crate::preview_player::PreviewRequest::new(),
            // A bounce should sound like playback, so it reads the same
            // switches the settings panel sets.
            self.audio_prefs.clone(),
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
    /// Shared with `DawEngine` so the UI can read real audio load. Written
    /// only here, on the audio thread.
    audio_load_permille: Arc<std::sync::atomic::AtomicU32>,
    audio_xruns: Arc<std::sync::atomic::AtomicU32>,
    /// Renders the click for this block. The offline renderer is built with
    /// permanently silent settings, so a bounce cannot contain it.
    metronome: crate::metronome::Metronome,
    /// Plays browser auditions. Like the click, mixed in after the tap, so an
    /// audition cannot end up in a recording or a bounce.
    preview: crate::preview_player::PreviewPlayer,
    /// The Audio settings panel's behaviour switches, shared with the engine
    /// so a change applies to the next block.
    audio_prefs: crate::audio_prefs::AudioPrefs,
    /// This block's clicks. A fixed-size array rather than a Vec because it
    /// is filled on the audio thread, which must not allocate.
    click_events: [crate::metronome::ClickEvent; crate::metronome::MAX_CLICKS_PER_BLOCK],
}

/// Audio-thread load counters shared with the UI: how much of each block's
/// budget was used, and how many blocks overran it.
#[derive(Clone)]
pub struct LoadCounters {
    pub load_permille: Arc<std::sync::atomic::AtomicU32>,
    pub xruns: Arc<std::sync::atomic::AtomicU32>,
}

impl LoadCounters {
    pub fn new() -> Self {
        Self {
            load_permille: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            xruns: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }
}

impl Default for LoadCounters {
    fn default() -> Self {
        Self::new()
    }
}

/// Share of a block's deadline that was spent working, in per mille.
///
/// The deadline is wall-clock: a 256-frame block at 48 kHz must be finished
/// within 5.33 ms or the device gets no audio and the user hears a click.
/// Over 1000 means the block missed it.
fn audio_load_reading(used_secs: f64, budget_secs: f64) -> u32 {
    if budget_secs <= 0.0 {
        return 0;
    }
    ((used_secs / budget_secs) * 1000.0)
        .round()
        .clamp(0.0, 10_000.0) as u32
}

/// Move the published value towards a new reading.
///
/// Rising fast and falling slowly on purpose: a climb towards the deadline is
/// what a producer needs to see before it starts clicking, while a dip is not
/// worth redrawing the bar for.
fn smooth_load(previous: u32, reading: u32) -> u32 {
    if reading > previous {
        (previous + (reading - previous) / 2).min(reading)
    } else {
        previous - (previous - reading) / 8
    }
}

impl EngineCallback {
    /// Publish how much of this block's budget the engine used.
    ///
    /// The budget is the wall-clock time the block represents: at 48 kHz a
    /// 256-frame block must be finished within 5.33 ms or the device gets no
    /// audio and the user hears a click. Spending 2.6 ms of that is 50%.
    ///
    /// Smoothed towards the new value so the meter is readable, except
    /// upwards past 100%, which is reported immediately: a spike that drops
    /// audio matters more than a tidy average. Overruns are counted
    /// separately because an average of 40% with regular xruns still
    /// crackles, and only the count reveals it.
    fn publish_audio_load(&self, started: std::time::Instant, num_frames: usize) {
        if num_frames == 0 || self.sample_rate == 0 {
            return;
        }
        let budget = num_frames as f64 / self.sample_rate as f64;
        let reading = audio_load_reading(started.elapsed().as_secs_f64(), budget);

        use std::sync::atomic::Ordering::Relaxed;
        if reading > 1000 {
            // Reported as-is, not smoothed: a spike that drops audio is
            // exactly what must not be averaged into a calm-looking number.
            self.audio_xruns.fetch_add(1, Relaxed);
            self.audio_load_permille.store(reading, Relaxed);
            return;
        }
        let previous = self.audio_load_permille.load(Relaxed);
        self.audio_load_permille
            .store(smooth_load(previous, reading), Relaxed);
    }

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
        load: LoadCounters,
        metronome: crate::metronome::MetronomeSettings,
        preview: crate::preview_player::PreviewRequest,
        audio_prefs: crate::audio_prefs::AudioPrefs,
    ) -> Self {
        let mut cb = Self {
            preview: crate::preview_player::PreviewPlayer::new(preview),
            audio_prefs,
            click_events: [crate::metronome::ClickEvent {
                frame_offset: 0,
                downbeat: false,
            }; crate::metronome::MAX_CLICKS_PER_BLOCK],
            audio_load_permille: load.load_permille,
            audio_xruns: load.xruns,
            metronome: crate::metronome::Metronome::new(metronome),
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

    /// Pool id of an already-baked pitch-preserving variant of `source_id`,
    /// or `None` if it hasn't been baked yet.
    ///
    /// LOOKUP ONLY — deliberately never bakes. `rebuild_graph` runs on the
    /// audio thread (see `AudioCallback::process`), and the bake is an FFT
    /// over the whole source plus multi-megabyte allocations; doing it here
    /// would stall the callback for as long as the file is big. Baking is
    /// `DawEngine::prebake_stretch_sources`, driven from the UI thread before
    /// the rebuild is signalled. A miss simply falls back to the varispeed
    /// resample path for this rebuild, and the next one picks the bake up.
    fn stretched_source(&self, source_id: &str, stretch: f64, semitones: f64) -> Option<String> {
        let key = stretch_cache_key(source_id, stretch, semitones);
        self.audio_pool.contains(&key).then_some(key)
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
        // (track_id, slot_id, plugin_id, enabled, wet, sidechained, saved_state)
        let mut hydrated_any = false;
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
                        s.sidechain_source.is_some(),
                        state,
                    ));
                }
            }
            acc
        };
        for (track_id, slot_id, plugin_id, enabled, wet, sidechained, state) in plan {
            let Some(node_id) = self.track_id_to_node.get(&track_id).copied() else {
                continue;
            };
            let Some(mut plugin) = instantiate(&plugin_id) else {
                continue;
            };
            if let Some(bytes) = state {
                let _ = plugin.set_state(&bytes);
            }
            hydrated_any = true;
            if let Some(node) = self.graph.node_mut(node_id) {
                let slot = crate::insert_chain::LiveSlot {
                    slot_id,
                    plugin,
                    enabled,
                    wet,
                    // Honour the slot's sidechain routing offline too. The
                    // graph edges that feed a source track into this node's
                    // input ports 2/3 are built by `rebuild_graph` from the
                    // same project metadata, so the bus is already present in
                    // an offline graph — only this per-slot flag was missing,
                    // which silently dropped ducking from every export while
                    // playback ducked correctly.
                    sidechain_active: sidechained,
                };
                node.push_offline_slot(slot, sr, buffer_size);
            }
        }

        // Recompute PDC now that plug-ins are on the graph. `rebuild_graph`
        // already ran `finalize_pdc` from `EngineCallback::new`, but that was
        // before any insert existed and `needs_rebuild` is already false, so
        // `process()` never runs it again — an export would otherwise align a
        // graph in which every node still claimed zero latency, while live
        // playback re-runs the rebuild (and therefore PDC) on every mutation.
        if hydrated_any {
            self.graph.finalize_pdc();
            self.graph_latency_samples.store(
                self.graph.total_latency_samples(),
                std::sync::atomic::Ordering::Relaxed,
            );
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
                if self.audio_prefs.reset_on_transport() {
                    self.reset_nodes();
                }
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
                // A jump leaves whatever was sounding at the old position
                // behind: held instrument voices carry on at the new one,
                // which is the "note stuck on after I moved the playhead"
                // report. The settings panel offers this as a switch and
                // nothing read it, so it never happened either way.
                if self.audio_prefs.reset_on_transport() {
                    self.reset_nodes();
                }
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
                midi_node.set_prefs(self.audio_prefs.clone());
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
                    // A muted clip contributes nothing, the same way a muted
                    // audio clip is skipped.
                    if midi_ref.clip.muted {
                        continue;
                    }
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
                        let clip_pitch_factor = 2.0_f64.powf(audio_clip.pitch_semitones / 12.0);
                        let pitch_factor = clip_pitch_factor * track_pitch_factor;
                        let stretch = if audio_clip.stretch_ratio <= 0.01 {
                            1.0
                        } else {
                            audio_clip.stretch_ratio
                        };
                        // Pitch-preserving stretch. A plain `source_step` resample
                        // can only change duration and pitch *together* (varispeed:
                        // stretch_ratio 2.0 also dropped an octave), so when a clip
                        // asks for either, bake a stretched / pitch-shifted copy of
                        // the source once and play it back at unity step. That makes
                        // stretch_ratio change duration only and pitch_semitones
                        // change pitch only, as their names promise.
                        //
                        // Warped clips keep the resample path — warp markers define
                        // their own piecewise source map and take precedence below.
                        // The track-level pitch/fine-tune offset stays a resample, so
                        // it remains a varispeed control.
                        let needs_bake = audio_clip.warp_markers.is_empty()
                            && ((stretch - 1.0).abs() > 1e-4
                                || audio_clip.pitch_semitones.abs() > 1e-4);
                        let baked = if needs_bake {
                            self.stretched_source(
                                &audio_clip.source_path,
                                stretch,
                                audio_clip.pitch_semitones,
                            )
                        } else {
                            None
                        };
                        // Baking stretches the whole source, so an in-source start
                        // offset scales with the same ratio.
                        let (region_source_id, region_source_offset, source_step) = match baked {
                            Some(key) => (
                                key,
                                (audio_clip.source_start as f64 * stretch) as u64,
                                track_pitch_factor,
                            ),
                            None => (
                                audio_clip.source_path.clone(),
                                audio_clip.source_start,
                                pitch_factor / stretch,
                            ),
                        };
                        // Rate-match the source to the rate we're rendering at.
                        // Pool buffers are normalised to the *device* rate on
                        // import, but an export renders at whatever rate the
                        // user picked — so bouncing at 44.1 k from a 48 k
                        // device read every audio clip 8.8% too slow (flat and
                        // long) while MIDI/synth tracks, which are generated at
                        // the render rate, stayed in tune. A mix would come out
                        // with its samples and its synths in different keys.
                        // Ratio is 1.0 during playback, where the two agree.
                        let source_step = match self.audio_pool.get(&region_source_id) {
                            Some(buf)
                                if buf.sample_rate != 0
                                    && (buf.sample_rate as f64 - sample_rate).abs()
                                        > f64::EPSILON =>
                            {
                                source_step * (buf.sample_rate as f64 / sample_rate)
                            }
                            _ => source_step,
                        };
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
                            source_id: region_source_id,
                            timeline_start,
                            timeline_end,
                            source_offset: region_source_offset,
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
        let master_track_id = project
            .tracks
            .iter()
            .find(|t| matches!(t.kind, hardwave_project::track::TrackKind::Master))
            .map(|t| t.id.clone());
        let master_node = MasterNode::new(
            Arc::clone(&self.transport.master_volume_db),
            master_track_id.clone(),
        );
        let master_id = self.graph.add_node(Box::new(master_node));
        self.master_id = Some(master_id);

        // Point the Master track's id at the master node so its insert chain is
        // reachable, and carry the live chain across the rebuild the same way
        // track chains are carried.
        //
        // Master isn't audio-bearing, so it gets no `TrackNode` and never
        // landed in this map — every `InsertCommand` aimed at the master strip
        // resolved to no node and was dropped, and `hydrate_offline_inserts`
        // skipped it on export too. A plug-in added to the master was persisted
        // in the project and then silently processed nothing, anywhere.
        if let Some(mid) = &master_track_id {
            track_id_to_node.insert(mid.clone(), master_id);
            if let Some(chain) = stashed_chains.remove(mid) {
                if let Some(node) = self.graph.node_mut(master_id) {
                    node.restore_chain(chain);
                }
            }
        }

        // Connect each track either to its configured output_bus (another
        // track) or to master. Invalid targets (self-routing, unknown id,
        // cycles) silently fall back to master — the command layer already
        // rejects bad values, but the engine is defensive to keep audio
        // flowing even if a legacy project carries stale routing.
        for track in &project.tracks {
            if !track.kind.is_audio_bearing() {
                continue;
            }
            // Stem renders exclude every track but the target from the mix.
            // The node still exists and still processes, so it can key a
            // sidechain — it just doesn't reach the output.
            if track.stem_excluded {
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
            // An excluded track's sends are dropped too, so a stem carries only
            // the target's own send tail rather than everyone else's.
            if track.stem_excluded {
                continue;
            }
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
    /// Reset every node in the graph.
    ///
    /// Clears held instrument voices and the sample-buffer caches, so nothing
    /// from before a stop or a jump is still sounding afterwards. Cheap and
    /// allocation-free: each node's reset only touches state it already owns.
    fn reset_nodes(&mut self) {
        for node in self.graph.iter_nodes_mut() {
            node.reset();
        }
    }

    /// Work out which beats fall inside this block, and fill `click_events`.
    ///
    /// Returns how many events were filled. The scheduling itself lives in
    /// `metronome::schedule_block_clicks`; this is the audio-thread wrapper
    /// around it: it decides whether the click is audible at all and takes the
    /// project without waiting for it.
    fn schedule_clicks(&mut self, num_frames: usize) -> usize {
        use std::sync::atomic::Ordering::Relaxed;
        let recording = self.transport.recording.load(Relaxed);
        if !self.metronome.settings().is_audible(recording) {
            return 0;
        }
        // try_lock, never lock: the UI holds the project for a moment when it
        // edits it, and waiting here would be a dropout. Losing a block's
        // clicks costs at most one beat.
        let Some(project) = self.project.try_lock() else {
            return 0;
        };
        crate::metronome::schedule_block_clicks(
            &project.tempo_map,
            self.transport.position_samples.load(Relaxed),
            num_frames,
            self.sample_rate as f64,
            &mut self.click_events,
        )
    }
}

impl AudioCallback for EngineCallback {
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        // Real audio load starts here and is published at the end of the
        // block. Instant::now does not allocate or lock, so it is safe on
        // this thread.
        let block_started = std::time::Instant::now();

        self.process_commands();

        // Count-in. While it runs the song is silent and the playhead does
        // not move: the count happens before the take, not over it. Playback
        // starts from here, on the sample the count ends, rather than from a
        // timeout in the UI that guessed when the clicks had finished.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let remaining = self.transport.count_in_remaining.load(Relaxed);
            if remaining > 0 {
                output[..num_frames * 2].fill(0.0);
                let total = self.transport.count_in_total.load(Relaxed);
                let elapsed = total.saturating_sub(remaining);
                let bpm = self.transport.bpm.load(Relaxed).max(1.0);
                let (beats_per_bar, den) =
                    crate::transport::unpack_time_sig(self.transport.time_sig.load(Relaxed));
                // A beat is a note value, so the denominator sets its length:
                // in 7/8 the count-in clicks eighths.
                let samples_per_beat =
                    60.0 / bpm * self.sample_rate as f64 * 4.0 / den.max(1) as f64;
                self.metronome.render_count_in(
                    output,
                    num_frames,
                    elapsed,
                    samples_per_beat,
                    beats_per_bar,
                    self.sample_rate as f64,
                );

                let left = remaining.saturating_sub(num_frames as u64);
                self.transport.count_in_remaining.store(left, Relaxed);
                if left == 0 {
                    self.transport.count_in_total.store(0, Relaxed);
                    if self.transport.count_in_then_play.swap(false, Relaxed) {
                        self.transport.playing.store(true, Relaxed);
                    }
                }
                self.publish_audio_load(block_started, num_frames);
                return;
            }
        }

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
                    // The signature followed the same way. Only the tempo was
                    // pushed here, so a signature change part-way through a
                    // song was stored in the project and then ignored: the
                    // click kept accenting the old bar length and plug-ins
                    // kept being told the old meter.
                    let (num, den) = project.tempo_map.time_sig_at(cur_tick);
                    self.transport.time_sig.store(
                        crate::transport::pack_time_sig(num, den),
                        std::sync::atomic::Ordering::Relaxed,
                    );
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

        // The click, last of all.
        //
        // After the master tap and the capture path on purpose: the tap feeds
        // the UI's meters and visualisations and the capture feeds recording,
        // so mixing the click in before them would print the guide track into
        // recordings and bounces. Each beat is placed on its own sample, from
        // the project's tempo map, so the click follows the song's tempo and
        // signature rather than one number that was right at bar 1.
        {
            use std::sync::atomic::Ordering::Relaxed;
            let recording = self.transport.recording.load(Relaxed);
            let events = if playing {
                self.schedule_clicks(num_frames)
            } else {
                // Not playing: no new beats, but a click already sounding is
                // allowed to finish rather than being cut off.
                0
            };
            self.metronome.render_scheduled(
                output,
                num_frames,
                &self.click_events[..events],
                recording,
                self.sample_rate as f64,
            );
        }

        // Browser auditions, alongside the click and for the same reason:
        // after the tap and the capture path, so listening to a sample in the
        // browser cannot print itself into a recording or a bounce.
        self.preview.render(
            output,
            num_frames,
            &self.audio_pool,
            self.sample_rate as f64,
        );

        // Advance transport only when playing — input-monitoring alone must
        // not move the playhead.
        if playing {
            self.transport.advance(num_frames as u64);
        }

        self.publish_audio_load(block_started, num_frames);
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
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
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

    /// The CPU meter is only worth having if the number is real, so these
    /// pin the maths it reports.
    #[test]
    fn load_is_the_share_of_the_block_deadline_that_was_used() {
        // 256 frames at 48 kHz is a 5.33 ms deadline.
        let budget = 256.0 / 48_000.0;
        assert_eq!(audio_load_reading(budget / 2.0, budget), 500, "half = 50%");
        assert_eq!(audio_load_reading(budget, budget), 1000, "all of it = 100%");
        assert_eq!(audio_load_reading(0.0, budget), 0);
        // A bigger buffer is a longer deadline, so the same work reads lower:
        // this is why raising the buffer size fixes crackle.
        let roomier = 1024.0 / 48_000.0;
        assert_eq!(audio_load_reading(budget / 2.0, roomier), 125);
        // Never divide by a deadline that does not exist.
        assert_eq!(audio_load_reading(0.01, 0.0), 0);
    }

    #[test]
    fn an_overrun_reads_above_one_hundred_percent_so_it_can_be_counted() {
        let budget = 256.0 / 48_000.0;
        assert!(audio_load_reading(budget * 2.0, budget) > 1000);
    }

    #[test]
    fn the_meter_climbs_quickly_and_settles_slowly() {
        // Rising: within two samples it is most of the way to a spike.
        let first = smooth_load(0, 800);
        let second = smooth_load(first, 800);
        assert!(second >= 600, "too slow to warn: {second}");
        // Falling: it eases down instead of snapping to zero.
        let after = smooth_load(800, 0);
        assert!(after > 600 && after < 800, "got {after}");
    }

    /// Regression for the shape the APP writes, which the test below never
    /// exercised: `import_audio_file` puts the pool id in `source_path` and
    /// the file in `source_file`. Rehydrate used to feed `source_path` to the
    /// filesystem, so it tried to open a file named after a hash and reported
    /// every source missing, leaving reopened projects silent on the very
    /// machine that made them.
    #[test]
    fn a_project_saved_the_way_the_app_writes_it_reloads_its_audio() {
        let dir = std::env::temp_dir().join(format!("hw-import-shape-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("snare.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav, spec).unwrap();
        for i in 0..4_800 {
            w.write_sample(((i as f32 / 8.0).sin() * 6_000.0) as i16)
                .unwrap();
        }
        w.finalize().unwrap();

        // Exactly what import does: load the file, then store the returned
        // pool id in source_path and the path in source_file.
        let engine = DawEngine::new();
        let (pool_id, _) = engine.load_audio_file(&wav).expect("import");
        {
            let mut project = engine.project.lock();
            let track = project.add_audio_track("Snare".into());
            if let Some(t) = project.track_mut(&track) {
                t.clips.push(ClipPlacement {
                    content: ClipContent::Audio(AudioClip {
                        id: "clip-1".into(),
                        name: "snare".into(),
                        source_path: pool_id.clone(),
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
                        source_file: wav.to_string_lossy().into_owned(),
                    }),
                    track_id: track.clone(),
                    position_ticks: 0,
                    length_ticks: 1920,
                    lane: 0,
                });
            }
            let path = dir.join("song.hwp");
            project.save(&path).expect("save");
        }

        // Reopen in a fresh engine, as a restart does.
        let reopened = DawEngine::new();
        {
            let loaded = Project::load(&dir.join("song.hwp")).expect("load");
            *reopened.project.lock() = loaded;
        }
        assert!(
            reopened.audio_pool.get(&pool_id).is_none(),
            "pool starts empty before rehydrate"
        );

        let missing = reopened.rehydrate_audio_pool();

        assert!(missing.is_empty(), "nothing should be missing: {missing:?}");
        // Under the clip's OWN id, which is what playback looks up. Loading it
        // under an id derived from the path would leave the clip silent.
        let buffer = reopened
            .audio_pool
            .get(&pool_id)
            .expect("audio is back under the id the clip references");
        assert_eq!(buffer.num_frames, 4_800);

        std::fs::remove_dir_all(&dir).unwrap();
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
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
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
        //
        // These clips carry the path in `source_path`, the pre-`source_file`
        // shape, and playback looks a buffer up by `source_path` verbatim. So
        // the pool key that matters here is the path itself. This used to
        // assert `source_id_for_path(path)` instead, a key playback never
        // reads: the audio was loaded under a name no clip could find, so even
        // this shape played silence.
        let source_id = path_str.clone();
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
                    // Legacy shape on purpose: empty means the engine falls back
                    // to reading source_path as the file location.
                    source_file: String::new(),
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
mod history_group_tests {
    use super::*;

    /// One drag of the paint tool places many clips, each through its own
    /// command. Without grouping that is one undo step per clip, so undoing
    /// the drag means pressing undo as many times as it placed.
    #[test]
    fn a_group_is_one_undo_step() {
        let engine = DawEngine::new();
        assert_eq!(engine.history_sizes().0, 0);

        engine.begin_history_group();
        for _ in 0..20 {
            engine.snapshot_before_mutation();
        }
        engine.end_history_group();

        assert_eq!(engine.history_sizes().0, 1, "twenty clips, one undo step");
    }

    #[test]
    fn mutations_outside_a_group_keep_their_own_steps() {
        let engine = DawEngine::new();
        engine.snapshot_before_mutation();
        engine.snapshot_before_mutation();
        assert_eq!(engine.history_sizes().0, 2);
    }

    #[test]
    fn a_second_group_is_its_own_step() {
        let engine = DawEngine::new();
        for _ in 0..2 {
            engine.begin_history_group();
            engine.snapshot_before_mutation();
            engine.snapshot_before_mutation();
            engine.end_history_group();
        }
        assert_eq!(engine.history_sizes().0, 2, "two drags, two undo steps");
    }

    #[test]
    fn a_repeated_begin_does_not_open_a_second_group() {
        let engine = DawEngine::new();
        engine.begin_history_group();
        engine.snapshot_before_mutation();
        // A stray second begin would otherwise reset the group and let the
        // next mutation snapshot again.
        engine.begin_history_group();
        engine.snapshot_before_mutation();
        engine.end_history_group();
        assert_eq!(engine.history_sizes().0, 1);
        assert!(!engine.history_group_open());
    }

    #[test]
    fn the_group_ends_even_if_nothing_was_placed() {
        let engine = DawEngine::new();
        engine.begin_history_group();
        engine.end_history_group();
        assert_eq!(engine.history_sizes().0, 0, "an empty drag is not history");
        engine.snapshot_before_mutation();
        assert_eq!(engine.history_sizes().0, 1);
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
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            // Offline and test callbacks get their own switches: a bounce
            // must not change because of what the settings panel says now.
            crate::audio_prefs::AudioPrefs::new(),
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
            LoadCounters::new(),
            crate::metronome::MetronomeSettings::silent(),
            crate::preview_player::PreviewRequest::new(),
            // Offline and test callbacks get their own switches: a bounce
            // must not change because of what the settings panel says now.
            crate::audio_prefs::AudioPrefs::new(),
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
