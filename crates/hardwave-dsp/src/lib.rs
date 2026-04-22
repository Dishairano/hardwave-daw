//! Hardwave DSP — audio file I/O, sample rate conversion, fades, time stretching, distortion, filters.

pub mod audio_file;
pub mod auto_eq;
pub mod biquad;
pub mod chord_detect;
pub mod convolution;
pub mod delay_line;
pub mod distortion;
pub mod drum_machine;
pub mod dynamics;
pub mod fade;
pub mod fm_synth;
pub mod ir_library;
pub mod latency;
pub mod mix_feedback;
pub mod modulation;
pub mod multiband;
pub mod parametric_eq;
pub mod recording;
pub mod reverb;
pub mod sample_classify;
pub mod stereo;
pub mod synth;
pub mod wavetable;

pub use audio_file::{AudioFileInfo, AudioFileReader};
pub use biquad::{Biquad, BiquadKind};
pub use convolution::ConvolutionReverb;
pub use delay_line::StereoDelayLine;
pub use dynamics::{DetectMode, EnvelopeFollower, GainReductionMeter};
pub use fade::{apply_fade, FadeCurve};
pub use modulation::{AllpassStage, ModulatedDelay, PhaserChain};
pub use reverb::AlgorithmicReverb;
pub use stereo::{BassMono, CorrelationMeter, HaasDelay};
pub use synth::{AdsrEnvelope, AdsrStage, Oscillator, Waveform};
