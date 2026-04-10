//! Fade curve types and application.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    SCurve,
    Logarithmic,
}

/// Apply a fade-in to a buffer in place.
pub fn apply_fade(samples: &mut [f32], curve: FadeCurve, fade_in: bool) {
    let len = samples.len();
    if len == 0 {
        return;
    }

    for (i, sample) in samples.iter_mut().enumerate() {
        let t = i as f32 / len as f32;
        let t = if fade_in { t } else { 1.0 - t };

        let gain = match curve {
            FadeCurve::Linear => t,
            FadeCurve::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
            FadeCurve::SCurve => {
                let x = t * 2.0 - 1.0;
                (x * std::f32::consts::FRAC_PI_2).sin() * 0.5 + 0.5
            }
            FadeCurve::Logarithmic => {
                if t <= 0.0 {
                    0.0
                } else {
                    (1.0 + (t * 99.0).log10() / 2.0).clamp(0.0, 1.0)
                }
            }
        };

        *sample *= gain;
    }
}
