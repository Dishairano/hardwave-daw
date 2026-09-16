//! Hardwave Plugin Host — scan, load, and run VST3/CLAP plugins.

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
