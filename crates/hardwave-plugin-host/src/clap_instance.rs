//! CLAP plugin instance — opens the binary via `libloading`,
//! resolves `clap_entry`, creates a `clap_plugin_t` via the plugin
//! factory, drives its lifecycle (`init`, `activate`,
//! `start_processing`, `process`, `stop_processing`, `deactivate`,
//! `destroy`), and exposes parameters + state via the `clap.params`
//! and `clap.state` extensions.
//!
//! Audio processing is a real call through to the plugin's `process`
//! function pointer with a populated `ClapProcess` block. MIDI events
//! from the host side are translated into CLAP note-on / note-off /
//! raw-MIDI events via a minimal in-memory queue.

use crate::clap_ffi::{
    build_static_host, ClapAudioBuffer, ClapEventHeader, ClapEventMidi, ClapEventNote,
    ClapEventNoteExpression, ClapEventParamValue, ClapEventTransport, ClapHost, ClapHostParams,
    ClapInputEvents, ClapIstream, ClapOstream, ClapOutputEvents, ClapParamInfo, ClapPlugin,
    ClapPluginEntry, ClapPluginFactory, ClapPluginGui, ClapPluginLatency, ClapPluginParams,
    ClapPluginState, ClapPluginTail, ClapProcess, ClapWindow, ClapWindowHandle,
    CLAP_BEATTIME_FACTOR, CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_EXPRESSION,
    CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON, CLAP_EVENT_PARAM_VALUE, CLAP_EVENT_TRANSPORT,
    CLAP_EXT_GUI, CLAP_EXT_HOST_PARAMS, CLAP_EXT_LATENCY, CLAP_EXT_PARAMS, CLAP_EXT_STATE,
    CLAP_EXT_TAIL, CLAP_NOTE_EXPRESSION_PAN, CLAP_NOTE_EXPRESSION_TUNING, CLAP_PROCESS_CONTINUE,
    CLAP_PROCESS_CONTINUE_IF_NOT_QUIET, CLAP_PROCESS_ERROR, CLAP_PROCESS_SLEEP, CLAP_PROCESS_TAIL,
    CLAP_SECTIME_FACTOR, CLAP_TRANSPORT_HAS_BEATS_TIMELINE, CLAP_TRANSPORT_HAS_SECONDS_TIMELINE,
    CLAP_TRANSPORT_HAS_TEMPO, CLAP_TRANSPORT_HAS_TIME_SIGNATURE, CLAP_TRANSPORT_IS_PLAYING,
};
use crate::types::*;
use parking_lot::Mutex;
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::Arc;

pub struct ClapPluginInstance {
    descriptor: PluginDescriptor,
    active: bool,
    processing: bool,
    sample_rate: f64,
    max_block: u32,
    library: Option<libloading::Library>,
    entry: *const ClapPluginEntry,
    plugin: *const ClapPlugin,
    // Host context must outlive the plugin; keep owned even though the
    // field isn't read directly after create_plugin returns.
    #[allow(dead_code)]
    host: Box<ClapHost>,
    // Boxed host-side context hung off `host.host_data`. The
    // `clap.host-params` callbacks read it back to reach the plugin +
    // the shared param queue. Heap-stable for the instance's lifetime.
    host_ctx: Box<ClapHostContext>,
    /// Where the parameter changes the plug-in reports from its audio side
    /// (`process` and `params.flush`) go: the app's edit log. Heap-stable,
    /// handed to the plug-in as the output event list's context.
    out_sink: Box<OutputSink>,
    cached_params: Vec<ParameterInfo>,
    /// Latency the plug-in reported when it was activated. The extension
    /// may only be asked around activation, so it is read there and kept.
    latency: u32,
    initialized: bool,
    /// GUI→audio parameter queue. `set_parameter_value` (host-driven:
    /// automation, generic knob UI, IPC) and the editor's GUI edits
    /// (captured via `request_flush`) push here; `process` drains it and
    /// injects `CLAP_EVENT_PARAM_VALUE` events into the plugin each block.
    /// Shared with a separate editor instance via `load_with_shared_pending`.
    pending_params: SharedParamQueue,
    gui_edit_log: SharedEditLog,
    /// True between a successful `open_editor` and `close_editor` so we
    /// don't double-create / leak the plugin's GUI.
    gui_open: bool,
    /// Where the song is. Every block passed a null transport, so a
    /// tempo-synced delay, LFO or arp had no tempo to sync to.
    transport: TransportInfo,
    /// The window side of this instance, shared with the app's main thread
    /// (see `HostedPlugin::editor`).
    editor: Arc<Mutex<ClapEditorSide>>,
    /// The plug-in's own presets, found through its preset-discovery
    /// factory the first time they are asked for.
    presets: std::sync::OnceLock<Vec<crate::clap_presets::ClapPreset>>,
    /// Kept from block to block, so processing allocates nothing: copies
    /// of the inputs (a plug-in may write to them), the channel pointers,
    /// and the block's events. All of it used to be made and freed again
    /// every block, on the audio thread.
    input_scratch: Vec<Vec<f32>>,
    input_ptrs: Vec<*mut f32>,
    output_ptrs: Vec<*mut f32>,
    event_queue: Box<EncodedEventQueue>,
    /// The plug-in's `clap.tail` extension, or null when it has none.
    /// Found when it is activated.
    tail_ext: *const ClapPluginTail,
    /// What the plug-in's last `process` returned.
    last_status: i32,
    /// The host's word that the next block's input is exact silence, sent
    /// as the input buffer's constant mask.
    input_silent: bool,
}

/// What the main thread needs for this instance's window. CLAP has the
/// host call the gui extension from the main thread while the plug-in
/// processes on the audio thread, so the playing instance can show it.
struct ClapEditorSide {
    /// Null once the instance has gone.
    plugin: *const ClapPlugin,
    gui: *const ClapPluginGui,
    name: String,
    open: bool,
    /// What the plug-in's `request_resize` calls, shared with the host
    /// context it reaches through.
    resize_request: SharedResizeRequest,
}

type SharedResizeRequest = Arc<Mutex<Option<crate::types::ResizeRequest>>>;

/// The host resized the window: let the gui round the size to what it can
/// draw and take it. None when it cannot be resized.
///
/// # Safety
/// `gui` and `plugin` are alive, on the main thread.
unsafe fn gui_resize(
    gui: *const ClapPluginGui,
    plugin: *const ClapPlugin,
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    if !((*gui).can_resize)(plugin) {
        return None;
    }
    let (mut w, mut h) = (width, height);
    ((*gui).adjust_size)(plugin, &mut w, &mut h);
    (w > 0 && h > 0 && ((*gui).set_size)(plugin, w, h)).then_some((w, h))
}

// SAFETY: the pointers are the plug-in's own, valid until the instance
// clears them under the lock in its Drop; every call through them is
// made on the main thread, as CLAP asks for the gui extension.
unsafe impl Send for ClapEditorSide {}

/// The handle the app keeps for an instance's window.
struct ClapEditor(Arc<Mutex<ClapEditorSide>>);

impl PluginEditor for ClapEditor {
    fn open(&self, parent: raw_window_handle::RawWindowHandle) -> bool {
        let mut side = self.0.lock();
        if side.plugin.is_null() || side.gui.is_null() {
            return false;
        }
        if side.open {
            return true;
        }
        let Some(window) = clap_window(parent) else {
            return false;
        };
        // SAFETY: both pointers are alive while the lock is held (see above).
        side.open = unsafe { open_embedded_gui(&*side.gui, side.plugin, &window, &side.name) };
        side.open
    }

    fn size(&self) -> Option<(u32, u32)> {
        let side = self.0.lock();
        if !side.open || side.plugin.is_null() {
            return None;
        }
        let (mut w, mut h) = (0u32, 0u32);
        let ok = unsafe { ((*side.gui).get_size)(side.plugin, &mut w, &mut h) };
        (ok && w > 0 && h > 0).then_some((w, h))
    }

    fn close(&self) {
        let mut side = self.0.lock();
        if side.open && !side.plugin.is_null() {
            unsafe {
                ((*side.gui).hide)(side.plugin);
                ((*side.gui).destroy)(side.plugin);
            }
        }
        side.open = false;
    }

    fn resize(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        let side = self.0.lock();
        if !side.open || side.plugin.is_null() {
            return None;
        }
        // SAFETY: alive while the lock is held (see above).
        unsafe { gui_resize(side.gui, side.plugin, width, height) }
    }

    fn set_scale(&self, scale: f64) {
        let side = self.0.lock();
        if side.open && !side.plugin.is_null() {
            unsafe { ((*side.gui).set_scale)(side.plugin, scale) };
        }
    }

    fn set_resize_request(&self, request: crate::types::ResizeRequest) {
        *self.0.lock().resize_request.lock() = Some(request);
    }
}

/// The CLAP window description for a host window handle; None for window
/// systems CLAP does not define (Wayland).
fn clap_window(parent: raw_window_handle::RawWindowHandle) -> Option<ClapWindow> {
    use raw_window_handle::RawWindowHandle as Rwh;
    let (api, handle): (&[u8], ClapWindowHandle) = match parent {
        #[cfg(target_os = "windows")]
        Rwh::Win32(h) => (
            crate::clap_ffi::CLAP_WINDOW_API_WIN32,
            ClapWindowHandle {
                win32: h.hwnd.get() as *mut c_void,
            },
        ),
        #[cfg(target_os = "macos")]
        Rwh::AppKit(h) => (
            crate::clap_ffi::CLAP_WINDOW_API_COCOA,
            ClapWindowHandle {
                cocoa: h.ns_view.as_ptr(),
            },
        ),
        #[cfg(target_os = "linux")]
        Rwh::Xlib(h) => (
            crate::clap_ffi::CLAP_WINDOW_API_X11,
            ClapWindowHandle {
                // `XlibWindowHandle.window` is an X11 `Window`
                // (`c_ulong` = u64 on the 64-bit Linux we ship).
                x11: h.window,
            },
        ),
        #[cfg(target_os = "linux")]
        Rwh::Xcb(h) => (
            crate::clap_ffi::CLAP_WINDOW_API_X11,
            ClapWindowHandle {
                x11: u64::from(h.window.get()),
            },
        ),
        _ => return None,
    };
    Some(ClapWindow {
        api: api.as_ptr() as *const c_char,
        handle,
    })
}

