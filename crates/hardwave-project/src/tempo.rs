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
    ///
    /// The inverse of `tick_to_samples`, ramps included. It used to treat
    /// every segment as a constant tempo, so across a ramp the two disagreed:
    /// the playhead was drawn and the tempo was followed at a tick the audio
    /// was not at, and the error grew with the length of the ramp.
    pub fn samples_to_tick(&self, target_samples: u64, sample_rate: f64) -> u64 {
        let mut samples_accum = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_bpm = self.entries[0].bpm;
        let mut prev_ramp = self.entries[0].ramp;
        let mut next_bpm = prev_bpm;
        let mut span_ticks = 0_u64;

        for entry in &self.entries[1..] {
            let dt = entry.tick - prev_tick;
            let secs = if prev_ramp == TempoRamp::Linear {
                ramp_secs(dt, prev_bpm, entry.bpm)
            } else {
                ticks_to_secs(dt, prev_bpm)
            };
            let seg_samples = secs * sample_rate;

            if samples_accum + seg_samples > target_samples as f64 {
                // The target is inside this segment, so remember where the
                // ramp is heading and how long it has to get there.
                next_bpm = entry.bpm;
                span_ticks = dt;
                break;
            }
            samples_accum += seg_samples;
            prev_tick = entry.tick;
            prev_bpm = entry.bpm;
            prev_ramp = entry.ramp;
            // Past the last entry the tempo holds, so there is no ramp left.
            next_bpm = entry.bpm;
            span_ticks = 0;
        }

        let remaining_samples = target_samples as f64 - samples_accum;
        let remaining_secs = remaining_samples / sample_rate;
        let remaining_ticks = if prev_ramp == TempoRamp::Linear && span_ticks > 0 {
            ramp_ticks(remaining_secs, prev_bpm, next_bpm, span_ticks)
        } else {
            secs_to_ticks(remaining_secs, prev_bpm)
        };

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

    /// The time signature in force at a tick.
    ///
    /// A signature holds until an entry changes it, so this is the last entry
    /// at or before the tick. The engine reads it every block while playing,
    /// which is what makes a signature change part-way through a song reach
    /// the click, the plug-ins and the bar counter.
    pub fn time_sig_at(&self, tick: u64) -> (u32, u32) {
        let entry = self
            .entries
            .iter()
            .rev()
            .find(|e| e.tick <= tick)
            .unwrap_or(&self.entries[0]);
        (entry.time_sig_num.max(1), entry.time_sig_den.max(1))
    }

    /// Where the bars restart, as `(tick, ticks_per_bar)` pairs.
    ///
    /// Only an entry that changes the signature starts a new bar. A tempo
    /// change does not: entries can sit anywhere, and treating every entry as
    /// a bar line would move the grid whenever someone added a tempo point
    /// part-way through a bar.
    pub fn meter_segments(&self) -> Vec<(u64, u64)> {
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut current: Option<(u32, u32)> = None;
        for entry in &self.entries {
            let sig = (entry.time_sig_num.max(1), entry.time_sig_den.max(1));
            if current == Some(sig) {
                continue;
            }
            current = Some(sig);
            out.push((entry.tick, ticks_per_bar(sig.0, sig.1)));
        }
        if out.is_empty() {
            out.push((0, ticks_per_bar(4, 4)));
        }
        // A signature change before the first bar line would leave the song
        // starting mid-bar, so the first segment always counts from zero.
        out[0].0 = 0;
        out
    }

    /// Convert tick to (bar, beat), both 1-indexed.
    ///
    /// Bars are counted from the start of the song across every signature
    /// change. This used to count from the last tempo entry instead, so bar 1
    /// appeared again at every entry and the ruler showed the same bar number
    /// several times in one song.
    pub fn tick_to_bar_beat(&self, tick: u64) -> (u32, f64) {
        let segments = self.meter_segments();
        let mut bar: u64 = 1;

        for (i, &(start, per_bar)) in segments.iter().enumerate() {
            let end = segments.get(i + 1).map(|&(t, _)| t);
            let within_this_segment = match end {
                Some(end) => tick < end,
                None => true,
            };
            if within_this_segment {
                let into = tick.saturating_sub(start);
                bar += into / per_bar;
                let per_beat = ticks_per_beat(self.time_sig_at(tick).1).max(1);
                let beat = (into % per_bar) as f64 / per_beat as f64 + 1.0;
                return (bar as u32, beat);
            }
            // A signature change starts a new bar, so a segment that does not
            // divide evenly still consumes the bar it cut short.
            let span = end.unwrap_or(start).saturating_sub(start);
            bar += span.div_ceil(per_bar);
        }

        (bar as u32, 1.0)
    }
}

