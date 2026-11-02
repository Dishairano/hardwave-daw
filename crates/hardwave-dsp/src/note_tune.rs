//! Tuning a take by hand, note by note.
//!
//! `pitch_correct` moves every sung note towards the key, each by its own
//! amount. This is what sits under the sample window's NOTES view, where a
//! producer sees the notes a take holds and moves them one at a time: up
//! or down, its wobble flattened, its start or end moved.
//!
//! How it sounds: pitch-synchronous overlap-add (TD-PSOLA). The take is
//! cut into grains centred one period of the voice apart, each two periods
//! long, and laid down again closer together to raise a note or further
//! apart to lower it. A grain keeps the shape of the voice's spectrum, so
//! the formants stay where they were and a voice moved up a few semitones
//! is still the same singer, not a chipmunk. Laid down at a stretched
//! spacing, the same grains make a note longer with its pitch kept. For one
//! voice at a time it is cleaner than the phase vocoder the clip stretch
//! uses: consonants are not smeared and there is no phasey wash on a held
//! vowel. It is cheap, and what is not moved comes out as it went in,
//! sample for sample, because the grains of an unmoved stretch add back up
//! to the take exactly.
//!
//! Formants can be let go per note: each grain is then squeezed by the
//! same ratio as the pitch, which moves the voice's character with it.

use crate::audio_to_midi::{self, PitchTrack, Settings};
use std::f64::consts::PI;

/// A note the take holds, as the note finder heard it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoundNote {
    /// Where it was sung, in samples of the take.
    pub start: usize,
    pub end: usize,
    /// Its pitch as a MIDI note with the cents in the fraction: 63.38 is
    /// D#4, 38 cents sharp. The average over the note.
    pub pitch: f64,
}

/// What the take holds: its pitch, frame by frame, the notes in it, and
/// where its periods are, worked out once and kept while notes are moved.
#[derive(Debug, Clone, Default)]
pub struct TakeAnalysis {
    pub sample_rate: f64,
    /// The take's length in samples.
    pub len: usize,
    /// Frame i is centred on sample `i * hop + window / 2`.
    pub hop: usize,
    pub window: usize,
    /// The sung pitch in each frame, as a MIDI note with the cents in the
    /// fraction, smoothed over a few frames; NaN where nothing is sung.
    pub pitch: Vec<f32>,
    /// The notes, as `audio_to_midi::transcribe` (and so Tune this take)
    /// finds them.
    pub notes: Vec<FoundNote>,
    /// Grain centres covering the whole take, in order: one per period
    /// where it is sung, a few milliseconds apart where it is not.
    marks: Vec<Mark>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Mark {
    at: usize,
    voiced: bool,
}

/// A note as it is to play: where it was sung and where it goes now.
/// Positions are in samples of the take; pitches are MIDI notes with
/// cents in the fraction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteMove {
    pub start: f64,
    pub end: f64,
    /// Its pitch as sung, and where it plays now.
    pub sung: f64,
    pub pitch: f64,
    /// How much of the wobble around its pitch is kept: 1 as sung, 0 flat.
    pub drift: f64,
    /// Where it now starts and ends.
    pub new_start: f64,
    pub new_end: f64,
    /// Keep the voice's character as the pitch moves.
    pub formants: bool,
}

impl NoteMove {
    fn moves_pitch(&self) -> bool {
        (self.pitch - self.sung).abs() > 0.005 || self.drift < 0.999
    }

    fn moves_time(&self) -> bool {
        (self.new_start - self.start).abs() > 0.5 || (self.new_end - self.end).abs() > 0.5
    }

    /// Whether playing it differs from the take at all.
    pub fn changes_sound(&self) -> bool {
        self.moves_pitch() || self.moves_time()
    }
}

/// How far the wobble is followed when it is flattened: further than
/// this from the note is a slide from another note or a tracking slip,
/// and is left alone rather than turned into a jump.
const DRIFT_REACH: f64 = 2.0;
/// Grain spacing where nothing is sung, in seconds.
const UNVOICED_STEP: f64 = 0.005;
/// How far around a moved note the take is worked on, in seconds, and
/// how long the hand-over to the untouched take is.
const PAD: f64 = 0.04;
const CROSSFADE: f64 = 0.01;
/// How long the level match ramps in and out at a note's edges.
const GAIN_RAMP: f64 = 0.01;

/// Find the notes in a take and everything moving them needs. Mono: a
/// stereo take is mixed to one channel first, so both sides get the same
/// moves rather than two slightly different performances.
pub fn analyse(mono: &[f32], sample_rate: f64, progress: &mut dyn FnMut(f32)) -> TakeAnalysis {
    let settings = Settings {
        sample_rate,
        ..Default::default()
    };
    // Following the pitch is nearly all of the work.
    let track = audio_to_midi::pitch_track(mono, &settings, &mut |p| progress(p * 0.9));
    let notes = audio_to_midi::notes_in_track(&track, &settings)
        .into_iter()
        .filter_map(|n| {
            // The finder counts a note from the start of its first
            // window to the end of its last; what was sung is the middle
            // of those windows, half a step either side.
            let start = n.start_sample as usize + track.window / 2 - track.hop / 2;
            let end =
                (n.start_sample + n.length_samples) as usize + track.hop / 2 - track.window / 2;
            let end = end.min(mono.len());
            (start < end).then(|| FoundNote {
                start,
                end,
                pitch: n.pitch as f64 + n.cents_off as f64 / 100.0,
            })
        })
        .collect();
    let pitch = smoothed_pitch(&track);
    let marks = place_marks(mono, sample_rate, &track, &pitch);
    progress(1.0);
    TakeAnalysis {
        sample_rate,
        len: mono.len(),
        hop: track.hop,
        window: track.window,
        pitch,
        notes,
        marks,
    }
}

