//! Waveform colours the way FL Studio's "Multiband" waveforms colour them.
//!
//! Measured on FL Studio 2026 itself (19 October 2026) with pure tones from
//! 10 Hz to 16 kHz, tone mixes at set level ratios, white and pink noise, DC
//! and a real stereo kick loop, by reading the colours FL draws:
//!
//! * the sound is split into a low, a mid and a high band by first-order
//!   crossovers at about 153 Hz and 1.31 kHz;
//! * red, green and blue are the three bands' levels divided by their sum,
//!   so every pure tone comes out with R + G + B = 256 (50 Hz is a deep
//!   orange red, 500 Hz green, 5 kHz blue, and 50 Hz plus 5 kHz at the same
//!   level the dim purple FL draws);
//! * the levels are taken over both channels, so highs that live only in
//!   the side signal still count; DC counts as low;
//! * the colour holds still over about 30 ms, so it does not flicker within
//!   one cycle of a bass note at deep zoom.
//!
//! With these settings the 16 test sounds land within 2.8 (out of 255) of
//! FL's colours on average and the kick loop within 6.2 per screen column.
//!
//! The analysis runs once per file: [`WaveformColours::analyse`] keeps one
//! level per band for every [`HOP`] frames, and [`WaveformColours::colour`]
//! mixes any stretch of the file from those, so a waveform at any zoom costs
//! only a sum over the stretch each column covers.

/// Frames per stored set of band levels.
pub const HOP: usize = 128;

/// Crossover between the low and the mid band, in Hz.
const LOW_MID_HZ: f64 = 153.34;
/// Crossover between the mid and the high band, in Hz.
const MID_HIGH_HZ: f64 = 1307.9;
/// Level of the mid and the high band against the low one.
const MID_GAIN: f64 = 0.945;
const HIGH_GAIN: f64 = 0.967;
/// How long the colour holds still for.
const SMOOTH_SECONDS: f64 = 0.03;

/// A one-pole low-pass made with the bilinear transform. Its high-pass
/// partner is `x - lowpass(x)`, so the bands add back up to the input.
struct OnePole {
    b: f64,
    a: f64,
    x1: f64,
    y1: f64,
}

impl OnePole {
    fn lowpass(hz: f64, sample_rate: f64) -> Self {
        let k = (std::f64::consts::PI * hz / sample_rate).tan();
        Self {
            b: k / (1.0 + k),
            a: (k - 1.0) / (1.0 + k),
            x1: 0.0,
            y1: 0.0,
        }
    }

    #[inline]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b * (x + self.x1) - self.a * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The band levels of one file, ready to colour any stretch of it.
#[derive(Debug, Clone, Default)]
pub struct WaveformColours {
    /// Per [`HOP`] frames: the power of the low, mid and high band, the mean
    /// over the channels, smoothed over [`SMOOTH_SECONDS`].
    power: Vec<[f32; 3]>,
}

impl WaveformColours {
    /// Analyse a file: its channels, each the same length, at its own rate.
    pub fn analyse<C: AsRef<[f32]>>(channels: &[C], sample_rate: u32) -> Self {
        let frames = channels.iter().map(|c| c.as_ref().len()).max().unwrap_or(0);
        if frames == 0 || sample_rate == 0 {
            return Self::default();
        }
        let rate = f64::from(sample_rate);
        // Stop each crossover short of Nyquist, for a file at a very low rate.
        let low_mid = LOW_MID_HZ.min(rate * 0.45);
        let mid_high = MID_HIGH_HZ.min(rate * 0.45);
        let hops = frames.div_ceil(HOP);
        let mut sums = vec![[0.0f64; 3]; hops];
        for channel in channels {
            let channel = channel.as_ref();
            let mut low_pass = OnePole::lowpass(low_mid, rate);
            let mut mid_pass = OnePole::lowpass(mid_high, rate);
            for (i, &sample) in channel.iter().enumerate() {
                let x = f64::from(sample);
                let low = low_pass.process(x);
                let rest = x - low;
                let mid_raw = mid_pass.process(rest);
                let mid = MID_GAIN * mid_raw;
                let high = HIGH_GAIN * (rest - mid_raw);
                let sum = &mut sums[i / HOP];
                sum[0] += low * low;
                sum[1] += mid * mid;
                sum[2] += high * high;
            }
        }
        // Mean power per hop over all channels (the last hop may be short).
        let count = channels.len().max(1) as f64;
        let raw: Vec<[f64; 3]> = sums
            .iter()
            .enumerate()
            .map(|(h, s)| {
                let n = (frames - h * HOP).min(HOP) as f64 * count;
                [s[0] / n, s[1] / n, s[2] / n]
            })
            .collect();
        Self {
            power: smooth(&raw, rate),
        }
    }

