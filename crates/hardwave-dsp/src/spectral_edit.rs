//! Painting something out of a recording.
//!
//! A cough in a vocal take, a chair creak under a verse, a click in a
//! bounce: in time they are mixed in with everything else, but on a
//! spectrogram they sit in their own patch of time and frequency, and
//! a patch can be rubbed out.
//!
//! An STFT, a mask over the magnitudes, and the inverse. Phases are
//! kept as they were, which is what keeps the rest of the audio
//! untouched: only the patches the user painted are changed.

use rustfft::{num_complex::Complex, FftPlanner};

/// A patch of the spectrogram, as the user drew it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Patch {
    pub start_sample: u64,
    pub end_sample: u64,
    pub low_hz: f32,
    pub high_hz: f32,
    /// 0 leaves it alone, 1 takes it out completely. Part way is
    /// usually better on anything tonal, because a hole is as
    /// noticeable as the noise was.
    pub strength: f32,
}

/// The analysis settings. The window is the trade: long enough to tell
/// two close frequencies apart, short enough to tell two close moments
/// apart.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub sample_rate: f64,
    pub window: usize,
    pub hop: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000.0,
            window: 2048,
            hop: 512,
        }
    }
}

fn hann(n: usize, window: usize) -> f32 {
    let x = n as f32 / window as f32;
    0.5 - 0.5 * (std::f32::consts::TAU * x).cos()
}

/// Rub the patches out of a recording.
///
/// The audio comes back the same length. A sample the patches do not
/// cover comes back as it was, within what the overlap-add can do,
/// which is why the window and hop are fixed to a shape that sums to
/// one.
pub fn erase(samples: &[f32], patches: &[Patch], settings: &Settings) -> Vec<f32> {
    let window = settings.window.max(64);
    let hop = settings.hop.clamp(1, window / 2);
    if samples.is_empty() || patches.is_empty() {
        return samples.to_vec();
    }

    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(window);
    let inverse = planner.plan_fft_inverse(window);

    let mut out = vec![0.0f32; samples.len()];
    let mut weight = vec![0.0f32; samples.len()];
    let mut buffer: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); window];

    let bin_hz = settings.sample_rate as f32 / window as f32;
    let mut position = 0usize;
    while position < samples.len() {
        // Read a window, zero-padded at the end of the take.
        for (i, slot) in buffer.iter_mut().enumerate() {
            let sample = samples.get(position + i).copied().unwrap_or(0.0);
            *slot = Complex::new(sample * hann(i, window), 0.0);
        }
        forward.process(&mut buffer);

        // What the patches ask for at this moment, per bin.
        let frame_start = position as u64;
        let frame_end = (position + window) as u64;
        for patch in patches {
            if patch.end_sample <= frame_start || patch.start_sample >= frame_end {
                continue;
            }
            let strength = patch.strength.clamp(0.0, 1.0);
            if strength <= 0.0 {
                continue;
            }
            let low_bin = (patch.low_hz / bin_hz).floor().max(0.0) as usize;
            let high_bin = ((patch.high_hz / bin_hz).ceil() as usize).min(window / 2);
            for bin in low_bin..=high_bin {
                if bin > window / 2 {
                    break;
                }
                let gain = 1.0 - strength;
                buffer[bin] *= gain;
                // The top half of the spectrum mirrors the bottom; a
                // real signal has to come back out, so both go.
                if bin > 0 && bin < window / 2 {
                    let mirror = window - bin;
                    buffer[mirror] *= gain;
                }
            }
        }

        inverse.process(&mut buffer);
        let scale = 1.0 / window as f32;
        for i in 0..window {
            let Some(slot) = out.get_mut(position + i) else {
                break;
            };
            let w = hann(i, window);
            *slot += buffer[i].re * scale * w;
            weight[position + i] += w * w;
        }
        position += hop;
    }

    // Undo the two windows each sample went through, so untouched
    // audio comes back as itself rather than quieter.
    for (sample, w) in out.iter_mut().zip(weight.iter()) {
        if *w > 1e-6 {
            *sample /= *w;
        }
    }
    out
}