/// Each frame's pitch as a MIDI note, with single-frame slips (an octave
/// jump for one window is the tracker, not the singer) taken out by a
/// median over five frames.
fn smoothed_pitch(track: &PitchTrack) -> Vec<f32> {
    let raw: Vec<f64> = track
        .hz
        .iter()
        .map(|&hz| {
            if hz > 0.0 {
                69.0 + 12.0 * (hz / 440.0).log2()
            } else {
                f64::NAN
            }
        })
        .collect();
    let mut near = Vec::with_capacity(5);
    (0..raw.len())
        .map(|i| {
            if raw[i].is_nan() {
                return f32::NAN;
            }
            near.clear();
            near.extend(
                raw[i.saturating_sub(2)..(i + 3).min(raw.len())]
                    .iter()
                    .copied()
                    .filter(|v| !v.is_nan()),
            );
            near.sort_by(|a, b| a.total_cmp(b));
            near[near.len() / 2] as f32
        })
        .collect()
}

impl TakeAnalysis {
    /// The frame a sample falls in, fractionally: frame i is centred on
    /// `i * hop + window / 2`.
    fn frame_at(&self, sample: f64) -> f64 {
        (sample - (self.window / 2) as f64) / self.hop.max(1) as f64
    }

    /// The sung pitch at a sample, between frames, or NaN where nothing
    /// is sung.
    pub fn pitch_at(&self, sample: f64) -> f64 {
        let f = self.frame_at(sample);
        if self.pitch.is_empty() || f < -0.5 || f > self.pitch.len() as f64 - 0.5 {
            return f64::NAN;
        }
        let i0 = f.floor().max(0.0) as usize;
        let i1 = (i0 + 1).min(self.pitch.len() - 1);
        let (a, b) = (self.pitch[i0] as f64, self.pitch[i1] as f64);
        let frac = (f - i0 as f64).clamp(0.0, 1.0);
        match (a.is_nan(), b.is_nan()) {
            (false, false) => a + (b - a) * frac,
            (false, true) if frac < 0.5 => a,
            (true, false) if frac >= 0.5 => b,
            _ => f64::NAN,
        }
    }

    /// The average sung pitch between two samples, or None when nothing
    /// was sung there. What a note split in two is each side.
    pub fn average_pitch(&self, start: f64, end: f64) -> Option<f64> {
        let first = self.frame_at(start).ceil().max(0.0) as usize;
        let last = (self.frame_at(end).floor().max(-1.0) + 1.0) as usize;
        let (sum, count) = self
            .pitch
            .get(first..last.min(self.pitch.len()).max(first))
            .unwrap_or(&[])
            .iter()
            .filter(|p| !p.is_nan())
            .fold((0.0f64, 0usize), |(s, c), p| (s + *p as f64, c + 1));
        (count > 0).then(|| sum / count as f64)
    }

    /// The local period of the voice at a sample, in samples.
    fn period_at(&self, sample: f64) -> f64 {
        let midi = self.pitch_at(sample);
        let hz = if midi.is_nan() {
            200.0
        } else {
            440.0 * 2f64.powf((midi - 69.0) / 12.0)
        };
        (self.sample_rate / hz.clamp(50.0, 1500.0)).max(2.0)
    }
}

/// The key the notes sit in, by how long each pitch class is held against
/// the Krumhansl-Kessler key profiles: (root as a pitch class, minor).
/// None for fewer than three notes, where any answer is a guess.
pub fn guess_key(notes: &[FoundNote]) -> Option<(u8, bool)> {
    const MAJOR: [f64; 12] = [
        6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
    ];
    const MINOR: [f64; 12] = [
        6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
    ];
    if notes.len() < 3 {
        return None;
    }
    let mut held = [0.0f64; 12];
    for n in notes {
        let class = (n.pitch.round() as i64).rem_euclid(12) as usize;
        held[class] += (n.end - n.start) as f64;
    }
    let correlation = |profile: &[f64; 12], root: usize| {
        let mean_h = held.iter().sum::<f64>() / 12.0;
        let mean_p = profile.iter().sum::<f64>() / 12.0;
        let (mut num, mut dh, mut dp) = (0.0, 0.0, 0.0);
        for i in 0..12 {
            let h = held[(root + i) % 12] - mean_h;
            let p = profile[i] - mean_p;
            num += h * p;
            dh += h * h;
            dp += p * p;
        }
        if dh <= 0.0 || dp <= 0.0 {
            0.0
        } else {
            num / (dh * dp).sqrt()
        }
    };
    let mut best = (0u8, false, f64::MIN);
    for root in 0..12 {
        for (minor, profile) in [(false, &MAJOR), (true, &MINOR)] {
            let r = correlation(profile, root);
            if r > best.2 {
                best = (root as u8, minor, r);
            }
        }
    }
    Some((best.0, best.1))
}

