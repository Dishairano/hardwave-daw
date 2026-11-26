//! The child end of the plug-in bridge: one plug-in, answering frames.
//!
//! Shared by the sandbox child (the DAW re-run with a switch) and the
//! separate helper binary (the 32-bit host on Windows), so both speak
//! the protocol the same way.
//!
//! The frames go out on a private copy of the process's standard
//! output, and standard output itself is pointed at nothing before the
//! plug-in is loaded. A plug-in that prints, which many do, then prints
//! into nothing instead of into the middle of a frame.

use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::bridge_protocol::*;
use crate::types::{HostedPlugin, PluginEditor, SharedEditLog};

/// Take standard input and output for frames, leaving the plug-in a stdin
/// and stdout that go nowhere. Call before the plug-in is loaded.
///
/// The copies kept for frames cannot be inherited, and the originals are
/// closed: a plug-in that starts processes of its own (a WebView2 editor
/// starts several) used to hand them the pipe to the DAW, so killing this
/// process left the pipe open, the DAW's writes to it hung, and the crash
/// was never noticed.
pub fn private_stdio() -> std::io::Result<(std::fs::File, std::fs::File)> {
    let input = imp::private_handle(imp::Which::Input)?;
    let output = imp::private_handle(imp::Which::Output)?;
    Ok((input, output))
}

/// Take standard output alone (see `private_stdio`), for a child that only
/// answers once and reads nothing.
pub fn private_stdout() -> std::io::Result<std::fs::File> {
    imp::private_handle(imp::Which::Output)
}

/// Ready this thread to keep a plug-in's window: on Windows, OLE and COM in
/// a single-threaded apartment, as editors (and WebView2) expect. Call on
/// the thread that loads the plug-in, before loading it.
pub fn prepare_window_thread() {
    imp::prepare_window_thread();
}

#[cfg(unix)]
mod imp {
    use std::os::fd::FromRawFd;

    pub enum Which {
        Input,
        Output,
    }

    pub fn private_handle(which: Which) -> std::io::Result<std::fs::File> {
        let (fd, mode) = match which {
            Which::Input => (0, libc::O_RDONLY),
            Which::Output => (1, libc::O_WRONLY),
        };
        // Safety: plain descriptor calls on this process's own standard
        // streams; every result is checked before it is used.
        unsafe {
            let ours = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3);
            if ours < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let null = libc::open(c"/dev/null".as_ptr(), mode);
            if null >= 0 {
                libc::dup2(null, fd);
                libc::close(null);
            }
            Ok(std::fs::File::from_raw_fd(ours))
        }
    }

    pub fn prepare_window_thread() {}
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, DuplicateHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, SetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub enum Which {
        Input,
        Output,
    }

    pub fn private_handle(which: Which) -> std::io::Result<std::fs::File> {
        let (std_handle, access) = match which {
            Which::Input => (STD_INPUT_HANDLE, GENERIC_READ),
            Which::Output => (STD_OUTPUT_HANDLE, GENERIC_WRITE),
        };
        // Safety: handle calls on this process's own standard streams;
        // every result is checked before it is used.
        unsafe {
            let current = GetStdHandle(std_handle);
            if current == 0 || current == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut ours: HANDLE = 0;
            let process = GetCurrentProcess();
            // Not inheritable (the 0 before the access flag).
            if DuplicateHandle(
                process,
                current,
                process,
                &mut ours,
                0,
                0,
                windows_sys::Win32::Foundation::DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let nul: Vec<u16> = "NUL\0".encode_utf16().collect();
            let null = CreateFileW(
                nul.as_ptr(),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            );
            if null != INVALID_HANDLE_VALUE {
                SetStdHandle(std_handle, null);
            }
            // The inheritable original goes, so nothing started from here
            // holds the pipe.
            CloseHandle(current);
            Ok(std::fs::File::from_raw_handle(ours as _))
        }
    }

    pub fn prepare_window_thread() {
        // Safety: called once on this thread before any window or COM use.
        unsafe {
            windows_sys::Win32::System::Ole::OleInitialize(std::ptr::null());
        }
    }
}

/// What the frames thread asks of the thread that keeps the window.
enum UiCommand {
    Open,
    Close,
}

/// The stream back to the host, shared by the thread that plays and the
/// one that keeps the window.
type Out<W> = Arc<Mutex<BufWriter<W>>>;

