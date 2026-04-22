//! Hardwave DSP — audio file I/O, sample rate conversion, fades, time stretching, distortion, filters.

pub mod audio_file;
pub mod biquad;
pub mod delay_line;
pub mod distortion;
pub mod dynamics;
pub mod fade;
pub mod stereo;
pub mod synth;

pub use audio_file::{AudioFileInfo, AudioFileReader};
pub use biquad::{Biquad, BiquadKind};
pub use delay_line::StereoDelayLine;
pub use dynamics::{DetectMode, EnvelopeFollower};
pub use fade::{apply_fade, FadeCurve};
pub use stereo::{BassMono, CorrelationMeter, HaasDelay};
pub use synth::{AdsrEnvelope, AdsrStage, Oscillator, Waveform};