/// Host-side context reachable from the C callbacks via
/// `ClapHost::host_data`. Not `repr(C)` — C only ever holds it as an
/// opaque `*mut c_void` and hands it straight back to us.
struct ClapHostContext {
    plugin: *const ClapPlugin,
    /// What `clap.gui`'s `request_resize` calls (see `ResizeRequest`).
    resize_request: SharedResizeRequest,
    pending: SharedParamQueue,
    /// Every GUI edit again, drained by the app for automation recording.
    gui_edit_log: SharedEditLog,
    /// The plug-in is active. CLAP lets `params.flush` run on the main
    /// thread only while it is not; while it is, only on the audio thread
    /// and never during `process`.
    active: AtomicBool,
    /// A window thread is running `params.flush` (see `request_flush`);
    /// activating waits for it, so the two never overlap.
    window_flush: AtomicBool,
    /// The plug-in asked for a flush that is not done yet. While it is
    /// active, the audio thread answers it with the next block or flush.
    flush_requested: AtomicBool,
}

/// The output event list's context on the audio side: parameter changes
/// the plug-in reports there go to the app's edit log, except those of
/// read-only parameters (meters a plug-in publishes as parameters).
struct OutputSink {
    log: SharedEditLog,
    /// Sorted.
    read_only: Vec<u32>,
}

/// CLAP_PARAM_IS_AUTOMATABLE.
const CLAP_PARAM_IS_AUTOMATABLE: u32 = 1 << 5;

/// CLAP_PARAM_IS_READONLY.
const CLAP_PARAM_IS_READONLY: u32 = 1 << 3;

// SAFETY: all pointers point into the plugin binary (lifetime bound
// by `library`) or into host-owned boxes (`host`, `host_ctx`).
// The CLAP contract allows the host to call the plugin's function
// table from a single audio thread; we uphold that at the call sites.
unsafe impl Send for ClapPluginInstance {}

impl ClapPluginInstance {
    pub fn load(descriptor: PluginDescriptor) -> Result<Self, String> {
        Self::load_inner(descriptor, None, None)
    }

    /// Load the plug-in but reuse the supplied parameter queue instead of
    /// creating a fresh one. Used by the floating-editor path so the
    /// editor instance and the chain instance share one queue: the
    /// editor's GUI edits (captured via `request_flush` → `params.flush`)
    /// land in the same queue the chain instance drains in `process`,
    /// so knob movements in the editor reach the live audio.
    ///
    /// `shared_log` is the chain instance's copy of those edits, the one
    /// automation recording reads; see the VST3 loader.
    pub fn load_with_shared_pending(
        descriptor: PluginDescriptor,
        shared: SharedParamQueue,
        shared_log: Option<SharedEditLog>,
    ) -> Result<Self, String> {
        Self::load_inner(descriptor, Some(shared), shared_log)
    }

    fn load_inner(
        descriptor: PluginDescriptor,
        shared_pending: Option<SharedParamQueue>,
        shared_log: Option<SharedEditLog>,
    ) -> Result<Self, String> {
        let path = descriptor.path.clone();
        if !path.exists() {
            return Err(format!("CLAP binary not found: {}", path.display()));
        }
        let lib = unsafe { crate::load_plugin_library(&path) }
            .map_err(|e| format!("dlopen {}: {e}", path.display()))?;
        let entry_sym: libloading::Symbol<*const ClapPluginEntry> = unsafe {
            lib.get(b"clap_entry\0")
        }
        .map_err(|_| format!("{}: not a CLAP binary (clap_entry missing)", path.display()))?;
        let entry = *entry_sym;
        if entry.is_null() {
            return Err(format!("{}: clap_entry symbol is null", path.display()));
        }
        let path_c = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "CLAP path contains interior NUL".to_string())?;
        let init_ok = unsafe { ((*entry).init)(path_c.as_ptr()) };
        if !init_ok {
            return Err(format!(
                "{}: clap_entry.init returned false",
                path.display()
            ));
        }
        let factory_id =
            CString::new("clap.plugin-factory").map_err(|_| "invalid factory id".to_string())?;
        let factory_ptr = unsafe { ((*entry).get_factory)(factory_id.as_ptr()) };
        if factory_ptr.is_null() {
            unsafe { ((*entry).deinit)() };
            return Err(format!("{}: no plugin factory", path.display()));
        }
        let factory = factory_ptr as *const ClapPluginFactory;

        // Shared GUI→audio param queue (fresh, or the chain's queue when
        // this is an editor instance). The host context carries it so the
        // `clap.host-params.request_flush` callback can push captured GUI
        // edits into it. Both the Box<ClapHostContext> and Box<ClapHost>
        // are heap-stable, so the raw pointers we hand the plugin stay
        // valid even after the boxes move into `Self`.
        let pending_params: SharedParamQueue = shared_pending.unwrap_or_default();
        let gui_edit_log: SharedEditLog = shared_log.unwrap_or_default();
        let resize_request: SharedResizeRequest = Arc::new(Mutex::new(None));
        let mut host_ctx = Box::new(ClapHostContext {
            plugin: std::ptr::null(),
            resize_request: Arc::clone(&resize_request),
            pending: Arc::clone(&pending_params),
            gui_edit_log: Arc::clone(&gui_edit_log),
            active: AtomicBool::new(false),
            window_flush: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
        });
        let out_sink = Box::new(OutputSink {
            log: Arc::clone(&gui_edit_log),
            read_only: Vec::new(),
        });
        let mut host = Box::new(build_static_host());
        host.host_data = &*host_ctx as *const ClapHostContext as *mut c_void;
        host.get_extension = clap_host_get_extension;

        // The DAW keeps CLAP plug-ins under "clap:<id>"; the factory knows
        // them by the id alone. Passing the DAW's form made every CLAP
        // plug-in come back null, so none of them ever loaded.
        let plugin_id_c = CString::new(factory_plugin_id(&descriptor.id).as_bytes())
            .map_err(|_| "plugin id contains interior NUL".to_string())?;
        let plugin = unsafe {
            ((*factory).create_plugin)(factory, &*host as *const _, plugin_id_c.as_ptr())
        };
        if plugin.is_null() {
            unsafe { ((*entry).deinit)() };
            return Err(format!(
                "{}: create_plugin returned null for id {}",
                path.display(),
                descriptor.id
            ));
        }
        // Now that the plugin exists, point the host context at it so
        // `request_flush` can call `params.flush` on it.
        host_ctx.plugin = plugin;
        // Initialize the plugin.
        let init_ok = unsafe { ((*plugin).init)(plugin) };
        if !init_ok {
            unsafe {
                ((*plugin).destroy)(plugin);
                ((*entry).deinit)();
            }
            return Err(format!(
                "{}: clap_plugin.init returned false",
                path.display()
            ));
        }

        let mut me = Self {
            descriptor,
            active: false,
            processing: false,
            sample_rate: 48_000.0,
            max_block: 0,
            library: Some(lib),
            entry,
            plugin,
            host,
            host_ctx,
            out_sink,
            cached_params: Vec::new(),
            latency: 0,
            initialized: true,
            pending_params,
            gui_edit_log,
            gui_open: false,
            transport: TransportInfo::default(),
            editor: Arc::new(Mutex::new(ClapEditorSide {
                plugin,
                gui: std::ptr::null(),
                name: String::new(),
                open: false,
                resize_request,
            })),
            presets: std::sync::OnceLock::new(),
            input_scratch: Vec::new(),
            input_ptrs: Vec::with_capacity(8),
            output_ptrs: Vec::with_capacity(8),
            event_queue: Box::new(EncodedEventQueue {
                events: Vec::with_capacity(MAX_BLOCK_EVENTS),
            }),
            tail_ext: std::ptr::null(),
            last_status: CLAP_PROCESS_ERROR,
            input_silent: false,
        };
        me.refresh_params();
        {
            let mut side = me.editor.lock();
            side.gui = me.gui_ext().unwrap_or(std::ptr::null());
            side.name = me.descriptor.name.clone();
        }
        Ok(me)
    }

    /// Resolve the `clap.gui` extension, or `None` if the plugin has no
    /// embeddable editor.
    fn gui_ext(&self) -> Option<*const ClapPluginGui> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_GUI[..CLAP_EXT_GUI.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginGui)
        }
    }

    /// Start the event list over with the waiting parameter changes
    /// (automation, generic knob UI, the set_plugin_parameter IPC, and
    /// editor GUI edits captured via request_flush) as
    /// CLAP_EVENT_PARAM_VALUE events at its head. Never waits on the
    /// threads that push (the audio thread used to lock their queue here
    /// every block), and what does not fit in this block's list stays
    /// queued for the next one instead of being dropped.
    fn take_param_events(&mut self) {
        let events = &mut self.event_queue.events;
        events.clear();
        self.pending_params
            .drain(MAX_BLOCK_EVENTS, |param_id, value| {
                events.push(EncodedEvent::ParamValue(ClapEventParamValue {
                    header: ClapEventHeader {
                        size: std::mem::size_of::<ClapEventParamValue>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        event_type: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id,
                    cookie: std::ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value,
                }));
            });
    }

    fn params_ext(&self) -> Option<*const ClapPluginParams> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_PARAMS[..CLAP_EXT_PARAMS.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginParams)
        }
    }

    fn latency_ext(&self) -> Option<*const ClapPluginLatency> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_LATENCY[..CLAP_EXT_LATENCY.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginLatency)
        }
    }

    fn state_ext(&self) -> Option<*const ClapPluginState> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_STATE[..CLAP_EXT_STATE.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginState)
        }
    }

    fn refresh_params(&mut self) {
        self.cached_params.clear();
        self.out_sink.read_only.clear();
        let Some(params) = self.params_ext() else {
            return;
        };
        let count = unsafe { ((*params).count)(self.plugin) };
        for i in 0..count {
            let mut info: ClapParamInfo = unsafe { std::mem::zeroed() };
            let ok = unsafe { ((*params).get_info)(self.plugin, i, &mut info) };
            if !ok {
                continue;
            }
            let name = clap_fixed_string_to_rust(&info.name);
            if info.flags & CLAP_PARAM_IS_READONLY != 0 {
                self.out_sink.read_only.push(info.id);
            }
            self.cached_params.push(ParameterInfo {
                id: info.id,
                name,
                default_value: info.default_value,
                min: info.min_value,
                max: info.max_value,
                unit: String::new(),
                // Bit 5. Bit 0 (read before) is CLAP_PARAM_IS_STEPPED, so
                // every stepped switch counted as automatable and every
                // knob did not.
                automatable: info.flags & CLAP_PARAM_IS_AUTOMATABLE != 0,
            });
        }
        self.out_sink.read_only.sort_unstable();
    }

    /// The output event list for the audio side (see `OutputSink`).
    fn output_events(&self) -> ClapOutputEvents {
        ClapOutputEvents {
            ctx: &*self.out_sink as *const OutputSink as *mut c_void,
            try_push: output_sink_push,
        }
    }
}

