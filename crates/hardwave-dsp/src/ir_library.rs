//! Procedural impulse-response library for the convolution reverb.
//! Each preset is synthesized at runtime as exponentially-decaying
//! filtered noise with character-specific shaping — no sample files
//! needed to ship a useful default library.

use std::f32::consts::PI;

/// The bundled IR presets. Each one has a distinct decay length and
/// spectral shape so users hear four meaningfully different spaces
/// out of the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrPreset {
    /// Large hall — long decay, dense diffusion, slight high-frequency
    /// roll-off. ~3.5 s.
    LargeHall,
    /// Small room — short decay, early reflections dominant. ~0.6 s.
    SmallRoom,
    /// Plate reverb — bright, dense, ~1.8 s decay with no early
    /// reflection cluster.
    Plate,
    /// Spring reverb — fast flutter, band-limited mids, ~0.9 s decay
    /// with a pronounced resonant ring.
    Spring,
}

impl IrPreset {
    pub fn name(&self) -> &'static str {
        match self {
            IrPreset::LargeHall => "Large Hall",
            IrPreset::SmallRoom => "Small Room",
            IrPreset::Plate => "Plate",
            IrPreset::Spring => "Spring",
        }
    }

    pub fn decay_seconds(&self) -> f32 {
        match self {
            IrPreset::LargeHall => 3.5,
            IrPreset::SmallRoom => 0.6,
            IrPreset::Plate => 1.8,
            IrPreset::Spring => 0.9,
        }
    }
}

/// Generate a stereo impulse response for the given preset at the
/// requested sample rate. Returns `(left, right)` with identical
/// length. Decay is exponential (`exp(-t / tau)`), so the tail never
/// clicks off.
pub fn synthesize_ir(preset: IrPreset, sample_rate: f32) -> (Vec<f32>, Vec<f32>) {
    let decay = preset.decay_seconds();
    let n = (decay * sample_rate).round() as usize;
    if n == 0 || sample_rate <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    // Two independent LCG seeds so L and R decorrelate — stereo width
    // is what makes convolution reverb feel like a space.
    let mut seed_l: u32 = 0x12345678;
    let mut seed_r: u32 = 0x9ABCDEF0;
    // Exponential decay: target -60 dB at `decay` seconds, so
    // tau = decay / 6.91 (because ln(1000) ≈ 6.91).
    let tau = decay / 6.908;
    // Simple one-pole low-pass state for damping.
    let mut lp_l = 0.0_f32;
    let mut lp_r = 0.0_f32;
    // Character-specific shaping parameters.
    let (lp_cutoff_hz, brightness, flutter_hz, resonance) = match preset {
        IrPreset::LargeHall => (3_500.0, 0.7, 0.0, 0.0),
        IrPreset::SmallRoom => (5_000.0, 0.8, 0.0, 0.0),
        IrPreset::Plate => (7_500.0, 0.95, 0.0, 0.0),
        IrPreset::Spring => (1_800.0, 0.6, 6.0, 0.35),
    };
    let lp_alpha = {
        let dt = 1.0 / sample_rate;
        let rc = 1.0 / (2.0 * PI * lp_cutoff_hz);
        dt / (rc + dt)
    };
    for i in 0..n {
        let t = i as f32 / sample_rate;
        let env = (-t / tau).exp();
        let noise_l = noise(&mut seed_l);
        let noise_r = noise(&mut seed_r);
        lp_l = lp_l + lp_alpha * (noise_l - lp_l);
        lp_r = lp_r + lp_alpha * (noise_r - lp_r);
        // Flutter adds a mild AM ripple — characteristic of spring
        // tanks bouncing. 0.0 for halls/rooms/plate.
        let flutter = if flutter_hz > 0.0 {
            1.0 + resonance * (2.0 * PI * flutter_hz * t).sin()
        } else {
            1.0
        };
        let amp = env * brightness * flutter;
        left.push(lp_l * amp);
        right.push(lp_r * amp);
    }
    // Prepend a small early-reflection cluster for non-plate / non-
    // spring presets so transients have bite.
    if matches!(preset, IrPreset::LargeHall | IrPreset::SmallRoom) {
        let er_count = match preset {
            IrPreset::LargeHall => 6,
            IrPreset::SmallRoom => 10,
            _ => 0,
        };
        let max_delay_ms = match preset {
            IrPreset::LargeHall => 80.0,
            IrPreset::SmallRoom => 30.0,
            _ => 0.0,
        };
        let mut seed_er: u32 = 0xDEADBEEF;
        for k in 0..er_count {
            let delay_ms = max_delay_ms * (k as f32 + 1.0) / er_count as f32;
            let idx = ((delay_ms / 1000.0) * sample_rate) as usize;
            if idx < left.len() {
                let gain = 0.5 * (1.0 - k as f32 / er_count as f32);
                left[idx] += gain * noise(&mut seed_er);
                right[idx] += gain * noise(&mut seed_er);
            }
        }
    }
    // Peak-normalize so presets have consistent loudness regardless
    // of the random shaping.
    let peak = left
        .iter()
        .chain(right.iter())
        .fold(0.0_f32, |acc, v| acc.max(v.abs()));
    if peak > 0.0 {
        let inv = 0.98 / peak;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l *= inv;
            *r *= inv;
        }
    }
    (left, right)
}

