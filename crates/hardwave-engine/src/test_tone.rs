//! The test tone: a soft sine on the left output, then on the right, played
//! straight to the sound card.
//!
//! Apply in Settings > Audio switched the device in silence, so someone with
//! an interface and laptop speakers could not tell which one was live until a
//! song played, and the usual first sign was a silent DAW. After Apply (and on
//! the Sound step of the first run) this tone plays on the device just opened
//! while the window names it. Left then right, so a swapped cable, a dead
//! monitor or the wrong output pair shows at once.
//!
//! It is mixed in after the master, the meters' tap and the capture path,
//! like a browser audition: a muted master or a plug-in on it cannot hide
//! it, and it never reaches a recording. An export builds an engine callback
//! of its own whose tone nobody can start, so it cannot play into a bounce.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

/// The tone's pitch: A4, soft rather than piercing.
pub const TONE_HZ: f64 = 440.0;
/// Its level. Loud enough to hear on any speaker, far from hurting one.
pub const TONE_DBFS: f32 = -18.0;
/// How long it sounds on each side.
pub const SIDE_SECONDS: f64 = 0.4;
/// The fade in and out on each side, so it starts and stops without a click.
pub const FADE_SECONDS: f64 = 0.015;

/// Which output the tone is on now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneSide {
    Idle,
    Left,
    Right,
}

/// What the UI thread asks for and reads back. A few atomics: the audio
/// thread never waits on the window.
#[derive(Clone)]
pub struct TestToneRequest {
    shared: Arc<Shared>,
}

struct Shared {
    /// Set to start the tone (again, from the left, if it is playing).
    start: AtomicBool,
    /// The side playing now, as [`ToneSide`] numbers (0 idle, 1 left, 2 right).
    side: AtomicU8,
    /// The tone's peak on each side in the last block, as f32 bits: what the
    /// meters beside the device's name show.
    peak_left: AtomicU32,
    peak_right: AtomicU32,
}

impl TestToneRequest {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                start: AtomicBool::new(false),
                side: AtomicU8::new(0),
                peak_left: AtomicU32::new(0),
                peak_right: AtomicU32::new(0),
            }),
        }
    }

    /// Play the tone from the start: left, then right.
    pub fn play(&self) {
        self.shared.start.store(true, Ordering::Release);
    }

    /// The side the tone is on now, as of the last block.
    pub fn side(&self) -> ToneSide {
        match self.shared.side.load(Ordering::Relaxed) {
            1 => ToneSide::Left,
            2 => ToneSide::Right,
            _ => ToneSide::Idle,
        }
    }

    /// Whether the tone is asked for or still sounding.
    pub fn is_playing(&self) -> bool {
        self.shared.start.load(Ordering::Relaxed) || self.side() != ToneSide::Idle
    }

    /// The tone's peak on the left and the right output in the last block.
    pub fn peaks(&self) -> (f32, f32) {
        (
            f32::from_bits(self.shared.peak_left.load(Ordering::Relaxed)),
            f32::from_bits(self.shared.peak_right.load(Ordering::Relaxed)),
        )
    }
}

impl Default for TestToneRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Plays the tone on the audio thread. Holds no buffer: each sample is
/// worked out as it is written, so nothing is allocated.
pub struct TestTone {
    request: TestToneRequest,
    /// Frames into the tone, or none while it is not playing.
    position: Option<u64>,
    phase: f64,
}

impl TestTone {
    pub fn new(request: TestToneRequest) -> Self {
        Self {
            request,
            position: None,
            phase: 0.0,
        }
    }

    /// A tone nobody holds the request for, so it never plays: what an
    /// export's engine callback gets.
    pub fn unwired() -> Self {
        Self::new(TestToneRequest::new())
    }

    /// Add the tone to an interleaved stereo block.
    pub fn render(&mut self, output: &mut [f32], num_frames: usize, sample_rate: f64) {
        let shared = &self.request.shared;
        if shared.start.swap(false, Ordering::Acquire) {
            self.position = Some(0);
            self.phase = 0.0;
        }
        let Some(mut position) = self.position else {
            return;
        };
        if sample_rate <= 0.0 {
            return;
        }
        let side_len = ((SIDE_SECONDS * sample_rate).round() as u64).max(1);
        let fade_len = ((FADE_SECONDS * sample_rate).round() as u64)
            .min(side_len / 2)
            .max(1);
        let amplitude = 10f32.powf(TONE_DBFS / 20.0);
        let step = std::f64::consts::TAU * TONE_HZ / sample_rate;
        let (mut peak_left, mut peak_right) = (0.0f32, 0.0f32);
        let frames = num_frames.min(output.len() / 2);
        for frame in 0..frames {
            if position >= side_len * 2 {
                self.position = None;
                break;
            }
            let right = position >= side_len;
            let gain = amplitude * envelope(position % side_len, side_len, fade_len);
            let sample = gain * self.phase.sin() as f32;
            if right {
                output[frame * 2 + 1] += sample;
                peak_right = peak_right.max(sample.abs());
            } else {
                output[frame * 2] += sample;
                peak_left = peak_left.max(sample.abs());
            }
            self.phase = (self.phase + step) % std::f64::consts::TAU;
            position += 1;
        }
        let side = match self.position {
            None => 0,
            Some(_) if position >= side_len => 2,
            Some(_) => 1,
        };
        if self.position.is_some() {
            self.position = Some(position);
        } else {
            // Done: the meters fall to nothing rather than keep the last
            // block's level, which nothing would ever replace.
            peak_left = 0.0;
            peak_right = 0.0;
        }
        shared.side.store(side, Ordering::Relaxed);
        shared
            .peak_left
            .store(peak_left.to_bits(), Ordering::Relaxed);
        shared
            .peak_right
            .store(peak_right.to_bits(), Ordering::Relaxed);
    }
}

