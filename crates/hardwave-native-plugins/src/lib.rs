//! Hardwave Native Plugins — in-process adapters that wrap `hardwave-dsp`
//! primitives in the `HostedPlugin` trait so the audio engine can host
//! them alongside VST3 / CLAP plugins without an FFI roundtrip.

// Two clippy lints introduced by newer stable toolchains fire across the
// per-sample DSP process loops here. Both are style-only and the code is
// shipped + working, so we suppress them crate-wide rather than rewrite
// hot audio loops blind:
//   * needless_range_loop — the `for i in 0..n` loops index parallel
//     input/output buffers by the same sample index on purpose.
//   * approx_constant — `0.707` is the conventional, readable Butterworth
//     filter Q; spelling it `FRAC_1_SQRT_2` reads worse in a Q field.
#![allow(clippy::needless_range_loop)]
#![allow(clippy::approx_constant)]

pub mod auto_filter;
pub mod auto_pan;
pub mod bitcrush;
pub mod chorus;
pub mod clipper;
pub mod compressor;
pub mod conv_reverb;
pub mod delay;
pub mod distortion;
pub mod eq;
pub mod exciter;
pub mod filter;
pub mod flanger;
pub mod fm;
pub mod format;
pub mod gain;
pub mod gate;
pub mod limiter;
pub mod mid_side;
pub mod mono_fold;
pub mod multiband;
pub mod noise;
pub mod phaser;
pub mod reverb;
pub mod ring_mod;
pub mod saturator;
pub mod soundgoodizer;
pub mod stereo;
pub mod stereo_double;
pub mod sub_bass;
pub mod tape;
pub mod transient;
pub mod tremolo;
pub mod triple_osc;
pub mod vibrato;
pub mod wavetable;

pub mod sampler;
pub mod stutter;
pub mod vocoder;

pub use auto_filter::NativeAutoFilter;
pub use auto_pan::NativeAutoPan;
pub use bitcrush::NativeBitcrush;
pub use chorus::NativeChorus;
pub use clipper::NativeClipper;
pub use compressor::NativeCompressor;
pub use conv_reverb::NativeConvReverb;
pub use delay::NativeDelay;
pub use distortion::NativeDistortion;
pub use eq::NativeEq;
pub use exciter::NativeExciter;
pub use filter::NativeFilter;
pub use flanger::NativeFlanger;
pub use fm::NativeFmSynth;
pub use gain::NativeGain;
pub use gate::NativeGate;
pub use limiter::NativeLimiter;
pub use mid_side::NativeMidSide;
pub use mono_fold::NativeMonoFold;
pub use multiband::NativeMultiband;
pub use noise::NativeNoise;
pub use phaser::NativePhaser;
pub use reverb::NativeReverb;
pub use ring_mod::NativeRingMod;
pub use sampler::NativeSampler;
pub use saturator::NativeSaturator;
pub use soundgoodizer::NativeSoundgoodizer;
pub use stereo::NativeStereo;
pub use stereo_double::NativeStereoDouble;
pub use stutter::NativeStutter;
pub use sub_bass::NativeSubBass;
pub use tape::NativeTape;
pub use transient::NativeTransient;
pub use tremolo::NativeTremolo;
pub use triple_osc::NativeTripleOsc;
pub use vibrato::NativeVibrato;
pub use vocoder::NativeVocoder;
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
        NativeClipper::descriptor(),
        NativeExciter::descriptor(),
        NativeTape::descriptor(),
        NativeSoundgoodizer::descriptor(),
        NativeMonoFold::descriptor(),
        NativeRingMod::descriptor(),
        NativeAutoFilter::descriptor(),
        NativeStereoDouble::descriptor(),
        NativeVocoder::descriptor(),
        NativeStutter::descriptor(),
        NativeSampler::descriptor(),
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
        NativeClipper::ID,
        NativeExciter::ID,
        NativeTape::ID,
        NativeSoundgoodizer::ID,
        NativeMonoFold::ID,
        NativeRingMod::ID,
        NativeAutoFilter::ID,
        NativeStereoDouble::ID,
        NativeVocoder::ID,
        NativeStutter::ID,
        NativeSampler::ID,
        "hardwave.analyser",
        "hardwave.loudlab",
        "hardwave.wettboi",
        "hardwave.kickforge",
    ]
}

#[cfg(test)]
mod value_text_tests {
    use super::*;
    use hardwave_plugin_host::types::HostedPlugin;

    fn all() -> Vec<Box<dyn HostedPlugin>> {
        vec![
            Box::new(NativeEq::new()),
            Box::new(NativeCompressor::new()),
            Box::new(NativeLimiter::new()),
            Box::new(NativeDistortion::new()),
            Box::new(NativeFilter::new()),
            Box::new(NativeDelay::new()),
            Box::new(NativeReverb::new()),
            Box::new(NativeStereo::new()),
            Box::new(NativeMultiband::new()),
            Box::new(NativeTripleOsc::new()),
            Box::new(NativeFmSynth::new()),
            Box::new(NativeWavetable::new()),
            Box::new(NativeChorus::new()),
            Box::new(NativePhaser::new()),
            Box::new(NativeConvReverb::new()),
            Box::new(NativeTremolo::new()),
            Box::new(NativeFlanger::new()),
            Box::new(NativeAutoPan::new()),
            Box::new(NativeBitcrush::new()),
            Box::new(NativeGain::new()),
            Box::new(NativeSaturator::new()),
            Box::new(NativeNoise::new()),
            Box::new(NativeSubBass::new()),
            Box::new(NativeVibrato::new()),
            Box::new(NativeMidSide::new()),
            Box::new(NativeGate::new()),
            Box::new(NativeTransient::new()),
            Box::new(NativeClipper::new()),
            Box::new(NativeExciter::new()),
            Box::new(NativeTape::new()),
            Box::new(NativeSoundgoodizer::new()),
            Box::new(NativeMonoFold::new()),
            Box::new(NativeRingMod::new()),
            Box::new(NativeAutoFilter::new()),
            Box::new(NativeStereoDouble::new()),
            Box::new(NativeVocoder::new()),
            Box::new(NativeStutter::new()),
            Box::new(NativeSampler::new()),
        ]
    }

    /// Every parameter of every built-in says its value in words, across
    /// its whole range, and a parameter with choices names the one it is on.
    /// The plug-in windows show these instead of raw 0..1 numbers.
    #[test]
    fn every_built_in_parameter_has_text() {
        for p in all() {
            let name = p.descriptor().name.clone();
            for i in 0..p.get_parameter_count() {
                let info = p.get_parameter_info(i).expect("parameter info");
                for v in [
                    info.min,
                    info.default_value,
                    info.max,
                    (info.min + info.max) / 2.0,
                ] {
                    let text = p.parameter_text(info.id, v);
                    assert!(
                        text.as_deref().is_some_and(|t| !t.is_empty()),
                        "{name} / {} has no text at {v}",
                        info.name
                    );
                }
                if let Some(options) = p.parameter_options(info.id) {
                    assert!(
                        options.len() > 1,
                        "{name} / {}: one option is not a choice",
                        info.name
                    );
                    let n = options.len();
                    for (k, label) in options.iter().enumerate() {
                        let v = info.min + (info.max - info.min) * k as f64 / (n - 1) as f64;
                        assert_eq!(
                            p.parameter_text(info.id, v).as_deref(),
                            Some(label.as_str()),
                            "{name} / {}: option {k} and its text disagree",
                            info.name
                        );
                    }
                }
            }
        }
    }
}
