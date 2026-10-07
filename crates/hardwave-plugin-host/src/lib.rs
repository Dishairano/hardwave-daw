//! Hardwave Plugin Host — scan, load, and run VST3/CLAP plugins.

pub mod binary_arch;
pub mod bridge_child;
pub mod bridge_protocol;
pub mod clap_ffi;
pub mod clap_instance;
pub mod gui_scaling;
pub mod plugin_ops;
pub mod scanner;
pub mod types;
pub mod vst3;

pub use scanner::PluginScanner;
pub use types::*;

/// Load a plug-in far enough to know it will not take the host down.
///
/// Called in a throwaway child process (see the DAW's `--probe-plugin`): a
/// VST3 or CLAP is someone else's C++, and the dangerous moment is loading
/// the binary and instantiating the class, not scanning, which only reads
/// moduleinfo.json. If that kills a process, this is the process it should
/// kill.
///
/// Deliberately stops at instantiation. Rendering audio here would test more,
/// but it would also mean a plug-in that only misbehaves under load gets
/// blocked for everyone on a machine that happened to stutter during the
/// probe.
pub fn probe_load(path: &std::path::Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let descriptors = match ext.as_str() {
        "vst3" => scanner::describe_vst3(path),
        "clap" => scanner::describe_clap(path),
        other => return Err(format!("not a plug-in this host loads: .{other}")),
    };
    let descriptor = descriptors
        .into_iter()
        .next()
        .ok_or_else(|| format!("no plug-in class found in {}", path.display()))?;

    match descriptor.format {
        types::PluginFormat::Vst3 => {
            vst3::Vst3PluginInstance::load(descriptor).map(|_| ())?;
        }
        types::PluginFormat::Clap => {
            clap_instance::ClapPluginInstance::load(descriptor).map(|_| ())?;
        }
    }
    Ok(())
}

/// Load a plug-in's library.
///
/// On Windows the library's own dependencies are looked for beside it
/// and in the system folders, never in the current folder or along the
/// PATH: a DLL dropped next to wherever the DAW was started from, or in
/// a folder on the PATH, must not be loaded into it in place of the real
/// one. That is also where plug-ins that ship their own DLLs keep them.
///
/// # Safety
/// Loading a library runs its initialisation code; the caller accepts
/// that for a plug-in it has decided to host.
pub unsafe fn load_plugin_library(
    path: &std::path::Path,
) -> Result<libloading::Library, libloading::Error> {
    #[cfg(windows)]
    {
        use libloading::os::windows::{
            Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
        };
        // Safety: as the function's own contract.
        unsafe {
            Library::load_with_flags(
                path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        }
        .map(libloading::Library::from)
    }
    #[cfg(not(windows))]
    {
        // Safety: as the function's own contract.
        unsafe { libloading::Library::new(path) }
    }
}