/// Grain centres over the whole take. Where it is sung, one per period,
/// each found where the wave looks most like it did a period before, so
/// the grains line up with the voice; where it is not, evenly a few
/// milliseconds apart. The first is on the first sample and the last on
/// the last, so the grains cover everything.
fn place_marks(mono: &[f32], sample_rate: f64, track: &PitchTrack, pitch: &[f32]) -> Vec<Mark> {
    let len = mono.len();
    if len == 0 {
        return Vec::new();
    }
    let take = TakeAnalysis {
        sample_rate,
        len,
        hop: track.hop,
        window: track.window,
        pitch: pitch.to_vec(),
        notes: Vec::new(),
        marks: Vec::new(),
    };
    // The sung stretches: runs of frames with a pitch.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < pitch.len() {
        if pitch[i].is_nan() {
            i += 1;
            continue;
        }
        let first = i;
        while i < pitch.len() && !pitch[i].is_nan() {
            i += 1;
        }
        let centre = |f: usize| f * track.hop + track.window / 2;
        let from = centre(first).saturating_sub(track.hop / 2);
        let to = (centre(i - 1) + track.hop / 2).min(len);
        if to > from {
            runs.push((from, to));
        }
    }

    let mut voiced: Vec<Vec<usize>> = Vec::new();
    for &(from, to) in &runs {
        let mut marks = Vec::new();
        let period = take.period_at(from as f64) as usize;
        if from + 2 * period >= to {
            voiced.push(marks);
            continue;
        }
        // The first one on the loudest sample of the first period that
        // is really sung: a run starts half a window early, often in the
        // silence before the note.
        let loudest = mono[from..to].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let begins = (from..to)
            .find(|&i| mono[i].abs() >= loudest * 0.3)
            .unwrap_or(from);
        if begins + 2 * period >= to {
            voiced.push(marks);
            continue;
        }
        let mut at = (begins..begins + period)
            .max_by(|a, b| mono[*a].abs().total_cmp(&mono[*b].abs()))
            .unwrap_or(begins);
        marks.push(at);
        loop {
            let period = take.period_at(at as f64);
            let predicted = at as f64 + period;
            let reach = (period / 8.0).max(1.0) as i64;
            let half = (period / 2.0) as i64;
            if predicted as usize + half as usize >= to {
                break;
            }
            // Normalised by the candidate's own level: plain correlation
            // leans towards the louder side, which pulls the marks early
            // where a note dies away and late where it swells.
            let mut best = (predicted as i64, f64::MIN, 0.0f64);
            let mut reference = 0.0f64;
            for shift in -reach..=reach {
                let candidate = predicted as i64 + shift;
                let (mut sum, mut level, mut own) = (0.0f64, 1e-12f64, 0.0f64);
                for k in (-half..half).step_by(2) {
                    let (a, b) = (at as i64 + k, candidate + k);
                    if a < 0 || b < 0 || b as usize >= len {
                        continue;
                    }
                    let (x, y) = (mono[a as usize] as f64, mono[b as usize] as f64);
                    sum += x * y;
                    level += y * y;
                    own += x * x;
                }
                reference = own;
                let score = sum / level.sqrt();
                if score > best.1 {
                    best = (candidate, score, level);
                }
            }
            // Where the note has died away there is nothing to line up
            // with: the rest of the run is not sung.
            if best.2 < reference * 0.01 {
                break;
            }
            let next = best.0.max(at as i64 + 1) as usize;
            if next >= to {
                break;
            }
            marks.push(next);
            at = next;
        }
        voiced.push(marks);
    }

    // Everything between the sung stretches, filled evenly.
    let step = (UNVOICED_STEP * sample_rate).max(16.0);
    let mut out: Vec<Mark> = Vec::new();
    let fill = |out: &mut Vec<Mark>, from: usize, to: usize| {
        // From `from` (already placed, or the start) up to before `to`.
        let gap = to.saturating_sub(from) as f64;
        let count = (gap / step).round().max(1.0) as usize;
        for j in 1..count {
            out.push(Mark {
                at: from + (gap * j as f64 / count as f64) as usize,
                voiced: false,
            });
        }
    };
    out.push(Mark {
        at: 0,
        voiced: false,
    });
    for marks in voiced.iter().filter(|m| !m.is_empty()) {
        let last = out.last().map(|m| m.at).unwrap_or(0);
        let first = marks[0];
        if first <= last {
            continue;
        }
        fill(&mut out, last, first);
        out.extend(marks.iter().map(|&at| Mark { at, voiced: true }));
    }
    let last = out.last().map(|m| m.at).unwrap_or(0);
    if len - 1 > last {
        fill(&mut out, last, len - 1);
        out.push(Mark {
            at: len - 1,
            voiced: false,
        });
    }
    out
}

/// Play the take with its notes moved. Every channel gets the same moves
/// (`take` comes from their mix), and the result is as long as the take:
/// a note made longer takes its time from the gap around it.
pub fn render(channels: &[Vec<f32>], take: &TakeAnalysis, moves: &[NoteMove]) -> Vec<Vec<f32>> {
    let mut out: Vec<Vec<f32>> = channels.to_vec();
    let len = channels.iter().map(|c| c.len()).min().unwrap_or(0);
    if len == 0 || take.marks.len() < 2 || !moves.iter().any(|m| m.changes_sound()) {
        return out;
    }
    let plan = Plan::new(take, moves, len);
    if plan.zones.is_empty() {
        return out;
    }

    // The level of each moved note matched to how it was sung, measured
    // on the mix so both sides get the same gain.
    let mid: Vec<f32> = (0..len)
        .map(|i| channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32)
        .collect();
    let gains: Vec<Vec<f32>> = plan
        .zones
        .iter()
        .map(|zone| plan.level_match(zone, &mid, &plan.synthesise(zone, &mid)))
        .collect();

    let fade = (CROSSFADE * take.sample_rate).max(1.0);
    for (channel, target) in channels.iter().zip(out.iter_mut()) {
        for (zone, gain) in plan.zones.iter().zip(&gains) {
            let made = plan.synthesise(zone, channel);
            for (k, value) in made.iter().enumerate() {
                let o = zone.from + k;
                // Hand over from the take and back at the zone's edges,
                // where nothing is moved, so the seam cannot click.
                let edge = (k as f64).min((made.len() - 1 - k) as f64);
                let mix = (edge / fade).min(1.0) as f32;
                target[o] = channel[o] * (1.0 - mix) + value * gain[k] * mix;
            }
        }
    }
    out
}

/// One grain to lay down: taken around `src` in the take, centred on
/// `out` in the result, `left` and `right` samples of the take either
/// side, read `rate` samples of the take per sample of the result (1 keeps
/// the formants).
#[derive(Debug, Clone, Copy)]
struct Grain {
    out: f64,
    src: f64,
    left: f64,
    right: f64,
    rate: f64,
}

