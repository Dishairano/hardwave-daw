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

use std::collections::VecDeque;
use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};

use hardwave_plugin_host::bridge_protocol::*;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginDescriptor, SharedParamQueue, TransportInfo,
};
use raw_window_handle::RawWindowHandle;

/// How long a child may take over one block before it is treated as
/// gone. Generous: a block at 256 samples is five milliseconds, and a
/// child that is merely slow should recover rather than be declared
/// dead.
const BLOCK_DEADLINE: std::time::Duration = std::time::Duration::from_millis(250);
/// How long a plug-in may take to activate before it counts as hung.
const ACTIVATE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
/// Messages waiting for the child at most; past this, blocks are dropped.
const TO_CHILD_DEPTH: usize = 8;

/// Every sandbox started in this run, so the app can say which
/// plug-in's process went and when.
///
/// A list rather than a map: two slots can hold the same plug-in, and
/// which of them died matters less than the fact that one did.
static REGISTRY: Mutex<Vec<SandboxEntry>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct SandboxEntry {
    plugin_id: String,
    crashed: Arc<AtomicBool>,
    message: Arc<Mutex<Option<String>>>,
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

/// Forget the crashes already shown, so the same one is not reported
/// every time the app asks.
pub fn clear_crashed_sandboxes() {
    let mut registry = match REGISTRY.lock() {
        Ok(r) => r,
        Err(poisoned) => poisoned.into_inner(),
    };
    registry.retain(|entry| !entry.crashed.load(Ordering::Relaxed));
}

/// What the bridge sends to the worker thread.
enum ToChild {
    Audio(Vec<f32>, Vec<f32>),
    Param(u32, f64),
    /// Its own message rather than a parameter with a reserved id: a
    /// plug-in whose parameter happened to have that id re-activated
    /// itself at a sample rate of whatever the knob said.
    Activate(f64, u32),
    State(Vec<u8>),
    Shutdown,
}

/// What comes back.
enum FromChild {
    Audio(Vec<f32>, Vec<f32>),
    Crashed(String),
}

/// A plug-in that lives in another process, pretending to be an
/// ordinary one.
pub struct SandboxedPlugin {
    descriptor: PluginDescriptor,
    /// Bounded: a child that stops keeping up has blocks dropped rather
    /// than piling up in memory behind it.
    to_child: SyncSender<ToChild>,
    from_child: Receiver<FromChild>,
    /// Blocks that came back and have not been played yet. One deep in
    /// steady state: that is the latency this costs.
    pending: VecDeque<(Vec<f32>, Vec<f32>)>,
    crashed: Arc<AtomicBool>,
    crash_message: Arc<Mutex<Option<String>>>,
    block_size: Arc<AtomicU32>,
    /// Parameters as the host set them, so a restarted child can be
    /// put back where it was.
    param_cache: Vec<(u32, f64)>,
    state_cache: Vec<u8>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SandboxedPlugin {
    /// Start a child for this plug-in.
    ///
    /// `exe` is this program: the child is the same binary with a
    /// switch, so there is nothing else to install or keep in step.
    pub fn start(descriptor: PluginDescriptor, exe: &Path) -> Result<Self, String> {
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
        let child = command
            .arg(&descriptor.path)
            .arg(&descriptor.id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start the plug-in process: {e}"))?;

        let (to_tx, to_rx) = mpsc::sync_channel::<ToChild>(TO_CHILD_DEPTH);
        let (from_tx, from_rx) = mpsc::channel::<FromChild>();
        let crashed = Arc::new(AtomicBool::new(false));
        let crash_message = Arc::new(Mutex::new(None));

        let worker = {
            let crashed = Arc::clone(&crashed);
            let crash_message = Arc::clone(&crash_message);
            std::thread::Builder::new()
                .name("hardwave-plugin-bridge".into())
                .spawn(move || bridge_loop(child, to_rx, from_tx, crashed, crash_message))
                .map_err(|e| format!("could not start the bridge thread: {e}"))?
        };

        {
            let mut registry = match REGISTRY.lock() {
                Ok(r) => r,
                Err(poisoned) => poisoned.into_inner(),
            };
            registry.push(SandboxEntry {
                plugin_id: descriptor.id.clone(),
                crashed: Arc::clone(&crashed),
                message: Arc::clone(&crash_message),
            });
        }

        Ok(Self {
            descriptor,
            to_child: to_tx,
            from_child: from_rx,
            pending: VecDeque::with_capacity(4),
            crashed,
            crash_message,
            block_size: Arc::new(AtomicU32::new(0)),
            param_cache: Vec::new(),
            state_cache: Vec::new(),
            worker: Some(worker),
        })
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

impl HostedPlugin for SandboxedPlugin {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, max_block: u32) -> Result<(), String> {
        self.block_size.store(max_block, Ordering::Relaxed);
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
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        if inputs.len() < 2 || outputs.len() < 2 {
            return;
        }
        let left = &inputs[0][..num_samples.min(inputs[0].len())];
        let right = &inputs[1][..num_samples.min(inputs[1].len())];

        // A crashed child plays nothing. Passing the dry signal through
        // instead would hide the fact that a plug-in is missing from
        // the chain, which is worse than hearing the gap.
        if self.crashed.load(Ordering::Relaxed) {
            return;
        }

        // Hand this block over and take whatever came back. The audio
        // thread never waits on the child: that is the one block of
        // latency this costs, and it is reported below.
        // Never waits: a full queue means the child is behind, and the
        // block is dropped rather than queued without end.
        let _ = self
            .to_child
            .try_send(ToChild::Audio(left.to_vec(), right.to_vec()));
        loop {
            match self.from_child.try_recv() {
                Ok(FromChild::Audio(l, r)) => self.pending.push_back((l, r)),
                Ok(FromChild::Crashed(message)) => {
                    self.crashed.store(true, Ordering::Relaxed);
                    if let Ok(mut slot) = self.crash_message.lock() {
                        *slot = Some(message);
                    }
                    return;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.crashed.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
        if let Some((l, r)) = self.pending.pop_front() {
            let n = num_samples.min(l.len()).min(r.len());
            outputs[0][..n].copy_from_slice(&l[..n]);
            outputs[1][..n].copy_from_slice(&r[..n]);
        }
    }

    fn get_parameter_count(&self) -> u32 {
        0
    }

    fn get_parameter_info(&self, _index: u32) -> Option<ParameterInfo> {
        None
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
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
        let _ = self.to_child.try_send(ToChild::Param(id, value));
    }

    fn get_state(&self) -> Vec<u8> {
        // What the host last set. Asking the child would mean waiting
        // for it on whichever thread called this, and a saved project
        // that stalls on a sick plug-in is worse than one that saves
        // the state the host knows about.
        self.state_cache.clone()
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.state_cache = bytes.to_vec();
        self.to_child
            .try_send(ToChild::State(bytes.to_vec()))
            .map_err(|_| "the plug-in's process is not answering".to_string())
    }

    fn latency_samples(&self) -> u32 {
        // Exactly one block: the block handed over is played back on
        // the next one. Reported so delay compensation lines this
        // track up with the rest.
        self.block_size.load(Ordering::Relaxed)
    }

    fn open_editor(&mut self, _parent: RawWindowHandle) -> bool {
        false
    }

    fn close_editor(&mut self) {}

    fn has_editor(&self) -> bool {
        // A sandboxed plug-in uses the generic parameter sheet: showing
        // its own window means handing a window between processes,
        // which is its own piece of work.
        false
    }

    fn pending_params(&self) -> Option<SharedParamQueue> {
        None
    }

    fn set_transport(&mut self, _transport: TransportInfo) {}
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
/// killed, rather than holding this thread (and its queue) forever.
fn bridge_loop(
    mut child: Child,
    to_rx: Receiver<ToChild>,
    from_tx: Sender<FromChild>,
    crashed: Arc<AtomicBool>,
    crash_message: Arc<Mutex<Option<String>>>,
) {
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
    let _ = std::thread::Builder::new()
        .name("hardwave-plugin-bridge-read".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let frame = Frame::read(&mut reader);
                let last = !matches!(frame, Ok(Some(_)));
                if frames_tx.send(frame).is_err() || last {
                    break;
                }
            }
        });

    let fail = |message: String| {
        crashed.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = crash_message.lock() {
            *slot = Some(message.clone());
        }
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
                Err(RecvTimeoutError::Timeout) => {
                    return Err("the plug-in stopped answering".into())
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err("the plug-in's process closed".into())
                }
            }
        }
    };

    // The channel closing means the plug-in was removed: the child
    // goes with it.
    while let Ok(job) = to_rx.recv() {
        let written = match &job {
            ToChild::Shutdown => {
                let _ = Frame::new(REQ_SHUTDOWN, Vec::new()).write(&mut writer);
                break;
            }
            ToChild::Param(id, value) => Frame::set_param(*id, *value).write(&mut writer),
            ToChild::Activate(rate, block) => Frame::activate(*rate, *block).write(&mut writer),
            ToChild::State(bytes) => Frame::new(REQ_SET_STATE, bytes.clone()).write(&mut writer),
            ToChild::Audio(left, right) => Frame::audio(left, right).write(&mut writer),
        };
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
            }
            ToChild::Audio(left, right) => match wait_for(RES_AUDIO, BLOCK_DEADLINE) {
                Ok(frame) => {
                    let mut out_l = vec![0.0f32; left.len()];
                    let mut out_r = vec![0.0f32; right.len()];
                    frame.read_audio(&mut out_l, &mut out_r);
                    if from_tx.send(FromChild::Audio(out_l, out_r)).is_err() {
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

    let _ = child.kill();
    let _ = child.wait();
}

/// The child: load one plug-in and answer frames until told to stop.
///
/// Nothing else lives in this process. When the plug-in takes it down,
/// what dies is a process holding one plug-in, and the DAW carries on.
pub fn run_host_child(plugin_path: &str, plugin_id: &str) -> i32 {
    // Frames get a private copy of stdout before any plug-in code runs;
    // what the plug-in prints goes nowhere instead of into a frame.
    let frames_out = match hardwave_plugin_host::bridge_child::private_stdout() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("--host-plugin: no stream for frames: {e}");
            return 4;
        }
    };
    let descriptor = match crate::commands::plugins::descriptor_for_child(plugin_path, plugin_id) {
        Some(d) => d,
        None => {
            eprintln!("--host-plugin: nothing at {plugin_path}");
            return 2;
        }
    };
    let mut plugin = match crate::commands::plugins::instantiate_plugin(&descriptor) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("--host-plugin: {e}");
            return 3;
        }
    };
    hardwave_plugin_host::bridge_child::serve(plugin.as_mut(), std::io::stdin(), frames_out)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn a_fresh_sandbox_reports_no_crash() {
        assert!(crashed_sandboxes()
            .iter()
            .all(|(id, _)| id != "never.started"));
    }
}
