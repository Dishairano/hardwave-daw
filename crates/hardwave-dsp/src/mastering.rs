//! AI mastering assistant and stem separation contract.
//!
//! `analyze_for_mastering(stats)` picks a genre-appropriate mastering
//! chain from built-in presets based on spectral, loudness, and
//! dynamics cues — deterministic heuristic, not ML. Stem separation
//! is represented by `StemSeparationRequest` and
//! `StemSeparationResult`; the model itself runs in a worker binary.

/// Genre preset — concrete mastering chain parameters. These live
/// here so the UI + audio engine agree on what "Apply pop master"
/// actually does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MasteringPreset {
    pub genre: Genre,
    pub low_shelf_db: f32,
    pub high_shelf_db: f32,
    pub bus_compressor_threshold_db: f32,
    pub bus_compressor_ratio: f32,
    pub limiter_ceiling_db: f32,
    pub target_lufs: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Genre {
    Pop,
    Rock,
    HipHop,
    Edm,
    Jazz,
    Classical,
    Acoustic,
}

/// Built-in catalog — one concrete preset per supported genre.
pub fn genre_mastering_presets() -> Vec<MasteringPreset> {
    vec![
        MasteringPreset {
            genre: Genre::Pop,
            low_shelf_db: 1.0,
            high_shelf_db: 2.0,
            bus_compressor_threshold_db: -12.0,
            bus_compressor_ratio: 2.0,
            limiter_ceiling_db: -1.0,
            target_lufs: -11.0,
        },
        MasteringPreset {
            genre: Genre::Rock,
            low_shelf_db: 2.0,
            high_shelf_db: 1.5,
            bus_compressor_threshold_db: -14.0,
            bus_compressor_ratio: 3.0,
            limiter_ceiling_db: -0.3,
            target_lufs: -10.0,
        },
        MasteringPreset {
            genre: Genre::HipHop,
            low_shelf_db: 3.0,
            high_shelf_db: 1.0,
            bus_compressor_threshold_db: -8.0,
            bus_compressor_ratio: 2.0,
            limiter_ceiling_db: -0.3,
            target_lufs: -9.0,
        },
        MasteringPreset {
            genre: Genre::Edm,
            low_shelf_db: 2.5,
            high_shelf_db: 3.0,
            bus_compressor_threshold_db: -10.0,
            bus_compressor_ratio: 2.5,
            limiter_ceiling_db: -0.2,
            target_lufs: -8.0,
        },
        MasteringPreset {
            genre: Genre::Jazz,
            low_shelf_db: 0.0,
            high_shelf_db: 1.0,
            bus_compressor_threshold_db: -18.0,
            bus_compressor_ratio: 1.5,
            limiter_ceiling_db: -1.5,
            target_lufs: -16.0,
        },
        MasteringPreset {
            genre: Genre::Classical,
            low_shelf_db: 0.0,
            high_shelf_db: 0.5,
            bus_compressor_threshold_db: -24.0,
            bus_compressor_ratio: 1.2,
            limiter_ceiling_db: -2.0,
            target_lufs: -18.0,
        },
        MasteringPreset {
            genre: Genre::Acoustic,
            low_shelf_db: 1.0,
            high_shelf_db: 1.5,
            bus_compressor_threshold_db: -18.0,
            bus_compressor_ratio: 2.0,
            limiter_ceiling_db: -1.0,
            target_lufs: -14.0,
        },
    ]
}

/// Input summary the mastering assistant analyzes. Callers can
/// reuse `mix_feedback::MixStats` and pass the relevant fields into
/// `TrackSummary`.
#[derive(Debug, Clone, Copy)]
pub struct TrackSummary {
    pub integrated_lufs: f32,
    pub crest_factor_db: f32,
    pub low_band_energy: f32,
    pub mid_band_energy: f32,
    pub high_band_energy: f32,
}

/// Assistant output — the picked preset plus a list of human-
/// readable reasons (for the "why this preset" tooltip).
#[derive(Debug, Clone)]
pub struct SuggestedChain {
    pub preset: MasteringPreset,
    pub reasons: Vec<String>,
}

