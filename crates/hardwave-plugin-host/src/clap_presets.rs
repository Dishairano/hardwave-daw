//! A CLAP plug-in's own presets: found through its preset-discovery
//! factory and loaded through its `preset-load` extension.
//!
//! A CLAP does not publish a program list the way a VST3 does. It hands
//! the host a provider that declares where its presets are (inside the
//! plug-in, or files in folders) and describes each one; the host keeps
//! the location and load key and gives them back to load a preset. The
//! browser listed nothing for a CLAP before, since the host never asked.
//!
//! The structs mirror `factory/preset-discovery.h` and
//! `ext/preset-load.h` of CLAP 1.2; plug-ins built against the drafts
//! answer to the draft ids, which are tried too.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

use crate::clap_ffi::{ClapPlugin, ClapPluginEntry, ClapVersion};

const FACTORY_IDS: [&[u8]; 2] = [
    b"clap.preset-discovery-factory/2\0",
    b"clap.preset-discovery-factory/draft-2\0",
];
const LOAD_EXT_IDS: [&[u8]; 2] = [b"clap.preset-load/2\0", b"clap.preset-load.draft/2\0"];

/// Where a preset lives: a file, or inside the plug-in.
pub const LOCATION_FILE: u32 = 0;
pub const LOCATION_PLUGIN: u32 = 1;

/// One preset, as the host keeps it to load it again.
#[derive(Debug, Clone, PartialEq)]
pub struct ClapPreset {
    pub name: String,
    pub location_kind: u32,
    /// The file, for a preset in a file; none for one inside the plug-in.
    pub location: Option<String>,
    pub load_key: Option<String>,
}

// ---------------------------------------------------------------------------
// FFI
// ---------------------------------------------------------------------------

#[repr(C)]
struct UniversalPluginId {
    abi: *const c_char,
    id: *const c_char,
}

#[repr(C)]
struct MetadataReceiver {
    receiver_data: *mut c_void,
    on_error: unsafe extern "C" fn(*const MetadataReceiver, i32, *const c_char),
    begin_preset:
        unsafe extern "C" fn(*const MetadataReceiver, *const c_char, *const c_char) -> bool,
    add_plugin_id: unsafe extern "C" fn(*const MetadataReceiver, *const UniversalPluginId),
    set_soundpack_id: unsafe extern "C" fn(*const MetadataReceiver, *const c_char),
    set_flags: unsafe extern "C" fn(*const MetadataReceiver, u32),
    add_creator: unsafe extern "C" fn(*const MetadataReceiver, *const c_char),
    set_description: unsafe extern "C" fn(*const MetadataReceiver, *const c_char),
    set_timestamps: unsafe extern "C" fn(*const MetadataReceiver, u64, u64),
    add_feature: unsafe extern "C" fn(*const MetadataReceiver, *const c_char),
    add_extra_info: unsafe extern "C" fn(*const MetadataReceiver, *const c_char, *const c_char),
}

#[repr(C)]
struct Filetype {
    name: *const c_char,
    description: *const c_char,
    file_extension: *const c_char,
}

#[repr(C)]
struct Location {
    flags: u32,
    name: *const c_char,
    kind: u32,
    location: *const c_char,
}

#[repr(C)]
struct ProviderDescriptor {
    clap_version: ClapVersion,
    id: *const c_char,
    name: *const c_char,
    vendor: *const c_char,
}

#[repr(C)]
struct Provider {
    desc: *const ProviderDescriptor,
    provider_data: *mut c_void,
    init: unsafe extern "C" fn(*const Provider) -> bool,
    destroy: unsafe extern "C" fn(*const Provider),
    get_metadata:
        unsafe extern "C" fn(*const Provider, u32, *const c_char, *const MetadataReceiver) -> bool,
    get_extension: unsafe extern "C" fn(*const Provider, *const c_char) -> *const c_void,
}

#[repr(C)]
struct Indexer {
    clap_version: ClapVersion,
    name: *const c_char,
    vendor: *const c_char,
    url: *const c_char,
    version: *const c_char,
    indexer_data: *mut c_void,
    declare_filetype: unsafe extern "C" fn(*const Indexer, *const Filetype) -> bool,
    declare_location: unsafe extern "C" fn(*const Indexer, *const Location) -> bool,
    declare_soundpack: unsafe extern "C" fn(*const Indexer, *const c_void) -> bool,
    get_extension: unsafe extern "C" fn(*const Indexer, *const c_char) -> *const c_void,
}

#[repr(C)]
struct DiscoveryFactory {
    count: unsafe extern "C" fn(*const DiscoveryFactory) -> u32,
    get_descriptor: unsafe extern "C" fn(*const DiscoveryFactory, u32) -> *const ProviderDescriptor,
    create: unsafe extern "C" fn(
        *const DiscoveryFactory,
        *const Indexer,
        *const c_char,
    ) -> *const Provider,
}

#[repr(C)]
struct PresetLoad {
    from_location:
        unsafe extern "C" fn(*const ClapPlugin, u32, *const c_char, *const c_char) -> bool,
}

