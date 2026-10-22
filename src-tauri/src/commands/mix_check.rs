//! Tools > Mix check: render the song as an export would and say what is
//! worth knowing before it goes out.
//!
//! The DAW carried a mixdown analyser that nothing called, with limits taken
//! from pop: it told every hardstyle mix to back off the limiter and every
//! kick-heavy one to cut its low end. This check judges only what there is a
//! yardstick for. Overs and balance are judged on their own terms; loudness
//! is placed against what the harder-styles scene actually releases, measured
//! from 33,605 drops in October 2026 (brain: wiki/sources/scene-loudness-2026-10.md).
//! The low end and the crest factor have no such yardstick, so they are shown
//! as numbers with no verdict.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;

const SAMPLE_RATE: u32 = 48_000;
/// Chunks the loudness meter reads, in frames: short-term loudness is a 3 s
/// window of these.
const METER_BLOCK: usize = 512;

/// The drop loudness (max short-term LUFS) of the harder-styles releases
/// measured on 7-8 Oct 2026, as (percentile, LUFS).
const SCENE_DROPS: [(f32, f32); 5] = [
    (10.0, -11.0),
    (25.0, -8.3),
    (50.0, -5.9),
    (75.0, -4.1),
    (90.0, -2.7),
];
/// The median of the 150-159 BPM group, hardstyle and rawstyle.
const SCENE_HARDSTYLE_MEDIAN: f32 = -4.8;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// "problem", "warning" or "info".
    pub severity: &'static str,
    pub title: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Measured {
    pub duration_secs: f64,
    pub peak_db: f32,
    pub true_peak_db: f32,
    pub true_peak_at_secs: f64,
    pub lufs_integrated: Option<f32>,
    /// The loudest 3 seconds: how loud the drop is.
    pub lufs_short_max: Option<f32>,
    pub lufs_short_max_at_secs: f64,
    pub crest_db: f32,
    /// Left minus right, RMS, in dB.
    pub balance_db: f32,
    /// Shares of the energy below 250 Hz, up to 4 kHz and above, 0 to 1.
    pub low_share: f32,
    pub mid_share: f32,
    pub high_share: f32,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MixCheck {
    pub measured: Measured,
    pub findings: Vec<Finding>,
}

fn at(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Where `lufs` falls among the scene's drops, as a percentage of them it is
/// louder than, read between the measured percentiles.
fn scene_percentile(lufs: f32) -> f32 {
    let (first, last) = (SCENE_DROPS[0], SCENE_DROPS[SCENE_DROPS.len() - 1]);
    if lufs <= first.1 {
        return first.0 * (1.0 - ((first.1 - lufs) / 10.0).min(1.0));
    }
    if lufs >= last.1 {
        return last.0 + (100.0 - last.0) * ((lufs - last.1) / 3.0).min(1.0);
    }
    for w in SCENE_DROPS.windows(2) {
        let ((p0, l0), (p1, l1)) = (w[0], w[1]);
        if lufs <= l1 {
            return p0 + (p1 - p0) * (lufs - l0) / (l1 - l0);
        }
    }
    50.0
}

/// What to say about a song, from its numbers.
pub fn findings(m: &Measured) -> Vec<Finding> {
    let mut out = Vec::new();
    if m.true_peak_db > 0.0 {
        out.push(Finding {
            severity: "problem",
            title: "The true peak goes over 0 dB".into(),
            detail: format!(
                "{:+.1} dBTP at {}. Streaming services and MP3s turn this into distortion. Set the limiter on the master to a ceiling of -1 dB.",
                m.true_peak_db,
                at(m.true_peak_at_secs)
            ),
        });
    } else if m.peak_db >= -0.05 {
        out.push(Finding {
            severity: "warning",
            title: "The peaks touch 0 dB".into(),
            detail: "Between samples the signal can go over. A ceiling of -1 dB on the master limiter leaves room for it.".into(),
        });
    }
    if m.balance_db.abs() > 1.5 {
        let side = if m.balance_db > 0.0 { "left" } else { "right" };
        out.push(Finding {
            severity: "warning",
            title: format!("Left and right differ by {:.1} dB", m.balance_db.abs()),
            detail: format!("The {side} side is louder over the whole song. Check the panning of the loud tracks."),
        });
    }
    if let Some(drop) = m.lufs_short_max {
        let louder_than = scene_percentile(drop).round().clamp(0.0, 100.0);
        let (severity, title) = if drop < SCENE_DROPS[0].1 {
            (
                "warning",
                format!("The drop is quieter than 9 in 10 releases: {drop:.1} LUFS"),
            )
        } else {
            (
                "info",
                format!(
                    "The drop: {drop:.1} LUFS short-term, at {}",
                    at(m.lufs_short_max_at_secs)
                ),
            )
        };
        out.push(Finding {
            severity,
            title,
            detail: format!(
                "Louder than about {louder_than:.0}% of 33,605 harder-styles drops measured in October 2026 (middle half {:.1} to {:.1}, median {:.1}; hardstyle and rawstyle around 150 BPM: {:.1}).",
                SCENE_DROPS[1].1, SCENE_DROPS[3].1, SCENE_DROPS[2].1, SCENE_HARDSTYLE_MEDIAN
            ),
        });
    }
    out.push(Finding {
        severity: "info",
        title: format!("Low end: {:.0}% of the energy is below 250 Hz", m.low_share * 100.0),
        detail: "Shown for reference. Harder styles live on the kick, and there is no measured yardstick for this yet, so it is not judged.".into(),
    });
    out.push(Finding {
        severity: "info",
        title: format!("Crest factor {:.1} dB", m.crest_db),
        detail: "How far the peaks stand above the average level. Shown for reference, not judged."
            .into(),
    });
    out
}

/// Render the song and measure it. Reports progress as `mix-check-progress`
/// (percent).
#[tauri::command]
pub async fn mix_check(app: AppHandle) -> Result<MixCheck, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let engine = state.engine.lock();
        let total = engine.project_end_samples(SAMPLE_RATE);
        if total == 0 {
            return Err("The song is empty: there is nothing to check.".to_string());
        }
        let factory = super::export::offline_insert_factory(&engine);
        let render = engine.prepare_offline_render(SAMPLE_RATE, 0, Some(&factory), |_| {});
        drop(engine);

        let mut meter = hardwave_metering::ChannelMeter::new(SAMPLE_RATE as f64);
        let (mut left, mut right) = (
            Vec::with_capacity(total as usize),
            Vec::with_capacity(total as usize),
        );
        let (mut pend_l, mut pend_r) = (
            Vec::with_capacity(METER_BLOCK),
            Vec::with_capacity(METER_BLOCK),
        );
        let mut done = 0u64;
        let (mut true_peak, mut true_peak_at, mut short_max, mut short_at) =
            (-100.0f32, 0u64, None::<f32>, 0u64);
        let mut last_pct = -1i32;
        render.run(total, |block| {
            for frame in block.as_chunks::<2>().0 {
                left.push(frame[0]);
                right.push(frame[1]);
                pend_l.push(frame[0]);
                pend_r.push(frame[1]);
                if pend_l.len() == METER_BLOCK {
                    meter.process_block(&pend_l, &pend_r);
                    pend_l.clear();
                    pend_r.clear();
                    let pos = left.len() as u64;
                    if meter.true_peak_db() > true_peak {
                        true_peak = meter.true_peak_db();
                        true_peak_at = pos;
                    }
                    if let Some(s) = meter.lufs_s(METER_BLOCK) {
                        if short_max.is_none_or(|m| s > m) {
                            short_max = Some(s);
                            short_at = pos.saturating_sub(SAMPLE_RATE as u64 * 3 / 2);
                        }
                    }
                }
            }
            done = left.len() as u64;
            let pct = (done * 100 / total.max(1)) as i32;
            if pct != last_pct {
                last_pct = pct;
                let _ = app.emit("mix-check-progress", pct);
            }
            true
        });

        let (stats, _) = hardwave_dsp::mix_feedback::analyze_mix(&left, &right, SAMPLE_RATE as f32);
        let to_db = |v: f32| if v <= 1e-9 { -100.0 } else { 20.0 * v.log10() };
        let energy = (stats.low_energy + stats.mid_energy + stats.high_energy).max(1e-12);
        let sr = SAMPLE_RATE as f64;
        let measured = Measured {
            duration_secs: done as f64 / sr,
            peak_db: to_db(stats.peak_l.max(stats.peak_r)),
            true_peak_db: true_peak,
            true_peak_at_secs: true_peak_at as f64 / sr,
            lufs_integrated: meter.lufs_i(),
            lufs_short_max: short_max,
            lufs_short_max_at_secs: short_at as f64 / sr,
            crest_db: stats.crest_factor_db,
            balance_db: to_db(stats.rms_l) - to_db(stats.rms_r),
            low_share: stats.low_energy / energy,
            mid_share: stats.mid_energy / energy,
            high_share: stats.high_energy / energy,
        };
        let findings = findings(&measured);
        Ok(MixCheck { measured, findings })
    })
    .await
    .map_err(|e| format!("the check stopped: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Measured {
        Measured {
            duration_secs: 192.0,
            peak_db: -0.3,
            true_peak_db: -0.6,
            true_peak_at_secs: 107.0,
            lufs_integrated: Some(-7.0),
            lufs_short_max: Some(-4.8),
            lufs_short_max_at_secs: 100.0,
            crest_db: 6.1,
            balance_db: 0.2,
            low_share: 0.58,
            mid_share: 0.33,
            high_share: 0.09,
        }
    }

    #[test]
    fn a_loud_kick_heavy_hardstyle_master_is_not_called_a_problem() {
        let f = findings(&song());
        assert!(f.iter().all(|x| x.severity == "info"), "{f:?}");
    }

    #[test]
    fn a_true_peak_over_zero_is_a_problem_with_its_time() {
        let mut m = song();
        m.true_peak_db = 0.4;
        let f = findings(&m);
        assert_eq!(f[0].severity, "problem");
        assert!(f[0].detail.contains("1:47"), "{}", f[0].detail);
    }

    #[test]
    fn an_unbalanced_mix_says_which_side() {
        let mut m = song();
        m.balance_db = 1.8;
        let f = findings(&m);
        assert!(f
            .iter()
            .any(|x| x.severity == "warning" && x.detail.contains("left")));
    }

    #[test]
    fn the_drop_is_placed_against_the_scene() {
        assert!((scene_percentile(-5.9) - 50.0).abs() < 0.01);
        assert!((scene_percentile(-4.1) - 75.0).abs() < 0.01);
        assert!(scene_percentile(-2.0) > 90.0);
        assert!(scene_percentile(-14.0) < 10.0);
        let mut m = song();
        m.lufs_short_max = Some(-12.0);
        assert!(findings(&m)
            .iter()
            .any(|x| x.severity == "warning" && x.title.contains("9 in 10")));
    }
}
