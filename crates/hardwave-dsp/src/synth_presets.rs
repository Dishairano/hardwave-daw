//! Factory preset catalogs for the built-in synths and drum
//! instruments. Values are concrete so the instruments can load a
//! sensible sound without user configuration. Separate from the
//! engine crates so the UI can enumerate presets without depending
//! on the full synth implementation.

use std::borrow::Cow;

/// FM synth factory preset.
#[derive(Debug, Clone)]
pub struct FmPreset {
    pub name: &'static str,
    pub category: FmCategory,
    pub algorithm_index: u8,
    /// Per-operator ratio / level / feedback. Arrays are length 4 for
    /// our 4-operator engine.
    pub ratios: [f32; 4],
    pub levels: [f32; 4],
    pub feedback: [f32; 4],
    /// Per-operator ADSR in seconds / level / seconds / seconds.
    pub adsr: [(f32, f32, f32, f32); 4],
    pub description: Cow<'static, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FmCategory {
    ElectricPiano,
    Bass,
    Bells,
    Pad,
    Lead,
}

/// Built-in FM presets: electric piano, bass, bells, pads, leads.
pub fn fm_presets() -> Vec<FmPreset> {
    vec![
        FmPreset {
            name: "Tines EP",
            category: FmCategory::ElectricPiano,
            algorithm_index: 2,
            ratios: [1.0, 14.0, 1.0, 1.0],
            levels: [1.0, 0.42, 0.6, 0.2],
            feedback: [0.0, 0.0, 0.0, 0.0],
            adsr: [
                (0.002, 0.8, 0.9, 0.3),
                (0.001, 0.4, 1.0, 0.15),
                (0.002, 1.0, 2.0, 0.5),
                (0.002, 0.8, 1.6, 0.5),
            ],
            description: Cow::Borrowed("DX-style electric piano with tine bell on attack"),
        },
        FmPreset {
            name: "FM Bass",
            category: FmCategory::Bass,
            algorithm_index: 0,
            ratios: [1.0, 2.0, 1.0, 3.0],
            levels: [1.0, 0.6, 0.4, 0.25],
            feedback: [0.3, 0.0, 0.0, 0.0],
            adsr: [
                (0.002, 0.9, 0.35, 0.1),
                (0.005, 0.5, 0.5, 0.1),
                (0.002, 0.3, 0.2, 0.1),
                (0.002, 0.2, 0.15, 0.1),
            ],
            description: Cow::Borrowed("Punchy FM bass with adjustable feedback growl"),
        },
        FmPreset {
            name: "Glass Bells",
            category: FmCategory::Bells,
            algorithm_index: 4,
            ratios: [1.0, 3.5, 7.0, 11.0],
            levels: [1.0, 0.7, 0.5, 0.3],
            feedback: [0.0, 0.0, 0.0, 0.0],
            adsr: [
                (0.002, 1.0, 3.5, 2.0),
                (0.002, 0.6, 1.8, 1.0),
                (0.002, 0.35, 1.2, 0.7),
                (0.002, 0.2, 0.7, 0.5),
            ],
            description: Cow::Borrowed("Shimmering inharmonic bells with long decay"),
        },
        FmPreset {
            name: "Warm Pad",
            category: FmCategory::Pad,
            algorithm_index: 6,
            ratios: [1.0, 1.0, 2.0, 0.5],
            levels: [1.0, 0.4, 0.3, 0.45],
            feedback: [0.15, 0.0, 0.0, 0.0],
            adsr: [
                (1.2, 0.85, 1.8, 1.6),
                (1.0, 0.6, 1.5, 1.3),
                (1.2, 0.45, 1.8, 1.4),
                (1.3, 0.7, 2.0, 1.8),
            ],
            description: Cow::Borrowed("Slow-attack pad with subtle inharmonic motion"),
        },
        FmPreset {
            name: "Bright Lead",
            category: FmCategory::Lead,
            algorithm_index: 1,
            ratios: [1.0, 2.0, 4.0, 1.0],
            levels: [1.0, 0.7, 0.55, 0.3],
            feedback: [0.25, 0.0, 0.0, 0.0],
            adsr: [
                (0.005, 0.95, 0.4, 0.2),
                (0.005, 0.6, 0.5, 0.2),
                (0.005, 0.45, 0.4, 0.2),
                (0.005, 0.2, 0.4, 0.2),
            ],
            description: Cow::Borrowed("Bright, assertive lead with harmonic bite"),
        },
    ]
}