fn text(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

// ---------------------------------------------------------------------------
// What the provider declares, and the presets it describes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Declared {
    extensions: Vec<String>,
    locations: Vec<(u32, Option<String>)>,
}

unsafe extern "C" fn declare_filetype(indexer: *const Indexer, filetype: *const Filetype) -> bool {
    let Some(d) = ((*indexer).indexer_data as *mut Declared).as_mut() else {
        return false;
    };
    if let Some(ft) = filetype.as_ref() {
        if let Some(ext) = text(ft.file_extension) {
            d.extensions
                .push(ext.trim_start_matches('.').to_ascii_lowercase());
        }
    }
    true
}

unsafe extern "C" fn declare_location(indexer: *const Indexer, location: *const Location) -> bool {
    let Some(d) = ((*indexer).indexer_data as *mut Declared).as_mut() else {
        return false;
    };
    if let Some(l) = location.as_ref() {
        d.locations.push((l.kind, text(l.location)));
    }
    true
}

unsafe extern "C" fn declare_soundpack(_: *const Indexer, _: *const c_void) -> bool {
    true
}

unsafe extern "C" fn indexer_extension(_: *const Indexer, _: *const c_char) -> *const c_void {
    std::ptr::null()
}

/// The presets one `get_metadata` call describes, kept when they are for
/// this plug-in (or name no plug-in at all).
struct Collected {
    plugin_id: String,
    kind: u32,
    location: Option<String>,
    presets: Vec<ClapPreset>,
    /// Whether the preset being described names this plug-in, or none.
    current_ours: Option<bool>,
}

impl Collected {
    fn close_current(&mut self) {
        if self.current_ours == Some(false) {
            self.presets.pop();
        }
        self.current_ours = None;
    }
}

unsafe extern "C" fn on_error(_: *const MetadataReceiver, _: i32, _: *const c_char) {}

unsafe extern "C" fn begin_preset(
    r: *const MetadataReceiver,
    name: *const c_char,
    key: *const c_char,
) -> bool {
    let Some(c) = ((*r).receiver_data as *mut Collected).as_mut() else {
        return false;
    };
    c.close_current();
    c.presets.push(ClapPreset {
        name: text(name).unwrap_or_else(|| "Preset".into()),
        location_kind: c.kind,
        location: c.location.clone(),
        load_key: text(key),
    });
    c.current_ours = None;
    true
}

unsafe extern "C" fn add_plugin_id(r: *const MetadataReceiver, id: *const UniversalPluginId) {
    let Some(c) = ((*r).receiver_data as *mut Collected).as_mut() else {
        return;
    };
    let Some(id) = id.as_ref() else { return };
    let ours = text(id.abi).as_deref() == Some("clap")
        && text(id.id).as_deref() == Some(c.plugin_id.as_str());
    c.current_ours = Some(c.current_ours == Some(true) || ours);
}

unsafe extern "C" fn ignore_text(_: *const MetadataReceiver, _: *const c_char) {}
unsafe extern "C" fn ignore_flags(_: *const MetadataReceiver, _: u32) {}
unsafe extern "C" fn ignore_times(_: *const MetadataReceiver, _: u64, _: u64) {}
unsafe extern "C" fn ignore_pair(_: *const MetadataReceiver, _: *const c_char, _: *const c_char) {}

fn receiver(c: &mut Collected) -> MetadataReceiver {
    MetadataReceiver {
        receiver_data: c as *mut Collected as *mut c_void,
        on_error,
        begin_preset,
        add_plugin_id,
        set_soundpack_id: ignore_text,
        set_flags: ignore_flags,
        add_creator: ignore_text,
        set_description: ignore_text,
        set_timestamps: ignore_times,
        add_feature: ignore_text,
        add_extra_info: ignore_pair,
    }
}

/// Preset files under `root` with one of `extensions` (any file when none
/// are declared), in name order, so a list made twice is the same list and
/// a preset is found again by its place in it. Bounded in depth and count:
/// a location could be a whole drive.
fn preset_files(root: &Path, extensions: &[String]) -> Vec<PathBuf> {
    const MAX_FILES: usize = 5000;
    const MAX_DEPTH: usize = 6;
    let mut out = Vec::new();
    if root.is_file() {
        out.push(root.to_path_buf());
        return out;
    }
    let mut dirs = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries.into_iter().rev() {
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    dirs.push((path, depth + 1));
                }
            } else {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .unwrap_or_default();
                if extensions.is_empty() || extensions.contains(&ext) {
                    out.push(path);
                }
            }
        }
        if out.len() >= MAX_FILES {
            out.truncate(MAX_FILES);
            break;
        }
    }
    out.sort();
    out
}