/// A spectrogram to draw on: magnitudes in decibels, one row of bins
/// per frame.
///
/// The UI needs the same analysis the editing uses, so it lives next
/// to it rather than being worked out twice with different windows.
pub fn spectrogram(samples: &[f32], settings: &Settings) -> (Vec<Vec<f32>>, usize) {
    let window = settings.window.max(64);
    let hop = settings.hop.clamp(1, window / 2);
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(window);
    let mut buffer: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); window];
    let bins = window / 2;

    let mut frames = Vec::new();
    let mut position = 0usize;
    while position < samples.len() {
        for (i, slot) in buffer.iter_mut().enumerate() {
            let sample = samples.get(position + i).copied().unwrap_or(0.0);
            *slot = Complex::new(sample * hann(i, window), 0.0);
        }
        forward.process(&mut buffer);
        let row: Vec<f32> = buffer[..bins]
            .iter()
            .map(|c| {
                let magnitude = c.norm() / (window as f32 * 0.5);
                20.0 * (magnitude.max(1e-7)).log10()
            })
            .collect();
        frames.push(row);
        position += hop;
    }
    (frames, bins)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, seconds: f64, sample_rate: f64) -> Vec<f32> {
        let n = (seconds * sample_rate) as usize;
        (0..n)
            .map(|i| (std::f64::consts::TAU * hz * i as f64 / sample_rate).sin() as f32 * 0.5)
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
    }

    #[test]
    fn with_nothing_painted_the_audio_comes_back_as_itself() {
        let settings = Settings::default();
        let audio = tone(440.0, 0.5, settings.sample_rate);
        let out = erase(&audio, &[], &settings);
        assert_eq!(out, audio);
    }

    #[test]
    fn a_painted_patch_takes_that_frequency_out_and_leaves_the_rest() {
        let settings = Settings::default();
        let low = tone(200.0, 1.0, settings.sample_rate);
        let high = tone(3000.0, 1.0, settings.sample_rate);
        let mixed: Vec<f32> = low.iter().zip(high.iter()).map(|(a, b)| a + b).collect();

        let out = erase(
            &mixed,
            &[Patch {
                start_sample: 0,
                end_sample: mixed.len() as u64,
                low_hz: 2500.0,
                high_hz: 3500.0,
                strength: 1.0,
            }],
            &settings,
        );
        assert_eq!(out.len(), mixed.len());

        // What is left should be the low tone on its own, so it should
        // sit close to the low tone's own level rather than the mix's.
        let middle = out.len() / 4..out.len() * 3 / 4;
        let left = rms(&out[middle.clone()]);
        let low_only = rms(&low[middle.clone()]);
        let both = rms(&mixed[middle]);
        assert!(
            (left - low_only).abs() < low_only * 0.2,
            "the low tone should survive: {left} against {low_only}"
        );
        assert!(left < both * 0.9, "the high tone should be gone");
    }

    #[test]
    fn a_patch_only_touches_the_moment_it_covers() {
        let settings = Settings::default();
        let audio = tone(1000.0, 1.0, settings.sample_rate);
        let half = audio.len() as u64 / 2;
        let out = erase(
            &audio,
            &[Patch {
                start_sample: 0,
                end_sample: half,
                low_hz: 800.0,
                high_hz: 1200.0,
                strength: 1.0,
            }],
            &settings,
        );
        let first = rms(&out[..(half / 2) as usize]);
        let second = rms(&out[(audio.len() * 3 / 4)..]);
        assert!(first < 0.05, "the painted half should be quiet: {first}");
        assert!(second > 0.2, "the rest should be untouched: {second}");
    }

    #[test]
    fn strength_part_way_leaves_part_of_it() {
        let settings = Settings::default();
        let audio = tone(1000.0, 0.5, settings.sample_rate);
        let patch = |strength: f32| Patch {
            start_sample: 0,
            end_sample: audio.len() as u64,
            low_hz: 800.0,
            high_hz: 1200.0,
            strength,
        };
        let middle = audio.len() / 4..audio.len() * 3 / 4;
        let full = rms(&erase(&audio, &[patch(1.0)], &settings)[middle.clone()]);
        let half = rms(&erase(&audio, &[patch(0.5)], &settings)[middle.clone()]);
        let none = rms(&audio[middle]);
        assert!(full < half, "{full} should be quieter than {half}");
        assert!(half < none, "{half} should be quieter than {none}");
    }

    #[test]
    fn a_spectrogram_puts_a_tone_in_the_bin_it_belongs_in() {
        let settings = Settings::default();
        let audio = tone(1000.0, 0.3, settings.sample_rate);
        let (frames, bins) = spectrogram(&audio, &settings);
        assert!(!frames.is_empty());
        assert_eq!(frames[0].len(), bins);
        let row = &frames[frames.len() / 2];
        let loudest = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(bin, _)| bin)
            .unwrap();
        let hz = loudest as f64 * settings.sample_rate / settings.window as f64;
        assert!((hz - 1000.0).abs() < 50.0, "the peak sat at {hz} Hz");
    }
}
