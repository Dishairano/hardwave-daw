//! Minimal CLAP FFI — hand-rolled subset of the CLAP C ABI sufficient
//! to host a plugin instance end-to-end: entry / factory metadata +
//! plugin lifecycle (`init`, `activate`, `process`, `destroy`) + the
//! `params`, `state`, and `note-ports` extensions the DAW uses.
//!
//! We avoid pulling in a full CLAP binding crate so this stays within
//! the plugin-host's own build graph. Every struct mirrors the layout
//! in the CLAP headers (`clap.h`, `ext/params.h`, `ext/state.h`, etc.)
//! as of spec 1.2.x; field offsets were cross-checked against the
//! reference C definitions.
//!
//! Safety: `std::ffi` + `libloading` — loading untrusted shared
//! libraries at scan time is inherent to the CLAP scan contract, and
//! we always call `clap_plugin_entry_t.deinit` before dropping the
//! library handle.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

// ---------------------------------------------------------------------------
// Core structs — entry, factory, descriptor
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapVersion {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
}

#[repr(C)]
pub struct ClapPluginDescriptor {
    pub clap_version: ClapVersion,
    pub id: *const c_char,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub manual_url: *const c_char,
    pub support_url: *const c_char,
    pub version: *const c_char,
    pub description: *const c_char,
    pub features: *const *const c_char,
}

#[repr(C)]
pub struct ClapPluginFactory {
    pub get_plugin_count: unsafe extern "C" fn(factory: *const ClapPluginFactory) -> u32,
    pub get_plugin_descriptor: unsafe extern "C" fn(
        factory: *const ClapPluginFactory,
        index: u32,
    ) -> *const ClapPluginDescriptor,
    pub create_plugin: unsafe extern "C" fn(
        factory: *const ClapPluginFactory,
        host: *const ClapHost,
        plugin_id: *const c_char,
    ) -> *const ClapPlugin,
}

#[repr(C)]
pub struct ClapPluginEntry {
    pub clap_version: ClapVersion,
    pub init: unsafe extern "C" fn(plugin_path: *const c_char) -> bool,
    pub deinit: unsafe extern "C" fn(),
    pub get_factory: unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void,
}

// ---------------------------------------------------------------------------
// Plugin instance function table (`clap_plugin_t`)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapPlugin {
    pub desc: *const ClapPluginDescriptor,
    pub plugin_data: *mut c_void,
    pub init: unsafe extern "C" fn(plugin: *const ClapPlugin) -> bool,
    pub destroy: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub activate: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        sample_rate: f64,
        min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool,
    pub deactivate: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub start_processing: unsafe extern "C" fn(plugin: *const ClapPlugin) -> bool,
    pub stop_processing: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub reset: unsafe extern "C" fn(plugin: *const ClapPlugin),
    pub process:
        unsafe extern "C" fn(plugin: *const ClapPlugin, process: *const ClapProcess) -> i32, // clap_process_status
    pub get_extension:
        unsafe extern "C" fn(plugin: *const ClapPlugin, id: *const c_char) -> *const c_void,
    pub on_main_thread: unsafe extern "C" fn(plugin: *const ClapPlugin),
}

// ---------------------------------------------------------------------------
// Host callbacks (`clap_host_t`) — minimal stub: we advertise the
// host's identity + return null for every extension the plugin might
// request, so the plugin falls back to defaults.
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapHost {
    pub clap_version: ClapVersion,
    pub host_data: *mut c_void,
    pub name: *const c_char,
    pub vendor: *const c_char,
    pub url: *const c_char,
    pub version: *const c_char,
    pub get_extension:
        unsafe extern "C" fn(host: *const ClapHost, id: *const c_char) -> *const c_void,
    pub request_restart: unsafe extern "C" fn(host: *const ClapHost),
    pub request_process: unsafe extern "C" fn(host: *const ClapHost),
    pub request_callback: unsafe extern "C" fn(host: *const ClapHost),
}

unsafe impl Send for ClapHost {}
unsafe impl Sync for ClapHost {}

unsafe extern "C" fn host_get_extension_null(
    _host: *const ClapHost,
    _id: *const c_char,
) -> *const c_void {
    std::ptr::null()
}
unsafe extern "C" fn host_request_noop(_host: *const ClapHost) {}

