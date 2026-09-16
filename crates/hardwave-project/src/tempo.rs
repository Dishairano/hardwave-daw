use serde::{Deserialize, Serialize};

use hardwave_midi::PPQ;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TempoRamp {
    Instant,
    Linear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoEntry {
    pub tick: u64,
    pub bpm: f64,
    pub time_sig_num: u32,
    pub time_sig_den: u32,
    pub ramp: TempoRamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoMap {
    pub entries: Vec<TempoEntry>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            entries: vec![TempoEntry {
                tick: 0,
                bpm: 140.0,
                time_sig_num: 4,
                time_sig_den: 4,
                ramp: TempoRamp::Instant,
            }],
        }
    }
}

impl TempoMap {
    /// Convert a tick position to an absolute sample position.
    pub fn tick_to_samples(&self, tick: u64, sample_rate: f64) -> u64 {
        let mut samples = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_bpm = self.entries[0].bpm;

        let mut prev_ramp = self.entries[0].ramp;

        for entry in &self.entries[1..] {
            if entry.tick >= tick {
                break;
            }
            let dt = entry.tick - prev_tick;
            // A ramped segment is integrated; a stepped one is a constant.
            let secs = if prev_ramp == TempoRamp::Linear {
                ramp_secs(dt, prev_bpm, entry.bpm)
            } else {
                ticks_to_secs(dt, prev_bpm)
            };
            samples += secs * sample_rate;
            prev_tick = entry.tick;
            prev_bpm = entry.bpm;
            prev_ramp = entry.ramp;
        }

        let remaining = tick - prev_tick;
        // Part-way into a ramp: integrate only as far as the target, using
        // the tempo the interpolation says is there.
        let secs = if prev_ramp == TempoRamp::Linear {
            ramp_secs(remaining, prev_bpm, self.bpm_at(tick))
        } else {
            ticks_to_secs(remaining, prev_bpm)
        };
        samples += secs * sample_rate;

        samples.round() as u64
    }

    /// Convert an absolute sample position to ticks.
    pub fn samples_to_tick(&self, target_samples: u64, sample_rate: f64) -> u64 {
        let mut samples_accum = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_bpm = self.entries[0].bpm;

        for entry in &self.entries[1..] {
            let dt = entry.tick - prev_tick;
            let secs = ticks_to_secs(dt, prev_bpm);
            let seg_samples = secs * sample_rate;

            if samples_accum + seg_samples > target_samples as f64 {
                break;
            }
            samples_accum += seg_samples;
            prev_tick = entry.tick;
            prev_bpm = entry.bpm;
        }

        let remaining_samples = target_samples as f64 - samples_accum;
        let remaining_secs = remaining_samples / sample_rate;
        let remaining_ticks = secs_to_ticks(remaining_secs, prev_bpm);

        prev_tick + remaining_ticks
    }

    /// Get the BPM at a given tick.
    pub fn bpm_at(&self, tick: u64) -> f64 {
        let mut bpm = self.entries[0].bpm;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.tick > tick {
                break;
            }
            bpm = entry.bpm;
            // A ramped entry slides towards the next one instead of holding
            // its value until the next step.
            if entry.ramp == TempoRamp::Linear {
                if let Some(next) = self.entries.get(i + 1) {
                    if next.tick > tick && next.tick > entry.tick {
                        let span = (next.tick - entry.tick) as f64;
                        let into = (tick - entry.tick) as f64;
                        bpm = entry.bpm + (next.bpm - entry.bpm) * (into / span);
                    }
                }
            }
        }
        bpm
    }

    /// Convert tick to (bar, beat) tuple (1-indexed).
    pub fn tick_to_bar_beat(&self, tick: u64) -> (u32, f64) {
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|e| e.tick <= tick)
            .unwrap_or(&self.entries[0]);

        let ticks_per_beat = PPQ;
        let ticks_per_bar = ticks_per_beat * entry.time_sig_num as u64;

        let relative_tick = tick - entry.tick;
        let bar = (relative_tick / ticks_per_bar) as u32 + 1;
        let beat = (relative_tick % ticks_per_bar) as f64 / ticks_per_beat as f64 + 1.0;

        (bar, beat)
    }
}

fn ticks_to_secs(ticks: u64, bpm: f64) -> f64 {
    let beats = ticks as f64 / PPQ as f64;
    beats * 60.0 / bpm
}

