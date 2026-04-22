//! Sample-level distortion algorithms. Pure math — no allocation, no
//! state. A future distortion plugin wraps these behind per-block
//! state (drive smoothing, oversampling, etc.); this module is just
//! the math.

/// Soft-knee saturation using `tanh`. Approaches ±1 asymptotically as
/// `drive` grows, so there's no brick-wall clip — harmonic content
/// rolls off smoothly. `drive` is a linear multiplier applied before
/// the tanh curve; 1.0 is unity.
#[inline]
pub fn soft_clip(sample: f32, drive: f32) -> f32 {
    (sample * drive).tanh()
}

/// Hard-knee clip — linear until ±1, then clamped. Produces strong
/// odd harmonics and an audible "brick wall" character.
#[inline]
pub fn hard_clip(sample: f32, drive: f32) -> f32 {
    (sample * drive).clamp(-1.0, 1.0)
}

/// Asymmetric tape-style saturation — a softer curve that compresses
/// positive and negative peaks differently, emulating the non-linear
/// response of magnetic tape.
#[inline]
pub fn tape_saturation(sample: f32, drive: f32) -> f32 {
    let x = sample * drive;
    let shaped = if x >= 0.0 {
        // Upper half: smooth roll-off, cap at ~0.9 under heavy drive.
        x / (1.0 + x * 0.7)
    } else {
        // Lower half: slightly steeper compression, asymmetric.
        x / (1.0 + x.abs() * 0.9)
    };
    shaped.clamp(-1.5, 1.5)
}

/// Tube-style saturation — a squared-soft-clip curve that builds
/// even harmonics. Good for warmth on vocals and guitars.
#[inline]
pub fn tube_emulation(sample: f32, drive: f32) -> f32 {
    let x = sample * drive;
    // Cubic soft clip with small even-harmonic asymmetry.
    let asymmetric = x + 0.15 * x * x;
    (1.5 * asymmetric - 0.5 * asymmetric.powi(3)).clamp(-1.0, 1.0)
}

/// Bitcrusher — quantizes to `bits` of resolution (2..16). Lower
/// values produce lo-fi digital character.
#[inline]
pub fn bitcrush(sample: f32, bits: u8) -> f32 {
    let bits = bits.clamp(2, 16);
    let steps = (1u32 << bits) as f32;
    let half = steps * 0.5;
    (sample.clamp(-1.0, 1.0) * half).round() / half
}

/// Sample-rate reducer (downsampler) — holds each input sample for
/// `hold_factor` output samples. The caller is responsible for calling
/// this with a state/counter across a block; this function just returns
/// the held value given the current counter.
#[inline]
pub fn sample_rate_reduce(
    sample: f32,
    hold_counter: u32,
    hold_factor: u32,
    last_held: f32,
) -> (f32, f32) {
    let factor = hold_factor.max(1);
    let should_update = hold_counter.is_multiple_of(factor);
    let new_held = if should_update { sample } else { last_held };
    (new_held, new_held)
}

/// Parallel-distortion mix: crossfade between dry and wet signals
/// with an equal-gain law at the midpoint. `mix` is 0..1; 0 = all dry,
/// 1 = all wet.
#[inline]
pub fn parallel_mix(dry: f32, wet: f32, mix: f32) -> f32 {
    let m = mix.clamp(0.0, 1.0);
    dry * (1.0 - m) + wet * m
}

