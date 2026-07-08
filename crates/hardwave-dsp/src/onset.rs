//! Energy-flux onset detection — transient suggestions for warp
//! markers ("Detect" in the warp editor).
//!
//! Deliberately simple and dependency-free: windowed RMS energy,
//! half-wave-rectified frame-to-frame rise (flux), adaptive threshold
//! from a sliding local mean, and a refractory gap so one drum hit
//! yields one onset. Good enough to anchor warp markers on percussive
//! harder-styles material; melodic/legato sources will under-detect,
//! which is the right failure mode for warping (fewer, safer anchors).

const WINDOW: usize = 512;
const HOP: usize = 256;
/// Local-mean context, in frames (~1.5 s at 48 kHz with HOP=256).
const CONTEXT_FRAMES: usize = 280;
/// Flux must exceed local mean by this factor to count as an onset.
const THRESHOLD_FACTOR: f32 = 2.5;
/// Minimum spacing between reported onsets, in seconds.
const MIN_GAP_SECS: f32 = 0.05;

/// Detect onsets in a mono signal. Returns onset positions in samples,
/// ascending. Stereo callers should pass an L+R mono mix.
pub fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<u64> {
    if samples.len() < WINDOW * 2 || sample_rate == 0 {
        return Vec::new();
    }

    // Windowed RMS energy per hop.
    let n_frames = (samples.len() - WINDOW) / HOP + 1;
    let mut energy = Vec::with_capacity(n_frames);
    for f in 0..n_frames {
        let start = f * HOP;
        let sum_sq: f32 = samples[start..start + WINDOW].iter().map(|s| s * s).sum();
        energy.push((sum_sq / WINDOW as f32).sqrt());
    }

    // Half-wave-rectified flux.
    let mut flux = vec![0.0f32; energy.len()];
    for i in 1..energy.len() {
        flux[i] = (energy[i] - energy[i - 1]).max(0.0);
    }

    // Adaptive threshold: local mean over a trailing context window.
    let min_gap_frames = ((MIN_GAP_SECS * sample_rate as f32) / HOP as f32).ceil() as usize;
    let mut onsets = Vec::new();
    let mut last_onset: Option<usize> = None;
    for i in 1..flux.len() {
        let ctx_start = i.saturating_sub(CONTEXT_FRAMES);
        let ctx = &flux[ctx_start..i];
        let mean: f32 = ctx.iter().sum::<f32>() / ctx.len().max(1) as f32;
        // Absolute floor keeps silence from producing onsets out of
        // numeric dust; relative factor does the real work.
        let threshold = (mean * THRESHOLD_FACTOR).max(1e-4);
        let is_peak = flux[i] > threshold
            && flux[i] >= flux[i - 1]
            && flux.get(i + 1).map(|&n| flux[i] >= n).unwrap_or(true);
        if is_peak {
            if let Some(last) = last_onset {
                if i - last < min_gap_frames {
                    continue;
                }
            }
            onsets.push((i * HOP) as u64);
            last_onset = Some(i);
        }
    }
    onsets
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Silence with decaying clicks at known positions.
    fn clicks_at(positions: &[usize], len: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; len];
        for &p in positions {
            for i in 0..1500.min(len - p) {
                v[p + i] += 0.9 * (-(i as f32) / 300.0).exp();
            }
        }
        v
    }

    #[test]
    fn finds_isolated_clicks_near_their_positions() {
        let sr = 48_000;
        let truth = [24_000usize, 96_000, 168_000];
        let sig = clicks_at(&truth, 240_000);
        let onsets = detect_onsets(&sig, sr);
        assert_eq!(onsets.len(), truth.len(), "one onset per click: {onsets:?}");
        for (o, t) in onsets.iter().zip(truth.iter()) {
            let err = (*o as i64 - *t as i64).unsigned_abs();
            assert!(err <= (2 * HOP) as u64, "onset {o} too far from click {t}");
        }
    }

    #[test]
    fn refractory_gap_collapses_double_triggers() {
        let sr = 48_000;
        // Two clicks 20 ms apart — inside MIN_GAP → must merge to one.
        let sig = clicks_at(&[48_000, 48_960], 120_000);
        let onsets = detect_onsets(&sig, sr);
        assert_eq!(onsets.len(), 1, "double trigger not collapsed: {onsets:?}");
    }

    #[test]
    fn silence_and_short_input_produce_nothing() {
        assert!(detect_onsets(&vec![0.0; 200_000], 48_000).is_empty());
        assert!(detect_onsets(&[0.1; 100], 48_000).is_empty());
    }
}