fn send<W: Write>(out: &Out<W>, frame: Frame) -> bool {
    match out.lock() {
        Ok(mut w) => frame.write(&mut *w).is_ok(),
        Err(_) => false,
    }
}

/// Answer frames for one plug-in until the host says stop or goes away.
///
/// The plug-in plays on a thread of its own. This thread, the one that
/// loaded it, keeps its window: a plug-in's editor belongs to the thread
/// its controller was made on, and needs a message loop there. Returns
/// when the host has gone.
pub fn serve<R, W>(plugin: Box<dyn HostedPlugin>, input: R, output: W, title: &str) -> i32
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    serve_with(plugin, input, output, title, None)
}

/// [`serve`], with something to run first on the thread that plays: the
/// DAW raises that thread to the priority its own audio thread has, so a
/// busy computer does not make the plug-in's process late with its blocks.
pub fn serve_with<R, W>(
    plugin: Box<dyn HostedPlugin>,
    input: R,
    output: W,
    title: &str,
    on_play_thread: Option<Box<dyn FnOnce() + Send>>,
) -> i32
where
    R: Read + Send + 'static,
    W: Write + Send + 'static,
{
    let editor = plugin.editor();
    let gui_log = plugin.gui_edit_log();
    let out: Out<W> = Arc::new(Mutex::new(BufWriter::new(output)));
    let (ui_tx, ui_rx) = mpsc::channel();
    let waker = window::Waker::current();
    let player = {
        let out = Arc::clone(&out);
        let waker = waker.clone();
        std::thread::Builder::new()
            .name("hardwave-bridge-play".into())
            .spawn(move || {
                if let Some(prepare) = on_play_thread {
                    prepare();
                }
                let code = serve_frames(plugin, input, &out, &ui_tx, &waker);
                drop(ui_tx);
                waker.wake();
                code
            })
    };
    let Ok(player) = player else {
        return 5;
    };
    window::run(editor, ui_rx, &out, gui_log.as_ref(), title);
    player.join().unwrap_or(6)
}

/// How long after a block a knob set by the host still counts as "the
/// song is playing": the next block hands it to the plug-in. Past this,
/// nothing is calling the plug-in, and the value is handed over at once.
const PLAYING_WITHIN: std::time::Duration = std::time::Duration::from_millis(100);

/// The block size a plug-in is activated with when a knob or a state
/// request reaches it before the host has activated it. The host's own
/// activation, with the real size, replaces it.
const IDLE_BLOCK: u32 = 512;

/// How many blocks pass between looks at the plug-in's delay, which some
/// plug-ins change while they play (a limiter's lookahead knob).
const LATENCY_EVERY: u32 = 32;

/// The parameter list with where each one stands, as a frame.
fn params_frame(plugin: &dyn HostedPlugin) -> Frame {
    let list: Vec<_> = (0..plugin.get_parameter_count())
        .filter_map(|index| plugin.get_parameter_info(index))
        .map(|info| {
            let value = plugin.get_parameter_value(info.id);
            (info, value)
        })
        .collect();
    Frame::params(&list)
}

/// What the plug-in says about itself now.
fn info_of(plugin: &dyn HostedPlugin) -> ChildInfo {
    ChildInfo {
        latency: plugin.latency_samples(),
        class_id: plugin.vst3_class_id(),
    }
}