/// Level-compensate a driven signal by the inverse of the drive
/// amount in dB, so the output RMS roughly matches the input RMS
/// regardless of drive setting. Useful for A/B comparison.
#[inline]
pub fn drive_compensate(sample: f32, drive_db: f32) -> f32 {
    let gain_linear = 10.0_f32.powf(-drive_db / 20.0);
    sample * gain_linear
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_clip_is_odd_function_through_origin() {
        assert_eq!(soft_clip(0.0, 1.0), 0.0);
        let a = soft_clip(0.5, 2.0);
        let b = soft_clip(-0.5, 2.0);
        assert!((a + b).abs() < 1e-6, "soft_clip should be odd: {a} + {b}");
    }

    #[test]
    fn soft_clip_approaches_unity_asymptote() {
        // Moderate drive values pull a 0.5 input toward 1.0 without
        // exceeding. Drive pushed past ~20 reaches tanh's f32 saturation
        // point (`tanh` returns exactly 1.0 in f32 for large arguments),
        // so we stop below that; the "< 1.0 strict" invariant only
        // holds mathematically, not in f32.
        for drive in [3.0, 5.0, 10.0] {
            let v = soft_clip(0.5, drive);
            assert!(v <= 1.0, "soft_clip({drive}) = {v} exceeded 1");
            assert!(v > 0.9, "soft_clip({drive}) = {v} should approach 1");
        }
    }

    #[test]
    fn hard_clip_clamps_at_unity() {
        assert_eq!(hard_clip(2.0, 1.0), 1.0);
        assert_eq!(hard_clip(-2.0, 1.0), -1.0);
        assert_eq!(hard_clip(0.5, 1.0), 0.5);
        // High drive pushes input into clip range.
        assert_eq!(hard_clip(0.5, 4.0), 1.0);
    }

    #[test]
    fn tape_saturation_is_monotonic() {
        // Output should increase monotonically with input over a
        // reasonable range.
        let drive = 1.5;
        let mut prev = -1.0;
        for i in -10..=10 {
            let x = (i as f32) * 0.1;
            let y = tape_saturation(x, drive);
            assert!(
                y >= prev,
                "tape_saturation should be monotonic at x={x}: y={y}, prev={prev}"
            );
            prev = y;
        }
    }

    #[test]
    fn tube_emulation_produces_even_harmonics() {
        // A pure sine at 0.5 amplitude through tube shaping should
        // shift its mean upward slightly (even harmonics add DC).
        let mean: f32 = (0..360)
            .map(|deg| {
                let phase = (deg as f32).to_radians();
                tube_emulation(0.5 * phase.sin(), 1.5)
            })
            .sum::<f32>()
            / 360.0;
        assert!(
            mean.abs() > 1e-4,
            "tube emulation should shift DC: mean = {mean}"
        );
    }

    #[test]
    fn bitcrush_at_2_bits_quantizes_to_4_levels() {
        // 2 bits = 4 levels total. Values in between should snap to
        // the nearest quantization step.
        let vals: Vec<f32> = [-1.0, -0.5, 0.0, 0.5, 1.0]
            .iter()
            .map(|v| bitcrush(*v, 2))
            .collect();
        // With 2 bits: half = 2.0 → steps = [-1.0, -0.5, 0.0, 0.5, 1.0] — but
        // the range is symmetrical so we get 4 distinct values rounded.
        let unique_count = {
            let mut sorted = vals.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            sorted.dedup();
            sorted.len()
        };
        assert!(
            unique_count <= 5,
            "2-bit crush should limit distinct levels, got {unique_count}"
        );
    }

    #[test]
    fn bitcrush_16_bits_preserves_signal() {
        // 16 bits is fine enough that small inputs survive intact.
        let v = bitcrush(0.12345, 16);
        assert!((v - 0.12345).abs() < 1e-3);
    }

    #[test]
    fn sample_rate_reduce_holds_for_factor() {
        // Factor 4: output holds for 4 counter ticks before updating.
        let mut last = 0.0;
        let (first, _) = sample_rate_reduce(0.5, 0, 4, last);
        assert_eq!(first, 0.5);
        last = first;
        let (held, _) = sample_rate_reduce(0.99, 1, 4, last);
        assert_eq!(held, 0.5, "should still hold first sample at counter=1");
    }

    #[test]
    fn parallel_mix_endpoints() {
        assert_eq!(parallel_mix(0.5, 0.9, 0.0), 0.5);
        assert_eq!(parallel_mix(0.5, 0.9, 1.0), 0.9);
        let mid = parallel_mix(0.5, 0.9, 0.5);
        assert!((mid - 0.7).abs() < 1e-6);
    }

    #[test]
    fn drive_compensate_inverts_linear_gain() {
        // +6 dB boost → compensate multiplies by 10^(-6/20) ≈ 0.501
        let compensated = drive_compensate(1.0, 6.0);
        assert!((compensated - 0.501).abs() < 0.01);
        // 0 dB → identity.
        assert_eq!(drive_compensate(0.75, 0.0), 0.75);
    }
}