/// Analyze a summary and return the best matching preset. Simple
/// rule-based classifier — "heavy low end + crushed dynamics = hip
/// hop", "bright + punchy = EDM", etc.
pub fn analyze_for_mastering(summary: TrackSummary) -> SuggestedChain {
    let mut reasons = Vec::new();
    let crest = summary.crest_factor_db;
    let low = summary.low_band_energy;
    let high = summary.high_band_energy;

    let genre = if low > 0.5 && crest < 8.0 {
        reasons.push("Sub-heavy low end + compressed dynamics → hip hop master".into());
        Genre::HipHop
    } else if high > 0.35 && low > 0.3 && crest < 9.0 {
        reasons.push("Bright + punchy with modest dynamics → EDM master".into());
        Genre::Edm
    } else if crest > 14.0 && high < 0.2 {
        reasons.push("Wide dynamic range + dark tone → classical master".into());
        Genre::Classical
    } else if crest > 11.0 && high < 0.3 {
        reasons.push("Preserved dynamics + natural tone → jazz master".into());
        Genre::Jazz
    } else if low > 0.35 && crest < 10.0 {
        reasons.push("Full low end + moderate compression → rock master".into());
        Genre::Rock
    } else if crest > 10.0 {
        reasons.push("Moderate dynamics + balanced spectrum → acoustic master".into());
        Genre::Acoustic
    } else {
        reasons.push("Balanced spectrum + contemporary dynamics → pop master".into());
        Genre::Pop
    };
    if summary.integrated_lufs < -24.0 {
        reasons.push(format!(
            "Input is quiet ({:.1} LUFS) — limiter will raise toward target.",
            summary.integrated_lufs
        ));
    }
    if summary.integrated_lufs > -6.0 {
        reasons.push(format!(
            "Input is already loud ({:.1} LUFS) — chain will preserve transients.",
            summary.integrated_lufs
        ));
    }
    let preset = genre_mastering_presets()
        .into_iter()
        .find(|p| p.genre == genre)
        .expect("preset for genre");
    SuggestedChain { preset, reasons }
}

/// Stem-separation request — the DAW submits one of these to the
/// stem-separation worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StemSeparationRequest {
    pub quality: StemQuality,
    pub target_stems: StemTargets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemQuality {
    Fast,
    Balanced,
    HighQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StemTargets {
    pub vocals: bool,
    pub drums: bool,
    pub bass: bool,
    pub other: bool,
}

impl StemTargets {
    pub fn all() -> Self {
        Self {
            vocals: true,
            drums: true,
            bass: true,
            other: true,
        }
    }

    pub fn selected_count(&self) -> u8 {
        [self.vocals, self.drums, self.bass, self.other]
            .into_iter()
            .map(u8::from)
            .sum()
    }
}

/// Result from the separator. Each stem is a stereo buffer; unused
/// targets come back empty.
#[derive(Debug, Clone, Default)]
pub struct StemSeparationResult {
    pub sample_rate: f32,
    pub vocals: Option<(Vec<f32>, Vec<f32>)>,
    pub drums: Option<(Vec<f32>, Vec<f32>)>,
    pub bass: Option<(Vec<f32>, Vec<f32>)>,
    pub other: Option<(Vec<f32>, Vec<f32>)>,
    pub processing_time_secs: f32,
}

impl StemSeparationResult {
    pub fn any_stem_present(&self) -> bool {
        self.vocals.is_some() || self.drums.is_some() || self.bass.is_some() || self.other.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(lufs: f32, crest: f32, low: f32, mid: f32, high: f32) -> TrackSummary {
        TrackSummary {
            integrated_lufs: lufs,
            crest_factor_db: crest,
            low_band_energy: low,
            mid_band_energy: mid,
            high_band_energy: high,
        }
    }

    #[test]
    fn presets_cover_every_genre() {
        let presets = genre_mastering_presets();
        for g in [
            Genre::Pop,
            Genre::Rock,
            Genre::HipHop,
            Genre::Edm,
            Genre::Jazz,
            Genre::Classical,
            Genre::Acoustic,
        ] {
            assert!(presets.iter().any(|p| p.genre == g));
        }
    }

    #[test]
    fn hip_hop_detection_on_sub_heavy_squashed_mix() {
        let s = summary(-8.0, 6.0, 0.6, 0.3, 0.1);
        let chain = analyze_for_mastering(s);
        assert_eq!(chain.preset.genre, Genre::HipHop);
        assert!(chain.reasons.iter().any(|r| r.contains("hip hop")));
    }

    #[test]
    fn classical_detection_on_dynamic_dark_mix() {
        let s = summary(-20.0, 18.0, 0.3, 0.55, 0.15);
        let chain = analyze_for_mastering(s);
        assert_eq!(chain.preset.genre, Genre::Classical);
    }

    #[test]
    fn pop_is_default_for_balanced_mix() {
        let s = summary(-12.0, 9.0, 0.3, 0.45, 0.25);
        let chain = analyze_for_mastering(s);
        assert_eq!(chain.preset.genre, Genre::Pop);
    }

    #[test]
    fn reasons_mention_loudness_when_very_quiet() {
        let s = summary(-28.0, 14.0, 0.3, 0.5, 0.2);
        let chain = analyze_for_mastering(s);
        assert!(chain.reasons.iter().any(|r| r.contains("quiet")));
    }

    #[test]
    fn stem_targets_all_selects_four_stems() {
        assert_eq!(StemTargets::all().selected_count(), 4);
        let only_vocals = StemTargets {
            vocals: true,
            drums: false,
            bass: false,
            other: false,
        };
        assert_eq!(only_vocals.selected_count(), 1);
    }

    #[test]
    fn stem_result_any_stem_present_reports_correctly() {
        let mut r = StemSeparationResult {
            sample_rate: 48_000.0,
            ..Default::default()
        };
        assert!(!r.any_stem_present());
        r.vocals = Some((vec![0.0; 10], vec![0.0; 10]));
        assert!(r.any_stem_present());
    }
}