/// Static host context — fields are `'static` strings owned by the
/// binary. Plugins borrow these but never free them.
pub fn build_static_host() -> ClapHost {
    // These CStrings are leaked so their pointers remain valid for
    // the process lifetime. Host context is a singleton.
    fn leak(s: &'static str) -> *const c_char {
        let cs = CString::new(s).unwrap();
        let ptr = cs.as_ptr() as *const c_char;
        std::mem::forget(cs);
        ptr
    }
    ClapHost {
        clap_version: ClapVersion {
            major: 1,
            minor: 2,
            revision: 0,
        },
        host_data: std::ptr::null_mut(),
        name: leak("Hardwave DAW"),
        vendor: leak("Hardwave"),
        url: leak("https://hardwave.codeflowly.com"),
        version: leak(env!("CARGO_PKG_VERSION")),
        get_extension: host_get_extension_null,
        request_restart: host_request_noop,
        request_process: host_request_noop,
        request_callback: host_request_noop,
    }
}

// ---------------------------------------------------------------------------
// Process block types
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapAudioBuffer {
    pub data32: *mut *mut f32,
    pub data64: *mut *mut f64,
    pub channel_count: u32,
    pub latency: u32,
    pub constant_mask: u64,
}

#[repr(C)]
pub struct ClapEventHeader {
    pub size: u32,
    pub time: u32,
    pub space_id: u16,
    pub event_type: u16,
    pub flags: u32,
}

#[repr(C)]
pub struct ClapInputEvents {
    pub ctx: *mut c_void,
    pub size: unsafe extern "C" fn(list: *const ClapInputEvents) -> u32,
    pub get:
        unsafe extern "C" fn(list: *const ClapInputEvents, index: u32) -> *const ClapEventHeader,
}

#[repr(C)]
pub struct ClapOutputEvents {
    pub ctx: *mut c_void,
    pub try_push:
        unsafe extern "C" fn(list: *const ClapOutputEvents, event: *const ClapEventHeader) -> bool,
}

#[repr(C)]
pub struct ClapProcess {
    pub steady_time: i64,
    pub frames_count: u32,
    pub transport: *const c_void, // clap_event_transport_t — unused for now
    pub audio_inputs: *const ClapAudioBuffer,
    pub audio_outputs: *mut ClapAudioBuffer,
    pub audio_inputs_count: u32,
    pub audio_outputs_count: u32,
    pub in_events: *const ClapInputEvents,
    pub out_events: *const ClapOutputEvents,
}

pub const CLAP_PROCESS_ERROR: i32 = 0;
pub const CLAP_PROCESS_CONTINUE: i32 = 1;
pub const CLAP_PROCESS_CONTINUE_IF_NOT_QUIET: i32 = 2;
pub const CLAP_PROCESS_TAIL: i32 = 3;
pub const CLAP_PROCESS_SLEEP: i32 = 4;

// ---------------------------------------------------------------------------
// MIDI event types — note on / note off / param-value at minimum.
// ---------------------------------------------------------------------------

pub const CLAP_EVENT_NOTE_ON: u16 = 0;
pub const CLAP_EVENT_NOTE_OFF: u16 = 1;
pub const CLAP_EVENT_NOTE_CHOKE: u16 = 2;
pub const CLAP_EVENT_MIDI: u16 = 6;
pub const CLAP_CORE_EVENT_SPACE_ID: u16 = 0;

#[repr(C)]
pub struct ClapEventNote {
    pub header: ClapEventHeader,
    pub note_id: i32,
    pub port_index: i16,
    pub channel: i16,
    pub key: i16,
    pub velocity: f64,
}

#[repr(C)]
pub struct ClapEventMidi {
    pub header: ClapEventHeader,
    pub port_index: u16,
    pub data: [u8; 3],
}

// ---------------------------------------------------------------------------
// params extension (`clap.params`)
// ---------------------------------------------------------------------------

pub const CLAP_EXT_PARAMS: &[u8] = b"clap.params\0";
pub const CLAP_EXT_STATE: &[u8] = b"clap.state\0";

pub const CLAP_NAME_SIZE: usize = 256;
pub const CLAP_PATH_SIZE: usize = 1024;

