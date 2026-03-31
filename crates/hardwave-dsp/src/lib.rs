//! Hardwave DSP — audio file I/O, sample rate conversion, fades, time stretching.

pub mod audio_file;
pub mod fade;

pub use audio_file::{AudioFileReader, AudioFileInfo};
pub use fade::{FadeCurve, apply_fade};
