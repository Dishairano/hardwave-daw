//! Minimal CLAP FFI — just enough to read plugin descriptors at scan time.
//!
//! We hand-roll the subset of the CLAP C ABI we need (entry, factory, and
//! descriptor structs) to avoid pulling in a full CLAP binding crate for what
//! amounts to a metadata read. Only descriptor fields — never audio process —
//! are accessed through this module.
//!
//! Safety: `std::ffi` + `libloading` — loading untrusted shared libraries at
//! scan time is inherent to the CLAP scan contract, and we always call
//! `clap_plugin_entry_t.deinit` before dropping the library handle.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

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
    pub create_plugin: *const c_void,
}

#[repr(C)]
pub struct ClapPluginEntry {
    pub clap_version: ClapVersion,
    pub init: unsafe extern "C" fn(plugin_path: *const c_char) -> bool,
    pub deinit: unsafe extern "C" fn(),
    pub get_factory: unsafe extern "C" fn(factory_id: *const c_char) -> *const c_void,
}

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
            break; // defensive cap
        }
    }
    out
}

/// Load a `.clap` shared library and read every plugin descriptor it exposes.
///
/// Returns `None` if the library has no `clap_entry` symbol or initialization
/// fails. Errors loading individual descriptors are logged and skipped.
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
        // `lib` dropped here, library unloaded.
        Some(out)
    }
}
