//! Running a plug-in in a process of its own.
//!
//! The loader already protects the moment a plug-in is added: it is
//! opened in a throwaway process first. Once loaded, though, it ran
//! inside the DAW, so a null pointer in its code was a null pointer in
//! ours and the window disappeared with whatever was unsaved.
//!
//! A sandboxed plug-in runs in a child process instead. The audio
//! thread never waits for it: it hands a block to a bridge thread and
//! takes back the block from before, which is one buffer of latency,
//! reported so delay compensation lines it up again. A child that dies
//! or misses its deadline is marked crashed, its slot goes quiet, and
//! the song keeps playing.
//!
//! Since third-party plug-ins run this way unless the producer says
//! otherwise, the bridge carries everything the chain gives a plug-in in
//! the DAW's own process: the sidechain, the plug-in's own delay, how its
//! values read, its presets. The delay it adds is held at exactly one
//! buffer whatever the child does: a block that comes back late is left
//! out, not played late, so the track never slides against the others.
//! An export waits for each block instead, and adds no delay at all.

use std::collections::VecDeque;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};

use hardwave_plugin_host::bridge_protocol::*;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginDescriptor, SharedEditLog, SharedParamQueue, TransportInfo,
};
use raw_window_handle::RawWindowHandle;

/// How long a child may take over one block before it is treated as
/// gone. Generous: a block at 256 samples is five milliseconds, and a
/// child that is merely slow should recover rather than be declared
/// dead.
const BLOCK_DEADLINE: std::time::Duration = std::time::Duration::from_millis(250);
/// How long one write to the child may take before it counts as gone. A
/// pipe that nobody reads fills and then blocks the writer for ever, which
/// the read deadlines above never see.
const WRITE_DEADLINE: std::time::Duration = std::time::Duration::from_millis(1500);
/// How long a plug-in may take to activate before it counts as hung.
const ACTIVATE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// How long one block may take in an export, where nothing is heard while
/// it renders and a heavy plug-in may take its time.
const OFFLINE_BLOCK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// How long a question to the plug-in (how a value reads, its presets) may
/// take. The asker is a window waiting to draw, never the audio thread.
const ASK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);
/// How long loading a state may take before the caller stops waiting for
/// the plug-in's new values. The state still arrives; only the wait ends.
const STATE_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);
/// What a wait that ran out says.
const STOPPED_ANSWERING: &str = "the plug-in stopped answering";
/// Messages waiting for the child at most; past this, blocks are dropped.
const TO_CHILD_DEPTH: usize = 8;

/// Every sandbox still in use, and every one whose process went until
/// that is shown, so the app can say which plug-in's process went and
/// when.
///
/// A list rather than a map: two slots can hold the same plug-in, and
/// which of them died matters less than the fact that one did.
static REGISTRY: Mutex<Vec<SandboxEntry>> = Mutex::new(Vec::new());

/// Forget the sandboxes nothing uses any more: the plug-in was removed,
/// or it was a copy made for a freeze or an export. One whose process
/// has been let go counts as unused whoever still holds a handle to it
/// (the list of plug-in windows keeps one until the slot is used again):
/// after a new song the old song's plug-ins stayed in the counters. Their entries were
/// kept for the whole run, each holding the plug-in's last state, and
/// showed in the counters as plug-ins that had stopped playing.
fn forget_finished(registry: &mut Vec<SandboxEntry>) {
    registry.retain(|entry| {
        entry.crashed.load(Ordering::Relaxed)
            || (Arc::strong_count(&entry.shared) > 1 && !entry.shared.ended.load(Ordering::Relaxed))
    });
}

#[derive(Clone)]
struct SandboxEntry {
    plugin_id: String,
    crashed: Arc<AtomicBool>,
    message: Arc<Mutex<Option<String>>>,
    shared: Arc<Shared>,
    /// The process, for support and for tests that end it on purpose.
    pid: u32,
}

/// Plug-ins whose process went during an export, bounce or freeze, by
/// name. The render plays silence where the plug-in was from then on, so
/// whoever asked for the file must be told rather than handed it.
static OFFLINE_CRASHES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Take the names of the plug-ins that stopped during a render since this
/// was last asked. Call it before a render (to forget older ones) and
/// after it (to learn whether the file can be trusted).
pub fn take_offline_crashes() -> Vec<String> {
    let mut list = match OFFLINE_CRASHES.lock() {
        Ok(l) => l,
        Err(poisoned) => poisoned.into_inner(),
    };
    std::mem::take(&mut *list)
}

fn note_offline_crash(name: &str) {
    let mut list = match OFFLINE_CRASHES.lock() {
        Ok(l) => l,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !list.iter().any(|n| n == name) {
        list.push(name.to_string());
    }
}

/// How a plug-in in its own process is driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    /// Playing live: the audio thread never waits, and the plug-in is one
    /// buffer behind (compensated).
    Live,
    /// Rendering to a file: each block is waited for, so nothing is left
    /// out however fast the render runs, and no delay is added.
    Offline,
}

/// What each plug-in in its own process has done with its blocks.
pub fn sandbox_stats() -> Vec<SandboxStats> {
    let mut registry = match REGISTRY.lock() {
        Ok(r) => r,
        Err(poisoned) => poisoned.into_inner(),
    };
    forget_finished(&mut registry);
    registry
        .iter()
        .map(|entry| {
            let s = &entry.shared.stats;
            let n = |a: &AtomicU64| a.load(Ordering::Relaxed);
            SandboxStats {
                plugin_id: entry.plugin_id.clone(),
                crashed: entry.crashed.load(Ordering::Relaxed),
                calls: n(&s.calls),
                sent: n(&s.sent),
                dropped: n(&s.dropped),
                no_buffer: n(&s.no_buffer),
                too_long: n(&s.too_long),
                back: n(&s.back),
                no_outputs: n(&s.no_outputs),
                late: n(&s.late),
                missed: n(&s.missed),
                plugin_latency: entry.shared.plugin_latency.load(Ordering::Relaxed),
                pid: entry.pid,
            }
        })
        .collect()
}

/// The plug-ins whose process has gone, with what it said on the way.
pub fn crashed_sandboxes() -> Vec<(String, String)> {
    let registry = match REGISTRY.lock() {
        Ok(r) => r,
        Err(poisoned) => poisoned.into_inner(),
    };
    registry
        .iter()
        .filter(|entry| entry.crashed.load(Ordering::Relaxed))
        .map(|entry| {
            let message = entry
                .message
                .lock()
                .ok()
                .and_then(|m| m.clone())
                .unwrap_or_else(|| "its process stopped".to_string());
            (entry.plugin_id.clone(), message)
        })
        .collect()
}

/// Bring the state the bridge holds for every plug-in in its own process
/// up to date, before it is read for a save, a preset, the parameter
/// sheet or a copy of the track.
///
/// The bridge never asks the plug-in's process while the audio waits, so
/// what it hands out is the state it last fetched: after a knob move that
/// is up to a second or two old, and a song saved right after a knob turn
/// lost that turn. This asks the processes whose knobs moved since the
/// last fetch for their state now and waits for it, `within` at most. With
/// nothing moved it returns at once. Never called on the audio thread.
pub fn settle_states(within: std::time::Duration) {
    let waiting: Vec<Arc<Shared>> = {
        let registry = match REGISTRY.lock() {
            Ok(r) => r,
            Err(poisoned) => poisoned.into_inner(),
        };
        registry
            .iter()
            .filter(|entry| !entry.crashed.load(Ordering::Relaxed))
            .map(|entry| Arc::clone(&entry.shared))
            .filter(|shared| {
                shared.state_stale.load(Ordering::Relaxed)
                    || shared.host_dirty.load(Ordering::Relaxed)
                    || shared.state_pending.load(Ordering::Relaxed)
                    || shared.editor_open.load(Ordering::Relaxed)
            })
            .collect()
    };
    if waiting.is_empty() {
        return;
    }
    for shared in &waiting {
        // Marked stale first, so the wait below does not end before the
        // bridge has picked the request up.
        shared.state_stale.store(true, Ordering::Relaxed);
        shared.fetch_now.store(true, Ordering::Relaxed);
    }
    let deadline = std::time::Instant::now() + within;
    while std::time::Instant::now() < deadline {
        let done = waiting.iter().all(|shared| {
            !(shared.state_stale.load(Ordering::Relaxed)
                || shared.state_pending.load(Ordering::Relaxed))
        });
        if done {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    log::warn!("a plug-in in its own process did not hand over its settings in time");
}

/// Forget the crashes already shown, so the same one is not reported
/// every time the app asks.
pub fn clear_crashed_sandboxes() {
    let mut registry = match REGISTRY.lock() {
        Ok(r) => r,
        Err(poisoned) => poisoned.into_inner(),
    };
    registry.retain(|entry| !entry.crashed.load(Ordering::Relaxed));
}

/// Which plug-ins run in a process of their own.
///
/// Third-party plug-ins do unless the producer says otherwise: a crash in
/// one of them used to close the DAW with whatever was unsaved. Hardwave's
/// own plug-ins and the built-in ones stay in the DAW's process, where
/// they add no delay and show their window inside the DAW's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choices {
    /// Run in a process of its own, whoever made it.
    pub always: std::collections::HashSet<String>,
    /// Run inside the DAW although it is a third-party plug-in: the
    /// producer's choice per plug-in, for one that needs no delay at all.
    pub never: std::collections::HashSet<String>,
    /// Third-party plug-ins run inside the DAW unless listed in `always`
    /// (the behaviour before version 0.266.49). No switch on screen: a
    /// line in the settings file, for support.
    pub third_party_inside: bool,
}

/// Where a plug-in runs, and whether that can be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// Inside the DAW. `fixed`: built in, there is nothing to move out.
    Inside { fixed: bool },
    /// In a process of its own. `fixed`: built for another kind of
    /// processor, so the DAW's own process cannot load it at all.
    Own { fixed: bool },
}

impl Home {
    pub fn is_own(self) -> bool {
        matches!(self, Home::Own { .. })
    }
}

/// Whether Hardwave made this plug-in, by the vendor it names. The same
/// test the plug-in picker uses. Not a security check (a plug-in can name
/// any vendor): it only decides where a crash would land.
pub fn is_hardwave(descriptor: &PluginDescriptor) -> bool {
    let vendor = descriptor.vendor.trim().to_ascii_lowercase();
    vendor == "hardwave" || vendor == "hardwave studios"
}

impl Choices {
    /// Where this plug-in runs. `foreign`: it is built for another kind of
    /// processor than this program.
    pub fn home(&self, descriptor: &PluginDescriptor, foreign: bool) -> Home {
        if descriptor.path == Path::new("<native>") {
            return Home::Inside { fixed: true };
        }
        if foreign {
            return Home::Own { fixed: true };
        }
        if self.always.contains(&descriptor.id) {
            return Home::Own { fixed: false };
        }
        if self.never.contains(&descriptor.id) || self.third_party_inside || is_hardwave(descriptor)
        {
            return Home::Inside { fixed: false };
        }
        Home::Own { fixed: false }
    }

