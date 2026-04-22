//! Heuristic mix-feedback analyzer. Inspects a rendered mixdown and
//! reports common issues: clipping, muddy low-mids, harsh upper-mids,
//! stereo imbalance, low headroom, crushed dynamics. Deterministic —
//! not ML. Useful as a "pre-flight check" before export.

use rustfft::{num_complex::Complex32, FftPlanner};

/// A single finding from the analyzer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixIssue {
    pub severity: Severity,
    pub category: IssueCategory,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueCategory {
    Clipping,
    Headroom,
    Dynamics,
    StereoBalance,
    FrequencyBalance,
    LowEnd,
    HighEnd,
}

/// Summary numbers computed from the mixdown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixStats {
    pub peak_l: f32,
    pub peak_r: f32,
    pub rms_l: f32,
    pub rms_r: f32,
    pub crest_factor_db: f32,
    pub low_energy: f32,
    pub mid_energy: f32,
    pub high_energy: f32,
}

/// Analyze a stereo mixdown and return a list of issues along with
/// the raw stats used to derive them.
pub fn analyze_mix(left: &[f32], right: &[f32], sample_rate: f32) -> (MixStats, Vec<MixIssue>) {
    let stats = compute_stats(left, right, sample_rate);
    let issues = derive_issues(&stats);
    (stats, issues)
}

fn compute_stats(left: &[f32], right: &[f32], sample_rate: f32) -> MixStats {
    let n = left.len().min(right.len());
    let mut peak_l = 0.0_f32;
    let mut peak_r = 0.0_f32;
    let mut sum_sq_l = 0.0_f64;
    let mut sum_sq_r = 0.0_f64;
    for i in 0..n {
        let l = left[i];
        let r = right[i];
        peak_l = peak_l.max(l.abs());
        peak_r = peak_r.max(r.abs());
        sum_sq_l += (l as f64) * (l as f64);
        sum_sq_r += (r as f64) * (r as f64);
    }
    let rms_l = if n > 0 {
        (sum_sq_l / n as f64).sqrt() as f32
    } else {
        0.0
    };
    let rms_r = if n > 0 {
        (sum_sq_r / n as f64).sqrt() as f32
    } else {
        0.0
    };
    let peak_overall = peak_l.max(peak_r).max(1e-9);
    let rms_overall = (rms_l + rms_r) * 0.5;
    let crest = if rms_overall > 0.0 {
        20.0 * (peak_overall / rms_overall).log10()
    } else {
        0.0
    };
    let (low, mid, high) = three_band_energy(left, right, sample_rate);
    MixStats {
        peak_l,
        peak_r,
        rms_l,
        rms_r,
        crest_factor_db: crest,
        low_energy: low,
        mid_energy: mid,
        high_energy: high,
    }
}

fn three_band_energy(left: &[f32], right: &[f32], sample_rate: f32) -> (f32, f32, f32) {
    let n = left.len().min(right.len()).min(16_384).next_power_of_two();
    if n < 2 || sample_rate <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let mut buf: Vec<Complex32> = (0..n)
        .map(|i| {
            let l = *left.get(i).unwrap_or(&0.0);
            let r = *right.get(i).unwrap_or(&0.0);
            let mono = 0.5 * (l + r);
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos());
            Complex32::new(mono * w, 0.0)
        })
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    fft.process(&mut buf);
    let mut low = 0.0_f32;
    let mut mid = 0.0_f32;
    let mut high = 0.0_f32;
    for (bin, c) in buf.iter().take(n / 2).enumerate().skip(1) {
        let freq = bin as f32 * sample_rate / n as f32;
        let mag = c.norm_sqr();
        if freq < 250.0 {
            low += mag;
        } else if freq < 4000.0 {
            mid += mag;
        } else {
            high += mag;
        }
    }
    let total = low + mid + high;
    if total > 0.0 {
        (low / total, mid / total, high / total)
    } else {
        (0.0, 0.0, 0.0)
    }
}

