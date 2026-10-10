//! Running the mix at 44.1 or 48 kHz on a device that only takes a
//! multiple of that.
//!
//! In WASAPI shared mode an output plays at the rate Windows has set for
//! it, and many onboard and gaming devices are set to 96 or 192 kHz. The
//! engine then ran at 192 kHz: four times the work for every track and
//! plug-in, blocks of 2.7 ms, and a project of 48 tracks crackled and
//! broke up within seconds. When the device's rate is 2 or 4 times the
//! rate asked for, the engine now runs at the rate asked for and the
//! callback raises it to the device's rate here.
//!
//! The raise is a polyphase windowed-sinc filter: the k output samples
//! for each input sample are k short dot products, about 64 multiplies an
//! output sample for both channels, with a quarter of a millisecond of
//! delay at 192 kHz.

use crate::AudioCallback;

/// Taps per phase: the filter is `k * TAPS_PER_PHASE` long.
const TAPS_PER_PHASE: usize = 16;

/// How many times the device's rate is the rate asked for, when that is a
/// rate the engine is happy to run at and the device's is 2 or 4 times it.
/// 1 means play at the device's rate as it is.
pub fn factor_for(requested: u32, device: u32) -> u32 {
    if requested == 0 || device <= requested {
        return 1;
    }
    for base in [requested, 48_000, 44_100] {
        if base > 0 && device.is_multiple_of(base) {
            let k = device / base;
            if k == 2 || k == 4 {
                return k;
            }
        }
    }
    1
}

/// One channel's polyphase filter.
struct Phase {
    /// The last TAPS_PER_PHASE input samples, newest at `pos`.
    history: [f32; TAPS_PER_PHASE],
    pos: usize,
}

/// Raises a stereo signal by an integer factor.
pub struct Upsampler {
    k: usize,
    /// Coefficients laid out by phase: `coeffs[p][j]` is tap `p + j * k`.
    coeffs: Vec<[f32; TAPS_PER_PHASE]>,
    channels: [Phase; 2],
}

impl Upsampler {
    pub fn new(k: usize) -> Self {
        let k = k.max(1);
        let len = k * TAPS_PER_PHASE;
        let centre = (len - 1) as f64 / 2.0;
        // Cut a little under the input's Nyquist so nothing folds back.
        let cutoff = 0.45 / k as f64;
        let mut taps: Vec<f64> = (0..len)
            .map(|n| {
                let x = n as f64 - centre;
                let sinc = if x.abs() < 1e-9 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                // Blackman window.
                let w = 0.42
                    - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (len - 1) as f64).cos()
                    + 0.08 * (4.0 * std::f64::consts::PI * n as f64 / (len - 1) as f64).cos();
                sinc * w
            })
            .collect();
        // Each phase sums to one, so a steady signal comes out at the same
        // level (zero-stuffing loses a factor of k, which this puts back).
        for p in 0..k {
            let sum: f64 = (0..TAPS_PER_PHASE).map(|j| taps[p + j * k]).sum();
            if sum.abs() > 1e-12 {
                for j in 0..TAPS_PER_PHASE {
                    taps[p + j * k] /= sum;
                }
            }
        }
        let coeffs = (0..k)
            .map(|p| {
                let mut c = [0.0f32; TAPS_PER_PHASE];
                for (j, v) in c.iter_mut().enumerate() {
                    *v = taps[p + j * k] as f32;
                }
                c
            })
            .collect();
        let fresh = || Phase {
            history: [0.0; TAPS_PER_PHASE],
            pos: 0,
        };
        Self {
            k,
            coeffs,
            channels: [fresh(), fresh()],
        }
    }

    pub fn factor(&self) -> usize {
        self.k
    }

    /// Raise interleaved stereo `input` (frames) into `output`, which must
    /// hold `k` times as many frames.
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        let frames = input.len() / 2;
        debug_assert!(output.len() >= frames * 2 * self.k);
        for f in 0..frames {
            for (ch, phase) in self.channels.iter_mut().enumerate() {
                phase.pos = (phase.pos + 1) % TAPS_PER_PHASE;
                phase.history[phase.pos] = input[f * 2 + ch];
            }
            for p in 0..self.k {
                for (ch, phase) in self.channels.iter().enumerate() {
                    let c = &self.coeffs[p];
                    let mut acc = 0.0f32;
                    for (j, coeff) in c.iter().enumerate() {
                        let idx = (phase.pos + TAPS_PER_PHASE - j) % TAPS_PER_PHASE;
                        acc += coeff * phase.history[idx];
                    }
                    output[(f * self.k + p) * 2 + ch] = acc;
                }
            }
        }
    }
}