    /// The producer's choice for one plug-in. Choosing what it would do
    /// anyway leaves no entry behind, so a later change of the default
    /// still reaches it.
    pub fn choose(&mut self, descriptor: &PluginDescriptor, own: bool) {
        self.always.remove(&descriptor.id);
        self.never.remove(&descriptor.id);
        let by_default = self.home(descriptor, false).is_own();
        if own && !by_default {
            self.always.insert(descriptor.id.clone());
        } else if !own && by_default {
            self.never.insert(descriptor.id.clone());
        }
    }
}

/// How many blocks are made for the trip to the child and back. They go
/// round and round, so the audio thread never allocates: one in the
/// child, one on its way back, a few queued, one being played.
const POOL_BLOCKS: usize = TO_CHILD_DEPTH + 4;
/// The longest block a pooled buffer holds without growing.
const POOL_FRAMES: usize = 8192;
/// How long a freshly started plug-in may take to load and list its
/// parameters. Big sample libraries take their time.
pub const PARAMS_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the bridge waits for work before it looks whether the state
/// is wanted. Short, so a save does not wait on an idle plug-in.
const BRIDGE_TURN: std::time::Duration = std::time::Duration::from_millis(50);
/// How often the plug-in's state is fetched while its window is open or
/// its knobs have moved, so a save has what the person did in there.
const STATE_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// One block's audio, notes and transport, made once and passed between
/// the audio thread and the bridge.
struct Block {
    left: Vec<f32>,
    right: Vec<f32>,
    /// The sidechain, when the slot has one routed (`keyed`).
    key_left: Vec<f32>,
    key_right: Vec<f32>,
    keyed: bool,
    midi: Vec<hardwave_midi::MidiEvent>,
    transport: TransportInfo,
    /// Where this block starts, in samples since the plug-in began to
    /// play. What came back is played exactly one buffer after this.
    start: u64,
}

impl Block {
    fn new() -> Self {
        Self {
            left: Vec::with_capacity(POOL_FRAMES),
            right: Vec::with_capacity(POOL_FRAMES),
            key_left: Vec::with_capacity(POOL_FRAMES),
            key_right: Vec::with_capacity(POOL_FRAMES),
            keyed: false,
            start: 0,
            midi: Vec::with_capacity(MAX_BLOCK_EVENTS),
            transport: TransportInfo::default(),
        }
    }
}

/// A plug-in process's life as the bridge sees it, in a log of its own
/// (logs/plugin-<id>-host.log): started, activated, first audio back, and
/// how it ended. Lifecycle only, a handful of lines, for support and for
/// telling a crash that was noticed from one that was not.
#[derive(Clone, Default)]
struct Trace(Option<Arc<Mutex<std::fs::File>>>);

/// Most a plug-in's logs grow to before the next start begins them again.
const LOG_LIMIT: u64 = 256 * 1024;

/// A plug-in's log, added to at each start rather than begun again: a
/// restart after a crash used to wipe the lines that said how it crashed.
/// Begun again once it passes [`LOG_LIMIT`].
fn plugin_log(file_name: &str) -> Option<std::fs::File> {
    let dir = crate::diagnostics::logs_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(file_name);
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT);
    std::fs::OpenOptions::new()
        .create(true)
        .append(!too_big)
        .write(true)
        .truncate(too_big)
        .open(path)
        .ok()
}

impl Trace {
    fn open(name: &str) -> Self {
        let file = plugin_log(&format!("plugin-{name}-host.log"));
        Trace(file.map(|f| Arc::new(Mutex::new(f))))
    }

    fn line(&self, what: &str) {
        use std::io::Write;
        if let Some(file) = &self.0 {
            if let Ok(mut f) = file.lock() {
                let _ = writeln!(
                    f,
                    "{} {what}",
                    jiff::ZonedDateTime::now().strftime("%H:%M:%S%.3f")
                );
            }
        }
    }
}

/// What the bridge sends to the worker thread.
enum ToChild {
    Block(Block),
    Param(u32, f64),
    /// Its own message rather than a parameter with a reserved id: a
    /// plug-in whose parameter happened to have that id re-activated
    /// itself at a sample rate of whatever the knob said.
    Activate(f64, u32),
    State(Vec<u8>),
    Params,
    /// A question, and where its answer goes. Never from the audio thread.
    Ask(Ask, SyncSender<Answer>),
    LoadPreset(u32),
    OpenEditor,
    CloseEditor,
    Shutdown,
}

/// What comes back.
enum FromChild {
    Block(Block),
    Crashed(String),
}

/// What the child tells the host outside the block-by-block exchange,
/// kept where the app's threads can read it.
#[derive(Default)]
struct Shared {
    /// The parameter list, once the child has sent it.
    params: parking_lot::Mutex<Option<Vec<ParameterInfo>>>,
    params_ready: parking_lot::Condvar,
    /// Where each knob stands, as the host set it or the plug-in's own
    /// window moved it.
    values: parking_lot::Mutex<Vec<(u32, f64)>>,
    /// Knob moves in the plug-in's window, for the app to record.
    gui_log: SharedEditLog,
    /// The plug-in's state, as last fetched or set.
    state: parking_lot::Mutex<Vec<u8>>,
    /// Its window is open, or its knobs moved since the state was fetched.
    editor_open: AtomicBool,
    state_stale: AtomicBool,
    /// The host moved a knob (the parameter sheet, automation). The state
    /// is fetched once the moves have stopped for a moment, so a save, or
    /// a restart after a crash, has them.
    host_moved: AtomicBool,
    /// A knob the host moved is not in the fetched state yet.
    host_dirty: AtomicBool,
    /// Someone is about to read the state (a save, the parameter sheet):
    /// fetch it now rather than at the next turn (`settle_states`).
    fetch_now: AtomicBool,
    /// The state has been asked for and has not come back yet.
    state_pending: AtomicBool,
    /// The delay the plug-in itself adds, as its process last said.
    plugin_latency: AtomicU32,
    /// A VST3 plug-in's class id, which names it in a `.vstpreset`.
    class_id: parking_lot::Mutex<Option<[u8; 16]>>,
    /// Counts the parameter lists the child has sent, so a caller that
    /// loaded a state can wait for the values that came of it.
    values_epoch: AtomicU64,
    /// An export is driving it: a block may take as long as it takes.
    patient: AtomicBool,
    /// The bridge has ended and the process has been let go.
    ended: AtomicBool,
    /// What the audio thread did with this plug-in, counted, for telling
    /// where blocks stop when a plug-in in its own process plays nothing.
    stats: BlockStats,
}

/// Counters the audio thread keeps for one plug-in in its own process.
#[derive(Default)]
struct BlockStats {
    /// `process` calls.
    calls: AtomicU64,
    /// Blocks handed to the bridge.
    sent: AtomicU64,
    /// Blocks not sent: the bridge's queue was full (the child behind).
    dropped: AtomicU64,
    /// Blocks not sent: no free buffer in the pool.
    no_buffer: AtomicU64,
    /// Blocks not sent: longer than a pooled buffer holds.
    too_long: AtomicU64,
    /// Blocks played back from the child.
    back: AtomicU64,
    /// Calls with fewer than two outputs, which play nothing.
    no_outputs: AtomicU64,
    /// Blocks that came back too late to be played in their place, and
    /// were left out so the track keeps its timing.
    late: AtomicU64,
    /// Calls that played a gap because the plug-in's audio for that moment
    /// had not come back (or was never sent).
    missed: AtomicU64,
}

/// One plug-in in its own process, as `sandbox_stats` reports it.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStats {
    pub plugin_id: String,
    pub crashed: bool,
    pub calls: u64,
    pub sent: u64,
    pub dropped: u64,
    pub no_buffer: u64,
    pub too_long: u64,
    pub back: u64,
    pub no_outputs: u64,
    pub late: u64,
    pub missed: u64,
    /// The delay the plug-in itself reports, in samples.
    pub plugin_latency: u32,
    pub pid: u32,
}

impl Shared {
    fn set_value(&self, id: u32, value: f64) {
        let mut values = self.values.lock();
        match values.iter_mut().find(|(pid, _)| *pid == id) {
            Some(slot) => slot.1 = value,
            None => values.push((id, value)),
        }
    }
}