/// Every preset the plug-in's providers describe for `plugin_id` (the
/// factory's own id, not the DAW's "clap:" form). Empty when the plug-in
/// has no discovery factory.
///
/// # Safety
/// `entry` is the plug-in's initialised `clap_entry`.
pub unsafe fn discover(entry: *const ClapPluginEntry, plugin_id: &str) -> Vec<ClapPreset> {
    let factory = FACTORY_IDS
        .iter()
        .map(|id| ((*entry).get_factory)(id.as_ptr() as *const c_char) as *const DiscoveryFactory)
        .find(|f| !f.is_null());
    let Some(factory) = factory else {
        return Vec::new();
    };
    let name = CString::new("Hardwave DAW").unwrap();
    let vendor = CString::new("Hardwave Studios").unwrap();
    let mut presets = Vec::new();
    for i in 0..((*factory).count)(factory) {
        let desc = ((*factory).get_descriptor)(factory, i);
        let Some(desc) = desc.as_ref() else { continue };
        let mut declared = Declared::default();
        let indexer = Indexer {
            clap_version: ClapVersion {
                major: 1,
                minor: 2,
                revision: 0,
            },
            name: name.as_ptr(),
            vendor: vendor.as_ptr(),
            url: std::ptr::null(),
            version: std::ptr::null(),
            indexer_data: &mut declared as *mut Declared as *mut c_void,
            declare_filetype,
            declare_location,
            declare_soundpack,
            get_extension: indexer_extension,
        };
        let provider = ((*factory).create)(factory, &indexer, desc.id);
        if provider.is_null() {
            continue;
        }
        if ((*provider).init)(provider) {
            for (kind, location) in declared.locations.clone() {
                let targets: Vec<Option<String>> = match (kind, &location) {
                    (LOCATION_FILE, Some(root)) => {
                        preset_files(Path::new(root), &declared.extensions)
                            .into_iter()
                            .map(|p| Some(p.to_string_lossy().into_owned()))
                            .collect()
                    }
                    (LOCATION_FILE, None) => Vec::new(),
                    _ => vec![None],
                };
                for target in targets {
                    let mut collected = Collected {
                        plugin_id: plugin_id.to_string(),
                        kind,
                        location: target.clone(),
                        presets: Vec::new(),
                        current_ours: None,
                    };
                    let rx = receiver(&mut collected);
                    let loc = target.as_deref().and_then(|t| CString::new(t).ok());
                    ((*provider).get_metadata)(
                        provider,
                        kind,
                        loc.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
                        &rx,
                    );
                    collected.close_current();
                    presets.append(&mut collected.presets);
                }
            }
        }
        ((*provider).destroy)(provider);
    }
    presets
}

/// Load `preset` into the plug-in through its `preset-load` extension.
///
/// # Safety
/// `plugin` is a live, initialised `clap_plugin`.
pub unsafe fn load(plugin: *const ClapPlugin, preset: &ClapPreset) -> Result<(), String> {
    let ext = LOAD_EXT_IDS
        .iter()
        .map(|id| {
            ((*plugin).get_extension)(plugin, id.as_ptr() as *const c_char) as *const PresetLoad
        })
        .find(|e| !e.is_null())
        .ok_or("this plug-in cannot load its presets for the host")?;
    let location = preset
        .location
        .as_deref()
        .and_then(|l| CString::new(l).ok());
    let key = preset
        .load_key
        .as_deref()
        .and_then(|k| CString::new(k).ok());
    let ok = ((*ext).from_location)(
        plugin,
        preset.location_kind,
        location.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
        key.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()),
    );
    if ok {
        Ok(())
    } else {
        Err(format!("the plug-in did not load \"{}\"", preset.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// A provider describing three presets: one for this plug-in, one for
    /// another, one naming no plug-in. The other plug-in's is left out.
    #[test]
    fn only_this_plug_ins_presets_are_kept() {
        let mut col = Collected {
            plugin_id: "com.maker.synth".into(),
            kind: LOCATION_PLUGIN,
            location: None,
            presets: Vec::new(),
            current_ours: None,
        };
        let rx = receiver(&mut col);
        let (abi, ours, theirs) = (c("clap"), c("com.maker.synth"), c("com.maker.fx"));
        unsafe {
            (rx.begin_preset)(&rx, c("Lead").as_ptr(), c("k1").as_ptr());
            (rx.add_plugin_id)(
                &rx,
                &UniversalPluginId {
                    abi: abi.as_ptr(),
                    id: ours.as_ptr(),
                },
            );
            (rx.begin_preset)(&rx, c("Reverb").as_ptr(), c("k2").as_ptr());
            (rx.add_plugin_id)(
                &rx,
                &UniversalPluginId {
                    abi: abi.as_ptr(),
                    id: theirs.as_ptr(),
                },
            );
            (rx.begin_preset)(&rx, c("Init").as_ptr(), std::ptr::null());
        }
        col.close_current();
        let names: Vec<&str> = col.presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Lead", "Init"]);
        assert_eq!(col.presets[0].load_key.as_deref(), Some("k1"));
        assert_eq!(col.presets[1].load_key, None);
    }

    /// Preset files are found by their extension, in name order, also in
    /// folders below.
    #[test]
    fn preset_files_are_found_in_order() {
        let dir = std::env::temp_dir().join(format!("hw-clap-presets-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Bass")).unwrap();
        for f in ["b.fxp2", "a.fxp2", "notes.txt", "Bass/sub.FXP2"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let found: Vec<String> = preset_files(&dir, &["fxp2".into()])
            .iter()
            .map(|p| {
                p.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(found, vec!["Bass/sub.FXP2", "a.fxp2", "b.fxp2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