/// A stretch of the result that is worked on; the rest is the take.
#[derive(Debug, Clone)]
struct Zone {
    from: usize,
    to: usize,
    grains: Vec<Grain>,
}

/// The moves, worked out once for every channel: where the result reads
/// the take from, how far each part is shifted, and the grains.
struct Plan<'a> {
    take: &'a TakeAnalysis,
    moves: Vec<NoteMove>,
    /// Result time to take time: pairs (result, take), both rising.
    anchors: Vec<(f64, f64)>,
    /// The shift in cents at each frame's centre, in take time.
    shift: Vec<f64>,
    zones: Vec<Zone>,
}

impl<'a> Plan<'a> {
    fn new(take: &'a TakeAnalysis, moves: &[NoteMove], len: usize) -> Self {
        let n = len as f64;
        let mut moves: Vec<NoteMove> = moves
            .iter()
            .copied()
            .filter(|m| m.end > m.start && m.start >= 0.0 && m.end <= n)
            .collect();
        moves.sort_by(|a, b| a.start.total_cmp(&b.start));

        // The time map: each note's edges pin where it was sung to where
        // it is now. A move that would run the map backwards is played in
        // its own time instead.
        let mut anchors = vec![(0.0, 0.0)];
        for m in moves.iter_mut() {
            let (t0, s0) = *anchors.last().unwrap();
            let fits = m.new_start >= t0
                && m.new_end > m.new_start
                && m.start >= s0
                && m.new_end <= n
                && m.new_start >= 0.0;
            if !fits {
                m.new_start = m.start;
                m.new_end = m.end;
            }
            if m.new_start < t0 || m.start < s0 {
                // Overlapping notes: the second keeps its time.
                continue;
            }
            anchors.push((m.new_start, m.start));
            anchors.push((m.new_end, m.end));
        }
        anchors.push((n, n));
        anchors.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);

        // The shift at each frame: the note's move, less the wobble that
        // is flattened.
        let mut shift = vec![0.0f64; take.pitch.len()];
        for m in moves.iter().filter(|m| m.moves_pitch()) {
            let base = (m.pitch - m.sung) * 100.0;
            let first = take.frame_at(m.start).ceil().max(0.0) as usize;
            let last = (take.frame_at(m.end).ceil().max(0.0) as usize).min(shift.len());
            for f in first..last {
                let deviation = |f: usize| {
                    let p = take.pitch.get(f).copied().unwrap_or(f32::NAN) as f64;
                    if p.is_nan() || (p - m.sung).abs() > DRIFT_REACH {
                        0.0
                    } else {
                        (p - m.sung) * 100.0
                    }
                };
                // Averaged over three frames: the tracker's own jitter is
                // not wobble to take out, and would come back as roughness.
                let near = [f.saturating_sub(1), f, (f + 1).min(shift.len() - 1)];
                let wobble = near.iter().map(|&f| deviation(f)).sum::<f64>() / 3.0;
                shift[f] = base - (1.0 - m.drift.clamp(0.0, 1.0)) * wobble;
            }
        }