/// A plug-in that lives in another process, pretending to be an
/// ordinary one.
pub struct SandboxedPlugin {
    descriptor: PluginDescriptor,
    /// Bounded: a child that stops keeping up has blocks dropped rather
    /// than piling up in memory behind it.
    to_child: SyncSender<ToChild>,
    from_child: Receiver<FromChild>,
    /// Empty blocks, ready for the next trip.
    free: Receiver<Block>,
    free_return: SyncSender<Block>,
    /// Blocks that came back and have not been played yet. One deep in
    /// steady state: that is the latency this costs.
    pending: VecDeque<Block>,
    /// Samples handed to `process` so far: where the next block starts.
    clock: u64,
    /// How far behind the plug-in's audio is played, in samples: one
    /// buffer of the size it was activated with. Fixed, so compensation
    /// holds: late audio is left out rather than played late.
    delay: u32,
    pace: Pace,
    transport: TransportInfo,
    crashed: Arc<AtomicBool>,
    crash_message: Arc<Mutex<Option<String>>>,
    block_size: Arc<AtomicU32>,
    shared: Arc<Shared>,
    /// The parameter list, copied here once it is known, so the audio
    /// thread reads it without a lock.
    params: Vec<ParameterInfo>,
    /// Parameters as the host set them, for when the shared table is busy.
    param_cache: Vec<(u32, f64)>,
    state_cache: Vec<u8>,
    child_pid: u32,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SandboxedPlugin {
    /// Start a child for this plug-in, playing live.
    #[cfg(test)]
    pub fn start(descriptor: PluginDescriptor, exe: &Path) -> Result<Self, String> {
        Self::start_paced(descriptor, exe, Pace::Live)
    }

    /// Start a child for this plug-in, to play live or to render.
    ///
    /// `exe` is this program: the child is the same binary with a
    /// switch, so there is nothing else to install or keep in step.
    pub fn start_paced(
        descriptor: PluginDescriptor,
        exe: &Path,
        pace: Pace,
    ) -> Result<Self, String> {
        // A plug-in built for another architecture cannot be loaded by
        // this process at all, whatever we do with threads, so it goes
        // to the helper built for that architecture instead. This is
        // what makes a 32-bit plug-in from 2008 run at all.
        let (program, first_arg) = match helper_for(&descriptor) {
            Some(helper) => (helper, None),
            None => (exe.to_path_buf(), Some("--host-plugin")),
        };
        let mut command = Command::new(program);
        if let Some(flag) = first_arg {
            command.arg(flag);
        }
        // What the child says on its way (it cannot load the plug-in, its
        // window would not open) goes to a log of its own, for support.
        let log_name: String = descriptor
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .take(80)
            .collect();
        let trace = Trace::open(&log_name);
        let stderr = plugin_log(&format!("plugin-{log_name}.log"))
            .map(Stdio::from)
            .unwrap_or_else(Stdio::null);
        let child = command
            .arg(&descriptor.path)
            .arg(&descriptor.id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(|e| format!("could not start the plug-in process: {e}"))?;
        let child_pid = child.id();
        trace.line(&format!(
            "started process {child_pid} for {}",
            descriptor.id
        ));

        let (to_tx, to_rx) = mpsc::sync_channel::<ToChild>(TO_CHILD_DEPTH);
        // Bounded too, so neither end of the audio thread's exchange
        // allocates: an unbounded channel grows in blocks, and the audio
        // thread would free them.
        let (from_tx, from_rx) = mpsc::sync_channel::<FromChild>(POOL_BLOCKS + 2);
        let (free_tx, free_rx) = mpsc::sync_channel::<Block>(POOL_BLOCKS);
        for _ in 0..POOL_BLOCKS {
            let _ = free_tx.try_send(Block::new());
        }
        let crashed = Arc::new(AtomicBool::new(false));
        let crash_message = Arc::new(Mutex::new(None));
        let shared = Arc::new(Shared::default());
        shared
            .patient
            .store(pace == Pace::Offline, Ordering::Relaxed);
        let worker = {
            let crashed = Arc::clone(&crashed);
            let crash_message = Arc::clone(&crash_message);
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("hardwave-plugin-bridge".into())
                .spawn(move || {
                    bridge_loop(child, to_rx, from_tx, crashed, crash_message, shared, trace)
                })
                .map_err(|e| format!("could not start the bridge thread: {e}"))?
        };
        // The list is asked for at once: the child answers when the
        // plug-in has loaded (see `wait_for_params`).
        let _ = to_tx.try_send(ToChild::Params);

        {
            let mut registry = match REGISTRY.lock() {
                Ok(r) => r,
                Err(poisoned) => poisoned.into_inner(),
            };
            forget_finished(&mut registry);
            registry.push(SandboxEntry {
                plugin_id: descriptor.id.clone(),
                crashed: Arc::clone(&crashed),
                message: Arc::clone(&crash_message),
                shared: Arc::clone(&shared),
                pid: child_pid,
            });
        }

        Ok(Self {
            descriptor,
            to_child: to_tx,
            from_child: from_rx,
            free: free_rx,
            free_return: free_tx,
            pending: VecDeque::with_capacity(POOL_BLOCKS + 2),
            clock: 0,
            delay: 0,
            pace,
            transport: TransportInfo::default(),
            crashed,
            crash_message,
            block_size: Arc::new(AtomicU32::new(0)),
            shared,
            params: Vec::new(),
            param_cache: Vec::new(),
            state_cache: Vec::new(),
            child_pid,
            worker: Some(worker),
        })
    }

    /// Wait for the child to load the plug-in and list its parameters,
    /// and keep the list. Off the audio thread, before the plug-in goes
    /// into a chain: the chain reads the list when it takes the slot.
    /// False when the list did not come in time (the plug-in still plays,
    /// with no parameters to automate).
    pub fn wait_for_params(&mut self, within: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + within;
        let mut list = self.shared.params.lock();
        while list.is_none() && !self.crashed.load(Ordering::Relaxed) {
            if self
                .shared
                .params_ready
                .wait_until(&mut list, deadline)
                .timed_out()
            {
                break;
            }
        }
        match list.as_ref() {
            Some(params) => {
                self.params = params.clone();
                true
            }
            None => false,
        }
    }

    /// Whether the child has gone, and what it said on the way out.
    ///
    /// The app reads crashes from the registry, which covers every
    /// sandbox at once; this is the same answer for one of them.
    #[allow(dead_code)]
    pub fn crash_message(&self) -> Option<String> {
        if !self.crashed.load(Ordering::Relaxed) {
            return None;
        }
        self.crash_message
            .lock()
            .ok()
            .and_then(|m| m.clone())
            .or_else(|| Some("the plug-in's process stopped".to_string()))
    }

    #[allow(dead_code)]
    pub fn has_crashed(&self) -> bool {
        self.crashed.load(Ordering::Relaxed)
    }
}

impl Drop for SandboxedPlugin {
    fn drop(&mut self) {
        // try_send: the queue may be full of a wedged child's blocks, and
        // dropping the sender ends the bridge thread (which kills the
        // child) whether or not this arrives.
        let _ = self.to_child.try_send(ToChild::Shutdown);
        // The thread is not waited for. It owns everything it touches,
        // and a child that has wedged can leave a grandchild holding
        // the pipe open, so a read can outlive the kill. Removing a
        // plug-in must not hang the app while that resolves.
        self.worker.take();
    }
}

/// The plug-in's own window, shown by its process. Not inside the DAW's
/// plug-in window: a window parented across processes ties their input
/// together, and a plug-in that hangs would hang the DAW's window too.
struct SandboxEditor {
    to_child: SyncSender<ToChild>,
    child_pid: u32,
    shared: Arc<Shared>,
}

impl hardwave_plugin_host::types::PluginEditor for SandboxEditor {
    fn open(&self, _parent: RawWindowHandle) -> bool {
        self.open_own_window().unwrap_or(false)
    }

    fn size(&self) -> Option<(u32, u32)> {
        None
    }

    fn close(&self) {
        let _ = self.to_child.try_send(ToChild::CloseEditor);
    }

    fn open_own_window(&self) -> Option<bool> {
        // A window from a background process is not allowed in front on
        // its own; the app, which the person just clicked, lets it.
        #[cfg(windows)]
        // Safety: a plain call with a process id.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(self.child_pid);
        }
        #[cfg(not(windows))]
        let _ = self.child_pid;
        self.shared.state_stale.store(true, Ordering::Relaxed);
        Some(self.to_child.try_send(ToChild::OpenEditor).is_ok())
    }
}

impl SandboxedPlugin {
    /// The child has gone: remember why, and say so if this was a render.
    fn mark_crashed(&mut self, message: Option<String>) {
        self.crashed.store(true, Ordering::Relaxed);
        if let Some(message) = message {
            if let Ok(mut slot) = self.crash_message.lock() {
                slot.get_or_insert(message);
            }
        }
        if self.pace == Pace::Offline {
            note_offline_crash(&self.descriptor.name);
        }
    }

    /// Take whatever came back from the child, and play the part of it
    /// that belongs to this call: what the plug-in made of the audio
    /// handed over exactly `delay` samples before `start`.
    ///
    /// Playing "the oldest block that came back" instead let one late
    /// block push everything after it a buffer further behind, for good,
    /// with nothing compensating for it; and a call shorter than the last
    /// (a loop point, a driver that varies its block) cut blocks short.
    fn play_back(&mut self, outputs: &mut [Vec<f32>], num_samples: usize, start: u64) {
        loop {
            match self.from_child.try_recv() {
                Ok(FromChild::Block(block)) => self.pending.push_back(block),
                Ok(FromChild::Crashed(message)) => {
                    self.mark_crashed(Some(message));
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.mark_crashed(None);
                    return;
                }
            }
        }
        let want_start = start as i64 - i64::from(self.delay);
        let want_end = want_start + num_samples as i64;
        // Before the plug-in's first audio is due there is nothing to miss.
        let due = (want_end - want_start.max(0)).max(0) as usize;
        let mut played = 0usize;
        while let Some(block) = self.pending.front() {
            let block_start = block.start as i64;
            let block_end = block_start + block.left.len().min(block.right.len()) as i64;
            if block_end <= want_start {
                // Its moment has passed: left out, so what follows stays
                // where it belongs.
                self.shared.stats.late.fetch_add(1, Ordering::Relaxed);
                if let Some(block) = self.pending.pop_front() {
                    let _ = self.free_return.try_send(block);
                }
                continue;
            }
            if block_start >= want_end {
                break;
            }
            let from = block_start.max(want_start);
            let to = block_end.min(want_end);
            let src = (from - block_start) as usize..(to - block_start) as usize;
            let dst = (from - want_start) as usize..(to - want_start) as usize;
            outputs[0][dst.clone()].copy_from_slice(&block.left[src.clone()]);
            outputs[1][dst].copy_from_slice(&block.right[src]);
            played += (to - from) as usize;
            if block_end > want_end {
                // The rest of it belongs to the next call.
                break;
            }
            if let Some(block) = self.pending.pop_front() {
                let _ = self.free_return.try_send(block);
            }
            self.shared.stats.back.fetch_add(1, Ordering::Relaxed);
        }
        if played < due {
            self.shared.stats.missed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// In a render: wait for the block that was just handed over and play
    /// it now. Nothing is heard while a file renders, so the wait costs
    /// nothing, and the plug-in adds no delay and loses no block.
    fn play_now(&mut self, outputs: &mut [Vec<f32>], num_samples: usize, start: u64) {
        let deadline = std::time::Instant::now() + OFFLINE_BLOCK_DEADLINE;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match self.from_child.recv_timeout(left) {
                Ok(FromChild::Block(block)) => {
                    let mine = block.start == start;
                    if mine {
                        let n = num_samples.min(block.left.len()).min(block.right.len());
                        outputs[0][..n].copy_from_slice(&block.left[..n]);
                        outputs[1][..n].copy_from_slice(&block.right[..n]);
                        self.shared.stats.back.fetch_add(1, Ordering::Relaxed);
                    }
                    let _ = self.free_return.try_send(block);
                    if mine {
                        return;
                    }
                }
                Ok(FromChild::Crashed(message)) => {
                    self.mark_crashed(Some(message));
                    return;
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.mark_crashed(Some("the plug-in stopped answering".into()));
                    return;
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.mark_crashed(None);
                    return;
                }
            }
        }
    }

    /// Ask the plug-in's process something and wait a moment for the
    /// answer. Not for the audio thread.
    fn ask(&self, ask: Ask) -> Option<Answer> {
        if self.crashed.load(Ordering::Relaxed) {
            return None;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.to_child.try_send(ToChild::Ask(ask, tx)).ok()?;
        rx.recv_timeout(ASK_DEADLINE + std::time::Duration::from_millis(250))
            .ok()
    }
}

impl HostedPlugin for SandboxedPlugin {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, max_block: u32) -> Result<(), String> {
        self.block_size.store(max_block, Ordering::Relaxed);
        self.delay = max_block;
        self.to_child
            .try_send(ToChild::Activate(sample_rate, max_block))
            .map_err(|_| "the plug-in's process is not answering".to_string())?;
        Ok(())
    }

    fn deactivate(&mut self) {}

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        let stats = &self.shared.stats;
        stats.calls.fetch_add(1, Ordering::Relaxed);
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        if outputs.len() < 2 {
            stats.no_outputs.fetch_add(1, Ordering::Relaxed);
            return;
        }
        // An instrument has no input: it plays silence through.
        let silent: &[f32] = &[];
        let left = inputs.first().copied().unwrap_or(silent);
        let right = inputs.get(1).copied().unwrap_or(left);
        // A slot with a sidechain routed is given four channels.
        let key = match (inputs.get(2), inputs.get(3)) {
            (Some(l), Some(r)) => Some((*l, *r)),
            _ => None,
        };

        // A crashed child plays nothing. Passing the dry signal through
        // instead would hide the fact that a plug-in is missing from
        // the chain, which is worse than hearing the gap.
        if self.crashed.load(Ordering::Relaxed) {
            return;
        }

        // Never activated (a test, a copy loaded for a look): one call's
        // worth behind, as it always was.
        if self.delay == 0 {
            self.delay = num_samples.min(u32::MAX as usize) as u32;
        }
        let start = self.clock;
        self.clock += num_samples as u64;

        // Hand this block over and take whatever came back. The audio
        // thread never waits on the child: that is the one block of
        // latency this costs, and it is reported below. Nor does it
        // allocate: the block is one of the pool's, and a block longer
        // than a pooled buffer is not sent rather than grown.
        if num_samples > POOL_FRAMES {
            stats.too_long.fetch_add(1, Ordering::Relaxed);
        } else {
            let Ok(mut block) = self.free.try_recv() else {
                stats.no_buffer.fetch_add(1, Ordering::Relaxed);
                return self.play_back(outputs, num_samples, start);
            };
            {
                let fill = |into: &mut Vec<f32>, from: &[f32]| {
                    into.clear();
                    let n = num_samples.min(from.len());
                    into.extend_from_slice(&from[..n]);
                    into.resize(num_samples, 0.0);
                };
                fill(&mut block.left, left);
                fill(&mut block.right, right);
                block.keyed = key.is_some();
                if let Some((key_left, key_right)) = key {
                    fill(&mut block.key_left, key_left);
                    fill(&mut block.key_right, key_right);
                }
                block.midi.clear();
                let events = midi_in.len().min(block.midi.capacity());
                block.midi.extend_from_slice(&midi_in[..events]);
                block.transport = self.transport;
                block.start = start;
                // A full queue means the child is behind: the block is
                // dropped (back to the pool) rather than queued without end.
                // A render waits for room instead: it has all the time.
                let sent = match self.pace {
                    Pace::Live => self.to_child.try_send(ToChild::Block(block)),
                    Pace::Offline => self
                        .to_child
                        .send(ToChild::Block(block))
                        .map_err(|e| mpsc::TrySendError::Disconnected(e.0)),
                };
                match sent {
                    Ok(()) => {
                        stats.sent.fetch_add(1, Ordering::Relaxed);
                        if self.pace == Pace::Offline {
                            return self.play_now(outputs, num_samples, start);
                        }
                    }
                    Err(mpsc::TrySendError::Full(ToChild::Block(block)))
                    | Err(mpsc::TrySendError::Disconnected(ToChild::Block(block))) => {
                        stats.dropped.fetch_add(1, Ordering::Relaxed);
                        let _ = self.free_return.try_send(block);
                    }
                    Err(_) => {}
                }
            }
        }
        self.play_back(outputs, num_samples, start);
    }

