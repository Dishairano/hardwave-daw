//! Heuristic sample classifier — categorizes a sample file by its
//! filename + audio characteristics into `one-shot`, `loop`, `fx`,
//! `vocal`, or `other`. No ML — keyword matching + duration /
//! zero-crossing-rate analysis.

use std::path::Path;

/// Top-level sample category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleCategory {
    /// Short one-shot hits — kicks, snares, stabs, etc.
    OneShot,
    /// Long looping material — drum loops, chord loops, etc.
    Loop,
    /// Effects — risers, impacts, sweeps, transitions.
    Fx,
    /// Vocal content — one-shots or loops with "vocal", "vox", "phrase".
    Vocal,
    /// Couldn't classify confidently.
    Other,
}

/// Summary statistics a caller can gather cheaply from an audio
/// buffer without needing full analysis.
#[derive(Debug, Clone, Copy)]
pub struct SampleStats {
    pub duration_secs: f32,
    pub zero_crossing_rate: f32,
}

impl SampleStats {
    /// Compute basic stats from a raw mono sample buffer at the
    /// given sample rate.
    pub fn from_mono(samples: &[f32], sample_rate: f32) -> Self {
        let n = samples.len();
        let duration = if sample_rate > 0.0 {
            n as f32 / sample_rate
        } else {
            0.0
        };
        let mut crossings = 0_usize;
        for i in 1..n {
            let a = samples[i - 1];
            let b = samples[i];
            if (a < 0.0 && b >= 0.0) || (a >= 0.0 && b < 0.0) {
                crossings += 1;
            }
        }
        let zcr = if n > 1 {
            crossings as f32 / (n - 1) as f32
        } else {
            0.0
        };
        Self {
            duration_secs: duration,
            zero_crossing_rate: zcr,
        }
    }
}

/// Classify by filename + stats. The filename is matched against
/// category-indicative keywords; duration and zero-crossing rate
/// tie-break ambiguous cases.
pub fn classify_sample(filename: &Path, stats: SampleStats) -> SampleCategory {
    let name = filename
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    if contains_any(&name, &["vocal", "vox", "phrase", "chant", "adlib"]) {
        return SampleCategory::Vocal;
    }
    if contains_any(
        &name,
        &[
            "riser",
            "impact",
            "sweep",
            "transition",
            "whoosh",
            "downshift",
            "uplifter",
            "foley",
            "fx",
            "sfx",
            "stinger",
        ],
    ) {
        return SampleCategory::Fx;
    }
    if contains_any(&name, &["loop", "groove", "break"]) {
        return SampleCategory::Loop;
    }
    if contains_any(
        &name,
        &[
            "kick", "snare", "hat", "clap", "perc", "crash", "ride", "rim", "stab", "hit",
            "oneshot", "one_shot", "one-shot",
        ],
    ) {
        return SampleCategory::OneShot;
    }

    // Fall back to duration heuristics when filename isn't conclusive.
    if stats.duration_secs < 1.5 {
        SampleCategory::OneShot
    } else if stats.duration_secs > 4.0 {
        SampleCategory::Loop
    } else {
        SampleCategory::Other
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Compute a simple similarity score in `[0, 1]` between two sets
/// of stats. 1.0 = identical, 0.0 = very different. Uses duration
/// and ZCR distance. Useful as the core of a "find similar samples"
/// UI feature.
pub fn sample_similarity(a: SampleStats, b: SampleStats) -> f32 {
    let dur_diff = (a.duration_secs - b.duration_secs).abs();
    let zcr_diff = (a.zero_crossing_rate - b.zero_crossing_rate).abs();
    let dur_score = (1.0 - (dur_diff / 10.0)).clamp(0.0, 1.0);
    let zcr_score = (1.0 - zcr_diff * 5.0).clamp(0.0, 1.0);
    // Weight ZCR a bit less than duration since it's noisier.
    0.6 * dur_score + 0.4 * zcr_score
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn stats(dur: f32, zcr: f32) -> SampleStats {
        SampleStats {
            duration_secs: dur,
            zero_crossing_rate: zcr,
        }
    }

    #[test]
    fn vocal_keywords_match_first() {
        let path = PathBuf::from("EricaVocalPhrase.wav");
        assert_eq!(
            classify_sample(&path, stats(2.0, 0.1)),
            SampleCategory::Vocal
        );
    }

    #[test]
    fn fx_keywords_detected() {
        for name in [
            "BigRiser.wav",
            "impact_01.wav",
            "downshift.wav",
            "stinger.wav",
        ] {
            let path = PathBuf::from(name);
            assert_eq!(
                classify_sample(&path, stats(2.0, 0.1)),
                SampleCategory::Fx,
                "file {name}"
            );
        }
    }

    #[test]
    fn loop_keywords_detected() {
        for name in ["DrumLoop_120.wav", "BreakBeat_88.wav", "Groove_Basic.wav"] {
            let path = PathBuf::from(name);
            assert_eq!(
                classify_sample(&path, stats(2.0, 0.1)),
                SampleCategory::Loop,
                "file {name}"
            );
        }
    }

    #[test]
    fn one_shot_keywords_detected() {
        for name in [
            "Kick_808.wav",
            "Snare_Clap.wav",
            "OneShot_Vocal.wav",
            "perc_hit.wav",
        ] {
            let path = PathBuf::from(name);
            let cat = classify_sample(&path, stats(0.5, 0.1));
            // Vocal keyword in "OneShot_Vocal" beats one-shot keyword.
            let expected = if name.to_lowercase().contains("vocal") {
                SampleCategory::Vocal
            } else {
                SampleCategory::OneShot
            };
            assert_eq!(cat, expected, "file {name}");
        }
    }

    #[test]
    fn falls_back_to_duration_when_filename_is_ambiguous() {
        // No keywords — short sample classifies as OneShot.
        assert_eq!(
            classify_sample(&PathBuf::from("mystery1.wav"), stats(0.5, 0.2)),
            SampleCategory::OneShot
        );
        // Long sample classifies as Loop.
        assert_eq!(
            classify_sample(&PathBuf::from("mystery2.wav"), stats(8.0, 0.2)),
            SampleCategory::Loop
        );
        // Medium unknown sample is "Other".
        assert_eq!(
            classify_sample(&PathBuf::from("mystery3.wav"), stats(2.5, 0.2)),
            SampleCategory::Other
        );
    }

    #[test]
    fn similarity_identical_samples_score_one() {
        let s = stats(2.0, 0.15);
        assert!((sample_similarity(s, s) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn similarity_drops_with_duration_and_zcr_difference() {
        let a = stats(1.0, 0.1);
        let b = stats(9.0, 0.3);
        let score = sample_similarity(a, b);
        assert!(score < 0.5, "similarity should drop, got {score}");
    }

    #[test]
    fn sample_stats_zero_crossings() {
        let samples: Vec<f32> = (0..100)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let s = SampleStats::from_mono(&samples, 100.0);
        // Alternating values → crossing every sample (after the first).
        assert!(s.zero_crossing_rate > 0.9);
        assert!((s.duration_secs - 1.0).abs() < 1e-3);
    }
}