/// Ticks in one beat for a signature's denominator: a quarter note for /4, an
/// eighth for /8. The denominator was ignored before, so 7/8 was counted in
/// quarter notes and every bar came out twice as long as it sounded.
pub fn ticks_per_beat(den: u32) -> u64 {
    match den {
        0 => PPQ,
        den => (PPQ * 4) / den as u64,
    }
}

/// Ticks in one bar of a signature.
pub fn ticks_per_bar(num: u32, den: u32) -> u64 {
    (ticks_per_beat(den) * num.max(1) as u64).max(1)
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

/// Ticks covered in `secs` while the tempo slides from `from_bpm` towards
/// `to_bpm` over `span_ticks`.
///
/// The inverse of `ramp_secs` within one segment. With the tempo linear in
/// ticks, b(x) = b0 + k x for x in beats, the elapsed time is
///
///   s(x) = (60 / k) * ln((b0 + k x) / b0)
///
/// so the position is
///
///   x = (b0 / k) * (exp(k s / 60) - 1)
///
/// Equal tempos, or a span of nothing, fall back to the plain division, since
/// the formula divides by the tempo difference.
fn ramp_ticks(secs: f64, from_bpm: f64, to_bpm: f64, span_ticks: u64) -> u64 {
    if secs <= 0.0 {
        return 0;
    }
    let span_beats = span_ticks as f64 / PPQ as f64;
    if span_beats <= 0.0 || from_bpm <= 0.0 || to_bpm <= 0.0 {
        return secs_to_ticks(secs, from_bpm.max(1.0));
    }
    let k = (to_bpm - from_bpm) / span_beats;
    if k.abs() < 1e-9 {
        return secs_to_ticks(secs, from_bpm);
    }
    let beats = (from_bpm / k) * ((k * secs / 60.0).exp() - 1.0);
    // Clamped to the segment: a caller past the end of the ramp belongs to the
    // next segment, not to an extrapolation of this one.
    let beats = beats.clamp(0.0, span_beats);
    (beats * PPQ as f64).round() as u64
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

    /// `samples_to_tick` ignored ramps, so it did not invert
    /// `tick_to_samples`: the playhead's tick and the audio's tick drifted
    /// apart across a ramp, and the drift grew with the ramp.
    #[test]
    fn a_position_in_samples_converts_back_to_the_tick_it_came_from() {
        let sr = 48_000.0;
        let m = map(vec![
            entry(0, 120.0, TempoRamp::Linear),
            entry(PPQ * 16, 240.0, TempoRamp::Instant),
        ]);

        for beats in [1, 4, 8, 12, 15, 16, 24] {
            let tick = PPQ * beats;
            let samples = m.tick_to_samples(tick, sr);
            let back = m.samples_to_tick(samples, sr);
            assert!(
                back.abs_diff(tick) <= 2,
                "beat {beats}: {tick} ticks became {back}"
            );
        }
    }

    #[test]
    fn a_stepped_tempo_map_still_converts_both_ways() {
        let sr = 48_000.0;
        let m = map(vec![
            entry(0, 140.0, TempoRamp::Instant),
            entry(PPQ * 8, 90.0, TempoRamp::Instant),
        ]);
        for beats in [2, 8, 9, 20] {
            let tick = PPQ * beats;
            let back = m.samples_to_tick(m.tick_to_samples(tick, sr), sr);
            assert!(back.abs_diff(tick) <= 2, "beat {beats} became {back}");
        }
    }

    fn meter(tick: u64, num: u32, den: u32) -> TempoEntry {
        TempoEntry {
            tick,
            bpm: 140.0,
            time_sig_num: num,
            time_sig_den: den,
            ramp: TempoRamp::Instant,
        }
    }

    #[test]
    fn the_signature_at_a_tick_is_the_last_one_set_before_it() {
        let m = map(vec![meter(0, 4, 4), meter(PPQ * 16, 3, 4)]);
        assert_eq!(m.time_sig_at(0), (4, 4));
        assert_eq!(m.time_sig_at(PPQ * 15), (4, 4));
        assert_eq!(m.time_sig_at(PPQ * 16), (3, 4), "the change applies");
        assert_eq!(m.time_sig_at(PPQ * 99), (3, 4), "and it holds");
    }

    /// Bars used to be counted from the last tempo entry, so bar 1 appeared
    /// again at every entry and one song showed several bar 1s.
    #[test]
    fn bars_keep_counting_across_a_signature_change() {
        // 4 bars of 4/4, then 3/4 from bar 5.
        let m = map(vec![meter(0, 4, 4), meter(PPQ * 16, 3, 4)]);

        assert_eq!(m.tick_to_bar_beat(0), (1, 1.0));
        assert_eq!(m.tick_to_bar_beat(PPQ * 12).0, 4);
        assert_eq!(m.tick_to_bar_beat(PPQ * 16).0, 5, "not back to bar 1");
        // Bars are three beats long now, so bar 6 starts three beats later.
        assert_eq!(m.tick_to_bar_beat(PPQ * 19).0, 6);
        assert_eq!(m.tick_to_bar_beat(PPQ * 22).0, 7);
    }

    #[test]
    fn a_tempo_change_does_not_move_the_bar_lines() {
        // A tempo point part-way through bar 3, same signature.
        let m = map(vec![
            entry(0, 140.0, TempoRamp::Instant),
            entry(PPQ * 9, 90.0, TempoRamp::Instant),
        ]);
        assert_eq!(m.tick_to_bar_beat(PPQ * 9), (3, 2.0), "still bar 3 beat 2");
        assert_eq!(m.tick_to_bar_beat(PPQ * 12).0, 4);
    }

    #[test]
    fn an_eighth_note_signature_is_counted_in_eighths() {
        // 7/8 is seven eighths, so a bar is 3.5 quarter notes, not seven.
        let m = map(vec![meter(0, 7, 8)]);
        assert_eq!(ticks_per_bar(7, 8), PPQ * 7 / 2);
        assert_eq!(m.tick_to_bar_beat(0), (1, 1.0));
        assert_eq!(m.tick_to_bar_beat(PPQ / 2), (1, 2.0), "second eighth");
        assert_eq!(m.tick_to_bar_beat(PPQ * 7 / 2).0, 2);
    }

    #[test]
    fn a_signature_change_off_the_grid_still_starts_a_bar() {
        // The change lands half way through bar 2 of 4/4. That bar is cut
        // short and the new signature starts a bar of its own, which is what
        // a DAW shows rather than carrying a fragment into the next bar.
        let m = map(vec![meter(0, 4, 4), meter(PPQ * 6, 3, 4)]);
        assert_eq!(m.tick_to_bar_beat(PPQ * 5).0, 2);
        assert_eq!(m.tick_to_bar_beat(PPQ * 6), (3, 1.0));
        assert_eq!(m.tick_to_bar_beat(PPQ * 9).0, 4);
    }

    #[test]
    fn segments_only_start_where_the_signature_changes() {
        let m = map(vec![
            meter(0, 4, 4),
            // Tempo-only entry: no new segment.
            entry(PPQ * 8, 90.0, TempoRamp::Instant),
            meter(PPQ * 16, 5, 4),
            // Repeat of the same signature: no new segment either.
            meter(PPQ * 24, 5, 4),
        ]);
        assert_eq!(m.meter_segments(), vec![(0, PPQ * 4), (PPQ * 16, PPQ * 5)]);
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