    fn get_parameter_count(&self) -> u32 {
        self.params.len() as u32
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        self.params.get(index as usize).cloned()
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        // The shared table has the plug-in's own window's moves too.
        if let Some(values) = self.shared.values.try_lock() {
            if let Some((_, v)) = values.iter().find(|(pid, _)| *pid == id) {
                return *v;
            }
        }
        self.param_cache
            .iter()
            .find(|(pid, _)| *pid == id)
            .map(|(_, v)| *v)
            .unwrap_or(0.0)
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        match self.param_cache.iter_mut().find(|(pid, _)| *pid == id) {
            Some(slot) => slot.1 = value,
            None => self.param_cache.push((id, value)),
        }
        if let Some(mut values) = self.shared.values.try_lock() {
            if let Some(slot) = values.iter_mut().find(|(pid, _)| *pid == id) {
                slot.1 = value;
            }
        }
        self.shared.host_moved.store(true, Ordering::Relaxed);
        self.shared.host_dirty.store(true, Ordering::Relaxed);
        let _ = self.to_child.try_send(ToChild::Param(id, value));
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        match self.ask(Ask::Text(id, value))? {
            Answer::Text(text) => text,
            _ => None,
        }
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match self.ask(Ask::Options(id))? {
            Answer::Options(options) => options,
            _ => None,
        }
    }

    fn factory_presets(&self) -> Vec<String> {
        match self.ask(Ask::Presets) {
            Some(Answer::Presets(names)) => names,
            _ => Vec::new(),
        }
    }

    fn load_factory_preset(&mut self, index: usize) -> Result<(), String> {
        // The chain calls this on the audio thread: handed over, not
        // waited for. The plug-in's new values and state follow by
        // themselves.
        self.shared.state_stale.store(true, Ordering::Relaxed);
        self.to_child
            .try_send(ToChild::LoadPreset(index.min(u32::MAX as usize) as u32))
            .map_err(|_| "the plug-in's process is not answering".to_string())
    }

    fn vst3_class_id(&self) -> Option<[u8; 16]> {
        *self.shared.class_id.lock()
    }

    fn get_state(&self) -> Vec<u8> {
        // What the child last sent, or the host last set. Asking the
        // child now would mean waiting for it on whichever thread called
        // this, and a saved project that stalls on a sick plug-in is
        // worse than one that saves the state from a second ago.
        match self.shared.state.try_lock() {
            Some(state) if !state.is_empty() => state.clone(),
            _ => self.state_cache.clone(),
        }
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.state_cache = bytes.to_vec();
        if let Some(mut state) = self.shared.state.try_lock() {
            *state = bytes.to_vec();
        }
        let before = self.shared.values_epoch.load(Ordering::Relaxed);
        self.to_child
            .try_send(ToChild::State(bytes.to_vec()))
            .map_err(|_| "the plug-in's process is not answering".to_string())?;
        // Before it plays (a slot being loaded, a copy made for the
        // parameter sheet) the caller is not the audio thread and reads
        // the values next: wait for the ones this state gives, or it reads
        // the plug-in's defaults. Once it plays, nothing waits.
        if self.block_size.load(Ordering::Relaxed) == 0 {
            let deadline = std::time::Instant::now() + STATE_SETTLE;
            let mut list = self.shared.params.lock();
            while self.shared.values_epoch.load(Ordering::Relaxed) == before
                && !self.crashed.load(Ordering::Relaxed)
            {
                if self
                    .shared
                    .params_ready
                    .wait_until(&mut list, deadline)
                    .timed_out()
                {
                    break;
                }
            }
        }
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        // The plug-in's own delay, plus exactly one buffer when it plays
        // live: the audio handed over is played back one buffer later.
        // Reported so delay compensation lines this track up with the
        // rest. A render waits for each block and adds nothing.
        let own = self.shared.plugin_latency.load(Ordering::Relaxed);
        match self.pace {
            Pace::Live => own.saturating_add(self.block_size.load(Ordering::Relaxed)),
            Pace::Offline => own,
        }
    }

    fn open_editor(&mut self, _parent: RawWindowHandle) -> bool {
        false
    }

    fn close_editor(&mut self) {}

    fn has_editor(&self) -> bool {
        self.descriptor.has_editor
    }

    fn editor(&self) -> Option<Arc<dyn hardwave_plugin_host::types::PluginEditor>> {
        self.descriptor.has_editor.then(|| {
            Arc::new(SandboxEditor {
                to_child: self.to_child.clone(),
                child_pid: self.child_pid,
                shared: Arc::clone(&self.shared),
            }) as Arc<dyn hardwave_plugin_host::types::PluginEditor>
        })
    }

    fn pending_params(&self) -> Option<SharedParamQueue> {
        // Knob moves in its window reach its processor in its own process;
        // there is nothing for the chain here to pass on.
        None
    }

    fn gui_edit_log(&self) -> Option<SharedEditLog> {
        Some(Arc::clone(&self.shared.gui_log))
    }

