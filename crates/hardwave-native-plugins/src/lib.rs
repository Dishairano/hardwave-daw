//! Hardwave Native Plugins — in-process adapters that wrap `hardwave-dsp`
//! primitives in the `HostedPlugin` trait so the audio engine can host
//! them alongside VST3 / CLAP plugins without an FFI roundtrip.

pub mod compressor;
pub mod eq;

pub use compressor::NativeCompressor;
pub use eq::NativeEq;

use hardwave_plugin_host::types::PluginDescriptor;

/// Return the catalog of native-plugin descriptors the scanner should
/// register alongside external VST3 / CLAP scan results.
pub fn native_plugin_descriptors() -> Vec<PluginDescriptor> {
    vec![NativeEq::descriptor(), NativeCompressor::descriptor()]
}

/// Stable plugin ids — matches `PluginDescriptor::id`. Kept for
/// backwards compatibility; the four prior webview-only plugins
/// (analyser / loudlab / wettboi / kickforge) are still listed so the
/// UI can reference them even before the webview host is wired into
/// `native_plugin_descriptors`.
pub fn native_plugin_ids() -> Vec<&'static str> {
    vec![
        NativeEq::ID,
        NativeCompressor::ID,
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}