/// Drum synth factory preset — parameters for the built-in
/// kick / tom / perc synth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrumSynthPreset {
    pub name: &'static str,
    pub style: DrumStyle,
    pub pitch_start_hz: f32,
    pub pitch_end_hz: f32,
    pub pitch_decay_secs: f32,
    pub body_level: f32,
    pub click_level: f32,
    pub click_decay_secs: f32,
    pub sub_level: f32,
    pub sub_freq_hz: f32,
    pub drive_amount: f32,
    pub length_secs: f32,
    pub low_cut_hz: f32,
    pub high_cut_hz: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumStyle {
    EightOhEight,
    Hardstyle,
    Techno,
    Boomy,
    Punchy,
}

/// Built-in drum-synth presets: 808, hardstyle, techno, boomy,
/// punchy.
pub fn drum_synth_presets() -> Vec<DrumSynthPreset> {
    vec![
        DrumSynthPreset {
            name: "808 Sub",
            style: DrumStyle::EightOhEight,
            pitch_start_hz: 180.0,
            pitch_end_hz: 55.0,
            pitch_decay_secs: 0.08,
            body_level: 1.0,
            click_level: 0.15,
            click_decay_secs: 0.01,
            sub_level: 0.9,
            sub_freq_hz: 40.0,
            drive_amount: 0.2,
            length_secs: 1.2,
            low_cut_hz: 30.0,
            high_cut_hz: 6_000.0,
        },
        DrumSynthPreset {
            name: "Hardstyle Kick",
            style: DrumStyle::Hardstyle,
            pitch_start_hz: 260.0,
            pitch_end_hz: 60.0,
            pitch_decay_secs: 0.06,
            body_level: 1.0,
            click_level: 0.6,
            click_decay_secs: 0.008,
            sub_level: 0.5,
            sub_freq_hz: 55.0,
            drive_amount: 0.8,
            length_secs: 0.45,
            low_cut_hz: 45.0,
            high_cut_hz: 9_000.0,
        },
        DrumSynthPreset {
            name: "Techno Thump",
            style: DrumStyle::Techno,
            pitch_start_hz: 150.0,
            pitch_end_hz: 65.0,
            pitch_decay_secs: 0.04,
            body_level: 1.0,
            click_level: 0.45,
            click_decay_secs: 0.005,
            sub_level: 0.55,
            sub_freq_hz: 50.0,
            drive_amount: 0.4,
            length_secs: 0.35,
            low_cut_hz: 40.0,
            high_cut_hz: 8_000.0,
        },
        DrumSynthPreset {
            name: "Boomy",
            style: DrumStyle::Boomy,
            pitch_start_hz: 220.0,
            pitch_end_hz: 50.0,
            pitch_decay_secs: 0.12,
            body_level: 1.0,
            click_level: 0.1,
            click_decay_secs: 0.02,
            sub_level: 0.8,
            sub_freq_hz: 45.0,
            drive_amount: 0.15,
            length_secs: 1.6,
            low_cut_hz: 25.0,
            high_cut_hz: 4_500.0,
        },
        DrumSynthPreset {
            name: "Punchy",
            style: DrumStyle::Punchy,
            pitch_start_hz: 200.0,
            pitch_end_hz: 68.0,
            pitch_decay_secs: 0.03,
            body_level: 1.0,
            click_level: 0.55,
            click_decay_secs: 0.004,
            sub_level: 0.35,
            sub_freq_hz: 55.0,
            drive_amount: 0.3,
            length_secs: 0.28,
            low_cut_hz: 50.0,
            high_cut_hz: 10_000.0,
        },
    ]
}

