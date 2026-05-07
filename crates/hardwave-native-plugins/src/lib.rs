//! Hardwave Native Plugins — in-process adapters that wrap `hardwave-dsp`
//! primitives in the `HostedPlugin` trait so the audio engine can host
//! them alongside VST3 / CLAP plugins without an FFI roundtrip.

pub mod auto_pan;
pub mod bitcrush;
pub mod chorus;
pub mod compressor;
pub mod conv_reverb;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod filter;
pub mod flanger;
pub mod fm;
pub mod gain;
pub mod gate;
pub mod limiter;
pub mod mid_side;
pub mod multiband;
pub mod noise;
pub mod phaser;
pub mod reverb;
pub mod saturator;
pub mod stereo;
pub mod sub_bass;
pub mod transient;
pub mod tremolo;
pub mod triple_osc;
pub mod vibrato;
pub mod wavetable;

pub use auto_pan::NativeAutoPan;
pub use bitcrush::NativeBitcrush;
pub use chorus::NativeChorus;
pub use compressor::NativeCompressor;
pub use conv_reverb::NativeConvReverb;
pub use delay::NativeDelay;
pub use distortion::NativeDistortion;
pub use eq::NativeEq;
pub use filter::NativeFilter;
pub use flanger::NativeFlanger;
pub use fm::NativeFmSynth;
pub use gain::NativeGain;
pub use gate::NativeGate;
pub use limiter::NativeLimiter;
pub use mid_side::NativeMidSide;
pub use multiband::NativeMultiband;
pub use noise::NativeNoise;
pub use phaser::NativePhaser;
pub use reverb::NativeReverb;
pub use saturator::NativeSaturator;
pub use stereo::NativeStereo;
pub use sub_bass::NativeSubBass;
pub use transient::NativeTransient;
pub use tremolo::NativeTremolo;
pub use triple_osc::NativeTripleOsc;
pub use vibrato::NativeVibrato;
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
        NativeChorus::descriptor(),
        NativePhaser::descriptor(),
        NativeConvReverb::descriptor(),
        NativeTremolo::descriptor(),
        NativeFlanger::descriptor(),
        NativeAutoPan::descriptor(),
        NativeBitcrush::descriptor(),
        NativeGain::descriptor(),
        NativeSaturator::descriptor(),
        NativeNoise::descriptor(),
        NativeSubBass::descriptor(),
        NativeVibrato::descriptor(),
        NativeMidSide::descriptor(),
        NativeGate::descriptor(),
        NativeTransient::descriptor(),
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
        NativeChorus::ID,
        NativePhaser::ID,
        NativeConvReverb::ID,
        NativeTremolo::ID,
        NativeFlanger::ID,
        NativeAutoPan::ID,
        NativeBitcrush::ID,
        NativeGain::ID,
        NativeSaturator::ID,
        NativeNoise::ID,
        NativeSubBass::ID,
        NativeVibrato::ID,
        NativeMidSide::ID,
        NativeGate::ID,
        NativeTransient::ID,
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}