/// The frames, one after another, on the thread that plays.
fn serve_frames<R: Read, W: Write>(
    mut plugin: Box<dyn HostedPlugin>,
    input: R,
    out: &Out<W>,
    ui: &Sender<UiCommand>,
    waker: &window::Waker,
) -> i32 {
    let mut reader = BufReader::new(input);
    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    let mut key_left = vec![0.0f32; 4096];
    let mut key_right = vec![0.0f32; 4096];
    let mut outputs = vec![Vec::new(), Vec::new()];
    let mut midi = Vec::with_capacity(MAX_BLOCK_EVENTS);
    let mut midi_out = Vec::with_capacity(64);
    let mut sample_rate = 48_000.0;
    let mut told = None::<ChildInfo>;
    // Whether the plug-in has been activated, by the host or here.
    let mut active = false;
    let mut last_block = None::<std::time::Instant>;
    let mut blocks_since_look = 0u32;

    // Tell the host what the plug-in says about itself, when that changed.
    let tell = |plugin: &dyn HostedPlugin, told: &mut Option<ChildInfo>| -> bool {
        let now = info_of(plugin);
        if *told == Some(now) {
            return true;
        }
        *told = Some(now);
        send(out, Frame::info(&now))
    };

    // The host going away, or the stream ending, ends this process too:
    // there is nothing here without it.
    while let Ok(Some(frame)) = Frame::read(&mut reader) {
        match frame.kind {
            REQ_SHUTDOWN => break,
            REQ_ACTIVATE => {
                if let Some((rate, max_block)) = frame.read_activate() {
                    let block = if max_block == 0 {
                        4096
                    } else {
                        max_block.min(65_536)
                    };
                    if left.len() < block as usize {
                        left.resize(block as usize, 0.0);
                        right.resize(block as usize, 0.0);
                        key_left.resize(block as usize, 0.0);
                        key_right.resize(block as usize, 0.0);
                    }
                    sample_rate = rate;
                    // Activated here already, for a knob that came before
                    // the host's own activation: put down first, as the
                    // plug-in formats ask.
                    if active {
                        plugin.deactivate();
                    }
                    let _ = plugin.activate(rate, block);
                    active = true;
                }
                // Its delay is only certain once it knows the rate, and the
                // host hears it before it hears the plug-in is ready.
                if !tell(plugin.as_ref(), &mut told) || !send(out, Frame::new(RES_OK, Vec::new())) {
                    break;
                }
            }
            REQ_SET_PARAM => {
                if let Some((id, value)) = frame.read_set_param() {
                    plugin.set_parameter_value(id, value);
                    // VST3 and CLAP plug-ins only hear a value inside a
                    // block. With the song stopped, or the plug-in left out
                    // while it is silent, no block comes: hand it over now,
                    // so its window and its saved state have it.
                    let playing = last_block.is_some_and(|at| at.elapsed() < PLAYING_WITHIN);
                    if !playing {
                        // A plug-in on a track that has not played yet was
                        // never activated, and one that is not active takes
                        // no value at all: the knob the host set was lost,
                        // and the state read back had the default.
                        if !active {
                            active = plugin.activate(sample_rate, IDLE_BLOCK).is_ok();
                        }
                        plugin.flush_parameters();
                    }
                }
            }
            REQ_SET_STATE | REQ_LOAD_PRESET => {
                if frame.kind == REQ_SET_STATE {
                    let _ = plugin.set_state(&frame.payload);
                } else if let Some(index) = frame.read_preset_index() {
                    let _ = plugin.load_factory_preset(index as usize);
                }
                // Every knob may stand somewhere else now, and the delay
                // may differ: the host shows and compensates what is true.
                if !send(out, params_frame(plugin.as_ref())) || !tell(plugin.as_ref(), &mut told) {
                    break;
                }
            }
            REQ_GET_STATE => {
                // Values set since the last block go in first, or a
                // plug-in asked while the song is stopped answers with
                // the settings from before them.
                if !active {
                    active = plugin.activate(sample_rate, IDLE_BLOCK).is_ok();
                }
                plugin.flush_parameters();
                if !send(out, Frame::new(RES_STATE, plugin.get_state())) {
                    break;
                }
            }
            REQ_PARAMS => {
                if !send(out, params_frame(plugin.as_ref())) || !tell(plugin.as_ref(), &mut told) {
                    break;
                }
            }
            REQ_ASK => {
                // Always answered, whatever was asked: the host is waiting.
                let answer = match frame.read_ask() {
                    Some(Ask::Text(id, value)) => Answer::Text(plugin.parameter_text(id, value)),
                    Some(Ask::Options(id)) => Answer::Options(plugin.parameter_options(id)),
                    Some(Ask::Presets) => Answer::Presets(plugin.factory_presets()),
                    None => Answer::Text(None),
                };
                if !send(out, Frame::answer(&answer)) {
                    break;
                }
            }
            REQ_OPEN_EDITOR | REQ_CLOSE_EDITOR => {
                eprintln!(
                    "[hardwave bridge] the DAW asks to {} the window",
                    if frame.kind == REQ_OPEN_EDITOR {
                        "open"
                    } else {
                        "close"
                    }
                );
                let command = if frame.kind == REQ_OPEN_EDITOR {
                    UiCommand::Open
                } else {
                    UiCommand::Close
                };
                if ui.send(command).is_ok() {
                    waker.wake();
                }
            }
            REQ_AUDIO | REQ_PROCESS => {
                let mut keyed = false;
                let frames = if frame.kind == REQ_PROCESS {
                    let wanted = frame
                        .payload
                        .get(0..4)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
                        .unwrap_or(0)
                        .min(65_536);
                    if left.len() < wanted {
                        left.resize(wanted, 0.0);
                        right.resize(wanted, 0.0);
                        key_left.resize(wanted, 0.0);
                        key_right.resize(wanted, 0.0);
                    }
                    match frame.read_process(
                        &mut left,
                        &mut right,
                        Some((&mut key_left, &mut key_right)),
                        &mut midi,
                        sample_rate,
                    ) {
                        Some((frames, transport, with_key)) => {
                            plugin.set_transport(transport);
                            keyed = with_key;
                            frames
                        }
                        // A block that does not add up gets silence back,
                        // so the host's next answer is still the next one.
                        None => 0,
                    }
                } else {
                    // The older block: audio only.
                    midi.clear();
                    let frames = (frame.payload.len() / 8).min(65_536);
                    if left.len() < frames {
                        left.resize(frames, 0.0);
                        right.resize(frames, 0.0);
                    }
                    frame.read_audio(&mut left[..frames], &mut right[..frames]);
                    frames
                };
                if frames > 0 {
                    midi_out.clear();
                    if keyed {
                        // The same four channels the chain gives a keyed
                        // plug-in in the DAW's own process.
                        let inputs: [&[f32]; 4] = [
                            &left[..frames],
                            &right[..frames],
                            &key_left[..frames],
                            &key_right[..frames],
                        ];
                        plugin.process(&inputs, &mut outputs, &midi, &mut midi_out, frames);
                    } else {
                        let inputs: [&[f32]; 2] = [&left[..frames], &right[..frames]];
                        plugin.process(&inputs, &mut outputs, &midi, &mut midi_out, frames);
                    }
                }
                last_block = Some(std::time::Instant::now());
                let out_l = outputs
                    .first()
                    .map(|c| &c[..frames.min(c.len())])
                    .unwrap_or(&[]);
                let out_r = outputs
                    .get(1)
                    .map(|c| &c[..frames.min(c.len())])
                    .unwrap_or(out_l);
                let mut reply = Frame::audio(out_l, out_r);
                reply.kind = RES_AUDIO;
                if !send(out, reply) {
                    break;
                }
                blocks_since_look += 1;
                if blocks_since_look >= LATENCY_EVERY {
                    blocks_since_look = 0;
                    if !tell(plugin.as_ref(), &mut told) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    0
}

/// Knobs moved in the plug-in's own window since last time, sent to the
/// host so it can record them and show where they are.
fn send_moves<W: Write>(out: &Out<W>, gui_log: Option<&SharedEditLog>) {
    let Some(log) = gui_log else {
        return;
    };
    if !log.has_pending() {
        return;
    }
    let mut moves: Vec<(u32, f64)> = Vec::new();
    log.drain(usize::MAX, |id, value| moves.push((id, value)));
    if !moves.is_empty() {
        send(out, Frame::param_moves(&moves));
    }
}

/// Where there is no window to keep: wait for the host to go, passing on
/// knob moves meanwhile.
#[cfg(not(windows))]
mod window {
    use super::*;

    #[derive(Clone)]
    pub struct Waker;

    impl Waker {
        pub fn current() -> Self {
            Waker
        }
        pub fn wake(&self) {}
    }

    pub fn run<W: Write>(
        _editor: Option<Arc<dyn PluginEditor>>,
        commands: Receiver<UiCommand>,
        out: &Out<W>,
        gui_log: Option<&SharedEditLog>,
        _title: &str,
    ) {
        loop {
            match commands.recv_timeout(std::time::Duration::from_millis(30)) {
                Ok(UiCommand::Open) => {
                    send(out, Frame::new(RES_EDITOR, vec![0]));
                }
                Ok(UiCommand::Close) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            send_moves(out, gui_log);
        }
    }
}

/// The plug-in's window, a top-level window of this process.
///
/// Not put inside the host's window: a window parented across processes
/// ties the two processes' input together, so a plug-in that hangs would
/// hang the DAW's window with it, which is the one thing a plug-in in its
/// own process must not be able to do.
#[cfg(windows)]
mod window {
    use super::*;
    use std::cell::{Cell, RefCell};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::HiDpi::{
        AdjustWindowRectExForDpi, GetDpiForWindow, SetThreadDpiAwarenessContext,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const WM_WAKE: u32 = WM_APP + 1;
    const WM_EDITOR_SIZE: u32 = WM_APP + 2;
    const STYLE: u32 = WS_OVERLAPPEDWINDOW;

    /// Wakes the window's thread from another one.
    #[derive(Clone)]
    pub struct Waker(u32);

    impl Waker {
        pub fn current() -> Self {
            // Safety: no arguments, no failure.
            Waker(unsafe { GetCurrentThreadId() })
        }
        pub fn wake(&self) {
            // Safety: a thread message with no pointers in it.
            unsafe {
                PostThreadMessageW(self.0, WM_WAKE, 0, 0);
            }
        }
    }

    thread_local! {
        static EDITOR: RefCell<Option<Arc<dyn PluginEditor>>> = const { RefCell::new(None) };
        static CLOSE_ASKED: Cell<bool> = const { Cell::new(false) };
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SIZE => {
                let (w, h) = ((lparam & 0xFFFF) as u32, ((lparam >> 16) & 0xFFFF) as u32);
                if w > 0 && h > 0 {
                    EDITOR.with(|e| {
                        if let Some(editor) = e.borrow().as_ref() {
                            editor.resize(w, h);
                        }
                    });
                }
                0
            }
            WM_DPICHANGED => {
                let dpi = (wparam & 0xFFFF) as f64;
                EDITOR.with(|e| {
                    if let Some(editor) = e.borrow().as_ref() {
                        editor.set_scale(dpi / 96.0);
                    }
                });
                // The size Windows suggests for the new screen.
                let r = &*(lparam as *const RECT);
                SetWindowPos(
                    hwnd,
                    0,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                0
            }
            WM_CLOSE => {
                // Closed by the person: the loop puts the editor away and
                // tells the host, rather than this window going on its own.
                CLOSE_ASKED.with(|c| c.set(true));
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    /// Make the window big enough for a client area of `w` by `h`.
    unsafe fn fit(hwnd: HWND, w: u32, h: u32) {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: w as i32,
            bottom: h as i32,
        };
        AdjustWindowRectExForDpi(&mut rect, STYLE, 0, 0, GetDpiForWindow(hwnd));
        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }

    pub fn run<W: Write>(
        editor: Option<Arc<dyn PluginEditor>>,
        commands: Receiver<UiCommand>,
        out: &Out<W>,
        gui_log: Option<&SharedEditLog>,
        title: &str,
    ) {
        // Safety: Win32 calls on this thread's own windows and messages;
        // every handle is checked before use and the class outlives them.
        unsafe {
            // Sharp at any scale, in the helper too, which has no manifest.
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let thread = GetCurrentThreadId();
            if let Some(editor) = &editor {
                // A size the plug-in asks for itself comes to this thread.
                editor.set_resize_request(Arc::new(move |w, h| {
                    PostThreadMessageW(thread, WM_EDITOR_SIZE, w as usize, h as isize) != 0
                }));
            }
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("HardwavePluginWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(window_proc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: 0,
                hCursor: LoadCursorW(0, IDC_ARROW),
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
                hIconSm: 0,
            };
            RegisterClassExW(&wc);
            let title = wide(title);
            let mut hwnd: HWND = 0;
            let close = |hwnd: &mut HWND| {
                if *hwnd != 0 {
                    if let Some(editor) = &editor {
                        editor.close();
                    }
                    EDITOR.with(|e| e.borrow_mut().take());
                    DestroyWindow(*hwnd);
                    *hwnd = 0;
                    send(out, Frame::new(RES_EDITOR, vec![0]));
                }
            };
            loop {
                MsgWaitForMultipleObjects(0, std::ptr::null(), 0, 30, QS_ALLINPUT);
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, 0, 0, 0, PM_REMOVE) != 0 {
                    if msg.hwnd == 0 && msg.message == WM_EDITOR_SIZE {
                        if hwnd != 0 {
                            fit(hwnd, msg.wParam as u32, msg.lParam as u32);
                        }
                        continue;
                    }
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if CLOSE_ASKED.with(|c| c.replace(false)) {
                    close(&mut hwnd);
                }
                loop {
                    match commands.try_recv() {
                        Ok(UiCommand::Open) => {
                            if hwnd != 0 {
                                ShowWindow(hwnd, SW_RESTORE);
                                SetForegroundWindow(hwnd);
                                continue;
                            }
                            let Some(ed) = editor.as_ref() else {
                                eprintln!("[hardwave bridge] the plug-in has no window");
                                send(out, Frame::new(RES_EDITOR, vec![0]));
                                continue;
                            };
                            hwnd = CreateWindowExW(
                                0,
                                class.as_ptr(),
                                title.as_ptr(),
                                STYLE,
                                CW_USEDEFAULT,
                                CW_USEDEFAULT,
                                640,
                                480,
                                0,
                                0,
                                instance,
                                std::ptr::null(),
                            );
                            if hwnd == 0 {
                                send(out, Frame::new(RES_EDITOR, vec![0]));
                                continue;
                            }
                            EDITOR.with(|e| *e.borrow_mut() = Some(Arc::clone(ed)));
                            ed.set_scale(GetDpiForWindow(hwnd) as f64 / 96.0);
                            let Some(id) = std::num::NonZeroIsize::new(hwnd) else {
                                continue;
                            };
                            let handle = raw_window_handle::RawWindowHandle::Win32(
                                raw_window_handle::Win32WindowHandle::new(id),
                            );
                            let opened = ed.open(handle);
                            eprintln!("[hardwave bridge] the plug-in's window opened: {opened}");
                            if opened {
                                if let Some((w, h)) = ed.size() {
                                    fit(hwnd, w, h);
                                }
                                ShowWindow(hwnd, SW_SHOW);
                                SetForegroundWindow(hwnd);
                                send(out, Frame::new(RES_EDITOR, vec![1]));
                            } else {
                                EDITOR.with(|e| e.borrow_mut().take());
                                DestroyWindow(hwnd);
                                hwnd = 0;
                                send(out, Frame::new(RES_EDITOR, vec![0]));
                            }
                        }
                        Ok(UiCommand::Close) => close(&mut hwnd),
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            close(&mut hwnd);
                            return;
                        }
                    }
                }
                send_moves(out, gui_log);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat, TransportInfo,
    };
    use hardwave_midi::MidiEvent;
    use std::time::{Duration, Instant};

    /// A plug-in that doubles its input, puts 1.0 where a note starts,
    /// remembers the tempo it was told, and has a knob already moved in
    /// its "window".
    struct Fake {
        desc: PluginDescriptor,
        gui_log: SharedEditLog,
        tempo: Arc<Mutex<f64>>,
        cutoff: f64,
    }

    impl HostedPlugin for Fake {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
            Ok(())
        }
        fn deactivate(&mut self) {}
        fn process(
            &mut self,
            inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            n: usize,
        ) {
            for (ch, out) in outputs.iter_mut().enumerate() {
                out.clear();
                out.extend(inputs[ch][..n].iter().map(|s| s * 2.0));
            }
            for ev in midi_in {
                if let MidiEvent::NoteOn { timing, .. } = ev {
                    outputs[0][*timing as usize] = 1.0;
                }
            }
        }
        fn get_parameter_count(&self) -> u32 {
            2
        }
        fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
            Some(ParameterInfo {
                id: 10 + index,
                name: ["Cutoff", "Drive"][index as usize].into(),
                default_value: 0.5,
                min: 0.0,
                max: 1.0,
                unit: String::new(),
                automatable: true,
            })
        }
        fn get_parameter_value(&self, id: u32) -> f64 {
            if id == 10 {
                self.cutoff
            } else {
                0.0
            }
        }
        fn set_parameter_value(&mut self, id: u32, value: f64) {
            if id == 10 {
                self.cutoff = value;
            }
        }
        fn get_state(&self) -> Vec<u8> {
            vec![7]
        }
        fn set_state(&mut self, _s: &[u8]) -> Result<(), String> {
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
        fn gui_edit_log(&self) -> Option<SharedEditLog> {
            Some(Arc::clone(&self.gui_log))
        }
        fn set_transport(&mut self, transport: TransportInfo) {
            *self.tempo.lock().unwrap() = transport.tempo;
        }
    }

    #[test]
    fn the_child_lists_parameters_plays_notes_and_passes_on_its_window_s_knob_moves() {
        let (from_host, to_child) = std::io::pipe().expect("pipe");
        let (from_child, to_host) = std::io::pipe().expect("pipe");
        let gui_log: SharedEditLog = Default::default();
        gui_log.push(11, 0.75);
        let tempo = Arc::new(Mutex::new(0.0));
        let fake = Fake {
            desc: PluginDescriptor {
                id: "fake".into(),
                name: "Fake".into(),
                vendor: String::new(),
                version: String::new(),
                format: PluginFormat::Clap,
                path: "fake.clap".into(),
                category: PluginCategory::Instrument,
                num_inputs: 2,
                num_outputs: 2,
                has_midi_input: true,
                has_editor: false,
            },
            gui_log,
            tempo: Arc::clone(&tempo),
            cutoff: 0.25,
        };
        let child = std::thread::spawn(move || serve(Box::new(fake), from_host, to_host, "Fake"));
        let mut to_child = to_child;
        let mut from_child = BufReader::new(from_child);
        // The next frame of a kind; the others seen on the way are kept.
        let mut seen = Vec::new();
        let mut next = |kind: u32, seen: &mut Vec<Frame>| -> Frame {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "no frame of kind {kind}");
                let frame = Frame::read(&mut from_child).expect("read").expect("frame");
                if frame.kind == kind {
                    return frame;
                }
                seen.push(frame);
            }
        };

        Frame::new(REQ_PARAMS, Vec::new())
            .write(&mut to_child)
            .unwrap();
        let params = next(RES_PARAMS, &mut seen).read_params().expect("list");
        let names: Vec<_> = params
            .iter()
            .map(|(p, v)| (p.id, p.name.clone(), *v))
            .collect();
        assert_eq!(
            names,
            vec![(10, "Cutoff".into(), 0.25), (11, "Drive".into(), 0.0)]
        );

        Frame::activate(48_000.0, 256).write(&mut to_child).unwrap();
        next(RES_OK, &mut seen);
        let input = vec![0.25f32; 64];
        let note = [MidiEvent::NoteOn {
            timing: 5,
            channel: 0,
            note: 60,
            velocity: 1.0,
        }];
        let transport = TransportInfo {
            tempo: 174.0,
            playing: true,
            ..TransportInfo::default()
        };
        let mut payload = Vec::new();
        Frame::write_process(&mut payload, &input, &input, None, &note, &transport);
        Frame::new(REQ_PROCESS, payload)
            .write(&mut to_child)
            .unwrap();
        let audio = next(RES_AUDIO, &mut seen);
        let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
        assert_eq!(audio.read_audio(&mut l, &mut r), 64);
        assert_eq!((l[0], r[0]), (0.5, 0.5), "the plug-in played the block");
        assert_eq!(l[5], 1.0, "and the note, on its sample");
        assert_eq!(*tempo.lock().unwrap(), 174.0, "with the song's tempo");

        // The knob moved in its window comes back on its own.
        let moved = seen
            .iter()
            .find(|f| f.kind == RES_PARAM_MOVES)
            .cloned()
            .unwrap_or_else(|| next(RES_PARAM_MOVES, &mut seen));
        assert_eq!(moved.read_param_moves(), vec![(11, 0.75)]);

        // No window here: asked for one, it says so.
        Frame::open_editor(0).write(&mut to_child).unwrap();
        assert_eq!(next(RES_EDITOR, &mut seen).payload, vec![0]);

        Frame::new(REQ_SHUTDOWN, Vec::new())
            .write(&mut to_child)
            .unwrap();
        assert_eq!(child.join().expect("child"), 0);
    }

    /// A plug-in that takes a knob the way VST3 and CLAP ones do: the value
    /// waits until a block or a flush hands it over, and nothing is handed
    /// over while the plug-in is not active. Its state is the value it has
    /// taken. It counts how often it is activated and put down.
    struct Waits {
        desc: PluginDescriptor,
        waiting: Option<f64>,
        taken: f64,
        active: bool,
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl HostedPlugin for Waits {
        fn descriptor(&self) -> &PluginDescriptor {
            &self.desc
        }
        fn activate(&mut self, _sr: f64, _max: u32) -> Result<(), String> {
            assert!(!self.active, "activated twice without being put down");
            self.active = true;
            self.log.lock().unwrap().push("activate");
            Ok(())
        }
        fn deactivate(&mut self) {
            self.active = false;
            self.log.lock().unwrap().push("deactivate");
        }
        fn process(
            &mut self,
            _inputs: &[&[f32]],
            outputs: &mut [Vec<f32>],
            _midi_in: &[MidiEvent],
            _midi_out: &mut Vec<MidiEvent>,
            n: usize,
        ) {
            self.flush_parameters();
            for out in outputs.iter_mut() {
                out.clear();
                out.resize(n, self.taken as f32);
            }
        }
        fn get_parameter_count(&self) -> u32 {
            0
        }
        fn get_parameter_info(&self, _index: u32) -> Option<ParameterInfo> {
            None
        }
        fn get_parameter_value(&self, _id: u32) -> f64 {
            self.taken
        }
        fn set_parameter_value(&mut self, _id: u32, value: f64) {
            self.waiting = Some(value);
        }
        fn flush_parameters(&mut self) {
            if self.active {
                if let Some(value) = self.waiting.take() {
                    self.taken = value;
                }
            }
        }
        fn get_state(&self) -> Vec<u8> {
            self.taken.to_le_bytes().to_vec()
        }
        fn set_state(&mut self, _s: &[u8]) -> Result<(), String> {
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

    /// A knob the host sets on a plug-in that has never played (a track
    /// with nothing on it yet, the song stopped) is in the state read back
    /// right after. The plug-in was not active, took nothing, and the state
    /// had the default: on the test machine WettBoi's Size and Decay set
    /// from the DAW read back as their defaults.
    #[test]
    fn a_knob_set_before_the_plug_in_ever_played_is_in_its_state() {
        let (from_host, to_child) = std::io::pipe().expect("pipe");
        let (from_child, to_host) = std::io::pipe().expect("pipe");
        let log = Arc::new(Mutex::new(Vec::new()));
        let plugin = Waits {
            desc: PluginDescriptor {
                id: "waits".into(),
                name: "Waits".into(),
                vendor: String::new(),
                version: String::new(),
                format: PluginFormat::Vst3,
                path: "waits.vst3".into(),
                category: PluginCategory::Effect,
                num_inputs: 2,
                num_outputs: 2,
                has_midi_input: false,
                has_editor: false,
            },
            waiting: None,
            taken: 0.5,
            active: false,
            log: Arc::clone(&log),
        };
        let child =
            std::thread::spawn(move || serve(Box::new(plugin), from_host, to_host, "Waits"));
        let mut to_child = to_child;
        let mut from_child = BufReader::new(from_child);
        let mut next = |kind: u32| -> Frame {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                assert!(Instant::now() < deadline, "no frame of kind {kind}");
                let frame = Frame::read(&mut from_child).expect("read").expect("frame");
                if frame.kind == kind {
                    return frame;
                }
            }
        };
        // No activation, no block: a knob, then the state.
        Frame::set_param(1, 0.73).write(&mut to_child).unwrap();
        Frame::new(REQ_GET_STATE, Vec::new())
            .write(&mut to_child)
            .unwrap();
        assert_eq!(
            next(RES_STATE).payload,
            0.73f64.to_le_bytes().to_vec(),
            "the state has the knob the host set"
        );
        // The host's own activation comes later and replaces the one made
        // for the knob, put down first.
        Frame::activate(44_100.0, 256).write(&mut to_child).unwrap();
        next(RES_OK);
        // While it plays, a knob rides in with the next block.
        let mut payload = Vec::new();
        Frame::write_process(
            &mut payload,
            &[0.0; 16],
            &[0.0; 16],
            None,
            &[],
            &TransportInfo::default(),
        );
        Frame::new(REQ_PROCESS, payload.clone())
            .write(&mut to_child)
            .unwrap();
        next(RES_AUDIO);
        Frame::set_param(1, 0.2).write(&mut to_child).unwrap();
        Frame::new(REQ_PROCESS, payload)
            .write(&mut to_child)
            .unwrap();
        let audio = next(RES_AUDIO);
        let (mut l, mut r) = (vec![0.0f32; 16], vec![0.0f32; 16]);
        audio.read_audio(&mut l, &mut r);
        assert_eq!(l[0], 0.2, "automation reaches the audio in the next block");
        Frame::new(REQ_SHUTDOWN, Vec::new())
            .write(&mut to_child)
            .unwrap();
        assert_eq!(child.join().expect("child"), 0);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["activate", "deactivate", "activate"],
            "activated for the knob, then put down and activated by the host"
        );
    }
}