/// Return the full catalog of bundled presets in display order.
pub fn catalog() -> &'static [IrPreset] {
    &[
        IrPreset::LargeHall,
        IrPreset::SmallRoom,
        IrPreset::Plate,
        IrPreset::Spring,
    ]
}

fn noise(seed: &mut u32) -> f32 {
    // Simple LCG (Numerical Recipes constants). Deterministic so IRs
    // are reproducible across builds.
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    // Map top 24 bits to [-1, 1).
    let bits = (*seed >> 8) as f32;
    (bits / 8_388_608.0) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preset_has_expected_length() {
        let sr = 48_000.0;
        for preset in catalog() {
            let (l, r) = synthesize_ir(*preset, sr);
            assert_eq!(l.len(), r.len());
            let expected = (preset.decay_seconds() * sr).round() as usize;
            assert_eq!(l.len(), expected, "preset {:?}", preset);
        }
    }

    #[test]
    fn irs_are_peak_normalized() {
        let sr = 44_100.0;
        for preset in catalog() {
            let (l, r) = synthesize_ir(*preset, sr);
            let peak = l
                .iter()
                .chain(r.iter())
                .fold(0.0_f32, |acc, v| acc.max(v.abs()));
            assert!(
                (peak - 0.98).abs() < 1e-3,
                "preset {:?} peak {:.4}",
                preset,
                peak
            );
        }
    }

    #[test]
    fn irs_decay_monotonically_in_rms_windows() {
        // Check a coarse RMS-per-window envelope only — sample-by-sample
        // noise is random, but window averages should fall.
        let sr = 48_000.0;
        let (l, _) = synthesize_ir(IrPreset::LargeHall, sr);
        let win = (sr * 0.2) as usize; // 200 ms windows
        let windows: Vec<f32> = l
            .chunks(win)
            .map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt())
            .collect();
        // Compare first quarter to last quarter; last should be smaller.
        let q = windows.len() / 4;
        let head: f32 = windows[..q].iter().sum::<f32>() / q as f32;
        let tail: f32 = windows[windows.len() - q..].iter().sum::<f32>() / q as f32;
        assert!(tail < head * 0.2, "head {head:.4} tail {tail:.4}");
    }

    #[test]
    fn left_and_right_channels_decorrelate() {
        let sr = 44_100.0;
        let (l, r) = synthesize_ir(IrPreset::Plate, sr);
        // Compute Pearson correlation over the whole IR.
        let n = l.len().min(r.len());
        let mean_l: f32 = l.iter().sum::<f32>() / n as f32;
        let mean_r: f32 = r.iter().sum::<f32>() / n as f32;
        let mut num = 0.0_f32;
        let mut den_l = 0.0_f32;
        let mut den_r = 0.0_f32;
        for i in 0..n {
            let dl = l[i] - mean_l;
            let dr = r[i] - mean_r;
            num += dl * dr;
            den_l += dl * dl;
            den_r += dr * dr;
        }
        let corr = num / (den_l.sqrt() * den_r.sqrt() + 1e-12);
        assert!(corr.abs() < 0.2, "L/R correlation too high: {corr:.3}");
    }

    #[test]
    fn preset_names_are_distinct() {
        let names: Vec<&str> = catalog().iter().map(|p| p.name()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }
}