        let mut plan = Plan {
            take,
            moves,
            anchors,
            shift,
            zones: Vec::new(),
        };
        plan.zones = plan.find_zones(len);
        plan
    }

    /// Where in the take a sample of the result comes from.
    fn source(&self, t: f64) -> f64 {
        let a = &self.anchors;
        let j = a
            .partition_point(|p| p.0 <= t)
            .saturating_sub(1)
            .min(a.len() - 2);
        let ((t0, s0), (t1, s1)) = (a[j], a[j + 1]);
        if t1 - t0 < 1e-9 {
            s1
        } else {
            s0 + (t - t0) * (s1 - s0) / (t1 - t0)
        }
    }

    /// Whether the result plays the take in its own time around `t`.
    fn in_own_time(&self, t: f64) -> bool {
        let a = &self.anchors;
        let j = a
            .partition_point(|p| p.0 <= t)
            .saturating_sub(1)
            .min(a.len() - 2);
        a[j].0 == a[j].1 && a[j + 1].0 == a[j + 1].1
    }

    /// The shift in cents at a sample of the take.
    fn cents_at(&self, s: f64) -> f64 {
        if self.shift.is_empty() {
            return 0.0;
        }
        let f = self
            .take
            .frame_at(s)
            .clamp(0.0, (self.shift.len() - 1) as f64);
        let i0 = f.floor() as usize;
        let i1 = (i0 + 1).min(self.shift.len() - 1);
        self.shift[i0] + (self.shift[i1] - self.shift[i0]) * (f - i0 as f64)
    }

    /// Whether the note sung at a sample of the take keeps its formants.
    fn formants_at(&self, s: f64) -> bool {
        let i = self.moves.partition_point(|m| m.start <= s);
        i == 0 || self.moves[i - 1].end <= s || self.moves[i - 1].formants
    }

    /// The stretches of the result that differ from the take, padded so
    /// the hand-over happens where nothing moves.
    fn find_zones(&self, len: usize) -> Vec<Zone> {
        let pad = PAD * self.take.sample_rate;
        let mut spans: Vec<(f64, f64)> = Vec::new();
        for (k, m) in self.moves.iter().enumerate() {
            if !m.changes_sound() {
                continue;
            }
            let mut lo = m.start.min(m.new_start);
            let mut hi = m.end.max(m.new_end);
            if m.moves_time() {
                // The gaps either side give or take the time.
                lo = lo.min(if k > 0 {
                    self.moves[k - 1].new_end
                } else {
                    0.0
                });
                hi = hi.max(self.moves.get(k + 1).map_or(len as f64, |n| n.new_start));
            }
            spans.push(((lo - pad).max(0.0), (hi + pad).min(len as f64)));
        }
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for (lo, hi) in spans {
            match merged.last_mut() {
                Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
                _ => merged.push((lo, hi)),
            }
        }
        merged
            .into_iter()
            .map(|(lo, hi)| {
                let (from, to) = (lo.floor() as usize, (hi.ceil() as usize).min(len));
                Zone {
                    from,
                    to,
                    grains: self.grains(from, to),
                }
            })
            .filter(|z| z.to > z.from)
            .collect()
    }

    /// The grains for one zone of the result.
    fn grains(&self, from: usize, to: usize) -> Vec<Grain> {
        let marks = &self.take.marks;
        let gap = |i: usize, j: usize| (marks[j].at as f64 - marks[i].at as f64).abs().max(1.0);
        // The zone starts where the result is the take, so its first grain
        // sits on a mark of the take.
        let first = marks.partition_point(|m| m.at <= from).saturating_sub(1);
        let mut t = marks[first].at as f64;
        let mut grains = Vec::new();
        let end = to as f64 + gap(first, (first + 1).min(marks.len() - 1)) * 4.0;
        while t < end {
            let s = self.source(t);
            let i = nearest(marks, s);
            let left = if i > 0 { gap(i - 1, i) } else { gap(i, i + 1) };
            let right = if i + 1 < marks.len() {
                gap(i, i + 1)
            } else {
                left
            };
            let mark = marks[i];
            if mark.voiced {
                let ratio = 2f64.powf(self.cents_at(s) / 1200.0);
                let rate = if self.formants_at(s) { 1.0 } else { ratio };
                grains.push(Grain {
                    out: t,
                    src: mark.at as f64,
                    left,
                    right,
                    rate,
                });
                let mut next = t + right / ratio;
                if ratio == 1.0 && self.in_own_time(t) && i + 1 < marks.len() {
                    // Back in the take's own time after a moved note, the
                    // grains drift back onto the take's periods a little
                    // each period instead of jumping there.
                    next += (marks[i + 1].at as f64 - next) * 0.3;
                }
                t = next.max(t + 1.0);
            } else {
                // Breath and consonants are not pitched: they keep their
                // pitch, are read where the time map says, and only
                // change length.
                grains.push(Grain {
                    out: t,
                    src: s,
                    left,
                    right,
                    rate: 1.0,
                });
                t += right;
            }
        }
        grains
    }

    /// One channel of a zone, its grains laid down.
    fn synthesise(&self, zone: &Zone, channel: &[f32]) -> Vec<f32> {
        let size = zone.to - zone.from;
        let mut sum = vec![0.0f64; size];
        let mut weight = vec![0.0f64; size];
        for g in &zone.grains {
            let first = (g.out - g.left / g.rate).ceil().max(zone.from as f64) as usize;
            let last = (g.out + g.right / g.rate).floor().min(zone.to as f64 - 1.0);
            if last < first as f64 {
                continue;
            }
            for o in first..=last as usize {
                let tau = (o as f64 - g.out) * g.rate;
                let w = if tau < 0.0 {
                    0.5 * (1.0 + (PI * tau / g.left).cos())
                } else {
                    0.5 * (1.0 + (PI * tau / g.right).cos())
                };
                sum[o - zone.from] += w * sample_at(channel, g.src + tau);
                weight[o - zone.from] += w;
            }
        }
        // Grains laid closer than they were overlap more than they did, and
        // are brought back down; laid further apart they leave the quiet
        // between a voice's pulses, which is how a lower voice sounds.
        sum.iter()
            .zip(&weight)
            .map(|(s, w)| (s / w.max(1.0)) as f32)
            .collect()
    }

    /// A gain per sample of the zone that brings each moved note back to
    /// the level it was sung at.
    fn level_match(&self, zone: &Zone, take: &[f32], made: &[f32]) -> Vec<f32> {
        let mut gain = vec![1.0f32; made.len()];
        let rms = |v: &[f32]| {
            if v.is_empty() {
                return 0.0;
            }
            (v.iter().map(|s| *s as f64 * *s as f64).sum::<f64>() / v.len() as f64).sqrt()
        };
        for m in self.moves.iter().filter(|m| m.changes_sound()) {
            let (a, b) = (
                m.new_start.max(zone.from as f64),
                m.new_end.min(zone.to as f64),
            );
            if b <= a {
                continue;
            }
            let (a, b) = (a as usize - zone.from, b as usize - zone.from);
            let sung = rms(&take[m.start as usize..(m.end as usize).min(take.len())]);
            let now = rms(&made[a..b]);
            if sung < 1e-6 || now < 1e-6 {
                continue;
            }
            let g = (sung / now).clamp(0.5, 2.0) as f32;
            gain[a..b].iter_mut().for_each(|v| *v = g);
        }
        // Smoothed so a note's level eases in and out.
        let width = (GAIN_RAMP * self.take.sample_rate) as usize;
        if width > 1 && gain.len() > width {
            let mut out = gain.clone();
            let mut running: f64 = gain[..width].iter().map(|v| *v as f64).sum();
            for (i, slot) in out
                .iter_mut()
                .enumerate()
                .skip(width / 2)
                .take(gain.len() - width)
            {
                *slot = (running / width as f64) as f32;
                let leaving = i - width / 2;
                running += gain[leaving + width] as f64 - gain[leaving] as f64;
            }
            gain = out;
        }
        gain
    }
}

/// The mark nearest a sample of the take.
fn nearest(marks: &[Mark], s: f64) -> usize {
    let i = marks.partition_point(|m| (m.at as f64) < s);
    if i == 0 {
        0
    } else if i >= marks.len() {
        marks.len() - 1
    } else if s - marks[i - 1].at as f64 <= marks[i].at as f64 - s {
        i - 1
    } else {
        i
    }
}

