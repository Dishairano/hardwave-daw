//! Hardwave Native Plugins — in-process adapters that wrap `hardwave-dsp`
//! primitives in the `HostedPlugin` trait so the audio engine can host
//! them alongside VST3 / CLAP plugins without an FFI roundtrip.

pub mod compressor;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod filter;
pub mod limiter;
pub mod reverb;
pub mod stereo;

pub use compressor::NativeCompressor;
pub use delay::NativeDelay;
pub use distortion::NativeDistortion;
pub use eq::NativeEq;
pub use filter::NativeFilter;
pub use limiter::NativeLimiter;
pub use reverb::NativeReverb;
pub use stereo::NativeStereo;

use hardwave_plugin_host::types::PluginDescriptor;

pub fn native_plugin_descriptors() -> Vec<PluginDescriptor> {
    vec![
        NativeEq::descriptor(),
        NativeCompressor::descriptor(),
        NativeLimiter::descriptor(),
        NativeDistortion::descriptor(),
        NativeFilter::descriptor(),
        NativeDelay::descriptor(),
        NativeReverb::descriptor(),
        NativeStereo::descriptor(),
    ]
}

pub fn native_plugin_ids() -> Vec<&'static str> {
    vec![
        NativeEq::ID,
        NativeCompressor::ID,
        NativeLimiter::ID,
        NativeDistortion::ID,
        NativeFilter::ID,
        NativeDelay::ID,
        NativeReverb::ID,
        NativeStereo::ID,
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}
