//! Hardwave DSP — audio file I/O, sample rate conversion, fades, time stretching, distortion.

pub mod audio_file;
pub mod distortion;
pub mod fade;

pub use audio_file::{AudioFileInfo, AudioFileReader};
pub use fade::{apply_fade, FadeCurve};