/// Seconds spent crossing `ticks` while the tempo slides from `from_bpm` to
/// `to_bpm`.
///
/// `TempoRamp::Linear` was stored and offered but never applied: the tempo
/// stepped at each entry, so a ramp sounded exactly like an instant change.
/// Interpolating the tempo is only half of it, because a position in samples
/// is the integral of the beat length, not the length at the end point. With
/// the tempo linear in ticks that integral is exact:
///
///   seconds = (beats * 60) * ln(b1 / b0) / (b1 - b0)
///
/// which stays correct however long the ramp is, where averaging the two
/// tempos would drift. Equal tempos fall back to the plain division, since
/// the formula divides by their difference.
fn ramp_secs(ticks: u64, from_bpm: f64, to_bpm: f64) -> f64 {
    let beats = ticks as f64 / PPQ as f64;
    if beats <= 0.0 {
        return 0.0;
    }
    if from_bpm <= 0.0 || to_bpm <= 0.0 {
        return 0.0;
    }
    if (to_bpm - from_bpm).abs() < 1e-9 {
        return beats * 60.0 / from_bpm;
    }
    beats * 60.0 * (to_bpm / from_bpm).ln() / (to_bpm - from_bpm)
}

fn secs_to_ticks(secs: f64, bpm: f64) -> u64 {
    let beats = secs * bpm / 60.0;
    (beats * PPQ as f64).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(entries: Vec<TempoEntry>) -> TempoMap {
        TempoMap { entries }
    }

    fn entry(tick: u64, bpm: f64, ramp: TempoRamp) -> TempoEntry {
        TempoEntry {
            tick,
            bpm,
            time_sig_num: 4,
            time_sig_den: 4,
            ramp,
        }
    }

    /// A ramp used to behave exactly like an instant change: the tempo was
    /// stored, offered in the UI, and then stepped at the entry.
    #[test]
    fn a_linear_ramp_slides_the_tempo_instead_of_stepping() {
        let ramped = map(vec![
            entry(0, 120.0, TempoRamp::Linear),
            entry(PPQ * 8, 160.0, TempoRamp::Instant),
        ]);

        assert_eq!(ramped.bpm_at(0), 120.0);
        // Half way through the ramp is half way between the tempos.
        assert!((ramped.bpm_at(PPQ * 4) - 140.0).abs() < 0.001);
        assert!((ramped.bpm_at(PPQ * 8) - 160.0).abs() < 0.001);

        let stepped = map(vec![
            entry(0, 120.0, TempoRamp::Instant),
            entry(PPQ * 8, 160.0, TempoRamp::Instant),
        ]);
        assert_eq!(stepped.bpm_at(PPQ * 4), 120.0, "an instant change holds");
    }

    /// The position of a beat is the integral of the beat length, not the
    /// length at the end point, so a ramp cannot be measured by averaging the
    /// two tempos: that drifts, and the drift grows with the ramp.
    #[test]
    fn a_ramp_is_integrated_not_averaged() {
        let sr = 48_000.0;
        let m = map(vec![
            entry(0, 120.0, TempoRamp::Linear),
            entry(PPQ * 8, 240.0, TempoRamp::Instant),
        ]);

        // 8 beats from 120 to 240 bpm: exactly 8 * 60 * ln(2) / 120 seconds.
        let expected_secs = 8.0 * 60.0 * (2.0f64).ln() / 120.0;
        let got = m.tick_to_samples(PPQ * 8, sr) as f64 / sr;
        assert!(
            (got - expected_secs).abs() < 0.001,
            "ramp took {got:.4}s, expected {expected_secs:.4}s"
        );

        // Averaging the endpoints would give 8 beats at 180 bpm = 2.667s,
        // which is nearly 0.11s out: audible, and worse over longer ramps.
        let averaged = 8.0 * 60.0 / 180.0;
        assert!(
            (averaged - expected_secs).abs() > 0.05,
            "this test only means something if averaging really is wrong"
        );
    }

    #[test]
    fn a_ramp_between_equal_tempos_is_just_that_tempo() {
        let sr = 48_000.0;
        let m = map(vec![
            entry(0, 140.0, TempoRamp::Linear),
            entry(PPQ * 4, 140.0, TempoRamp::Instant),
        ]);
        // The closed form divides by the tempo difference, so this is the
        // case that would produce NaN if it were applied blindly.
        let secs = m.tick_to_samples(PPQ * 4, sr) as f64 / sr;
        assert!((secs - 4.0 * 60.0 / 140.0).abs() < 0.001, "got {secs}");
    }

    #[test]
    fn a_position_part_way_into_a_ramp_is_still_correct() {
        let sr = 48_000.0;
        let m = map(vec![
            entry(0, 120.0, TempoRamp::Linear),
            entry(PPQ * 8, 240.0, TempoRamp::Instant),
        ]);

        let half = m.tick_to_samples(PPQ * 4, sr) as f64 / sr;
        let whole = m.tick_to_samples(PPQ * 8, sr) as f64 / sr;
        // Tempo rises, so the first half of the ramp takes longer than the
        // second: more than half the total, and less than all of it.
        assert!(half > whole / 2.0, "first half should be the slower half");
        assert!(half < whole);
    }
}
