//! Granular time-stretch — cuts the input into short windowed grains
//! and lays them down at a different hop than they were sampled from.
//! Character: breathier / less coherent than phase vocoder, but cheap
//! and robust; ideal for pads, textures, sound-design, and extreme
//! ratios where phase tracking collapses.

use std::f32::consts::PI;

/// Time-stretch `samples` by `ratio` using overlapping windowed
/// grains. `grain_ms` controls grain length (20–80 ms typical) and
/// `overlap` the inter-grain overlap (0.25–0.5 typical). Returns an
/// output buffer whose length is roughly `input_len × ratio`.
pub fn granular_stretch(
    samples: &[f32],
    sample_rate: f32,
    ratio: f32,
    grain_ms: f32,
    overlap: f32,
) -> Vec<f32> {
    if samples.is_empty() || ratio <= 0.0 || sample_rate <= 0.0 {
        return samples.to_vec();
    }
    let grain_len = ((grain_ms * 0.001) * sample_rate).round() as usize;
    let grain_len = grain_len.max(32);
    let overlap = overlap.clamp(0.1, 0.9);
    let hop_out = ((grain_len as f32) * (1.0 - overlap)).round().max(1.0) as usize;
    let hop_in = ((hop_out as f32) / ratio).round().max(1.0) as usize;

    let window: Vec<f32> = (0..grain_len)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / (grain_len - 1) as f32).cos()))
        .collect();

    let target_out_len = ((samples.len() as f32) * ratio).round() as usize + grain_len;
    let mut out = vec![0.0_f32; target_out_len];
    let mut weight = vec![0.0_f32; target_out_len];

    let mut input_pos = 0_usize;
    let mut output_pos = 0_usize;
    while input_pos + grain_len <= samples.len() && output_pos + grain_len <= out.len() {
        for k in 0..grain_len {
            let s = samples[input_pos + k] * window[k];
            out[output_pos + k] += s;
            weight[output_pos + k] += window[k];
        }
        input_pos = input_pos.saturating_add(hop_in);
        output_pos = output_pos.saturating_add(hop_out);
    }
    for (o, w) in out.iter_mut().zip(weight.iter()) {
        if *w > 1e-6 {
            *o /= *w;
        }
    }
    let target_len = ((samples.len() as f32) * ratio).round() as usize;
    out.truncate(target_len);
    out
}

/// Granular pitch-shift — stretch by `2^(st/12)` then resample back
/// to the original length. Produces the characteristic granular
/// "cloud" texture when pushed to extreme ratios.
pub fn pitch_shift_granular(samples: &[f32], sample_rate: f32, semitones: f32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let ratio = 2_f32.powf(semitones / 12.0);
    if (ratio - 1.0).abs() < 1e-4 {
        return samples.to_vec();
    }
    let stretched = granular_stretch(samples, sample_rate, ratio, 40.0, 0.5);
    crate::time_pitch::resample_to_length(&stretched, samples.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn stretch_ratio_2_doubles_length_within_tolerance() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let out = granular_stretch(&input, sr, 2.0, 40.0, 0.5);
        let target = (input.len() as f32 * 2.0) as usize;
        let diff = (out.len() as i32 - target as i32).abs();
        assert!(diff < 2048, "len {} target {}", out.len(), target);
    }

    #[test]
    fn stretch_ratio_half_halves_length_within_tolerance() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 8192);
        let out = granular_stretch(&input, sr, 0.5, 40.0, 0.5);
        let target = input.len() / 2;
        let diff = (out.len() as i32 - target as i32).abs();
        assert!(diff < 2048, "len {} target {}", out.len(), target);
    }

    #[test]
    fn granular_output_is_audible() {
        let sr = 44_100.0;
        let input = sine(440.0, sr, 16_384);
        let out = granular_stretch(&input, sr, 1.25, 40.0, 0.5);
        let energy: f32 = out.iter().map(|v| v * v).sum();
        assert!(energy > 0.1, "output silent, energy {}", energy);
    }

    #[test]
    fn granular_pitch_zero_is_identity() {
        let input = sine(440.0, 44_100.0, 4096);
        let out = pitch_shift_granular(&input, 44_100.0, 0.0);
        assert_eq!(out.len(), input.len());
        for (a, b) in input.iter().zip(out.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn granular_pitch_up_raises_zero_crossings() {
        let sr = 44_100.0;
        let input = sine(220.0, sr, 16_384);
        let shifted = pitch_shift_granular(&input, sr, 12.0);
        let zc_in: usize = input.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        let zc_out: usize = shifted.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        assert!(
            zc_out as f32 > zc_in as f32 * 1.3,
            "in {} out {}",
            zc_in,
            zc_out
        );
    }

    #[test]
    fn invalid_inputs_pass_through_untouched() {
        let input = sine(440.0, 44_100.0, 4096);
        let out = granular_stretch(&input, 44_100.0, 0.0, 40.0, 0.5);
        assert_eq!(out.len(), input.len());
        let out2 = granular_stretch(&input, 0.0, 2.0, 40.0, 0.5);
        assert_eq!(out2.len(), input.len());
    }
}