    /// How many sets of band levels there are.
    pub fn len(&self) -> usize {
        self.power.len()
    }

    pub fn is_empty(&self) -> bool {
        self.power.is_empty()
    }

    /// The colour of frames `start..end` as red, green and blue, or `None`
    /// for silence. A stretch shorter than one hop takes the hop it starts in.
    pub fn colour(&self, start: usize, end: usize) -> Option<[u8; 3]> {
        if self.power.is_empty() {
            return None;
        }
        let first = (start / HOP).min(self.power.len() - 1);
        let last = end.div_ceil(HOP).clamp(first + 1, self.power.len());
        let mut sum = [0.0f64; 3];
        for p in &self.power[first..last] {
            for k in 0..3 {
                sum[k] += f64::from(p[k]);
            }
        }
        let n = (last - first) as f64;
        let level = sum.map(|s| (s / n).sqrt());
        let total = level[0] + level[1] + level[2];
        if total < 1e-7 {
            return None;
        }
        Some(level.map(|l| (256.0 * l / total).round().min(255.0) as u8))
    }
}

/// Smooth the hop powers with a Hann window [`SMOOTH_SECONDS`] long.
fn smooth(raw: &[[f64; 3]], sample_rate: f64) -> Vec<[f32; 3]> {
    let len = ((SMOOTH_SECONDS * sample_rate / HOP as f64) as usize) | 1;
    if len < 3 {
        return raw.iter().map(|p| p.map(|v| v as f32)).collect();
    }
    let window: Vec<f64> = (0..len)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 0.5) / len as f64).cos())
        .collect();
    let half = len / 2;
    (0..raw.len())
        .map(|i| {
            let mut acc = [0.0f64; 3];
            let mut weight = 0.0;
            for (j, w) in window.iter().enumerate() {
                let Some(at) = (i + j).checked_sub(half) else {
                    continue;
                };
                let Some(p) = raw.get(at) else { break };
                for k in 0..3 {
                    acc[k] += w * p[k];
                }
                weight += w;
            }
            acc.map(|v| (v / weight) as f32)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn tone(hz: f64, amp: f64, seconds: f64, rate: u32) -> Vec<f32> {
        let n = (seconds * f64::from(rate)) as usize;
        (0..n)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin()) as f32
            })
            .collect()
    }

    fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(x, y)| x + y).collect()
    }

    /// The colour of the steady middle of a sound, as FL would show it.
    fn middle(channels: &[Vec<f32>], rate: u32) -> [u8; 3] {
        let n = channels[0].len();
        WaveformColours::analyse(channels, rate)
            .colour(n / 5, n * 4 / 5)
            .expect("not silent")
    }

    fn distance(a: [u8; 3], hex: u32) -> f64 {
        let b = [(hex >> 16) & 255, (hex >> 8) & 255, hex & 255];
        a.iter()
            .zip(b)
            .map(|(&x, y)| (f64::from(x) - y as f64).powi(2))
            .sum::<f64>()
            .sqrt()
    }

    #[test]
    fn pure_tones_take_the_colour_fl_draws() {
        // FL Studio 2026's own colours for these tones.
        for (hz, fl) in [
            (25.0, 0xdc2201),
            (50.0, 0xbe3907),
            (120.0, 0x8d650d),
            (500.0, 0x34913c),
            (2000.0, 0x0f5b97),
            (5600.0, 0x072bcf),
            (12000.0, 0x0313e9),
        ] {
            let got = middle(&[tone(hz, 0.5, 0.5, RATE)], RATE);
            assert!(
                distance(got, fl) < 10.0,
                "{hz} Hz: {got:?} against FL #{fl:06x}"
            );
        }
    }

    #[test]
    fn a_low_and_a_high_tone_together_are_the_dim_purple_fl_draws() {
        let low = tone(50.0, 0.25, 0.5, RATE);
        for (high_amp, fl) in [(0.25, 0x692a6b), (0.125, 0x8a2e45), (0.0625, 0xa13429)] {
            let got = middle(&[add(&low, &tone(5000.0, high_amp, 0.5, RATE))], RATE);
            assert!(
                distance(got, fl) < 10.0,
                "5 kHz at {high_amp}: {got:?} against FL #{fl:06x}"
            );
        }
    }

    #[test]
    fn white_noise_is_mostly_blue() {
        let mut seed = 1u32;
        let noise: Vec<f32> = (0..RATE / 2)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        let got = middle(&[noise], RATE);
        assert!(distance(got, 0x1332b8) < 12.0, "{got:?} against FL #1332b8");
    }

    #[test]
    fn highs_only_in_the_side_signal_still_count() {
        let bass = tone(45.0, 0.4, 0.5, RATE);
        let hiss = tone(6000.0, 0.2, 0.5, RATE);
        let left = add(&bass, &hiss);
        let right: Vec<f32> = bass.iter().zip(&hiss).map(|(b, h)| b - h).collect();
        let stereo = middle(&[left, right], RATE);
        let bass_only = middle(&[bass.clone(), bass], RATE);
        assert!(
            stereo[2] > bass_only[2] + 40,
            "{stereo:?} against bass alone {bass_only:?}"
        );
    }

    #[test]
    fn dc_counts_as_low() {
        let plain = tone(50.0, 0.35, 0.5, RATE);
        let offset: Vec<f32> = plain.iter().map(|x| x + 0.15).collect();
        let (a, b) = (middle(&[plain], RATE), middle(&[offset], RATE));
        assert!(b[0] > a[0] && b[1] < a[1], "with DC {b:?}, without {a:?}");
    }

    #[test]
    fn the_colour_does_not_depend_on_level_or_sample_rate() {
        let loud = middle(&[tone(120.0, 0.9, 0.5, RATE)], RATE);
        let quiet = middle(&[tone(120.0, 0.01, 0.5, RATE)], RATE);
        let cd = middle(&[tone(120.0, 0.5, 0.5, 44_100)], 44_100);
        assert!(distance(loud, u32::from_be_bytes([0, quiet[0], quiet[1], quiet[2]])) < 2.0);
        assert!(distance(loud, u32::from_be_bytes([0, cd[0], cd[1], cd[2]])) < 3.0);
    }

    #[test]
    fn silence_has_no_colour_and_short_stretches_take_their_hop() {
        let silent = WaveformColours::analyse(&[vec![0.0f32; 4096]], RATE);
        assert_eq!(silent.colour(0, 4096), None);
        assert_eq!(
            WaveformColours::analyse::<Vec<f32>>(&[], RATE).colour(0, 10),
            None
        );
        let colours = WaveformColours::analyse(&[tone(500.0, 0.5, 0.2, RATE)], RATE);
        assert_eq!(colours.len(), (RATE as usize / 5).div_ceil(HOP));
        assert!(colours.colour(3000, 3001).is_some());
        assert!(
            colours.colour(10_000_000, 10_000_010).is_some(),
            "past the end takes the last hop"
        );
    }
}