    fn set_transport(&mut self, transport: TransportInfo) {
        self.transport = transport;
    }
}

/// The helper to run a plug-in this process cannot load itself, if
/// there is one next to the app.
///
/// On Windows that is the 32-bit build, shipped beside the exe. When
/// the plug-in matches this process, or no helper is installed, the
/// answer is nothing and the app hosts it in a child of its own.
fn helper_for(descriptor: &PluginDescriptor) -> Option<std::path::PathBuf> {
    use hardwave_plugin_host::binary_arch::{plugin_arch, BinaryArch};
    let arch = plugin_arch(&descriptor.path);
    if arch.matches_host() {
        return None;
    }
    let beside = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = match arch {
        BinaryArch::X86 => "hardwave-plugin-bridge-x86",
        BinaryArch::X86_64 => "hardwave-plugin-bridge-x64",
        BinaryArch::Arm64 => "hardwave-plugin-bridge-arm64",
        BinaryArch::Unknown => return None,
    };
    // Beside the app, and in the `binaries` folder the installer puts
    // bundled resources in.
    [
        beside.join(format!("{name}.exe")),
        beside.join("binaries").join(format!("{name}.exe")),
        beside.join(name),
        beside.join("binaries").join(name),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

/// The thread between the audio thread and the child process.
///
/// Reading from the child happens on a thread of its own, so this one
/// only ever waits with a deadline: a child that hangs is noticed and
/// killed, rather than holding this thread (and its queue) forever. The
/// reading thread keeps what the child says on its own (parameters, knob
/// moves, state, its window) and passes on the answers this one waits for.
fn bridge_loop(
    mut child: Child,
    to_rx: Receiver<ToChild>,
    from_tx: SyncSender<FromChild>,
    crashed: Arc<AtomicBool>,
    crash_message: Arc<Mutex<Option<String>>>,
    shared: Arc<Shared>,
    trace: Trace,
) {
    // This thread and the one that reads are in the path of the audio: at
    // ordinary priority a busy computer made them late, and a late block
    // is a gap.
    let _ = hardwave_engine::thread_priority::raise_current_thread();
    let Some(stdin) = child.stdin.take() else {
        crashed.store(true, Ordering::Relaxed);
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        crashed.store(true, Ordering::Relaxed);
        return;
    };
    let mut writer = BufWriter::new(stdin);
    let (frames_tx, frames) = mpsc::sync_channel::<std::io::Result<Option<Frame>>>(4);
    // A write that does not come back means the child stopped reading (or
    // something it started holds the pipe): the watchdog says it is gone
    // and kills it, so the app can start it again. When a write began, in
    // milliseconds since `started`, or 0 between writes.
    let started = std::time::Instant::now();
    let writing_since = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bridge_done = Arc::new(AtomicBool::new(false));
    // Shared with the watchdog, which kills it when a write hangs.
    let child = Arc::new(Mutex::new(child));
    {
        let child = Arc::clone(&child);
        let writing_since = Arc::clone(&writing_since);
        let bridge_done = Arc::clone(&bridge_done);
        let crashed = Arc::clone(&crashed);
        let crash_message = Arc::clone(&crash_message);
        let from_tx = from_tx.clone();
        let shared = Arc::clone(&shared);
        let trace = trace.clone();
        let _ = std::thread::Builder::new()
            .name("hardwave-plugin-bridge-watch".into())
            .spawn(move || {
                while !bridge_done.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    let since = writing_since.load(Ordering::Relaxed);
                    if since == 0 || crashed.load(Ordering::Relaxed) {
                        continue;
                    }
                    let now = started.elapsed().as_millis() as u64;
                    if now.saturating_sub(since) < WRITE_DEADLINE.as_millis() as u64 {
                        continue;
                    }
                    let message = "the plug-in's process stopped taking audio".to_string();
                    log::warn!("plug-in process: {message}");
                    trace.line(&format!("watchdog: {message}"));
                    crashed.store(true, Ordering::Relaxed);
                    if let Ok(mut slot) = crash_message.lock() {
                        *slot = Some(message.clone());
                    }
                    shared.params_ready.notify_all();
                    let _ = from_tx.try_send(FromChild::Crashed(message));
                    if let Ok(mut child) = child.lock() {
                        let _ = child.kill();
                    }
                    break;
                }
            });
    }
    {
        let shared = Arc::clone(&shared);
        let trace = trace.clone();
        let _ = std::thread::Builder::new()
            .name("hardwave-plugin-bridge-read".into())
            .spawn(move || {
                let _ = hardwave_engine::thread_priority::raise_current_thread();
                let mut reader = BufReader::new(stdout);
                loop {
                    let frame = Frame::read(&mut reader);
                    match &frame {
                        Ok(None) => trace.line("its output ended"),
                        Err(e) => trace.line(&format!("its output went wrong: {e}")),
                        _ => {}
                    }
                    if let Ok(Some(f)) = &frame {
                        if take_aside(f, &shared) {
                            continue;
                        }
                    }
                    let last = !matches!(frame, Ok(Some(_)));
                    if frames_tx.send(frame).is_err() || last {
                        break;
                    }
                }
                // Nothing more is coming: anyone waiting for the list stops.
                shared.params_ready.notify_all();
            });
    }

    let fail = |message: String| {
        log::warn!("plug-in process: {message}");
        trace.line(&format!("gone: {message}"));
        // The first reason stands: a write failing because the watchdog
        // already killed the child says less than the watchdog did.
        if !crashed.swap(true, Ordering::Relaxed) {
            if let Ok(mut slot) = crash_message.lock() {
                *slot = Some(message.clone());
            }
        }
        shared.params_ready.notify_all();
        let _ = from_tx.send(FromChild::Crashed(message));
    };

    // The next frame of a kind, within a deadline; anything else that
    // arrives on the way is skipped.
    let wait_for = |kind: u32, within: std::time::Duration| -> Result<Frame, String> {
        let deadline = std::time::Instant::now() + within;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match frames.recv_timeout(left) {
                Ok(Ok(Some(frame))) if frame.kind == kind => return Ok(frame),
                Ok(Ok(Some(frame))) if frame.kind == RES_ERROR => {
                    return Err(format!(
                        "the plug-in reported: {}",
                        String::from_utf8_lossy(&frame.payload)
                    ))
                }
                Ok(Ok(Some(_))) => continue,
                Ok(Ok(None)) => return Err("the plug-in's process closed".into()),
                Ok(Err(e)) => return Err(format!("the plug-in's process went wrong: {e}")),
                Err(RecvTimeoutError::Timeout) => return Err(STOPPED_ANSWERING.into()),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the plug-in's process closed".into())
                }
            }
        }
    };

    let mut payload = Vec::with_capacity(POOL_FRAMES * 8 + 4096);
    let mut audio_back = false;
    let mut last_state = std::time::Instant::now();
    // When the host last moved a knob, until the state has been fetched.
    let mut host_moved_at: Option<std::time::Instant> = None;
    loop {
        // While its window is open, or after its knobs moved, the state is
        // fetched now and then, so a save has what was done in there.
        let job = match to_rx.recv_timeout(BRIDGE_TURN) {
            Ok(job) => Some(job),
            Err(RecvTimeoutError::Timeout) => None,
            // The plug-in was removed: the child goes with it.
            Err(RecvTimeoutError::Disconnected) => break,
        };
        if shared.host_moved.swap(false, Ordering::Relaxed) {
            host_moved_at = Some(std::time::Instant::now());
        }
        // Knobs the host moves (automation runs for minutes) are waited
        // out: the state is fetched once they have been still a moment.
        let settled = host_moved_at.is_some_and(|at| at.elapsed() >= STATE_EVERY);
        let stale = shared.state_stale.load(Ordering::Relaxed)
            || shared.editor_open.load(Ordering::Relaxed)
            || settled;
        // Someone is about to read the state: no waiting for a turn.
        let now = shared.fetch_now.swap(false, Ordering::Relaxed);
        if now || (stale && last_state.elapsed() >= STATE_EVERY) {
            shared.state_stale.store(false, Ordering::Relaxed);
            shared.host_dirty.store(false, Ordering::Relaxed);
            shared.state_pending.store(true, Ordering::Relaxed);
            host_moved_at = None;
            last_state = std::time::Instant::now();
            writing_since.store(
                started.elapsed().as_millis().max(1) as u64,
                Ordering::Relaxed,
            );
            let asked = Frame::new(REQ_GET_STATE, Vec::new()).write(&mut writer);
            writing_since.store(0, Ordering::Relaxed);
            if asked.is_err() {
                fail("the plug-in's process stopped answering".into());
                break;
            }
        }
        let Some(job) = job else {
            continue;
        };
        writing_since.store(
            started.elapsed().as_millis().max(1) as u64,
            Ordering::Relaxed,
        );
        let written = match &job {
            ToChild::Shutdown => {
                let _ = Frame::new(REQ_SHUTDOWN, Vec::new()).write(&mut writer);
                break;
            }
            ToChild::Param(id, value) => Frame::set_param(*id, *value).write(&mut writer),
            ToChild::Activate(rate, block) => Frame::activate(*rate, *block).write(&mut writer),
            ToChild::State(bytes) => Frame::new(REQ_SET_STATE, bytes.clone()).write(&mut writer),
            ToChild::Params => Frame::new(REQ_PARAMS, Vec::new()).write(&mut writer),
            ToChild::Ask(ask, _) => Frame::ask(ask).write(&mut writer),
            ToChild::LoadPreset(index) => Frame::load_preset(*index).write(&mut writer),
            ToChild::OpenEditor => Frame::open_editor(0).write(&mut writer),
            ToChild::CloseEditor => Frame::new(REQ_CLOSE_EDITOR, Vec::new()).write(&mut writer),
            ToChild::Block(block) => {
                Frame::write_process(
                    &mut payload,
                    &block.left,
                    &block.right,
                    block
                        .keyed
                        .then_some((block.key_left.as_slice(), block.key_right.as_slice())),
                    &block.midi,
                    &block.transport,
                );
                let frame = Frame::new(REQ_PROCESS, std::mem::take(&mut payload));
                let written = frame.write(&mut writer);
                payload = frame.payload;
                written
            }
        };
        writing_since.store(0, Ordering::Relaxed);
        if written.is_err() {
            fail("the plug-in's process stopped answering".into());
            break;
        }
        match job {
            ToChild::Activate(..) => {
                // Its answer is consumed here, so the next block's answer
                // is the next block's, not this one.
                if let Err(why) = wait_for(RES_OK, ACTIVATE_DEADLINE) {
                    fail(why);
                    break;
                }
                trace.line("activated");
            }
            ToChild::Ask(_, reply) => match wait_for(RES_ANSWER, ASK_DEADLINE) {
                Ok(frame) => {
                    if let Some(answer) = frame.read_answer() {
                        let _ = reply.try_send(answer);
                    }
                }
                // A plug-in slow to say how a value reads is not a crashed
                // one: the asker goes without, the audio goes on.
                Err(why) if why == STOPPED_ANSWERING => {}
                Err(why) => {
                    fail(why);
                    break;
                }
            },
            ToChild::Block(mut block) => match wait_for(
                RES_AUDIO,
                if shared.patient.load(Ordering::Relaxed) {
                    OFFLINE_BLOCK_DEADLINE
                } else {
                    BLOCK_DEADLINE
                },
            ) {
                Ok(frame) => {
                    // The block's own buffers take the answer: same length
                    // as what went out.
                    let n = frame.read_audio(&mut block.left, &mut block.right);
                    if !audio_back {
                        audio_back = true;
                        trace.line("first audio back");
                    }
                    block.left[n..].fill(0.0);
                    block.right[n..].fill(0.0);
                    if from_tx.send(FromChild::Block(block)).is_err() {
                        break;
                    }
                }
                Err(why) => {
                    fail(why);
                    break;
                }
            },
            _ => {}
        }
    }

    bridge_done.store(true, Ordering::Relaxed);
    shared.ended.store(true, Ordering::Relaxed);
    trace.line("bridge ends");
    if let Ok(mut child) = child.lock() {
        let _ = child.kill();
        let _ = child.wait();
    };
}

/// Keep what the child says outside the block-by-block exchange, and say
/// whether this frame was one of those.
fn take_aside(frame: &Frame, shared: &Shared) -> bool {
    match frame.kind {
        RES_PARAMS => {
            let list = frame.read_params().unwrap_or_default();
            {
                let mut values = shared.values.lock();
                values.clear();
                values.extend(list.iter().map(|(info, value)| (info.id, *value)));
            }
            let mut params = shared.params.lock();
            *params = Some(list.into_iter().map(|(info, _)| info).collect());
            shared.values_epoch.fetch_add(1, Ordering::Relaxed);
            shared.params_ready.notify_all();
            true
        }
        RES_INFO => {
            if let Some(info) = frame.read_info() {
                shared.plugin_latency.store(info.latency, Ordering::Relaxed);
                *shared.class_id.lock() = info.class_id;
            }
            true
        }
        RES_PARAM_MOVES => {
            let moves = frame.read_param_moves();
            for (id, value) in &moves {
                shared.set_value(*id, *value);
            }
            for (id, value) in moves {
                shared.gui_log.push(id, value);
            }
            shared.state_stale.store(true, Ordering::Relaxed);
            true
        }
        RES_STATE => {
            *shared.state.lock() = frame.payload.clone();
            shared.state_pending.store(false, Ordering::Relaxed);
            true
        }
        RES_EDITOR => {
            let open = frame.payload.first() == Some(&1);
            shared.editor_open.store(open, Ordering::Relaxed);
            // Closing it is when the last of what was done in it counts.
            shared.state_stale.store(true, Ordering::Relaxed);
            true
        }
        _ => false,
    }
}