/// A sample between samples, by straight line; silence outside the take.
fn sample_at(channel: &[f32], at: f64) -> f64 {
    if at < 0.0 {
        return 0.0;
    }
    let i = at.floor() as usize;
    let frac = at - i as f64;
    let a = channel.get(i).copied().unwrap_or(0.0) as f64;
    if frac == 0.0 {
        return a;
    }
    let b = channel.get(i + 1).copied().unwrap_or(0.0) as f64;
    a + (b - a) * frac
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustfft::{num_complex::Complex32, FftPlanner};

    const SR: f64 = 48_000.0;

    fn seconds(s: f64) -> usize {
        (s * SR) as usize
    }

    /// A sung-like note: a few harmonics, eased in and out like a voice.
    fn tone(hz: f64, length: f64) -> Vec<f32> {
        wobbly(hz, length, 0.0, 0.0)
    }

    /// The same with vibrato: `depth` cents either way, `rate` times a second.
    fn wobbly(hz: f64, length: f64, depth: f64, rate: f64) -> Vec<f32> {
        let n = seconds(length);
        let ease = seconds(0.01) as f64;
        let mut phase = 0.0f64;
        (0..n)
            .map(|i| {
                let t = i as f64 / SR;
                let f = hz * 2f64.powf(depth * (std::f64::consts::TAU * rate * t).sin() / 1200.0);
                phase += std::f64::consts::TAU * f / SR;
                let edge = (i as f64 / ease).min((n - i) as f64 / ease).min(1.0);
                ((phase.sin() * 0.6 + (2.0 * phase).sin() * 0.25 + (3.0 * phase).sin() * 0.12)
                    * edge
                    * 0.8) as f32
            })
            .collect()
    }

    fn silence(length: f64) -> Vec<f32> {
        vec![0.0; seconds(length)]
    }

    fn join(parts: &[Vec<f32>]) -> Vec<f32> {
        parts.iter().flatten().copied().collect()
    }

    /// The pitch of a stretch, the finder's way, averaged over windows.
    fn measured_hz(audio: &[f32], from: f64, to: f64) -> f64 {
        let settings = Settings {
            sample_rate: SR,
            ..Default::default()
        };
        let mut tracker = audio_to_midi::PitchTracker::new(2048);
        let (mut sum, mut count) = (0.0, 0);
        let mut at = seconds(from);
        while at + 2048 <= seconds(to) {
            if let Some(hz) = tracker.pitch(&audio[at..at + 2048], &settings) {
                sum += hz;
                count += 1;
            }
            at += 512;
        }
        assert!(count > 0, "no pitch between {from} and {to}");
        sum / count as f64
    }

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    fn rms(v: &[f32]) -> f64 {
        (v.iter().map(|s| *s as f64 * *s as f64).sum::<f64>() / v.len().max(1) as f64).sqrt()
    }

    /// Every note as found, nothing moved yet.
    fn as_found(take: &TakeAnalysis) -> Vec<NoteMove> {
        take.notes
            .iter()
            .map(|n| NoteMove {
                start: n.start as f64,
                end: n.end as f64,
                sung: n.pitch,
                pitch: n.pitch,
                drift: 1.0,
                new_start: n.start as f64,
                new_end: n.end as f64,
                formants: true,
            })
            .collect()
    }

    #[test]
    fn the_notes_of_a_take_are_found_where_they_were_sung() {
        let sharp_c = 261.626 * 2f64.powf(30.0 / 1200.0);
        let audio = join(&[
            tone(220.0, 0.4),
            silence(0.2),
            tone(sharp_c, 0.5),
            silence(0.2),
            tone(329.63, 0.4),
            silence(0.1),
        ]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 3, "{:?}", take.notes);
        let want = [(57.0, 0.0, 0.4), (60.3, 0.6, 1.1), (64.0, 1.3, 1.7)];
        for (note, (pitch, start, end)) in take.notes.iter().zip(want) {
            assert!(
                (note.pitch - pitch).abs() < 0.1,
                "{note:?} should sit at {pitch}"
            );
            let (s, e) = (note.start as f64 / SR, note.end as f64 / SR);
            assert!(
                (s - start).abs() < 0.03,
                "{note:?} should start at {start}s, not {s}"
            );
            assert!(
                (e - end).abs() < 0.03,
                "{note:?} should end at {end}s, not {e}"
            );
        }
    }

    #[test]
    fn the_line_through_a_note_follows_its_wobble() {
        let audio = join(&[silence(0.1), wobbly(220.0, 1.0, 40.0, 5.0), silence(0.1)]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 1, "{:?}", take.notes);
        assert!((take.notes[0].pitch - 57.0).abs() < 0.1, "{:?}", take.notes);
        let line: Vec<f64> = (seconds(0.3)..seconds(0.9))
            .step_by(240)
            .map(|s| take.pitch_at(s as f64))
            .collect();
        let (low, high) = line
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(*p), h.max(*p)));
        assert!(
            (high - low) * 100.0 > 50.0,
            "the vibrato should show in the line: {low}..{high}"
        );
        // Where nothing is sung there is no line.
        assert!(take.pitch_at(seconds(0.02) as f64).is_nan());
    }

    #[test]
    fn the_key_of_a_line_is_guessed_from_its_notes() {
        let note = |pitch: f64, length: usize| FoundNote {
            start: 0,
            end: length,
            pitch,
        };
        // C major, C and G held longest.
        let c_major = [
            note(60.0, 4),
            note(62.0, 1),
            note(64.0, 2),
            note(65.0, 1),
            note(67.0, 3),
            note(69.0, 1),
            note(71.0, 1),
            note(72.0, 4),
        ];
        assert_eq!(guess_key(&c_major), Some((0, false)));
        // F minor, as the mockup's line: F, Ab and C held.
        let f_minor = [
            note(65.0, 4),
            note(67.0, 1),
            note(68.0, 3),
            note(70.0, 1),
            note(72.0, 3),
            note(73.0, 1),
            note(63.0, 1),
            note(65.08, 2),
        ];
        assert_eq!(guess_key(&f_minor), Some((5, true)));
        assert_eq!(guess_key(&c_major[..2]), None, "two notes is a guess");
    }

    #[test]
    fn a_voice_is_marked_once_a_period() {
        let audio = join(&[silence(0.1), tone(200.0, 0.5), silence(0.1)]);
        let take = analyse(&audio, SR, &mut |_| {});
        let voiced: Vec<usize> = take
            .marks
            .iter()
            .filter(|m| m.voiced)
            .map(|m| m.at)
            .collect();
        assert!(voiced.len() > 80, "{} marks for 100 periods", voiced.len());
        let gaps: Vec<usize> = voiced.windows(2).map(|p| p[1] - p[0]).collect();
        assert!(
            gaps.iter().all(|&g| (g as f64 - 240.0).abs() < 6.0),
            "a period of 200 Hz is 240 samples: {gaps:?}"
        );
        // The marks cover the take from its first sample to its last.
        assert_eq!(take.marks.first().map(|m| m.at), Some(0));
        assert_eq!(take.marks.last().map(|m| m.at), Some(audio.len() - 1));
    }

    #[test]
    fn grains_that_are_not_moved_add_back_up_to_the_take() {
        // Sung, then a breath of noise, then sung again: both kinds of
        // grain.
        let mut seed = 7u32;
        let noise: Vec<f32> = (0..seconds(0.15))
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                ((seed >> 16) % 1000) as f32 / 5000.0 - 0.1
            })
            .collect();
        let audio = join(&[tone(220.0, 0.4), noise, tone(330.0, 0.4)]);
        let take = analyse(&audio, SR, &mut |_| {});
        let mut moves = as_found(&take);
        moves[0].pitch += 1.0;
        let mut plan = Plan::new(&take, &moves, audio.len());
        plan.shift.iter_mut().for_each(|c| *c = 0.0);
        let (from, to) = (1000, audio.len() - 1000);
        let zone = Zone {
            from,
            to,
            grains: plan.grains(from, to),
        };
        let made = plan.synthesise(&zone, &audio);
        let worst = made
            .iter()
            .zip(&audio[from..to])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-5,
            "unmoved grains should give the take back: off by {worst}"
        );
    }

    #[test]
    fn a_note_moved_lands_on_its_new_pitch_and_keeps_its_place() {
        let audio = join(&[
            silence(0.2),
            tone(220.0, 0.6),
            silence(0.2),
            tone(220.0, 0.6),
            silence(0.2),
        ]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 2);
        for semitones in [2.0, -3.0, 0.5] {
            let mut moves = as_found(&take);
            moves[0].pitch += semitones;
            let out = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
            assert_eq!(out.len(), audio.len(), "the take keeps its length");
            let want = 220.0 * 2f64.powf(semitones / 12.0);
            let heard = measured_hz(&out, 0.3, 0.7);
            assert!(
                cents(heard, want).abs() < 5.0,
                "{semitones:+} semitones: heard {heard:.2} Hz, want {want:.2} Hz"
            );
            let level = 20.0
                * (rms(&out[seconds(0.3)..seconds(0.7)]) / rms(&audio[seconds(0.3)..seconds(0.7)]))
                    .log10();
            assert!(
                level.abs() < 1.0,
                "{semitones:+}: the level moved {level:.2} dB"
            );
            // The note that was not moved is the take, sample for sample.
            assert_eq!(
                &out[seconds(1.05)..seconds(1.55)],
                &audio[seconds(1.05)..seconds(1.55)]
            );
            assert!(cents(measured_hz(&out, 1.1, 1.5), 220.0).abs() < 3.0);
        }
    }

    /// A voice: pulses through a resonance, which is where its character
    /// (a formant) sits.
    fn voice(hz: f64, formant: f64, length: f64) -> Vec<f32> {
        let n = seconds(length);
        let period = SR / hz;
        let (r, theta) = (0.985f64, std::f64::consts::TAU * formant / SR);
        let (mut y1, mut y2) = (0.0f64, 0.0f64);
        let mut next = 0.0f64;
        let mut out: Vec<f32> = (0..n)
            .map(|i| {
                let x = if i as f64 >= next {
                    next += period;
                    1.0
                } else {
                    0.0
                };
                let y = x + 2.0 * r * theta.cos() * y1 - r * r * y2;
                y2 = y1;
                y1 = y;
                y as f32
            })
            .collect();
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        out.iter_mut().for_each(|s| *s *= 0.7 / peak);
        out
    }

    /// The loudest frequency between `low` and `high` Hz in a stretch.
    fn strongest(audio: &[f32], low: f64, high: f64) -> f64 {
        let n = 8192;
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);
        let mut buf: Vec<Complex32> = (0..n)
            .map(|i| {
                let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
                Complex32::new(audio[i] * w, 0.0)
            })
            .collect();
        fft.process(&mut buf);
        let bin = SR / n as f64;
        let (lo, hi) = ((low / bin) as usize, (high / bin) as usize);
        let best = (lo..hi)
            .max_by(|a, b| buf[*a].norm().total_cmp(&buf[*b].norm()))
            .unwrap();
        best as f64 * bin
    }

    #[test]
    fn formants_stay_where_they_were_unless_let_go() {
        let audio = join(&[silence(0.1), voice(150.0, 1000.0, 0.8), silence(0.1)]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 1, "{:?}", take.notes);
        assert!((strongest(&audio[seconds(0.3)..], 400.0, 2500.0) - 1000.0).abs() < 100.0);
        for (formants, near, far) in [(true, 1000.0, 1400.0), (false, 1400.0, 1000.0)] {
            let mut moves = as_found(&take);
            moves[0].pitch += 5.0;
            moves[0].formants = formants;
            let out = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
            let heard = measured_hz(&out, 0.3, 0.7);
            assert!(
                cents(heard, 150.0 * 2f64.powf(5.0 / 12.0)).abs() < 8.0,
                "formants kept {formants}: the pitch should be up 5 semitones, heard {heard:.1} Hz"
            );
            let peak = strongest(&out[seconds(0.3)..], 400.0, 2500.0);
            assert!(
                (peak - near).abs() < (peak - far).abs(),
                "formants kept {formants}: the voice's resonance is at {peak:.0} Hz"
            );
        }
    }

    #[test]
    fn drift_at_zero_flattens_the_wobble() {
        let audio = join(&[silence(0.1), wobbly(220.0, 1.2, 40.0, 5.0), silence(0.1)]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 1);
        let spread = |a: &[f32]| {
            let line = analyse(a, SR, &mut |_| {});
            let values: Vec<f64> = (seconds(0.4)..seconds(1.0))
                .step_by(240)
                .map(|s| line.pitch_at(s as f64))
                .filter(|p| !p.is_nan())
                .collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
        };
        let mut moves = as_found(&take);
        moves[0].drift = 0.0;
        let flat = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
        let (before, after) = (spread(&audio), spread(&flat));
        assert!(
            after < before * 0.4,
            "the wobble should be flattened: {:.0} cents before, {:.0} after",
            before * 100.0,
            after * 100.0
        );
        assert!(cents(measured_hz(&flat, 0.4, 1.0), 220.0).abs() < 10.0);
    }

    #[test]
    fn a_note_made_longer_lasts_longer_and_keeps_its_pitch() {
        let audio = join(&[silence(0.2), tone(220.0, 0.5), silence(0.5)]);
        let take = analyse(&audio, SR, &mut |_| {});
        assert_eq!(take.notes.len(), 1);
        let mut moves = as_found(&take);
        let end = moves[0].end;
        moves[0].new_end = end + 0.15 * SR;
        let out = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
        assert_eq!(out.len(), audio.len());
        let sung = rms(&audio[seconds(0.3)..seconds(0.6)]);
        let tail = rms(&out[(end + 0.02 * SR) as usize..(end + 0.12 * SR) as usize]);
        assert!(
            tail > sung * 0.5,
            "the note should go on: {tail} against {sung}"
        );
        assert!(rms(&audio[(end + 0.02 * SR) as usize..(end + 0.12 * SR) as usize]) < sung * 0.05);
        assert!(cents(measured_hz(&out, 0.3, end / SR + 0.1), 220.0).abs() < 5.0);

        // And one started earlier begins sooner.
        let mut moves = as_found(&take);
        let start = moves[0].start;
        moves[0].new_start = start - 0.1 * SR;
        let out = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
        let early = rms(&out[(start - 0.08 * SR) as usize..(start - 0.02 * SR) as usize]);
        assert!(early > sung * 0.5, "the note should start sooner: {early}");
    }

    #[test]
    fn both_sides_of_a_stereo_take_move_together() {
        let left = join(&[silence(0.1), tone(220.0, 0.6), silence(0.1)]);
        let right: Vec<f32> = left.iter().map(|s| s * 0.5).collect();
        let mid: Vec<f32> = left
            .iter()
            .zip(&right)
            .map(|(a, b)| (a + b) / 2.0)
            .collect();
        let take = analyse(&mid, SR, &mut |_| {});
        let mut moves = as_found(&take);
        moves[0].pitch += 3.0;
        let out = render(&[left, right], &take, &moves);
        let worst = out[0]
            .iter()
            .zip(&out[1])
            .map(|(l, r)| (l * 0.5 - r).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-5,
            "the right side should be the left at half: off by {worst}"
        );
    }

    #[test]
    fn a_wild_move_never_blows_up() {
        let mut seed = 99u32;
        let mut audio = join(&[
            tone(110.0, 0.5),
            tone(440.0, 0.3),
            silence(0.1),
            tone(660.0, 0.4),
        ]);
        for s in audio.iter_mut() {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            *s += ((seed >> 16) % 1000) as f32 / 20_000.0 - 0.025;
        }
        let take = analyse(&audio, SR, &mut |_| {});
        assert!(!take.notes.is_empty());
        let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        for (i, (shift, drift, stretch)) in [(12.0, 0.0, 1.8), (-12.0, 0.5, 0.6), (7.0, 1.0, 1.0)]
            .into_iter()
            .enumerate()
        {
            let mut moves = as_found(&take);
            for m in moves.iter_mut() {
                m.pitch += shift;
                m.drift = drift;
                m.formants = i % 2 == 0;
                let length = (m.end - m.start) * stretch;
                m.new_end = (m.start + length).min(audio.len() as f64);
            }
            // Keep them in order: a longer note ends before the next starts.
            for k in 0..moves.len().saturating_sub(1) {
                moves[k].new_end = moves[k].new_end.min(moves[k + 1].start);
            }
            let out = render(std::slice::from_ref(&audio), &take, &moves).remove(0);
            assert_eq!(out.len(), audio.len());
            assert!(out.iter().all(|s| s.is_finite()));
            let out_peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(
                out_peak < peak * 2.5,
                "{shift:+}: peak {out_peak} against {peak}"
            );
        }
    }

    #[test]
    fn half_a_note_is_its_own_average() {
        let audio = join(&[
            silence(0.1),
            tone(220.0, 0.5),
            tone(246.94, 0.5),
            silence(0.1),
        ]);
        let take = analyse(&audio, SR, &mut |_| {});
        let first = take
            .average_pitch(seconds(0.2) as f64, seconds(0.5) as f64)
            .unwrap();
        let second = take
            .average_pitch(seconds(0.7) as f64, seconds(1.0) as f64)
            .unwrap();
        assert!(
            (first - 57.0).abs() < 0.1 && (second - 59.0).abs() < 0.1,
            "{first} {second}"
        );
        assert_eq!(take.average_pitch(0.0, seconds(0.05) as f64), None);
    }
}
