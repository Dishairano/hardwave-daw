//! Lining two recordings of the same thing up in time.
//!
//! A kick recorded with a close mic and a room mic is the same hit
//! twice, a few milliseconds apart. Mixed together the gap eats the
//! low end, and finding it by dragging one clip a sample at a time is
//! a job nobody should do by hand.
//!
//! The measurement is a plain cross-correlation: slide one signal past
//! the other, see where they agree most, and say whether they agree
//! better with one of them turned upside down, which is the other half
//! of the same problem.

/// What the measurement found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How far the target sits behind the reference, in samples. A
    /// positive number means the target is late.
    pub offset_samples: i64,
    /// How well the two agree once lined up, -1 to 1. Near zero means
    /// they are not recordings of the same thing and the offset is not
    /// worth applying.
    pub correlation: f32,
    /// True when they agree better with the target inverted, which is
    /// what a microphone on the other side of a drum does.
    pub polarity_flip: bool,
}

/// Find the offset between two takes of the same sound.
///
/// `max_lag` bounds the search, so a mistake cannot drag a clip across
/// the whole song: for microphones a few metres apart, twenty
/// milliseconds is already generous.
pub fn best_offset(reference: &[f32], target: &[f32], max_lag: usize) -> Alignment {
    let n = reference.len().min(target.len());
    if n == 0 || max_lag == 0 {
        return Alignment {
            offset_samples: 0,
            correlation: 0.0,
            polarity_flip: false,
        };
    }
    let max_lag = max_lag.min(n.saturating_sub(1)).max(1);

    let total: f64 = reference[..n]
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        + target[..n]
            .iter()
            .map(|x| (*x as f64) * (*x as f64))
            .sum::<f64>();
    if total <= f64::EPSILON {
        return Alignment {
            offset_samples: 0,
            correlation: 0.0,
            polarity_flip: false,
        };
    }

    let mut best = Alignment {
        offset_samples: 0,
        correlation: 0.0,
        polarity_flip: false,
    };
    // Smallest shifts first, and a later lag has to be clearly better
    // to win. A periodic sound correlates nearly as well a whole cycle
    // away, and moving a clip a cycle when it was already lined up is
    // worse than leaving it alone.
    let order = std::iter::once(0i64).chain((1..=max_lag as i64).flat_map(|l| [l, -l]));
    for lag in order {
        // Only the part where the two actually overlap counts, and the
        // score is normalised by that part alone. Normalising by the
        // whole take instead would make every large offset look worse
        // than it is and pull the answer towards zero.
        let (ref_from, tgt_from) = if lag >= 0 {
            (0usize, lag as usize)
        } else {
            ((-lag) as usize, 0usize)
        };
        let overlap = n - ref_from.max(tgt_from);
        if overlap < 64 {
            continue;
        }
        let mut sum = 0.0f64;
        let mut ref_energy = 0.0f64;
        let mut tgt_energy = 0.0f64;
        for i in 0..overlap {
            let a = reference[ref_from + i] as f64;
            let b = target[tgt_from + i] as f64;
            sum += a * b;
            ref_energy += a * a;
            tgt_energy += b * b;
        }
        let norm = (ref_energy * tgt_energy).sqrt();
        if norm <= f64::EPSILON {
            continue;
        }
        let score = (sum / norm) as f32;
        // A tie goes to the smaller shift, which is why the margin is
        // only big enough to ignore rounding.
        if score.abs() > best.correlation.abs() + 1e-5 {
            best = Alignment {
                // The lag that lines them up is how late the target is:
                // positive means it has to come forward.
                offset_samples: lag,
                correlation: score,
                polarity_flip: score < 0.0,
            };
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(len: usize) -> Vec<f32> {
        (0..len)
            .map(|n| {
                let t = n as f32 / 48_000.0;
                let env = (-t * 40.0).exp();
                (std::f32::consts::TAU * 120.0 * t).sin() * env
            })
            .collect()
    }

    #[test]
    fn a_copy_that_is_late_is_measured_as_late() {
        let reference = burst(4096);
        let delay = 73usize;
        let mut target = vec![0.0f32; reference.len()];
        target[delay..].copy_from_slice(&reference[..reference.len() - delay]);

        let found = best_offset(&reference, &target, 500);
        assert_eq!(found.offset_samples, delay as i64);
        assert!(
            found.correlation > 0.9,
            "they are the same sound: {found:?}"
        );
        assert!(!found.polarity_flip);
    }

    #[test]
    fn a_copy_that_is_early_is_measured_as_early() {
        let reference = burst(4096);
        let early = 50usize;
        let mut target = vec![0.0f32; reference.len()];
        target[..reference.len() - early].copy_from_slice(&reference[early..]);

        let found = best_offset(&reference, &target, 500);
        assert_eq!(found.offset_samples, -(early as i64));
    }

    #[test]
    fn an_upside_down_copy_is_flagged() {
        let reference = burst(4096);
        let target: Vec<f32> = reference.iter().map(|s| -s).collect();
        let found = best_offset(&reference, &target, 200);
        assert_eq!(found.offset_samples, 0);
        assert!(found.polarity_flip, "{found:?}");
        assert!(found.correlation < -0.9);
    }

    #[test]
    fn two_unrelated_sounds_agree_about_nothing() {
        let reference = burst(4096);
        let target: Vec<f32> = (0..4096)
            .map(|n| ((n as f32 * 7919.0) % 1.0) - 0.5)
            .collect();
        let found = best_offset(&reference, &target, 200);
        assert!(
            found.correlation.abs() < 0.5,
            "noise should not look like the same take: {found:?}"
        );
    }

    #[test]
    fn silence_is_refused_rather_than_guessed_at() {
        let quiet = vec![0.0f32; 1024];
        let found = best_offset(&quiet, &quiet, 100);
        assert_eq!(found.correlation, 0.0);
        assert_eq!(found.offset_samples, 0);
    }
}