#[repr(C)]
pub struct ClapParamInfo {
    pub id: u32,
    pub flags: u32,
    pub cookie: *mut c_void,
    pub name: [u8; CLAP_NAME_SIZE],
    pub module: [u8; CLAP_PATH_SIZE],
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
}

#[repr(C)]
pub struct ClapPluginParams {
    pub count: unsafe extern "C" fn(plugin: *const ClapPlugin) -> u32,
    pub get_info: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_index: u32,
        param_info: *mut ClapParamInfo,
    ) -> bool,
    pub get_value:
        unsafe extern "C" fn(plugin: *const ClapPlugin, param_id: u32, value: *mut f64) -> bool,
    pub value_to_text: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_id: u32,
        value: f64,
        out: *mut c_char,
        out_size: u32,
    ) -> bool,
    pub text_to_value: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        param_id: u32,
        text: *const c_char,
        out_value: *mut f64,
    ) -> bool,
    pub flush: unsafe extern "C" fn(
        plugin: *const ClapPlugin,
        in_events: *const ClapInputEvents,
        out_events: *const ClapOutputEvents,
    ),
}

// ---------------------------------------------------------------------------
// state extension (`clap.state`) — stream-based save/load
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct ClapIstream {
    pub ctx: *mut c_void,
    pub read:
        unsafe extern "C" fn(stream: *const ClapIstream, buffer: *mut c_void, size: u64) -> i64,
}

#[repr(C)]
pub struct ClapOstream {
    pub ctx: *mut c_void,
    pub write:
        unsafe extern "C" fn(stream: *const ClapOstream, buffer: *const c_void, size: u64) -> i64,
}

#[repr(C)]
pub struct ClapPluginState {
    pub save: unsafe extern "C" fn(plugin: *const ClapPlugin, stream: *const ClapOstream) -> bool,
    pub load: unsafe extern "C" fn(plugin: *const ClapPlugin, stream: *const ClapIstream) -> bool,
}

// ---------------------------------------------------------------------------
// Scan-time metadata readers (unchanged from prior release)
// ---------------------------------------------------------------------------

pub struct ReadDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub features: Vec<String>,
}

unsafe fn cstr_to_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

unsafe fn read_features(ptr: *const *const c_char) -> Vec<String> {
    let mut out = Vec::new();
    if ptr.is_null() {
        return out;
    }
    let mut i = 0isize;
    loop {
        let s = unsafe { *ptr.offset(i) };
        if s.is_null() {
            break;
        }
        out.push(unsafe { cstr_to_string(s) });
        i += 1;
        if i > 64 {
            break;
        }
    }
    out
}

/// Load a `.clap` shared library and read every plugin descriptor it exposes.
pub fn read_clap_descriptors(library_path: &Path) -> Option<Vec<ReadDescriptor>> {
    unsafe {
        let lib = libloading::Library::new(library_path).ok()?;
        let entry: libloading::Symbol<*const ClapPluginEntry> = lib.get(b"clap_entry\0").ok()?;
        let entry = *entry;
        if entry.is_null() {
            return None;
        }
        let entry = &*entry;

        let path_c = CString::new(library_path.to_string_lossy().as_bytes()).ok()?;
        if !(entry.init)(path_c.as_ptr()) {
            log::warn!(
                "clap_entry.init returned false for {}",
                library_path.display()
            );
            return None;
        }

        let factory_id = CString::new("clap.plugin-factory").ok()?;
        let factory_ptr = (entry.get_factory)(factory_id.as_ptr());
        if factory_ptr.is_null() {
            (entry.deinit)();
            return None;
        }
        let factory = &*(factory_ptr as *const ClapPluginFactory);
        let count = (factory.get_plugin_count)(factory);

        let mut out = Vec::new();
        for i in 0..count {
            let desc_ptr = (factory.get_plugin_descriptor)(factory, i);
            if desc_ptr.is_null() {
                continue;
            }
            let d = &*desc_ptr;
            out.push(ReadDescriptor {
                id: cstr_to_string(d.id),
                name: cstr_to_string(d.name),
                vendor: cstr_to_string(d.vendor),
                version: cstr_to_string(d.version),
                features: read_features(d.features),
            });
        }

        (entry.deinit)();
        Some(out)
    }
}