fn derive_issues(s: &MixStats) -> Vec<MixIssue> {
    let mut out = Vec::new();
    let peak = s.peak_l.max(s.peak_r);
    if peak >= 1.0 {
        out.push(MixIssue {
            severity: Severity::Critical,
            category: IssueCategory::Clipping,
            message: format!(
                "Peak hit {:.2} — reduce master gain to avoid clipping.",
                peak
            ),
        });
    } else if peak > 0.99 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::Headroom,
            message: "Peak within 0.1 dB of full scale — leave ~0.5 dB of headroom.".into(),
        });
    } else if peak < 0.1 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::Headroom,
            message: "Mix is very quiet (peak < -20 dBFS) — raise the master bus.".into(),
        });
    }
    // Crest factor: modern masters sit near 10–14 dB. Below ~6 dB is
    // over-compressed; above ~20 dB suggests no bus processing.
    if s.crest_factor_db < 6.0 && s.rms_l + s.rms_r > 0.0 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::Dynamics,
            message: format!(
                "Crest factor {:.1} dB — mix is very squashed; back off the limiter.",
                s.crest_factor_db
            ),
        });
    } else if s.crest_factor_db > 22.0 {
        out.push(MixIssue {
            severity: Severity::Info,
            category: IssueCategory::Dynamics,
            message: format!(
                "Crest factor {:.1} dB — lots of transient punch; consider bus compression.",
                s.crest_factor_db
            ),
        });
    }
    // Stereo balance: > 3 dB RMS difference between L and R is worth
    // flagging.
    let rms_ratio_db = if s.rms_l > 0.0 && s.rms_r > 0.0 {
        20.0 * (s.rms_l / s.rms_r).log10()
    } else {
        0.0
    };
    if rms_ratio_db.abs() > 3.0 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::StereoBalance,
            message: format!(
                "Stereo imbalance: L vs R RMS differs by {:.1} dB.",
                rms_ratio_db
            ),
        });
    }
    // Frequency balance — relative energy per band.
    if s.low_energy > 0.65 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::LowEnd,
            message: format!(
                "Low-end heavy ({:.0}% of energy < 250 Hz) — HPF or cut mud around 200 Hz.",
                s.low_energy * 100.0
            ),
        });
    }
    if s.low_energy < 0.05 && s.low_energy > 0.0 {
        out.push(MixIssue {
            severity: Severity::Info,
            category: IssueCategory::LowEnd,
            message: "Low end is thin — consider adding sub or raising the kick/bass.".into(),
        });
    }
    if s.high_energy > 0.45 {
        out.push(MixIssue {
            severity: Severity::Warning,
            category: IssueCategory::HighEnd,
            message: format!(
                "Very bright mix ({:.0}% of energy > 4 kHz) — watch for harshness.",
                s.high_energy * 100.0
            ),
        });
    }
    if s.high_energy < 0.02 && s.high_energy > 0.0 {
        out.push(MixIssue {
            severity: Severity::Info,
            category: IssueCategory::HighEnd,
            message: "Top end is dull — consider a high-shelf lift above 8 kHz.".into(),
        });
    }
    if s.mid_energy > 0.85 {
        out.push(MixIssue {
            severity: Severity::Info,
            category: IssueCategory::FrequencyBalance,
            message: "Mix is mid-range dominant — tilt some energy to lows and highs.".into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silence(n: usize) -> Vec<f32> {
        vec![0.0; n]
    }
    fn const_level(n: usize, v: f32) -> Vec<f32> {
        vec![v; n]
    }
    fn sine(freq: f32, sr: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / sr).sin())
            .collect()
    }

    #[test]
    fn detects_clipping() {
        let sr = 44_100.0;
        let l = const_level(4096, 1.2);
        let r = const_level(4096, 1.2);
        let (_, issues) = analyze_mix(&l, &r, sr);
        assert!(issues
            .iter()
            .any(|i| i.category == IssueCategory::Clipping && i.severity == Severity::Critical));
    }

    #[test]
    fn detects_low_headroom() {
        let sr = 44_100.0;
        let l = const_level(4096, 0.995);
        let r = const_level(4096, 0.995);
        let (_, issues) = analyze_mix(&l, &r, sr);
        assert!(issues.iter().any(|i| i.category == IssueCategory::Headroom));
    }

    #[test]
    fn detects_stereo_imbalance() {
        let sr = 44_100.0;
        let l = sine(500.0, sr, 8192, 0.5);
        let r = sine(500.0, sr, 8192, 0.05);
        let (stats, issues) = analyze_mix(&l, &r, sr);
        assert!(stats.rms_l > stats.rms_r);
        assert!(issues
            .iter()
            .any(|i| i.category == IssueCategory::StereoBalance));
    }

    #[test]
    fn detects_low_heavy_mix() {
        let sr = 44_100.0;
        // Pure 80 Hz sine — all energy < 250 Hz band.
        let l = sine(80.0, sr, 16_384, 0.5);
        let r = l.clone();
        let (stats, issues) = analyze_mix(&l, &r, sr);
        assert!(stats.low_energy > 0.8);
        assert!(issues.iter().any(|i| i.category == IssueCategory::LowEnd));
    }

    #[test]
    fn detects_bright_mix() {
        let sr = 44_100.0;
        // Pure 10 kHz sine — all energy in the high band.
        let l = sine(10_000.0, sr, 16_384, 0.5);
        let r = l.clone();
        let (stats, issues) = analyze_mix(&l, &r, sr);
        assert!(stats.high_energy > 0.8);
        assert!(issues.iter().any(|i| i.category == IssueCategory::HighEnd));
    }

    #[test]
    fn silence_is_flagged_quiet_without_crashing() {
        let sr = 44_100.0;
        let (stats, issues) = analyze_mix(&silence(4096), &silence(4096), sr);
        assert_eq!(stats.peak_l, 0.0);
        assert_eq!(stats.peak_r, 0.0);
        // Silence shows up as very quiet headroom warning.
        assert!(issues.iter().any(|i| i.category == IssueCategory::Headroom));
    }
}