/// An engine callback running at a fraction of the device's rate: it is
/// asked for fixed blocks at its own rate, and what it makes is raised and
/// handed out in whatever size the device asks for.
pub struct Upsampled<C: AudioCallback> {
    inner: C,
    up: Upsampler,
    block: usize,
    /// One block at the engine's rate, interleaved stereo.
    engine: Vec<f32>,
    /// That block raised, and how much of it is still to be handed out.
    raised: Vec<f32>,
    left: usize,
}

impl<C: AudioCallback> Upsampled<C> {
    /// `block` is the engine's block size, in frames at its own rate.
    pub fn new(inner: C, k: usize, block: usize) -> Self {
        let block = block.max(16);
        let up = Upsampler::new(k);
        let k = up.factor();
        Self {
            inner,
            up,
            block,
            engine: vec![0.0; block * 2],
            raised: vec![0.0; block * k * 2],
            left: 0,
        }
    }
}

impl<C: AudioCallback> AudioCallback for Upsampled<C> {
    fn process(&mut self, output: &mut [f32], num_frames: usize, _num_channels: u16) {
        let total = self.raised.len() / 2;
        let mut done = 0;
        while done < num_frames {
            if self.left == 0 {
                self.engine.fill(0.0);
                self.inner.process(&mut self.engine, self.block, 2);
                self.up.process(&self.engine, &mut self.raised);
                self.left = total;
            }
            let from = total - self.left;
            let n = self.left.min(num_frames - done);
            output[done * 2..(done + n) * 2]
                .copy_from_slice(&self.raised[from * 2..(from + n) * 2]);
            self.left -= n;
            done += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factor_is_only_two_or_four_times_a_usual_rate() {
        assert_eq!(factor_for(44_100, 192_000), 4, "192 is four times 48");
        assert_eq!(factor_for(48_000, 192_000), 4);
        assert_eq!(factor_for(44_100, 176_400), 4);
        assert_eq!(factor_for(48_000, 96_000), 2);
        assert_eq!(factor_for(48_000, 48_000), 1);
        assert_eq!(
            factor_for(96_000, 192_000),
            2,
            "a higher request is honoured"
        );
        assert_eq!(
            factor_for(48_000, 44_100),
            1,
            "lower than asked: play as it is"
        );
        assert_eq!(
            factor_for(48_000, 384_000),
            1,
            "eight times: not a case we take"
        );
    }

    #[test]
    fn a_sine_comes_out_the_same_sine_at_the_higher_rate() {
        let k = 4;
        let mut up = Upsampler::new(k);
        let n = 4800;
        let input: Vec<f32> = (0..n)
            .flat_map(|i| {
                let v = (2.0 * std::f32::consts::PI * 1_000.0 * i as f32 / 48_000.0).sin() * 0.5;
                [v, -v]
            })
            .collect();
        let mut out = vec![0.0; n * 2 * k];
        up.process(&input, &mut out);
        // The filter delays by (len - 1) / 2 output samples.
        let delay = (k * TAPS_PER_PHASE - 1) as f32 / 2.0;
        let mut worst = 0.0f32;
        for i in (k * 200)..(n * k - 200) {
            let t = (i as f32 - delay) / 192_000.0;
            let want = (2.0 * std::f32::consts::PI * 1_000.0 * t).sin() * 0.5;
            worst = worst
                .max((out[i * 2] - want).abs())
                .max((out[i * 2 + 1] + want).abs());
        }
        assert!(worst < 0.01, "within 1 % of the true sine, worst {worst}");
    }

    struct Counter {
        frames: usize,
        calls: usize,
    }
    impl AudioCallback for Counter {
        fn process(&mut self, output: &mut [f32], num_frames: usize, _: u16) {
            for f in 0..num_frames {
                output[f * 2] = 0.25;
                output[f * 2 + 1] = 0.25;
            }
            self.frames += num_frames;
            self.calls += 1;
        }
    }

    #[test]
    fn any_device_block_size_is_filled_from_whole_engine_blocks() {
        let mut cb = Upsampled::new(
            Counter {
                frames: 0,
                calls: 0,
            },
            4,
            512,
        );
        let mut produced = 0;
        for size in [2048usize, 1000, 333, 4096, 7] {
            let mut out = vec![1.0; size * 2];
            cb.process(&mut out, size, 2);
            produced += size;
        }
        // The engine ran in whole blocks of 512 and stayed at most one block
        // ahead of what the device took.
        assert!(cb.inner.frames * 4 >= produced);
        assert!(cb.inner.frames * 4 < produced + 512 * 4);
        assert!(cb.inner.calls > 0);
        // A steady level comes out as the same level once the filter settles.
        let mut out = vec![0.0; 4096 * 2];
        cb.process(&mut out, 4096, 2);
        assert!(out[2000..].iter().all(|v| (v - 0.25).abs() < 1e-3));
    }
}
