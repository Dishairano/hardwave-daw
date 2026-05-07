//! Hardwave Native Plugins — in-process adapters that wrap `hardwave-dsp`
//! primitives in the `HostedPlugin` trait so the audio engine can host
//! them alongside VST3 / CLAP plugins without an FFI roundtrip.

pub mod compressor;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod filter;
pub mod fm;
pub mod limiter;
pub mod multiband;
pub mod reverb;
pub mod stereo;
pub mod triple_osc;
pub mod wavetable;

pub use compressor::NativeCompressor;
pub use delay::NativeDelay;
pub use distortion::NativeDistortion;
pub use eq::NativeEq;
pub use filter::NativeFilter;
pub use fm::NativeFmSynth;
pub use limiter::NativeLimiter;
pub use multiband::NativeMultiband;
pub use reverb::NativeReverb;
pub use stereo::NativeStereo;
pub use triple_osc::NativeTripleOsc;
pub use wavetable::NativeWavetable;

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
        NativeMultiband::descriptor(),
        NativeTripleOsc::descriptor(),
        NativeFmSynth::descriptor(),
        NativeWavetable::descriptor(),
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
        NativeMultiband::ID,
        NativeTripleOsc::ID,
        NativeFmSynth::ID,
        NativeWavetable::ID,
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}