/// The child: load one plug-in and answer frames until told to stop.
///
/// Nothing else lives in this process. When the plug-in takes it down,
/// what dies is a process holding one plug-in, and the DAW carries on.
pub fn run_host_child(plugin_path: &str, plugin_id: &str) -> i32 {
    // Frames get private copies of stdin and stdout before any plug-in
    // code runs; what the plug-in prints goes nowhere instead of into a
    // frame, and nothing it starts holds the pipes to the DAW.
    let (frames_in, frames_out) = match hardwave_plugin_host::bridge_child::private_stdio() {
        Ok(streams) => streams,
        Err(e) => {
            eprintln!("--host-plugin: no streams for frames: {e}");
            return 4;
        }
    };
    // This thread keeps the plug-in's window.
    hardwave_plugin_host::bridge_child::prepare_window_thread();
    let descriptor = match crate::commands::plugins::descriptor_for_child(plugin_path, plugin_id) {
        Some(d) => d,
        None => {
            eprintln!("--host-plugin: nothing at {plugin_path}");
            return 2;
        }
    };
    let plugin = match crate::commands::plugins::instantiate_plugin(&descriptor) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("--host-plugin: {e}");
            return 3;
        }
    };
    let title = descriptor.name.clone();
    // The thread that plays runs at the priority the DAW's audio thread
    // has: the DAW is waiting for each block.
    let raise: Box<dyn FnOnce() + Send> = Box::new(|| {
        let outcome = hardwave_engine::thread_priority::raise_current_thread();
        eprintln!(
            "[hardwave bridge] play thread priority: {}",
            outcome.describe()
        );
    });
    hardwave_plugin_host::bridge_child::serve_with(
        plugin,
        frames_in,
        frames_out,
        &title,
        Some(raise),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, shared: &Arc<Shared>, crashed: bool) -> SandboxEntry {
        SandboxEntry {
            plugin_id: id.into(),
            crashed: Arc::new(AtomicBool::new(crashed)),
            message: Arc::new(Mutex::new(None)),
            shared: Arc::clone(shared),
            pid: 0,
        }
    }

    #[test]
    fn a_sandbox_nothing_uses_is_forgotten_and_a_crash_is_kept() {
        let in_use = Arc::new(Shared::default());
        let dropped = Arc::new(Shared::default());
        let crashed = Arc::new(Shared::default());
        let mut registry = vec![
            entry("in use", &in_use, false),
            entry("dropped", &dropped, false),
            entry("crashed", &crashed, true),
        ];
        // Its process was let go, though something still holds a handle.
        let let_go = Arc::new(Shared::default());
        let_go.ended.store(true, Ordering::Relaxed);
        registry.push(entry("let go", &let_go, false));
        drop(dropped);
        drop(crashed);
        forget_finished(&mut registry);
        let left: Vec<&str> = registry.iter().map(|e| e.plugin_id.as_str()).collect();
        assert_eq!(left, vec!["in use", "crashed"]);
    }
    #[cfg(unix)]
    use hardwave_plugin_host::types::{PluginCategory, PluginFormat};

    #[cfg(unix)]
    fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: "test.plugin".into(),
            name: "Test".into(),
            vendor: String::new(),
            version: String::new(),
            format: PluginFormat::Vst3,
            path: std::path::PathBuf::from("/nonexistent.vst3"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    /// Stand in for the child process, so the test exercises the bridge
    /// without needing a real plug-in to crash.
    #[cfg(unix)]
    fn fake_child(dir: &Path, body: &str) -> std::path::PathBuf {
        let script = dir.join("fake-child.sh");
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        script
    }

    #[cfg(unix)]
    fn block(plugin: &mut SandboxedPlugin, samples: usize) -> Vec<f32> {
        let input = vec![0.5f32; samples];
        let inputs: [&[f32]; 2] = [&input, &input];
        let mut outputs = vec![Vec::new(), Vec::new()];
        plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), samples);
        outputs.remove(0)
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_dies_is_noticed_and_the_song_keeps_playing() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = fake_child(&dir, "exit 1");

        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        // Keep feeding it blocks: process must return, every time, with
        // silence rather than whatever the dead child left behind.
        let mut noticed = false;
        for _ in 0..200 {
            let out = block(&mut plugin, 128);
            assert_eq!(out.len(), 128, "a block always comes back the right length");
            assert!(
                out.iter().all(|s| *s == 0.0),
                "a dead plug-in plays nothing"
            );
            if plugin.has_crashed() {
                noticed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(noticed, "the bridge should have noticed the child is gone");
        assert!(plugin.crash_message().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_never_answers_leaves_silence_not_a_hang() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-quiet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Reads its input and says nothing back.
        // exec, so the kill reaches the process holding the pipe
        // rather than the shell that started it.
        let exe = fake_child(&dir, "exec cat > /dev/null");

        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        let started = std::time::Instant::now();
        for _ in 0..20 {
            let out = block(&mut plugin, 128);
            assert!(out.iter().all(|s| *s == 0.0));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_millis(500),
            "the audio thread must never wait for the child"
        );
        // Dropping it has to end, too: a child wedged mid-block never
        // reads a shutdown frame, so it is killed rather than asked.
        let dropped = std::time::Instant::now();
        drop(plugin);
        assert!(
            dropped.elapsed() < std::time::Duration::from_secs(5),
            "removing a wedged plug-in must not hang the app"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_plugin_that_prints_into_the_stream_is_stopped_not_waited_on() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-chatty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Text where a frame should be, then silence: the old bridge read
        // the text as a length and waited for megabytes that never came.
        let exe = fake_child(&dir, "echo 'Loading 4000 presets...'; exec cat > /dev/null");
        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        let started = std::time::Instant::now();
        while !plugin.has_crashed() && started.elapsed() < std::time::Duration::from_secs(5) {
            let _ = block(&mut plugin, 128);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(plugin.has_crashed(), "a garbled stream ends the plug-in");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_cannot_keep_up_does_not_fill_memory() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-slow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = fake_child(&dir, "exec sleep 30");
        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        // Thousands of blocks at a child that never reads: the queue to
        // it is bounded, so this returns at once and holds a handful.
        let started = std::time::Instant::now();
        for _ in 0..5_000 {
            let _ = block(&mut plugin, 512);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A child that speaks the protocol the way a plug-in process does:
    /// one parameter, doubling its input, a 1.0 where a note starts, a knob
    /// moved in its "window" once, and a window that opens.
    #[cfg(unix)]
    const SPEAKING_CHILD: &str = r#"
import sys, struct, json
inp, out = sys.stdin.buffer, sys.stdout.buffer
def send(kind, payload=b''):
    out.write(struct.pack('<III', 0x48574652, kind, len(payload)) + payload)
    out.flush()
moved = False
while True:
    head = inp.read(12)
    if len(head) < 12:
        break
    _, kind, n = struct.unpack('<III', head)
    p = inp.read(n)
    if kind == 6:
        break
    if kind == 8:
        info = {"id": 3, "name": "Mix", "default_value": 0.5, "min": 0.0, "max": 1.0, "unit": "", "automatable": True}
        send(105, json.dumps([[info, 0.4]]).encode())
    elif kind == 5:
        send(103)
    elif kind == 9:
        send(107, b'\x01')
    elif kind == 4:
        send(102, b'state')
    elif kind == 7:
        frames, events = struct.unpack_from('<II', p, 0)
        notes = [struct.unpack_from('<BBBBIf', p, 44 + e * 12) for e in range(events)]
        base = 44 + events * 12
        samples = [s * 2 for s in struct.unpack_from('<%df' % (frames * 2), p, base)]
        for kind_, _, _, _, timing, _ in notes:
            if kind_ == 1:
                samples[timing * 2] = 1.0
        if not moved:
            send(106, struct.pack('<Id', 3, 0.9))
            moved = True
        send(101, struct.pack('<%df' % (frames * 2), *samples))
"#;

    #[cfg(unix)]
    #[test]
    fn a_plug_in_in_its_own_process_lists_its_knobs_plays_notes_and_opens_its_window() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-speaks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("child.py");
        std::fs::write(&script, SPEAKING_CHILD).unwrap();
        let exe = fake_child(&dir, &format!("exec python3 {}", script.display()));
        let mut desc = descriptor();
        desc.has_editor = true;
        let mut plugin = SandboxedPlugin::start(desc, &exe).expect("start");

        assert!(
            plugin.wait_for_params(std::time::Duration::from_secs(5)),
            "the list came"
        );
        assert_eq!(plugin.get_parameter_count(), 1);
        assert_eq!(
            plugin.get_parameter_info(0).map(|p| p.name),
            Some("Mix".into())
        );
        assert_eq!(plugin.get_parameter_value(3), 0.4);

        plugin.activate(48_000.0, 128).expect("activate");
        let input = vec![0.25f32; 128];
        let note = [hardwave_midi::MidiEvent::NoteOn {
            timing: 7,
            channel: 0,
            note: 60,
            velocity: 1.0,
        }];
        let mut heard = None;
        for round in 0..300 {
            let inputs: [&[f32]; 2] = [&input, &input];
            let mut outputs = vec![Vec::new(), Vec::new()];
            let midi: &[hardwave_midi::MidiEvent] = if round == 0 { &note } else { &[] };
            plugin.process(&inputs, &mut outputs, midi, &mut Vec::new(), 128);
            if outputs[0].iter().any(|s| *s != 0.0) {
                heard = Some(outputs.remove(0));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let heard = heard.expect("the child's audio came back");
        assert_eq!(heard[0], 0.5, "doubled, a block later");
        assert_eq!(heard[7], 1.0, "with the note on its sample");

        // The knob moved in its window reaches the host and the recorder.
        let started = std::time::Instant::now();
        while plugin.get_parameter_value(3) != 0.9 && started.elapsed().as_secs() < 3 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(plugin.get_parameter_value(3), 0.9);
        let mut recorded = Vec::new();
        plugin
            .gui_edit_log()
            .unwrap()
            .drain(usize::MAX, |id, v| recorded.push((id, v)));
        assert_eq!(recorded, vec![(3, 0.9)]);

        // Its window opens from its own process, and while it is open the
        // state is fetched, so a save has what was done in it.
        let editor = plugin.editor().expect("it has a window");
        assert_eq!(editor.open_own_window(), Some(true));
        let started = std::time::Instant::now();
        while plugin.get_state() != b"state" && started.elapsed().as_secs() < 5 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(plugin.get_state(), b"state".to_vec());
        assert!(plugin.shared.editor_open.load(Ordering::Relaxed));
        assert!(!plugin.has_crashed());
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A child that stops reading fills the pipe, and a write to a full pipe
    /// never comes back: the watchdog says it is gone instead of the crash
    /// going unnoticed (a WebView2 editor's processes kept the pipe open
    /// after its process was killed).
    #[cfg(unix)]
    #[test]
    fn a_child_that_stops_reading_is_declared_gone() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-deaf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = fake_child(&dir, "exec sleep 30");
        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        // A song's worth of plug-in state is far more than a pipe holds.
        plugin.set_state(&vec![7u8; 4 << 20]).expect("handed over");
        let started = std::time::Instant::now();
        while !plugin.has_crashed() && started.elapsed() < std::time::Duration::from_secs(8) {
            let _ = block(&mut plugin, 128);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(4),
            "noticed within the deadline"
        );
        assert!(plugin.has_crashed(), "the hung write was noticed");
        assert_eq!(
            plugin.crash_message().as_deref(),
            Some("the plug-in's process stopped taking audio")
        );
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A child killed while it plays is noticed: the song goes on, and the
    /// app is told so it can start it again.
    #[cfg(unix)]
    #[test]
    fn a_child_killed_while_playing_is_noticed() {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-killed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("child.py");
        std::fs::write(&script, SPEAKING_CHILD).unwrap();
        let exe = fake_child(&dir, &format!("exec python3 {}", script.display()));
        let mut plugin = SandboxedPlugin::start(descriptor(), &exe).expect("start");
        assert!(plugin.wait_for_params(std::time::Duration::from_secs(5)));
        plugin.activate(48_000.0, 128).expect("activate");
        // Playing.
        for _ in 0..50 {
            let _ = block(&mut plugin, 128);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert!(!plugin.has_crashed());
        let _ = std::process::Command::new("kill")
            .arg("-9")
            .arg(plugin.child_pid.to_string())
            .status();
        let killed = std::time::Instant::now();
        while !plugin.has_crashed() && killed.elapsed() < std::time::Duration::from_secs(5) {
            let _ = block(&mut plugin, 128);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        assert!(plugin.has_crashed(), "the kill was noticed");
        assert!(
            killed.elapsed() < std::time::Duration::from_secs(2),
            "within a moment"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A child that speaks everything the bridge carries since plug-ins run
    /// in their own process by default: it doubles its input and adds the
    /// sidechain, reports a delay of 64 samples, answers questions, takes
    /// a state (which moves its one knob to 0.7) and a preset (0.2).
    /// `HICCUP`: the block at which it is 120 ms late, once. `DIE`: the
    /// block at which it exits.
    #[cfg(unix)]
    const FULL_CHILD: &str = r#"
import sys, struct, json, time, os
inp, out = sys.stdin.buffer, sys.stdout.buffer
hiccup = int(os.environ.get('HICCUP', '-1'))
die = int(os.environ.get('DIE', '-1'))
def send(kind, payload=b''):
    out.write(struct.pack('<III', 0x48574652, kind, len(payload)) + payload)
    out.flush()
value = 0.4
blocks = 0
def params():
    info = {"id": 3, "name": "Mix", "default_value": 0.5, "min": 0.0, "max": 1.0, "unit": "", "automatable": True}
    send(105, json.dumps([[info, value]]).encode())
def info():
    send(108, json.dumps({"latency": 64, "class_id": [9] * 16}).encode())
while True:
    head = inp.read(12)
    if len(head) < 12:
        break
    _, kind, n = struct.unpack('<III', head)
    p = inp.read(n)
    if kind == 6:
        break
    if kind == 8:
        params(); info()
    elif kind == 5:
        info(); send(103)
    elif kind == 3:
        value = 0.7
        params()
    elif kind == 12:
        value = 0.2
        params()
    elif kind == 4:
        send(102, b'state')
    elif kind == 11:
        ask = json.loads(p)
        if ask == "Presets":
            send(109, json.dumps({"Presets": ["Init", "Screech"]}).encode())
        elif "Text" in ask:
            send(109, json.dumps({"Text": "%d%%" % round(ask["Text"][1] * 100)}).encode())
        else:
            send(109, json.dumps({"Options": ["Low pass", "High pass"]}).encode())
    elif kind == 7:
        frames, events = struct.unpack_from('<II', p, 0)
        base = 44 + events * 12
        samples = [s * 2 for s in struct.unpack_from('<%df' % (frames * 2), p, base)]
        if len(p) == base + frames * 16:
            key = struct.unpack_from('<%df' % (frames * 2), p, base + frames * 8)
            samples = [a + b for a, b in zip(samples, key)]
        if blocks == die:
            os._exit(3)
        if blocks == hiccup:
            time.sleep(0.12)
        blocks += 1
        send(101, struct.pack('<%df' % (frames * 2), *samples))
"#;

    /// Start the full child, with these environment variables for it.
    #[cfg(unix)]
    fn full_child(tag: &str, env: &str, pace: Pace) -> (SandboxedPlugin, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("hw-sandbox-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("child.py");
        std::fs::write(&script, FULL_CHILD).unwrap();
        let exe = fake_child(&dir, &format!("{env} exec python3 {}", script.display()));
        let mut desc = descriptor();
        desc.name = format!("Test {tag}");
        let mut plugin = SandboxedPlugin::start_paced(desc, &exe, pace).expect("start");
        assert!(plugin.wait_for_params(std::time::Duration::from_secs(5)));
        (plugin, dir)
    }

    /// Play a ramp (sample t is t + 1) through the plug-in in calls of the
    /// given sizes, and hand back what came out, one value per sample.
    #[cfg(unix)]
    fn play_ramp(plugin: &mut SandboxedPlugin, sizes: &[usize], pause_ms: u64) -> Vec<f32> {
        let mut heard = Vec::new();
        let mut t = 0usize;
        for &n in sizes {
            let input: Vec<f32> = (t..t + n).map(|s| (s + 1) as f32).collect();
            let inputs: [&[f32]; 2] = [&input, &input];
            let mut outputs = vec![Vec::new(), Vec::new()];
            plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), n);
            assert_eq!(outputs[0].len(), n);
            heard.extend_from_slice(&outputs[0]);
            t += n;
            if pause_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(pause_ms));
            }
        }
        heard
    }

    /// The plug-in's audio is played exactly one buffer after it went in,
    /// whatever the size of each call and however late a block came back.
    /// It used to play "the oldest block back": one late block put the
    /// track a buffer further behind for the rest of the session, with
    /// nothing compensating for it, and a short call cut a block short.
    #[cfg(unix)]
    #[test]
    fn the_delay_stays_one_buffer_through_uneven_calls_and_a_late_block() {
        let (mut plugin, dir) = full_child("timing", "HICCUP=12", Pace::Live);
        plugin.activate(48_000.0, 128).expect("activate");
        // Uneven calls, as a loop point or a driver with a moving block
        // gives, then a long even run across the late block.
        let mut sizes = vec![128, 64, 64, 128, 32, 96, 128, 100, 28];
        sizes.extend(std::iter::repeat_n(128, 40));
        let heard = play_ramp(&mut plugin, &sizes, 25);
        let mut right = 0usize;
        for (t, &sample) in heard.iter().enumerate() {
            if sample == 0.0 {
                continue;
            }
            // Doubled, and exactly 128 samples after it went in.
            assert_eq!(
                sample,
                2.0 * (t - 128 + 1) as f32,
                "sample {t} is not the plug-in's audio from one buffer before"
            );
            right += 1;
        }
        let total: usize = sizes.iter().sum();
        assert!(
            right > total * 3 / 4,
            "most of it was heard: {right} of {total}"
        );
        // The tail, long after the late block: still heard, still in place.
        assert!(
            heard[total - 128..].iter().all(|s| *s != 0.0),
            "the plug-in is heard again after a late block"
        );
        let stats = &plugin.shared.stats;
        assert!(
            stats.missed.load(Ordering::Relaxed) >= 1,
            "the late block left a gap"
        );
        assert!(!plugin.has_crashed(), "late is not crashed");
        // Its own 64 samples and the one buffer.
        assert_eq!(plugin.latency_samples(), 64 + 128);
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A render waits for every block: nothing is left out however fast it
    /// runs, and the plug-in adds no delay beyond its own.
    #[cfg(unix)]
    #[test]
    fn a_render_waits_for_every_block_and_adds_no_delay() {
        let (mut plugin, dir) = full_child("render", "", Pace::Offline);
        plugin.activate(48_000.0, 256).expect("activate");
        let sizes = vec![256; 120];
        let started = std::time::Instant::now();
        let heard = play_ramp(&mut plugin, &sizes, 0);
        for (t, &sample) in heard.iter().enumerate() {
            assert_eq!(sample, 2.0 * (t + 1) as f32, "sample {t}, with no delay");
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(20));
        assert_eq!(plugin.latency_samples(), 64, "only the plug-in's own");
        let stats = &plugin.shared.stats;
        assert_eq!(stats.dropped.load(Ordering::Relaxed), 0);
        assert_eq!(stats.missed.load(Ordering::Relaxed), 0);
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A plug-in whose process goes during a render is named, so the file
    /// with a hole in it is not handed over as if it were whole.
    #[cfg(unix)]
    #[test]
    fn a_plug_in_that_dies_during_a_render_is_named() {
        let _ = take_offline_crashes();
        let (mut plugin, dir) = full_child("dies", "DIE=5", Pace::Offline);
        plugin.activate(48_000.0, 256).expect("activate");
        let started = std::time::Instant::now();
        let heard = play_ramp(&mut plugin, &[256; 20], 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        assert!(plugin.has_crashed());
        assert!(
            heard[256 * 10..].iter().all(|s| *s == 0.0),
            "silence after it went"
        );
        assert!(take_offline_crashes().contains(&"Test dies".to_string()));
        assert!(take_offline_crashes().is_empty(), "told once");
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sidechain reaches a plug-in in its own process. It got the main
    /// pair only, so a compressor keyed from the kick heard no kick.
    #[cfg(unix)]
    #[test]
    fn the_sidechain_reaches_a_plug_in_in_its_own_process() {
        let (mut plugin, dir) = full_child("keyed", "", Pace::Offline);
        plugin.activate(48_000.0, 64).expect("activate");
        let main = vec![0.25f32; 64];
        let key = vec![10.0f32; 64];
        let inputs: [&[f32]; 4] = [&main, &main, &key, &key];
        let mut outputs = vec![Vec::new(), Vec::new()];
        plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), 64);
        assert!(
            outputs[0].iter().all(|s| *s == 10.5),
            "the input doubled plus the key: {:?}",
            &outputs[0][..4]
        );
        // And a slot without one still plays.
        let inputs: [&[f32]; 2] = [&main, &main];
        plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), 64);
        assert!(outputs[0].iter().all(|s| *s == 0.5));
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What the parameter sheet, the preset menu and a `.vstpreset` need
    /// from a plug-in comes across from its process.
    #[cfg(unix)]
    #[test]
    fn a_plug_in_in_its_own_process_answers_for_its_values_presets_and_state() {
        let (mut plugin, dir) = full_child("answers", "", Pace::Live);
        assert_eq!(plugin.get_parameter_value(3), 0.4);
        assert_eq!(plugin.parameter_text(3, 0.25).as_deref(), Some("25%"));
        assert_eq!(
            plugin.parameter_options(3),
            Some(vec!["Low pass".to_string(), "High pass".to_string()])
        );
        assert_eq!(plugin.factory_presets(), vec!["Init", "Screech"]);
        assert_eq!(plugin.vst3_class_id(), Some([9; 16]));
        // A state loaded before it plays: the values it gives are there
        // when the call returns, not the ones from before it.
        plugin.set_state(b"saved").expect("state");
        assert_eq!(plugin.get_parameter_value(3), 0.7);
        // A preset is handed over without a wait; its values follow.
        plugin.load_factory_preset(1).expect("preset");
        let started = std::time::Instant::now();
        while plugin.get_parameter_value(3) != 0.2 && started.elapsed().as_secs() < 3 {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(plugin.get_parameter_value(3), 0.2);
        // A knob the host moves gets the state fetched at once when
        // someone is about to read it (a save right after a knob turn).
        plugin.set_parameter_value(3, 0.9);
        let asked = std::time::Instant::now();
        settle_states(std::time::Duration::from_secs(2));
        assert_eq!(plugin.get_state(), b"state".to_vec());
        assert!(asked.elapsed() < std::time::Duration::from_secs(1));
        // With nothing moved since, there is nothing to wait for.
        let asked = std::time::Instant::now();
        settle_states(std::time::Duration::from_secs(2));
        assert!(asked.elapsed() < std::time::Duration::from_millis(100));
        assert!(!plugin.has_crashed());
        drop(plugin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real thing, end to end: a real CLAP plug-in that writes through
    /// a null pointer on request, loaded by the real helper process. Not
    /// run by default: it needs the helper built (`hardwave-plugin-bridge`)
    /// and the crash-test plug-in, named in HW_TEST_BRIDGE and
    /// HW_TEST_CRASH_CLAP. Its knobs: 1 crashes, 2 hangs, 3 is a gain
    /// (output = input * gain * 2) kept in its state.
    #[cfg(unix)]
    #[test]
    #[ignore = "needs HW_TEST_BRIDGE and HW_TEST_CRASH_CLAP"]
    fn a_real_plug_in_that_crashes_or_hangs_is_survived_and_comes_back_as_it_was() {
        let bridge = std::env::var("HW_TEST_BRIDGE").expect("HW_TEST_BRIDGE");
        let clap = std::env::var("HW_TEST_CRASH_CLAP").expect("HW_TEST_CRASH_CLAP");
        let dir = std::env::temp_dir().join(format!("hw-sandbox-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The helper takes the plug-in's path and id; the switch the DAW's
        // own binary takes first is dropped.
        let exe = fake_child(&dir, &format!("shift; exec {bridge} \"$@\""));
        let desc = PluginDescriptor {
            id: "com.hardwavestudios.crash-test".into(),
            name: "Crash Test".into(),
            vendor: "Crash Test Lab".into(),
            version: String::new(),
            format: PluginFormat::Clap,
            path: std::path::PathBuf::from(&clap),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        };
        let start = |saved: Option<&[u8]>| -> SandboxedPlugin {
            let mut plugin = SandboxedPlugin::start(desc.clone(), &exe).expect("start");
            assert!(plugin.wait_for_params(std::time::Duration::from_secs(10)));
            assert_eq!(plugin.get_parameter_count(), 3);
            if let Some(bytes) = saved {
                plugin.set_state(bytes).expect("state");
            }
            plugin.activate(48_000.0, 256).expect("activate");
            plugin
        };
        // Play for a while; the level of the last block heard.
        let play = |plugin: &mut SandboxedPlugin, blocks: usize| -> f32 {
            let mut level = 0.0;
            for _ in 0..blocks {
                let out = {
                    let input = vec![0.25f32; 256];
                    let inputs: [&[f32]; 2] = [&input, &input];
                    let mut outputs = vec![Vec::new(), Vec::new()];
                    plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), 256);
                    outputs.remove(0)
                };
                level = out[255];
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            level
        };

        let mut plugin = start(None);
        assert_eq!(play(&mut plugin, 40), 0.25, "gain at its default");
        // A question is answered by the real process too (this plug-in
        // ships no presets).
        assert!(plugin.factory_presets().is_empty());
        plugin.set_parameter_value(3, 0.8);
        let level = play(&mut plugin, 40);
        assert!((level - 0.4).abs() < 1e-6, "the knob is heard: {level}");
        // Read right after the knob moved (a save, the parameter sheet):
        // the state is fetched for it, not a second later.
        settle_states(std::time::Duration::from_millis(500));
        assert_eq!(
            plugin.get_state(),
            0.8f64.to_le_bytes().to_vec(),
            "a state read straight after a knob move has the move"
        );
        plugin.set_parameter_value(3, 0.8);
        // The state follows the knob once it has been still a moment.
        let wanted = 0.8f64.to_le_bytes().to_vec();
        let started = std::time::Instant::now();
        while plugin.get_state() != wanted && started.elapsed().as_secs() < 6 {
            play(&mut plugin, 10);
        }
        assert_eq!(plugin.get_state(), wanted, "the state has the knob");

        // The plug-in writes through a null pointer in its next block.
        plugin.set_parameter_value(1, 1.0);
        let crashed_at = std::time::Instant::now();
        while !plugin.has_crashed() && crashed_at.elapsed().as_secs() < 5 {
            play(&mut plugin, 1);
        }
        assert!(plugin.has_crashed(), "the crash was noticed");
        assert!(crashed_at.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(
            play(&mut plugin, 5),
            0.0,
            "its slot is silent, the caller plays on"
        );
        let saved = plugin.get_state();
        assert_eq!(saved, wanted, "what it was set to outlives its process");
        drop(plugin);

        // Started again with what the bridge kept: as it was.
        let mut again = start(Some(&saved));
        assert_eq!(again.get_parameter_value(3), 0.8);
        let level = play(&mut again, 40);
        assert!(
            (level - 0.4).abs() < 1e-6,
            "back at the gain it had: {level}"
        );

        // A plug-in that never returns from a block is given up on too.
        again.set_parameter_value(2, 1.0);
        let hung_at = std::time::Instant::now();
        while !again.has_crashed() && hung_at.elapsed().as_secs() < 5 {
            play(&mut again, 1);
        }
        assert!(again.has_crashed(), "the hang was noticed");
        assert!(hung_at.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(
            again.crash_message().as_deref(),
            Some("the plug-in stopped answering")
        );
        drop(again);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn plugin(id: &str, vendor: &str, path: &str) -> PluginDescriptor {
        PluginDescriptor {
            id: id.into(),
            name: id.into(),
            vendor: vendor.into(),
            version: String::new(),
            format: hardwave_plugin_host::types::PluginFormat::Vst3,
            path: std::path::PathBuf::from(path),
            category: hardwave_plugin_host::types::PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: true,
        }
    }

    #[test]
    fn third_party_plug_ins_run_in_their_own_process_unless_told_otherwise() {
        let mut choices = Choices::default();
        let serum = plugin("xfer.serum", "Xfer Records", "C:/vst3/Serum.vst3");
        let wettboi = plugin("hw.wettboi", "Hardwave Studios", "C:/vst3/WettBoi.vst3");
        let built_in = plugin("hardwave.native.eq", "Hardwave", "<native>");
        let old = plugin("old.synth", "Someone", "C:/vst3/Old32.vst3");

        assert_eq!(choices.home(&serum, false), Home::Own { fixed: false });
        assert_eq!(choices.home(&wettboi, false), Home::Inside { fixed: false });
        assert_eq!(choices.home(&built_in, false), Home::Inside { fixed: true });
        // Built for another processor: no choice, whatever the lists say.
        choices.never.insert("old.synth".into());
        assert_eq!(choices.home(&old, true), Home::Own { fixed: true });

        // The producer's opt-out for one plug-in, and back.
        choices.choose(&serum, false);
        assert_eq!(choices.home(&serum, false), Home::Inside { fixed: false });
        assert!(choices.never.contains("xfer.serum"));
        choices.choose(&serum, true);
        assert_eq!(choices.home(&serum, false), Home::Own { fixed: false });
        assert!(
            !choices.never.contains("xfer.serum") && !choices.always.contains("xfer.serum"),
            "choosing the default leaves no entry"
        );

        // One of ours can be moved out, and back.
        choices.choose(&wettboi, true);
        assert_eq!(choices.home(&wettboi, false), Home::Own { fixed: false });
        choices.choose(&wettboi, false);
        assert!(choices.always.is_empty());

        // A built-in plug-in stays where it is whatever is chosen.
        choices.always.insert("hardwave.native.eq".into());
        assert_eq!(choices.home(&built_in, false), Home::Inside { fixed: true });
    }

    #[test]
    fn the_old_behaviour_can_be_asked_for_and_keeps_the_old_list() {
        // What a settings file from before 0.266.49 held: a list of
        // plug-ins to run in their own process.
        let mut choices = Choices {
            always: ["crashy.fx".to_string()].into_iter().collect(),
            third_party_inside: true,
            ..Choices::default()
        };
        let crashy = plugin("crashy.fx", "Someone", "C:/vst3/Crashy.vst3");
        let fine = plugin("fine.fx", "Someone", "C:/vst3/Fine.vst3");
        assert!(choices.home(&crashy, false).is_own());
        assert!(!choices.home(&fine, false).is_own());
        // And with the new default the old list still holds.
        choices.third_party_inside = false;
        assert!(choices.home(&crashy, false).is_own());
        assert!(choices.home(&fine, false).is_own());
    }

    #[test]
    fn a_fresh_sandbox_reports_no_crash() {
        assert!(crashed_sandboxes()
            .iter()
            .all(|(id, _)| id != "never.started"));
    }
}