/// A drum-kit preset — 16 pad slots assigned to semantic sample
/// categories. The actual sample paths are resolved by the drum
/// machine at load time via the user's sample library.
#[derive(Debug, Clone)]
pub struct DrumKitPreset {
    pub name: &'static str,
    pub style: DrumKitStyle,
    /// Pad 0 → 15 → semantic sample category (e.g. "kick", "snare").
    /// `None` = slot intentionally empty.
    pub slots: [Option<&'static str>; 16],
    pub description: Cow<'static, str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrumKitStyle {
    EightOhEight,
    NineOhNine,
    Trap,
    Acoustic,
}

/// Built-in drum kits: 808, 909, trap, acoustic.
pub fn drum_kit_presets() -> Vec<DrumKitPreset> {
    vec![
        DrumKitPreset {
            name: "808 Kit",
            style: DrumKitStyle::EightOhEight,
            slots: [
                Some("808/kick"),
                Some("808/snare"),
                Some("808/rim"),
                Some("808/clap"),
                Some("808/hat_closed"),
                Some("808/hat_open"),
                Some("808/cowbell"),
                Some("808/clave"),
                Some("808/tom_low"),
                Some("808/tom_mid"),
                Some("808/tom_high"),
                Some("808/conga_low"),
                Some("808/conga_mid"),
                Some("808/conga_high"),
                Some("808/maracas"),
                Some("808/cymbal"),
            ],
            description: Cow::Borrowed("Roland TR-808-style drum machine kit"),
        },
        DrumKitPreset {
            name: "909 Kit",
            style: DrumKitStyle::NineOhNine,
            slots: [
                Some("909/kick"),
                Some("909/snare"),
                Some("909/rim"),
                Some("909/clap"),
                Some("909/hat_closed"),
                Some("909/hat_open"),
                Some("909/ride"),
                Some("909/crash"),
                Some("909/tom_low"),
                Some("909/tom_mid"),
                Some("909/tom_high"),
                None,
                None,
                None,
                None,
                None,
            ],
            description: Cow::Borrowed("TR-909 house / techno kit"),
        },
        DrumKitPreset {
            name: "Trap Kit",
            style: DrumKitStyle::Trap,
            slots: [
                Some("trap/kick"),
                Some("trap/snare"),
                Some("trap/clap"),
                Some("trap/rim"),
                Some("trap/hat_closed"),
                Some("trap/hat_open"),
                Some("trap/hat_roll"),
                Some("trap/shaker"),
                Some("trap/808_a"),
                Some("trap/808_b"),
                Some("trap/perc_a"),
                Some("trap/perc_b"),
                Some("trap/snap"),
                Some("trap/fx_riser"),
                Some("trap/fx_impact"),
                Some("trap/vox"),
            ],
            description: Cow::Borrowed("Modern trap kit with rolled hats and 808 subs"),
        },
        DrumKitPreset {
            name: "Acoustic Kit",
            style: DrumKitStyle::Acoustic,
            slots: [
                Some("acoustic/kick"),
                Some("acoustic/snare_center"),
                Some("acoustic/snare_rim"),
                Some("acoustic/snare_cross"),
                Some("acoustic/hat_closed"),
                Some("acoustic/hat_half"),
                Some("acoustic/hat_open"),
                Some("acoustic/ride_bow"),
                Some("acoustic/ride_bell"),
                Some("acoustic/crash_1"),
                Some("acoustic/crash_2"),
                Some("acoustic/china"),
                Some("acoustic/tom_floor"),
                Some("acoustic/tom_low"),
                Some("acoustic/tom_mid"),
                Some("acoustic/tom_high"),
            ],
            description: Cow::Borrowed("Recorded acoustic drum-set kit"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fm_presets_cover_each_category() {
        let presets = fm_presets();
        for cat in [
            FmCategory::ElectricPiano,
            FmCategory::Bass,
            FmCategory::Bells,
            FmCategory::Pad,
            FmCategory::Lead,
        ] {
            assert!(
                presets.iter().any(|p| p.category == cat),
                "missing category {:?}",
                cat
            );
        }
    }

    #[test]
    fn fm_presets_have_valid_algorithm_indices() {
        for p in fm_presets() {
            assert!(p.algorithm_index < 8, "algo {} oob", p.algorithm_index);
        }
    }

    #[test]
    fn drum_synth_presets_cover_each_style() {
        let presets = drum_synth_presets();
        for style in [
            DrumStyle::EightOhEight,
            DrumStyle::Hardstyle,
            DrumStyle::Techno,
            DrumStyle::Boomy,
            DrumStyle::Punchy,
        ] {
            assert!(
                presets.iter().any(|p| p.style == style),
                "missing style {:?}",
                style
            );
        }
    }

    #[test]
    fn drum_synth_presets_have_sane_parameter_ranges() {
        for p in drum_synth_presets() {
            assert!(p.pitch_start_hz > p.pitch_end_hz, "{} pitch", p.name);
            assert!(
                p.length_secs > 0.1 && p.length_secs < 5.0,
                "{} length",
                p.name
            );
            assert!(p.low_cut_hz < p.high_cut_hz, "{} cuts", p.name);
            assert!(p.drive_amount >= 0.0 && p.drive_amount <= 1.0);
        }
    }

    #[test]
    fn drum_kit_presets_cover_each_style() {
        let presets = drum_kit_presets();
        for style in [
            DrumKitStyle::EightOhEight,
            DrumKitStyle::NineOhNine,
            DrumKitStyle::Trap,
            DrumKitStyle::Acoustic,
        ] {
            assert!(presets.iter().any(|p| p.style == style));
        }
    }

    #[test]
    fn drum_kits_have_kick_on_pad_zero() {
        for kit in drum_kit_presets() {
            let slot = kit.slots[0].expect("pad 0");
            assert!(slot.contains("kick"), "{} pad 0 = {}", kit.name, slot);
        }
    }
}