fn clap_fixed_string_to_rust(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl Drop for ClapPluginInstance {
    fn drop(&mut self) {
        // A window opened on this instance from the main thread goes first,
        // and the main thread's handle is left with nothing to call into.
        {
            let mut side = self.editor.lock();
            if side.open && !side.gui.is_null() && !self.plugin.is_null() {
                unsafe {
                    ((*side.gui).hide)(self.plugin);
                    ((*side.gui).destroy)(self.plugin);
                }
            }
            side.open = false;
            side.plugin = std::ptr::null();
        }
        // Tear the editor GUI down first — its lifetime is nested inside
        // the plugin's, so it must be destroyed before `plugin.destroy`.
        if self.gui_open {
            if let Some(gui) = self.gui_ext() {
                unsafe {
                    ((*gui).hide)(self.plugin);
                    ((*gui).destroy)(self.plugin);
                }
            }
            self.gui_open = false;
        }
        if self.processing && !self.plugin.is_null() {
            unsafe { ((*self.plugin).stop_processing)(self.plugin) };
        }
        if self.active && !self.plugin.is_null() {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
        }
        if self.initialized && !self.plugin.is_null() {
            unsafe { ((*self.plugin).destroy)(self.plugin) };
        }
        if !self.entry.is_null() {
            unsafe { ((*self.entry).deinit)() };
        }
        self.library.take();
    }
}

/// The song position as CLAP wants it.
fn clap_transport(t: &TransportInfo) -> ClapEventTransport {
    let (num, den) = (t.time_sig.0.max(1), t.time_sig.1.max(1));
    // Beats are quarter notes; a bar holds num * 4 / den of them.
    let bar_quarters = num as f64 * 4.0 / den as f64;
    let bar = (t.position_beats / bar_quarters).floor();
    let seconds = if t.sample_rate > 0.0 {
        t.position_samples as f64 / t.sample_rate
    } else {
        0.0
    };
    ClapEventTransport {
        header: ClapEventHeader {
            size: std::mem::size_of::<ClapEventTransport>() as u32,
            time: 0,
            space_id: CLAP_CORE_EVENT_SPACE_ID,
            event_type: CLAP_EVENT_TRANSPORT,
            flags: 0,
        },
        flags: CLAP_TRANSPORT_HAS_TEMPO
            | CLAP_TRANSPORT_HAS_BEATS_TIMELINE
            | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
            | CLAP_TRANSPORT_HAS_TIME_SIGNATURE
            | if t.playing {
                CLAP_TRANSPORT_IS_PLAYING
            } else {
                0
            },
        song_pos_beats: (t.position_beats * CLAP_BEATTIME_FACTOR) as i64,
        song_pos_seconds: (seconds * CLAP_SECTIME_FACTOR) as i64,
        tempo: t.tempo,
        tempo_inc: 0.0,
        loop_start_beats: 0,
        loop_end_beats: 0,
        loop_start_seconds: 0,
        loop_end_seconds: 0,
        bar_start: (bar * bar_quarters * CLAP_BEATTIME_FACTOR) as i64,
        bar_number: bar as i32,
        tsig_num: num as u16,
        tsig_denom: den as u16,
    }
}

impl ClapPluginInstance {
    fn own_presets(&self) -> &[crate::clap_presets::ClapPreset] {
        self.presets.get_or_init(|| {
            if self.entry.is_null() {
                return Vec::new();
            }
            // SAFETY: the entry stays initialised for the instance's life.
            unsafe {
                crate::clap_presets::discover(self.entry, factory_plugin_id(&self.descriptor.id))
            }
        })
    }
}

impl HostedPlugin for ClapPluginInstance {
    fn factory_presets(&self) -> Vec<String> {
        self.own_presets().iter().map(|p| p.name.clone()).collect()
    }

    /// Reads a file, so it is for an instance off the audio thread: the
    /// app loads the preset into a throwaway instance and gives the
    /// playing one the state that comes out.
    fn load_factory_preset(&mut self, index: usize) -> Result<(), String> {
        let preset = self
            .own_presets()
            .get(index)
            .cloned()
            .ok_or("no such preset")?;
        if self.plugin.is_null() {
            return Err("the plug-in is not loaded".into());
        }
        // SAFETY: a live, initialised plug-in.
        unsafe { crate::clap_presets::load(self.plugin, &preset) }
    }

    fn set_transport(&mut self, transport: TransportInfo) {
        self.transport = transport;
    }

    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate.max(1.0);
        self.max_block = max_block_size.max(1);
        // The input copies, made here at their full size rather than per block.
        self.input_scratch = (0..2)
            .map(|_| Vec::with_capacity(self.max_block as usize))
            .collect();
        // From here a flush the window asks for waits for the audio thread,
        // and one a window thread is running now finishes first: flush and
        // activate never overlap.
        self.host_ctx.active.store(true, AtomicOrdering::SeqCst);
        while self.host_ctx.window_flush.load(AtomicOrdering::SeqCst) {
            std::thread::yield_now();
        }
        let ok =
            unsafe { ((*self.plugin).activate)(self.plugin, self.sample_rate, 1, self.max_block) };
        if !ok {
            self.host_ctx.active.store(false, AtomicOrdering::SeqCst);
            return Err("clap_plugin.activate returned false".into());
        }
        self.active = true;
        // Asked right after activating, as the extension wants. The host
        // reported zero for every CLAP, so a CLAP with lookahead played late
        // against the rest of the song.
        self.latency = self
            .latency_ext()
            .map(|ext| unsafe { ((*ext).get)(self.plugin) })
            .unwrap_or(0);
        // The tail is asked while playing, from the audio thread, which the
        // extension allows; the lookup is made here, once. The id is the
        // constant itself, so finding it allocates nothing.
        self.tail_ext = unsafe {
            ((*self.plugin).get_extension)(self.plugin, CLAP_EXT_TAIL.as_ptr() as *const c_char)
        } as *const ClapPluginTail;
        self.last_status = CLAP_PROCESS_ERROR;
        let start_ok = unsafe { ((*self.plugin).start_processing)(self.plugin) };
        if !start_ok {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
            self.active = false;
            self.host_ctx.active.store(false, AtomicOrdering::SeqCst);
            return Err("clap_plugin.start_processing returned false".into());
        }
        self.processing = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.processing {
            unsafe { ((*self.plugin).stop_processing)(self.plugin) };
            self.processing = false;
        }
        if self.active {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
            self.active = false;
            self.host_ctx.active.store(false, AtomicOrdering::SeqCst);
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        if !self.processing || self.plugin.is_null() || num_samples == 0 {
            // Fail open — pass audio through if the plugin isn't ready.
            self.last_status = CLAP_PROCESS_ERROR;
            for (ch, output) in outputs.iter_mut().enumerate() {
                if ch < inputs.len() {
                    output.clear();
                    output.extend_from_slice(&inputs[ch][..num_samples.min(inputs[ch].len())]);
                }
            }
            return;
        }
        // Build input channel pointers. CLAP wants `*mut *mut f32`.
        let channel_count = inputs.len().min(outputs.len()).max(1) as u32;
        // A copy per channel the plug-in is told about, each exactly the
        // block's length: a short input (or none, for the one channel an
        // instrument is still given) was copied short, and the plug-in read
        // `num_samples` past its end.
        let n_in = inputs.len().max(channel_count as usize);
        if self.input_scratch.len() < n_in {
            self.input_scratch.resize_with(n_in, Vec::new);
        }
        for (i, copy) in self.input_scratch.iter_mut().take(n_in).enumerate() {
            copy.clear();
            if let Some(input) = inputs.get(i) {
                copy.extend_from_slice(&input[..num_samples.min(input.len())]);
            }
            copy.resize(num_samples, 0.0);
        }
        // Ensure outputs have enough capacity.
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        self.input_ptrs.clear();
        self.input_ptrs.extend(
            self.input_scratch
                .iter_mut()
                .take(n_in)
                .map(|v| v.as_mut_ptr()),
        );
        self.output_ptrs.clear();
        self.output_ptrs
            .extend(outputs.iter_mut().map(|v| v.as_mut_ptr()));
        // Exact silence in is constant on every channel: a plug-in that
        // reads the mask can skip its work on it.
        let constant_mask = if self.input_silent {
            if channel_count >= 64 {
                u64::MAX
            } else {
                (1u64 << channel_count) - 1
            }
        } else {
            0
        };
        let audio_in = ClapAudioBuffer {
            data32: self.input_ptrs.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count,
            latency: 0,
            constant_mask,
        };
        let mut audio_out = ClapAudioBuffer {
            data32: self.output_ptrs.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count,
            latency: 0,
            constant_mask: 0,
        };

        self.take_param_events();
        let events = &mut self.event_queue.events;
        encode_midi_events(midi_in, events);
        let events_ctx: *mut EncodedEventQueue = &mut *self.event_queue;

        let input_events = ClapInputEvents {
            ctx: events_ctx as *mut c_void,
            size: event_queue_size,
            get: event_queue_get,
        };
        // Changes the plug-in reports (its own window's, while it plays) go
        // to the app's edit log; they were thrown away.
        let output_events = self.output_events();
        // This block answers a flush the plug-in asked for: CLAP lets the
        // host answer one with `process`.
        self.host_ctx
            .flush_requested
            .store(false, AtomicOrdering::Relaxed);

        let transport = clap_transport(&self.transport);
        let process = ClapProcess {
            steady_time: -1,
            frames_count: num_samples as u32,
            transport: &transport,
            audio_inputs: &audio_in,
            audio_outputs: &mut audio_out,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };

        let status = unsafe { ((*self.plugin).process)(self.plugin, &process) };
        self.last_status = status;

        if status == CLAP_PROCESS_ERROR {
            log::warn!(
                "CLAP plugin '{}' returned CLAP_PROCESS_ERROR",
                self.descriptor.id
            );
            for out in outputs.iter_mut() {
                for s in out.iter_mut() {
                    *s = 0.0;
                }
            }
        }
    }

    fn tail(&self) -> Tail {
        if self.tail_ext.is_null() || !self.processing {
            return Tail::Unknown;
        }
        // SAFETY: the extension came from this plug-in when it was
        // activated and lives as long as it does.
        let samples = unsafe { ((*self.tail_ext).get)(self.plugin) };
        if samples >= i32::MAX as u32 {
            Tail::Infinite
        } else {
            Tail::Samples(samples)
        }
    }

    fn process_hint(&self) -> ProcessHint {
        match self.last_status {
            CLAP_PROCESS_CONTINUE => ProcessHint::Continue,
            CLAP_PROCESS_CONTINUE_IF_NOT_QUIET => ProcessHint::ContinueIfNotQuiet,
            CLAP_PROCESS_TAIL => ProcessHint::Tail,
            CLAP_PROCESS_SLEEP => ProcessHint::Sleep,
            _ => ProcessHint::None,
        }
    }

    fn set_input_silent(&mut self, silent: bool) {
        self.input_silent = silent;
    }

    fn get_parameter_count(&self) -> u32 {
        self.cached_params.len() as u32
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        self.cached_params.get(index as usize).cloned()
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let Some(params) = self.params_ext() else {
            return 0.0;
        };
        let mut out = 0.0_f64;
        let ok = unsafe { ((*params).get_value)(self.plugin, id, &mut out) };
        if ok {
            out
        } else {
            0.0
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        // CLAP takes parameter changes as events, not synchronous setters.
        // Queue the change; `process` drains the queue and submits a
        // CLAP_EVENT_PARAM_VALUE at the start of the next block, which is
        // the plugin's own synchronization path. De-dupe consecutive
        // edits to the same param so a fast automation ramp or knob spin
        // doesn't pile up redundant entries between blocks: the queue
        // keeps the newest value per parameter, and pushing never waits.
        self.pending_params.push(id, value);
    }

    fn flush_parameters(&mut self) {
        // Inactive, the first block after activation takes them.
        // Also when the plug-in asked for a flush while active: its window's
        // edits then come out of here, on the audio thread.
        let asked = self.host_ctx.flush_requested.load(AtomicOrdering::Acquire);
        if !self.active || self.plugin.is_null() || (!self.pending_params.has_pending() && !asked) {
            return;
        }
        self.host_ctx
            .flush_requested
            .store(false, AtomicOrdering::Relaxed);
        // The extension's id as it is stored (it ends in a nul), so the
        // audio thread makes no string to ask for it.
        let params = unsafe {
            ((*self.plugin).get_extension)(self.plugin, CLAP_EXT_PARAMS.as_ptr() as *const c_char)
        } as *const ClapPluginParams;
        if params.is_null() {
            return;
        }
        // `params.flush` is CLAP's way to hand over parameter changes
        // outside a block, so the plug-in (and the state a save or a
        // read-back takes) has the value with the song stopped.
        self.take_param_events();
        let events_ctx: *mut EncodedEventQueue = &mut *self.event_queue;
        let input_events = ClapInputEvents {
            ctx: events_ctx as *mut c_void,
            size: event_queue_size,
            get: event_queue_get,
        };
        let output_events = self.output_events();
        unsafe { ((*params).flush)(self.plugin, &input_events, &output_events) };
        self.event_queue.events.clear();
    }

    fn get_state(&self) -> Vec<u8> {
        let Some(state) = self.state_ext() else {
            return Vec::new();
        };
        let buf: Box<StateWriter> = Box::new(StateWriter { bytes: Vec::new() });
        let ctx = Box::into_raw(buf);
        let stream = ClapOstream {
            ctx: ctx as *mut c_void,
            write: state_writer_write,
        };
        let ok = unsafe { ((*state).save)(self.plugin, &stream) };
        let written = unsafe { Box::from_raw(ctx) };
        if ok {
            written.bytes
        } else {
            Vec::new()
        }
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Some(state) = self.state_ext() else {
            return Ok(());
        };
        let reader: Box<StateReader> = Box::new(StateReader {
            bytes: bytes.to_vec(),
            cursor: 0,
        });
        let ctx = Box::into_raw(reader);
        let stream = ClapIstream {
            ctx: ctx as *mut c_void,
            read: state_reader_read,
        };
        let ok = unsafe { ((*state).load)(self.plugin, &stream) };
        let _reader = unsafe { Box::from_raw(ctx) };
        if ok {
            // Values set before this state (by the host or in the plug-in's
            // window) are older than it: handed over after it, they put the
            // old knobs back over a loaded preset.
            self.pending_params.discard();
            Ok(())
        } else {
            Err("clap_plugin_state.load returned false".into())
        }
    }

    fn latency_samples(&self) -> u32 {
        self.latency
    }

    fn open_editor(&mut self, parent: raw_window_handle::RawWindowHandle) -> bool {
        if self.gui_open {
            return true;
        }
        let Some(gui) = self.gui_ext() else {
            return false;
        };

        let Some(window) = clap_window(parent) else {
            return false;
        };
        // SAFETY: `gui` is the plug-in's own gui extension and `self.plugin`
        // the instance it belongs to, both alive for this call.
        if !unsafe { open_embedded_gui(&*gui, self.plugin, &window, &self.descriptor.name) } {
            return false;
        }
        self.gui_open = true;
        true
    }

    fn editor(&self) -> Option<Arc<dyn PluginEditor>> {
        (!self.editor.lock().gui.is_null())
            .then(|| Arc::new(ClapEditor(Arc::clone(&self.editor))) as Arc<dyn PluginEditor>)
    }

    fn editor_resize(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if !self.gui_open {
            return None;
        }
        let gui = self.gui_ext()?;
        // SAFETY: the plug-in's own gui extension, on the main thread.
        unsafe { gui_resize(gui, self.plugin, width, height) }
    }

    fn editor_set_scale(&mut self, scale: f64) {
        if self.gui_open {
            if let Some(gui) = self.gui_ext() {
                unsafe { ((*gui).set_scale)(self.plugin, scale) };
            }
        }
    }

    fn set_editor_resize_request(&mut self, request: crate::types::ResizeRequest) {
        *self.host_ctx.resize_request.lock() = Some(request);
    }

    fn editor_size(&self) -> Option<(u32, u32)> {
        if !self.gui_open {
            return None;
        }
        let gui = self.gui_ext()?;
        let (mut w, mut h) = (0u32, 0u32);
        let ok = unsafe { ((*gui).get_size)(self.plugin, &mut w, &mut h) };
        (ok && w > 0 && h > 0).then_some((w, h))
    }

    fn close_editor(&mut self) {
        if !self.gui_open {
            return;
        }
        if let Some(gui) = self.gui_ext() {
            unsafe {
                ((*gui).hide)(self.plugin);
                ((*gui).destroy)(self.plugin);
            }
        }
        self.gui_open = false;
    }

    fn has_editor(&self) -> bool {
        self.descriptor.has_editor
    }

    fn gui_edit_log(&self) -> Option<SharedEditLog> {
        Some(Arc::clone(&self.gui_edit_log))
    }

    fn pending_params(&self) -> Option<SharedParamQueue> {
        Some(Arc::clone(&self.pending_params))
    }
}

/// Put a plug-in's editor into the host window `window` (embedded, not
/// floating): create, parent, size, show. False when the plug-in cannot,
/// with anything it built torn down again.
///
/// # Safety
/// `gui` must be `plugin`'s gui extension and `window.api` a valid
/// NUL-terminated CLAP window API name.
unsafe fn open_embedded_gui(
    gui: &ClapPluginGui,
    plugin: *const ClapPlugin,
    window: &ClapWindow,
    name: &str,
) -> bool {
    if !(gui.is_api_supported)(plugin, window.api, false) {
        return false;
    }
    if !(gui.create)(plugin, window.api, false) {
        return false;
    }
    if !(gui.set_parent)(plugin, window) {
        (gui.destroy)(plugin);
        return false;
    }
    // Size the host window to the plugin's preferred size when it
    // reports one; ignore failures (the window stays default-sized).
    let mut w: u32 = 0;
    let mut h: u32 = 0;
    if (gui.get_size)(plugin, &mut w, &mut h) && w > 0 && h > 0 {
        let _ = (gui.set_size)(plugin, w, h);
    }
    // For an embedded view `show` is only a request: the view is already
    // in the host's window from set_parent. nih-plug, which our own
    // plug-ins are built on, answers false for embedded editors ("this
    // wouldn't make any sense for embedded editors"), and treating that as
    // a failure destroyed the editor it had just built, so no CLAP of ours
    // could show its window.
    if !(gui.show)(plugin) {
        log::debug!("{name}: gui.show answered false for an embedded view");
    }
    true
}

// ---------------------------------------------------------------------------
// Event queue bridging: host-side MidiEvent list → CLAP event queue
// ---------------------------------------------------------------------------

/// The most events one block hands a CLAP plug-in; the rest wait.
const MAX_BLOCK_EVENTS: usize = 1024;

/// A note's id for note expression: its channel and key, the same for its
/// on, its expressions and its off.
fn clap_note_id(channel: u8, note: u8) -> i32 {
    (channel as i32 & 0x0F) * 128 + (note as i32 & 0x7F)
}

enum EncodedEvent {
    NoteOn(ClapEventNote),
    NoteOff(ClapEventNote),
    Midi(ClapEventMidi),
    ParamValue(ClapEventParamValue),
    NoteExpression(ClapEventNoteExpression),
}

struct EncodedEventQueue {
    events: Vec<EncodedEvent>,
}

/// The block's notes and controllers as CLAP events, after whatever is
/// already in `events`. Pitch bend, pressure and poly aftertouch go as raw
/// MIDI; they used to be dropped, so a bend never reached a CLAP synth.
fn encode_midi_events(midi: &[hardwave_midi::MidiEvent], events: &mut Vec<EncodedEvent>) {
    let header = |size: usize, time: u32, event_type: u16| ClapEventHeader {
        size: size as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        event_type,
        flags: 0,
    };
    let raw = |time: u32, data: [u8; 3]| {
        EncodedEvent::Midi(ClapEventMidi {
            header: header(std::mem::size_of::<ClapEventMidi>(), time, CLAP_EVENT_MIDI),
            port_index: 0,
            data,
        })
    };
    for ev in midi {
        if events.len() >= MAX_BLOCK_EVENTS {
            break;
        }
        let event = match *ev {
            hardwave_midi::MidiEvent::NoteOn {
                timing,
                channel,
                note,
                velocity,
            } => EncodedEvent::NoteOn(ClapEventNote {
                header: header(
                    std::mem::size_of::<ClapEventNote>(),
                    timing,
                    CLAP_EVENT_NOTE_ON,
                ),
                note_id: clap_note_id(channel, note),
                port_index: 0,
                channel: channel as i16,
                key: note as i16,
                velocity: velocity as f64,
            }),
            hardwave_midi::MidiEvent::NoteOff {
                timing,
                channel,
                note,
                velocity,
            } => EncodedEvent::NoteOff(ClapEventNote {
                header: header(
                    std::mem::size_of::<ClapEventNote>(),
                    timing,
                    CLAP_EVENT_NOTE_OFF,
                ),
                note_id: clap_note_id(channel, note),
                port_index: 0,
                channel: channel as i16,
                key: note as i16,
                velocity: velocity as f64,
            }),
            hardwave_midi::MidiEvent::ControlChange {
                timing,
                channel,
                cc,
                value,
            } => raw(
                timing,
                [
                    0xB0 | (channel & 0x0F),
                    cc & 0x7F,
                    (value.clamp(0.0, 1.0) * 127.0).round() as u8,
                ],
            ),
            hardwave_midi::MidiEvent::PitchBend {
                timing,
                channel,
                value,
            } => {
                let bend = (((value.clamp(-1.0, 1.0) + 1.0) * 0.5) * 16383.0).round() as u16;
                raw(
                    timing,
                    [
                        0xE0 | (channel & 0x0F),
                        (bend & 0x7F) as u8,
                        (bend >> 7) as u8,
                    ],
                )
            }
            hardwave_midi::MidiEvent::ChannelPressure {
                timing,
                channel,
                pressure,
            } => raw(
                timing,
                [
                    0xD0 | (channel & 0x0F),
                    (pressure.clamp(0.0, 1.0) * 127.0) as u8,
                    0,
                ],
            ),
            hardwave_midi::MidiEvent::Aftertouch {
                timing,
                channel,
                note,
                pressure,
            } => raw(
                timing,
                [
                    0xA0 | (channel & 0x0F),
                    note & 0x7F,
                    (pressure.clamp(0.0, 1.0) * 127.0) as u8,
                ],
            ),
            hardwave_midi::MidiEvent::NoteExpression {
                timing,
                channel,
                note,
                kind,
                value,
            } => {
                let (expression_id, value) = match kind {
                    hardwave_midi::NoteExpressionKind::Pan => (
                        CLAP_NOTE_EXPRESSION_PAN,
                        ((value.clamp(-1.0, 1.0) + 1.0) * 0.5) as f64,
                    ),
                    hardwave_midi::NoteExpressionKind::Tuning => (
                        CLAP_NOTE_EXPRESSION_TUNING,
                        value.clamp(-120.0, 120.0) as f64,
                    ),
                };
                EncodedEvent::NoteExpression(ClapEventNoteExpression {
                    header: header(
                        std::mem::size_of::<ClapEventNoteExpression>(),
                        timing,
                        CLAP_EVENT_NOTE_EXPRESSION,
                    ),
                    expression_id,
                    note_id: clap_note_id(channel, note),
                    port_index: 0,
                    channel: channel as i16,
                    key: note as i16,
                    value,
                })
            }
        };
        events.push(event);
    }
}

unsafe extern "C" fn event_queue_size(list: *const ClapInputEvents) -> u32 {
    let q = &*((*list).ctx as *const EncodedEventQueue);
    q.events.len() as u32
}

unsafe extern "C" fn event_queue_get(
    list: *const ClapInputEvents,
    index: u32,
) -> *const ClapEventHeader {
    let q = &*((*list).ctx as *const EncodedEventQueue);
    match q.events.get(index as usize) {
        Some(EncodedEvent::NoteOn(ev)) | Some(EncodedEvent::NoteOff(ev)) => {
            &ev.header as *const ClapEventHeader
        }
        Some(EncodedEvent::Midi(ev)) => &ev.header as *const ClapEventHeader,
        Some(EncodedEvent::ParamValue(ev)) => &ev.header as *const ClapEventHeader,
        Some(EncodedEvent::NoteExpression(ev)) => &ev.header as *const ClapEventHeader,
        None => std::ptr::null(),
    }
}

/// The audio side's output event list (see `OutputSink`): a parameter
/// change goes to the app's edit log, lock-free, unless the parameter is
/// read-only. Everything else is accepted and dropped, as before.
unsafe extern "C" fn output_sink_push(
    list: *const ClapOutputEvents,
    event: *const ClapEventHeader,
) -> bool {
    if list.is_null() || event.is_null() || (*list).ctx.is_null() {
        return true;
    }
    let sink = &*((*list).ctx as *const OutputSink);
    let header = &*event;
    if header.space_id == CLAP_CORE_EVENT_SPACE_ID && header.event_type == CLAP_EVENT_PARAM_VALUE {
        let pv = &*(event as *const ClapEventParamValue);
        if sink.read_only.binary_search(&pv.param_id).is_err() {
            sink.log.push(pv.param_id, pv.value);
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Host params extension — capture the plugin GUI's parameter edits.
//
// When the user moves a knob in the plugin's own editor, the plugin calls
// `clap_host_params.request_flush`. An inactive plug-in (an editor instance
// is never activated for audio) has the flush done at once on the calling
// (main) thread: invoke the plugin's `params.flush` with an empty input list
// and an output list that records every CLAP_EVENT_PARAM_VALUE into the
// shared queue. The chain instance (sharing that queue) drains it in
// `process` and re-emits the change to its own audio-side plugin, so editor
// knob moves are heard live.
//
// An active plug-in (the instance that plays, showing its own window) may
// only be flushed on the audio thread, and never while `process` runs: the
// request then only sets a flag, and the next block or the audio thread's
// own flush answers it, with the plug-in's reported changes going to the
// app's edit log. This used to call `params.flush` right there on the
// window's thread, at the same time as the audio thread's `process`.
// ---------------------------------------------------------------------------

static HOST_PARAMS: ClapHostParams = ClapHostParams {
    rescan: host_params_rescan_noop,
    clear: host_params_clear_noop,
    request_flush: host_params_request_flush,
};

unsafe extern "C" fn host_params_rescan_noop(_host: *const ClapHost, _flags: u32) {}
unsafe extern "C" fn host_params_clear_noop(_host: *const ClapHost, _param_id: u32, _flags: u32) {}

/// The host side of `clap.gui`: a plug-in asking its window for a new
/// size. Without it a CLAP that grew a panel was cut off.
#[repr(C)]
struct ClapHostGui {
    resize_hints_changed: unsafe extern "C" fn(host: *const ClapHost),
    request_resize: unsafe extern "C" fn(host: *const ClapHost, width: u32, height: u32) -> bool,
    request_show: unsafe extern "C" fn(host: *const ClapHost) -> bool,
    request_hide: unsafe extern "C" fn(host: *const ClapHost) -> bool,
    closed: unsafe extern "C" fn(host: *const ClapHost, was_destroyed: bool),
}

static HOST_GUI: ClapHostGui = ClapHostGui {
    resize_hints_changed: host_gui_noop,
    request_resize: host_gui_request_resize,
    request_show: host_gui_no,
    request_hide: host_gui_no,
    closed: host_gui_closed,
};

unsafe extern "C" fn host_gui_noop(_host: *const ClapHost) {}
unsafe extern "C" fn host_gui_no(_host: *const ClapHost) -> bool {
    false
}
unsafe extern "C" fn host_gui_closed(_host: *const ClapHost, _was_destroyed: bool) {}

unsafe extern "C" fn host_gui_request_resize(
    host: *const ClapHost,
    width: u32,
    height: u32,
) -> bool {
    let Some(host) = host.as_ref() else {
        return false;
    };
    let Some(ctx) = (host.host_data as *const ClapHostContext).as_ref() else {
        return false;
    };
    let request = ctx.resize_request.lock().clone();
    match request {
        Some(request) if width > 0 && height > 0 => request(width, height),
        _ => false,
    }
}

/// `clap_host.get_extension`: `clap.host-params` and `clap.gui`.
unsafe extern "C" fn clap_host_get_extension(
    _host: *const ClapHost,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return std::ptr::null();
    }
    let id = CStr::from_ptr(id).to_bytes_with_nul();
    if id == CLAP_EXT_HOST_PARAMS {
        return &HOST_PARAMS as *const ClapHostParams as *const c_void;
    }
    if id == CLAP_EXT_GUI {
        return &HOST_GUI as *const ClapHostGui as *const c_void;
    }
    std::ptr::null()
}

struct ParamCapture {
    out: SharedParamQueue,
    /// The same edits again, for the app. `out` is drained on the audio
    /// path, so without a second copy a knob moved in the plug-in's own
    /// window can be heard but never recorded.
    log: SharedEditLog,
}

/// Output-event sink passed to `params.flush`: records param-value
/// events into the shared queue, de-duping consecutive edits per param.
unsafe extern "C" fn param_capture_push(
    list: *const ClapOutputEvents,
    event: *const ClapEventHeader,
) -> bool {
    if list.is_null() || event.is_null() {
        return false;
    }
    let cap = &*((*list).ctx as *const ParamCapture);
    let header = &*event;
    if header.space_id == CLAP_CORE_EVENT_SPACE_ID && header.event_type == CLAP_EVENT_PARAM_VALUE {
        let pv = &*(event as *const ClapEventParamValue);
        cap.out.push(pv.param_id, pv.value);
        cap.log.push(pv.param_id, pv.value);
    }
    true
}

unsafe extern "C" fn host_params_request_flush(host: *const ClapHost) {
    if host.is_null() {
        return;
    }
    let ctx_ptr = (*host).host_data as *const ClapHostContext;
    if ctx_ptr.is_null() {
        return;
    }
    let ctx = &*ctx_ptr;
    if ctx.plugin.is_null() {
        return;
    }
    ctx.flush_requested.store(true, AtomicOrdering::SeqCst);
    // A few rounds at most: a plug-in that asks again from inside every
    // flush is answered by its next request instead of looping here.
    for _ in 0..4 {
        // A flush already running on a window thread (this one, asked
        // again from inside it, or another) answers this request.
        if ctx.window_flush.swap(true, AtomicOrdering::SeqCst) {
            return;
        }
        // Active (or activating): the audio thread answers it.
        if !ctx.active.load(AtomicOrdering::SeqCst)
            && ctx.flush_requested.swap(false, AtomicOrdering::SeqCst)
        {
            flush_on_window_thread(ctx);
        }
        ctx.window_flush.store(false, AtomicOrdering::SeqCst);
        // Done, unless a request came in meanwhile from a thread that saw
        // this flush running and left it to us.
        if ctx.active.load(AtomicOrdering::SeqCst)
            || !ctx.flush_requested.load(AtomicOrdering::SeqCst)
        {
            return;
        }
    }
}

/// `params.flush` on an inactive plug-in, from the thread that asked.
unsafe fn flush_on_window_thread(ctx: &ClapHostContext) {
    let Ok(id) = CString::new("clap.params") else {
        return;
    };
    let pext = ((*ctx.plugin).get_extension)(ctx.plugin, id.as_ptr()) as *const ClapPluginParams;
    if pext.is_null() {
        return;
    }
    // Empty input list + capturing output list, both released after flush.
    let in_q = Box::into_raw(Box::new(EncodedEventQueue { events: Vec::new() }));
    let in_events = ClapInputEvents {
        ctx: in_q as *mut c_void,
        size: event_queue_size,
        get: event_queue_get,
    };
    let cap = Box::into_raw(Box::new(ParamCapture {
        out: Arc::clone(&ctx.pending),
        log: Arc::clone(&ctx.gui_edit_log),
    }));
    let out_events = ClapOutputEvents {
        ctx: cap as *mut c_void,
        try_push: param_capture_push,
    };
    ((*pext).flush)(ctx.plugin, &in_events, &out_events);
    drop(Box::from_raw(in_q));
    drop(Box::from_raw(cap));
}

// ---------------------------------------------------------------------------
// State stream bridges — host-owned Vec<u8> for save/load.
// ---------------------------------------------------------------------------

struct StateWriter {
    bytes: Vec<u8>,
}

struct StateReader {
    bytes: Vec<u8>,
    cursor: usize,
}

unsafe extern "C" fn state_writer_write(
    stream: *const ClapOstream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    let writer = &mut *((*stream).ctx as *mut StateWriter);
    let src = std::slice::from_raw_parts(buffer as *const u8, size as usize);
    writer.bytes.extend_from_slice(src);
    size as i64
}

unsafe extern "C" fn state_reader_read(
    stream: *const ClapIstream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    let reader = &mut *((*stream).ctx as *mut StateReader);
    let remaining = reader.bytes.len() - reader.cursor;
    let n = (size as usize).min(remaining);
    if n == 0 {
        return 0;
    }
    let dst = std::slice::from_raw_parts_mut(buffer as *mut u8, n);
    dst.copy_from_slice(&reader.bytes[reader.cursor..reader.cursor + n]);
    reader.cursor += n;
    n as i64
}

/// The id a CLAP factory knows a plug-in by: the DAW's id without its
/// "clap:" prefix.
pub(crate) fn factory_plugin_id(daw_id: &str) -> &str {
    daw_id.strip_prefix("clap:").unwrap_or(daw_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CLAP plug-in hears the tempo, the position in beats and seconds,
    /// the bar and whether the song plays.
    #[test]
    fn the_transport_says_where_the_song_is() {
        let t = TransportInfo {
            playing: true,
            tempo: 150.0,
            position_beats: 9.5,
            time_sig: (4, 4),
            sample_rate: 48_000.0,
            position_samples: 96_000,
        };
        let c = clap_transport(&t);
        assert_eq!(c.tempo, 150.0);
        assert_eq!(c.song_pos_beats as f64 / CLAP_BEATTIME_FACTOR, 9.5);
        assert_eq!(c.song_pos_seconds as f64 / CLAP_SECTIME_FACTOR, 2.0);
        assert_eq!(c.bar_number, 2);
        assert_eq!(c.bar_start as f64 / CLAP_BEATTIME_FACTOR, 8.0);
        assert_eq!((c.tsig_num, c.tsig_denom), (4, 4));
        assert!(c.flags & CLAP_TRANSPORT_IS_PLAYING != 0);
        assert!(c.flags & CLAP_TRANSPORT_HAS_TEMPO != 0);
        assert_eq!(c.header.event_type, CLAP_EVENT_TRANSPORT);
        assert_eq!(
            c.header.size as usize,
            std::mem::size_of::<ClapEventTransport>()
        );
    }
    use std::path::PathBuf;

    fn descriptor_for(path: PathBuf) -> PluginDescriptor {
        PluginDescriptor {
            id: "test.bogus.clap".into(),
            name: "Bogus".into(),
            vendor: "Test".into(),
            version: "0.0.0".into(),
            format: PluginFormat::Clap,
            path,
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    #[test]
    fn load_missing_path_errors_cleanly() {
        let desc = descriptor_for(PathBuf::from("/definitely/does/not/exist.clap"));
        let result = ClapPluginInstance::load(desc);
        assert!(result.is_err());
    }

    #[test]
    fn load_non_clap_file_rejects_on_missing_entry() {
        let candidate = PathBuf::from("/bin/ls");
        if !candidate.exists() {
            return;
        }
        let desc = descriptor_for(candidate);
        let result = ClapPluginInstance::load(desc);
        assert!(result.is_err(), "non-CLAP binary must be rejected");
    }
}

#[cfg(test)]
mod gui_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    // A fake plug-in editor that answers `show` as nih-plug does (false)
    // and counts what the host asks of it.
    static CREATED: AtomicU32 = AtomicU32::new(0);
    static DESTROYED: AtomicU32 = AtomicU32::new(0);
    static PARENT_OK: AtomicU32 = AtomicU32::new(1);

    unsafe extern "C" fn yes(_: *const ClapPlugin, _: *const c_char, _: bool) -> bool {
        true
    }
    unsafe extern "C" fn create(_: *const ClapPlugin, _: *const c_char, _: bool) -> bool {
        CREATED.fetch_add(1, Ordering::SeqCst);
        true
    }
    unsafe extern "C" fn destroy(_: *const ClapPlugin) {
        DESTROYED.fetch_add(1, Ordering::SeqCst);
    }
    unsafe extern "C" fn pref(_: *const ClapPlugin, _: *mut *const c_char, _: *mut bool) -> bool {
        false
    }
    unsafe extern "C" fn scale(_: *const ClapPlugin, _: f64) -> bool {
        false
    }
    unsafe extern "C" fn size(_: *const ClapPlugin, w: *mut u32, h: *mut u32) -> bool {
        *w = 800;
        *h = 500;
        true
    }
    unsafe extern "C" fn no(_: *const ClapPlugin) -> bool {
        false
    }
    unsafe extern "C" fn hints(_: *const ClapPlugin, _: *mut c_void) -> bool {
        false
    }
    unsafe extern "C" fn adjust(_: *const ClapPlugin, _: *mut u32, _: *mut u32) -> bool {
        false
    }
    unsafe extern "C" fn set_size(_: *const ClapPlugin, _: u32, _: u32) -> bool {
        false
    }
    unsafe extern "C" fn parent(_: *const ClapPlugin, _: *const ClapWindow) -> bool {
        PARENT_OK.load(Ordering::SeqCst) == 1
    }
    unsafe extern "C" fn title(_: *const ClapPlugin, _: *const c_char) {}

    fn fake_gui() -> ClapPluginGui {
        ClapPluginGui {
            is_api_supported: yes,
            get_preferred_api: pref,
            create,
            destroy,
            set_scale: scale,
            get_size: size,
            can_resize: no,
            get_resize_hints: hints,
            adjust_size: adjust,
            set_size,
            set_parent: parent,
            set_transient: parent,
            suggest_title: title,
            show: no,
            hide: no,
        }
    }

    #[test]
    fn an_embedded_editor_stays_open_when_show_answers_false() {
        let gui = fake_gui();
        let window = ClapWindow {
            api: c"win32".as_ptr(),
            handle: ClapWindowHandle {
                ptr: std::ptr::null_mut(),
            },
        };
        let before = DESTROYED.load(Ordering::SeqCst);
        PARENT_OK.store(1, Ordering::SeqCst);
        assert!(unsafe { open_embedded_gui(&gui, std::ptr::null(), &window, "Fake") });
        assert_eq!(
            DESTROYED.load(Ordering::SeqCst),
            before,
            "the editor must not be torn down"
        );

        // A plug-in that cannot take the parent window is still a failure,
        // and what it built is torn down.
        PARENT_OK.store(0, Ordering::SeqCst);
        assert!(!unsafe { open_embedded_gui(&gui, std::ptr::null(), &window, "Fake") });
        assert_eq!(DESTROYED.load(Ordering::SeqCst), before + 1);
        assert!(CREATED.load(Ordering::SeqCst) >= 2);
    }
}

#[cfg(test)]
mod factory_id_tests {
    use super::factory_plugin_id;

    #[test]
    fn the_factory_gets_the_plug_in_s_own_id() {
        assert_eq!(
            factory_plugin_id("clap:com.hardwavestudios.wideboi"),
            "com.hardwavestudios.wideboi"
        );
        assert_eq!(factory_plugin_id("com.u-he.diva"), "com.u-he.diva");
        // Only the leading prefix goes.
        assert_eq!(factory_plugin_id("clap:clap:x"), "clap:x");
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use hardwave_midi::{MidiEvent, NoteExpressionKind};

    /// A bend, pressure and poly aftertouch reach a CLAP as raw MIDI (they
    /// were dropped), and a note's own pan and tuning as note expression on
    /// the note's id.
    #[test]
    fn bends_pressure_and_note_expression_reach_a_clap() {
        let mut events = Vec::with_capacity(16);
        encode_midi_events(
            &[
                MidiEvent::NoteOn {
                    timing: 2,
                    channel: 1,
                    note: 64,
                    velocity: 1.0,
                },
                MidiEvent::NoteExpression {
                    timing: 2,
                    channel: 1,
                    note: 64,
                    kind: NoteExpressionKind::Pan,
                    value: 1.0,
                },
                MidiEvent::NoteExpression {
                    timing: 2,
                    channel: 1,
                    note: 64,
                    kind: NoteExpressionKind::Tuning,
                    value: -0.5,
                },
                MidiEvent::PitchBend {
                    timing: 8,
                    channel: 1,
                    value: 1.0,
                },
                MidiEvent::ChannelPressure {
                    timing: 9,
                    channel: 1,
                    pressure: 1.0,
                },
                MidiEvent::Aftertouch {
                    timing: 10,
                    channel: 1,
                    note: 64,
                    pressure: 0.0,
                },
            ],
            &mut events,
        );
        assert_eq!(events.len(), 6);
        match (&events[0], &events[1], &events[2]) {
            (
                EncodedEvent::NoteOn(on),
                EncodedEvent::NoteExpression(pan),
                EncodedEvent::NoteExpression(tune),
            ) => {
                assert_eq!(on.note_id, 128 + 64);
                assert_eq!(
                    (pan.expression_id, pan.note_id, pan.value),
                    (CLAP_NOTE_EXPRESSION_PAN, 128 + 64, 1.0)
                );
                assert_eq!(
                    (tune.expression_id, tune.value),
                    (CLAP_NOTE_EXPRESSION_TUNING, -0.5)
                );
                assert_eq!(pan.header.event_type, CLAP_EVENT_NOTE_EXPRESSION);
            }
            _ => panic!("a note, then its pan and tuning"),
        }
        let raw: Vec<[u8; 3]> = events[3..]
            .iter()
            .map(|e| match e {
                EncodedEvent::Midi(m) => m.data,
                _ => panic!("raw MIDI"),
            })
            .collect();
        assert_eq!(raw, vec![[0xE1, 0x7F, 0x7F], [0xD1, 127, 0], [0xA1, 64, 0]]);
    }

    /// A full block stops at the limit rather than growing the list.
    #[test]
    fn a_flood_of_events_stops_at_the_limit() {
        let mut events = Vec::with_capacity(MAX_BLOCK_EVENTS);
        let flood: Vec<MidiEvent> = (0..MAX_BLOCK_EVENTS + 50)
            .map(|i| MidiEvent::ControlChange {
                timing: 0,
                channel: 0,
                cc: (i % 128) as u8,
                value: 0.5,
            })
            .collect();
        encode_midi_events(&flood, &mut events);
        assert_eq!(events.len(), MAX_BLOCK_EVENTS);
        assert_eq!(
            events.capacity(),
            MAX_BLOCK_EVENTS,
            "no growing on the audio thread"
        );
    }
}

#[cfg(test)]
mod flush_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // What the fake plug-in has taken in, as f64 bits.
    static TAKEN: AtomicU64 = AtomicU64::new(0);

    unsafe extern "C" fn count(_: *const ClapPlugin) -> u32 {
        1
    }
    /// The flags the fake gives its one parameter (id 7).
    static FLAGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    unsafe extern "C" fn get_info(_: *const ClapPlugin, _: u32, info: *mut ClapParamInfo) -> bool {
        (*info).id = 7;
        (*info).flags = FLAGS.load(Ordering::SeqCst);
        true
    }
    unsafe extern "C" fn get_value(_: *const ClapPlugin, _: u32, out: *mut f64) -> bool {
        *out = f64::from_bits(TAKEN.load(Ordering::SeqCst));
        true
    }
    unsafe extern "C" fn to_text(
        _: *const ClapPlugin,
        _: u32,
        _: f64,
        _: *mut c_char,
        _: u32,
    ) -> bool {
        false
    }
    unsafe extern "C" fn to_value(
        _: *const ClapPlugin,
        _: u32,
        _: *const c_char,
        _: *mut f64,
    ) -> bool {
        false
    }
    /// The fake's own flush count, when a test gives it one in
    /// `plugin_data`.
    unsafe fn count_flush(plugin: *const ClapPlugin) {
        let counter = (*plugin).plugin_data as *const std::sync::atomic::AtomicUsize;
        if !counter.is_null() {
            (*counter).fetch_add(1, Ordering::SeqCst);
        }
    }
    /// What the fake reports from its audio side: parameter 42 at 0.66.
    unsafe fn report(out_events: *const ClapOutputEvents) {
        if out_events.is_null() {
            return;
        }
        let ev = ClapEventParamValue {
            header: ClapEventHeader {
                size: std::mem::size_of::<ClapEventParamValue>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                event_type: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: 42,
            cookie: std::ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 0.66,
        };
        ((*out_events).try_push)(out_events, &ev.header);
    }
    unsafe extern "C" fn flush(
        plugin: *const ClapPlugin,
        in_events: *const ClapInputEvents,
        out_events: *const ClapOutputEvents,
    ) {
        count_flush(plugin);
        report(out_events);
        let list = &*in_events;
        for i in 0..(list.size)(in_events) {
            let header = (list.get)(in_events, i);
            if (*header).event_type == CLAP_EVENT_PARAM_VALUE {
                let ev = &*(header as *const ClapEventParamValue);
                TAKEN.store(ev.value.to_bits(), Ordering::SeqCst);
            }
        }
    }
    static PARAMS: ClapPluginParams = ClapPluginParams {
        count,
        get_info,
        get_value,
        value_to_text: to_text,
        text_to_value: to_value,
        flush,
    };
    unsafe extern "C" fn save(_: *const ClapPlugin, _: *const ClapOstream) -> bool {
        true
    }
    unsafe extern "C" fn load(_: *const ClapPlugin, _: *const ClapIstream) -> bool {
        true
    }
    static STATE: ClapPluginState = ClapPluginState { save, load };
    unsafe extern "C" fn get_extension(_: *const ClapPlugin, id: *const c_char) -> *const c_void {
        let id = CStr::from_ptr(id).to_bytes_with_nul();
        if id == CLAP_EXT_PARAMS {
            &PARAMS as *const ClapPluginParams as *const c_void
        } else if id == CLAP_EXT_STATE {
            &STATE as *const ClapPluginState as *const c_void
        } else {
            std::ptr::null()
        }
    }
    unsafe extern "C" fn init(_: *const ClapPlugin) -> bool {
        true
    }
    unsafe extern "C" fn nothing(_: *const ClapPlugin) {}
    unsafe extern "C" fn activate(_: *const ClapPlugin, _: f64, _: u32, _: u32) -> bool {
        true
    }
    unsafe extern "C" fn process(_: *const ClapPlugin, p: *const ClapProcess) -> i32 {
        report((*p).out_events);
        0
    }

    fn fake_plugin() -> Box<ClapPlugin> {
        Box::new(ClapPlugin {
            desc: std::ptr::null(),
            plugin_data: std::ptr::null_mut(),
            init,
            destroy: nothing,
            activate,
            deactivate: nothing,
            start_processing: init,
            stop_processing: nothing,
            reset: nothing,
            process,
            get_extension,
            on_main_thread: nothing,
        })
    }

    /// An active instance of the fake plug-in, with no library behind it.
    fn fake_instance(plugin: &ClapPlugin) -> ClapPluginInstance {
        let pending_params: SharedParamQueue = Default::default();
        let gui_edit_log: SharedEditLog = Default::default();
        let host_ctx = Box::new(ClapHostContext {
            plugin,
            resize_request: Arc::new(Mutex::new(None)),
            pending: Arc::clone(&pending_params),
            gui_edit_log: Arc::clone(&gui_edit_log),
            active: AtomicBool::new(true),
            window_flush: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
        });
        let mut host = Box::new(crate::clap_ffi::build_static_host());
        host.host_data = &*host_ctx as *const ClapHostContext as *mut c_void;
        ClapPluginInstance {
            descriptor: PluginDescriptor {
                id: "clap:test.fake".into(),
                name: "Fake".into(),
                vendor: "Test".into(),
                version: "0".into(),
                format: PluginFormat::Clap,
                path: "<test>".into(),
                category: PluginCategory::Effect,
                num_inputs: 2,
                num_outputs: 2,
                has_midi_input: false,
                has_editor: false,
            },
            active: true,
            processing: true,
            sample_rate: 48_000.0,
            max_block: 256,
            library: None,
            entry: std::ptr::null(),
            plugin,
            host,
            host_ctx,
            out_sink: Box::new(OutputSink {
                log: Arc::clone(&gui_edit_log),
                read_only: Vec::new(),
            }),
            cached_params: Vec::new(),
            latency: 0,
            initialized: true,
            pending_params,
            gui_edit_log,
            gui_open: false,
            transport: TransportInfo::default(),
            editor: Arc::new(Mutex::new(ClapEditorSide {
                plugin,
                gui: std::ptr::null(),
                name: String::new(),
                open: false,
                resize_request: Arc::new(Mutex::new(None)),
            })),
            presets: std::sync::OnceLock::new(),
            input_scratch: Vec::new(),
            input_ptrs: Vec::with_capacity(8),
            output_ptrs: Vec::with_capacity(8),
            event_queue: Box::new(EncodedEventQueue {
                events: Vec::with_capacity(MAX_BLOCK_EVENTS),
            }),
            tail_ext: std::ptr::null(),
            last_status: CLAP_PROCESS_ERROR,
            input_silent: false,
        }
    }

    /// A value set while no block runs (the song stopped) reaches the
    /// plug-in through `params.flush`, so the plug-in, its window and its
    /// saved state have it at once instead of after play starts.
    #[test]
    fn a_value_set_between_blocks_reaches_the_plug_in_on_flush() {
        TAKEN.store(0.333f64.to_bits(), Ordering::SeqCst);
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        instance.set_parameter_value(7, 0.73);
        assert_eq!(instance.get_parameter_value(7), 0.333, "waits for a block");
        instance.flush_parameters();
        assert_eq!(instance.get_parameter_value(7), 0.73);
        assert!(!instance.pending_params.has_pending(), "handed over once");
        // Nothing waiting: nothing is sent.
        TAKEN.store(0.5f64.to_bits(), Ordering::SeqCst);
        instance.flush_parameters();
        assert_eq!(instance.get_parameter_value(7), 0.5);
        drop(instance);
    }

    /// More changes than one block's event list holds: the rest used to be
    /// thrown away with the drained queue. Now they wait for the next block.
    #[test]
    fn changes_past_a_full_event_list_arrive_in_the_next_block() {
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        let window = Arc::clone(&instance.pending_params);
        let total = MAX_BLOCK_EVENTS + 10;
        for id in 0..total as u32 {
            window.push(id, 0.5);
        }
        instance.take_param_events();
        assert_eq!(instance.event_queue.events.len(), MAX_BLOCK_EVENTS);
        assert!(instance.pending_params.has_pending());
        instance.take_param_events();
        assert_eq!(instance.event_queue.events.len(), 10);
        assert!(!instance.pending_params.has_pending());
        instance.event_queue.events.clear();
        drop(instance);
    }

    fn logged(instance: &ClapPluginInstance) -> Vec<(u32, f64)> {
        let mut out = Vec::new();
        instance
            .gui_edit_log
            .drain(usize::MAX, |id, v| out.push((id, v)));
        out
    }

    /// A playing plug-in that asks for a flush from its window is not
    /// flushed there, at the same time as the audio thread's `process`: the
    /// request waits for the audio thread, whose flush hands the plug-in's
    /// reported change to the app.
    #[test]
    fn a_flush_asked_for_while_active_is_done_on_the_audio_thread() {
        let flushes = Box::new(std::sync::atomic::AtomicUsize::new(0));
        let mut plugin = fake_plugin();
        plugin.plugin_data = &*flushes as *const _ as *mut c_void;
        let mut instance = fake_instance(&plugin);
        unsafe { host_params_request_flush(&*instance.host) };
        assert_eq!(
            flushes.load(Ordering::SeqCst),
            0,
            "not on the window's thread"
        );
        assert!(instance.host_ctx.flush_requested.load(Ordering::SeqCst));
        instance.flush_parameters();
        assert_eq!(flushes.load(Ordering::SeqCst), 1, "on the audio thread");
        assert!(!instance.host_ctx.flush_requested.load(Ordering::SeqCst));
        assert_eq!(logged(&instance), vec![(42, 0.66)]);
        instance.flush_parameters();
        assert_eq!(flushes.load(Ordering::SeqCst), 1, "answered once");
        drop(instance);
    }

    /// An inactive plug-in (a window's own copy) is flushed at once, and its
    /// change goes to the queue the playing copy takes, and to the app.
    #[test]
    fn a_flush_asked_for_while_inactive_is_done_at_once() {
        let flushes = Box::new(std::sync::atomic::AtomicUsize::new(0));
        let mut plugin = fake_plugin();
        plugin.plugin_data = &*flushes as *const _ as *mut c_void;
        let mut instance = fake_instance(&plugin);
        instance.host_ctx.active.store(false, Ordering::SeqCst);
        unsafe { host_params_request_flush(&*instance.host) };
        assert_eq!(flushes.load(Ordering::SeqCst), 1);
        assert!(!instance.host_ctx.window_flush.load(Ordering::SeqCst));
        let mut queued = Vec::new();
        instance
            .pending_params
            .drain(usize::MAX, |id, v| queued.push((id, v)));
        assert_eq!(queued, vec![(42, 0.66)]);
        assert_eq!(logged(&instance), vec![(42, 0.66)]);
        instance.active = false;
        instance.processing = false;
        drop(instance);
    }

    /// What a plug-in reports from `process` reaches the app (the output
    /// list was a no-op), except read-only parameters.
    #[test]
    fn changes_a_plug_in_reports_while_playing_reach_the_app() {
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        let input = vec![0.25f32; 16];
        let mut outputs = vec![vec![0.0f32; 16]; 2];
        instance.process(&[&input, &input], &mut outputs, &[], &mut Vec::new(), 16);
        assert_eq!(logged(&instance), vec![(42, 0.66)]);

        instance.out_sink.read_only = vec![42];
        instance.process(&[&input, &input], &mut outputs, &[], &mut Vec::new(), 16);
        assert!(logged(&instance).is_empty(), "a meter is not a knob move");
        instance.event_queue.events.clear();
        drop(instance);
    }

    /// The plug-in reads `num_samples` from every input channel it is told
    /// about: a short input, or none at all, is filled out with silence.
    #[test]
    fn every_input_the_plug_in_reads_is_the_block_s_length() {
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        let short = vec![0.5f32; 8];
        let mut outputs = vec![vec![0.0f32; 16]; 2];
        instance.process(&[&short, &short], &mut outputs, &[], &mut Vec::new(), 16);
        assert!(instance.input_scratch.iter().take(2).all(|c| c.len() == 16));
        assert_eq!(instance.input_scratch[0][7], 0.5);
        assert_eq!(instance.input_scratch[0][8], 0.0);
        assert_eq!(instance.input_ptrs.len(), 2);

        instance.process(&[], &mut outputs, &[], &mut Vec::new(), 16);
        assert_eq!(instance.input_ptrs.len(), 1, "the one channel it is given");
        assert_eq!(instance.input_scratch[0].len(), 16);
        assert!(instance.input_scratch[0].iter().all(|s| *s == 0.0));
        instance.event_queue.events.clear();
        drop(instance);
    }

    /// Knob moves made before a state load are older than it: they are
    /// dropped instead of being handed over after it.
    #[test]
    fn a_state_load_drops_the_changes_made_before_it() {
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        instance.pending_params.push(3, 0.9);
        instance.set_parameter_value(4, 0.1);
        instance.set_state(&[1, 2, 3]).unwrap();
        assert!(!instance.pending_params.has_pending());
        instance.set_parameter_value(4, 0.2);
        assert!(
            instance.pending_params.has_pending(),
            "a change after it stays"
        );
        drop(instance);
    }

    /// CLAP_PARAM_IS_AUTOMATABLE is bit 5. Bit 0, read before, is
    /// CLAP_PARAM_IS_STEPPED.
    #[test]
    fn a_clap_parameter_is_automatable_by_its_own_flag() {
        let plugin = fake_plugin();
        let mut instance = fake_instance(&plugin);
        let read = |instance: &mut ClapPluginInstance, flags: u32| {
            FLAGS.store(flags, Ordering::SeqCst);
            instance.refresh_params();
            instance.cached_params[0].automatable
        };
        assert!(read(&mut instance, 1 << 5));
        assert!(!read(&mut instance, 1), "stepped alone is not automatable");
        assert!(read(&mut instance, (1 << 5) | 1));
        assert!(instance.out_sink.read_only.is_empty());
        read(&mut instance, 1 << 3);
        assert_eq!(instance.out_sink.read_only, vec![7], "read-only is listed");
        drop(instance);
    }
}