/// The gain at `at` frames into one side: a raised-cosine fade in over the
/// first `fade` frames, and out over the last.
fn envelope(at: u64, len: u64, fade: u64) -> f32 {
    let from_end = len - 1 - at.min(len - 1);
    let edge = at.min(from_end);
    if edge >= fade {
        return 1.0;
    }
    let x = edge as f64 / fade as f64;
    (0.5 - 0.5 * (std::f64::consts::PI * x).cos()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = 48_000.0;

    /// Render the whole tone in blocks, returning every block.
    fn play_all(tone: &mut TestTone, block: usize, blocks: usize) -> Vec<f32> {
        let mut all = Vec::new();
        for _ in 0..blocks {
            let mut out = vec![0.0f32; block * 2];
            tone.render(&mut out, block, RATE);
            all.extend_from_slice(&out);
        }
        all
    }

    fn peak(samples: impl Iterator<Item = f32>) -> f32 {
        samples.fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn nothing_plays_until_it_is_asked_for() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        let out = play_all(&mut tone, 512, 4);
        assert_eq!(peak(out.into_iter()), 0.0);
        assert!(!request.is_playing());
    }

    #[test]
    fn it_plays_left_then_right_at_minus_18_dbfs() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        let side = (SIDE_SECONDS * RATE) as usize;
        // Enough blocks for both sides and a little silence after.
        let out = play_all(&mut tone, 480, (2 * side) / 480 + 4);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();

        let expected = 10f32.powf(-18.0 / 20.0);
        let first_half = peak(left[..side].iter().copied());
        assert!(
            (first_half - expected).abs() < 0.002,
            "left peaks at {first_half}, not -18 dBFS ({expected})"
        );
        assert_eq!(
            peak(right[..side].iter().copied()),
            0.0,
            "nothing on the right while the left plays"
        );
        let second_half = peak(right[side..2 * side].iter().copied());
        assert!(
            (second_half - expected).abs() < 0.002,
            "right peaks at {second_half}"
        );
        assert_eq!(
            peak(left[side..].iter().copied()),
            0.0,
            "nothing on the left while the right plays"
        );
        assert_eq!(
            peak(out[4 * side..].iter().copied()),
            0.0,
            "silence once both sides played"
        );
        assert_eq!(request.side(), ToneSide::Idle, "and it says it is done");
        assert_eq!(request.peaks(), (0.0, 0.0), "with its meters down");
    }

    #[test]
    fn each_side_starts_and_ends_without_a_click() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        let side = (SIDE_SECONDS * RATE) as usize;
        let out = play_all(&mut tone, 480, (2 * side) / 480 + 1);
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        // The first and last few samples of each side are near silence.
        for (name, edge) in [
            ("left start", &left[..8]),
            ("left end", &left[side - 8..side]),
            ("right start", &right[side..side + 8]),
            ("right end", &right[2 * side - 8..2 * side]),
        ] {
            assert!(peak(edge.iter().copied()) < 0.003, "{name} jumps: {edge:?}");
        }
    }

    #[test]
    fn the_ui_reads_the_side_and_the_level_while_it_plays() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        assert!(request.is_playing(), "asked for counts as playing");
        let mut out = vec![0.0f32; 4800 * 2];
        tone.render(&mut out, 4800, RATE);
        assert_eq!(request.side(), ToneSide::Left);
        let (l, r) = request.peaks();
        assert!(l > 0.1 && r == 0.0, "left meter only: {l} {r}");

        // Past the middle: the right side.
        for _ in 0..4 {
            tone.render(&mut out, 4800, RATE);
        }
        assert_eq!(request.side(), ToneSide::Right);
        let (l, r) = request.peaks();
        assert!(l == 0.0 && r > 0.1, "right meter only: {l} {r}");
    }

    /// A tone that ends inside a block leaves its meters at nothing, not at
    /// that block's level: nothing plays after it to replace the reading,
    /// so the window's meter stayed up for good.
    #[test]
    fn the_meters_fall_to_nothing_when_it_ends_inside_a_block() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        // 38 400 frames of tone in blocks of 500: it ends inside the 77th.
        play_all(&mut tone, 500, 77);
        assert_eq!(request.side(), ToneSide::Idle);
        assert_eq!(request.peaks(), (0.0, 0.0));
        play_all(&mut tone, 500, 3);
        assert_eq!(request.peaks(), (0.0, 0.0), "and they stay there");
    }

    #[test]
    fn play_again_starts_over_from_the_left() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        let mut out = vec![0.0f32; 24_000 * 2];
        tone.render(&mut out, 24_000, RATE);
        assert_eq!(request.side(), ToneSide::Right);
        request.play();
        let mut out = vec![0.0f32; 480 * 2];
        tone.render(&mut out, 480, RATE);
        assert_eq!(request.side(), ToneSide::Left);
        assert_eq!(
            peak(out.iter().skip(1).step_by(2).copied()),
            0.0,
            "one tone, not two"
        );
    }

    #[test]
    fn it_adds_to_what_is_already_in_the_block() {
        let request = TestToneRequest::new();
        let mut tone = TestTone::new(request.clone());
        request.play();
        let mut out = vec![0.25f32; 64 * 2];
        tone.render(&mut out, 64, RATE);
        assert_eq!(out[1], 0.25, "the right side is left as it was");
        assert!(out.iter().step_by(2).any(|s| (*s - 0.25).abs() > 1e-6));
    }

    #[test]
    fn an_unwired_tone_never_plays() {
        let mut tone = TestTone::unwired();
        let out = play_all(&mut tone, 512, 100);
        assert_eq!(peak(out.into_iter()), 0.0);
    }
}
