//! MIDI clock output: 24 clocks a beat, and Start, Continue, Stop and the
//! song position, to every open MIDI output port while clock send is on.
//!
//! The clock follows where the song is, not a stopwatch started on Play.
//! It used to send Start wherever playback began, so a drum machine
//! always began its pattern from the top while the song was in bar 17;
//! its clocks started on the press of Play rather than on the beat; and a
//! loop jumping back was never told. Now: Start from the top, the song
//! position (to the next sixteenth) and Continue from anywhere else, the
//! first clock exactly on that sixteenth, and Stop, the new position and
//! Continue when the playhead jumps (a loop wrapping, a click on the
//! ruler).

use hardwave_engine::DawEngine;
use hardwave_midi::{MidiOutputManager, MIDI_CLOCK_TICK, MIDI_CONTINUE, MIDI_START, MIDI_STOP};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct MidiClockState {
    pub enabled: Arc<AtomicBool>,
    pub output: Arc<Mutex<MidiOutputManager>>,
}

impl MidiClockState {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            output: Arc::new(Mutex::new(MidiOutputManager::new())),
        }
    }
}

impl Default for MidiClockState {
    fn default() -> Self {
        Self::new()
    }
}

/// MIDI clocks per beat (quarter note).
const CLOCKS_PER_BEAT: f64 = 24.0;
/// MIDI clocks per sixteenth, the unit of the song position.
const CLOCKS_PER_SIXTEENTH: i64 = 6;

/// What to send, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClockEvent {
    Start,
    /// Song position (in sixteenths) and Continue.
    ContinueAt(u16),
    Stop,
    Tick,
}

impl ClockEvent {
    fn send(self, out: &MidiOutputManager) {
        match self {
            ClockEvent::Start => out.broadcast(&[MIDI_START]),
            ClockEvent::ContinueAt(sixteenths) => {
                out.broadcast(&song_position(sixteenths));
                out.broadcast(&[MIDI_CONTINUE]);
            }
            ClockEvent::Stop => out.broadcast(&[MIDI_STOP]),
            ClockEvent::Tick => out.broadcast(&[MIDI_CLOCK_TICK]),
        }
    }
}

/// The Song Position Pointer: sixteenths since the top of the song.
fn song_position(sixteenths: u16) -> [u8; 3] {
    let v = sixteenths.min(0x3FFF);
    [0xF2, (v & 0x7F) as u8, (v >> 7) as u8]
}

/// Turns where the song is into clock messages.
#[derive(Default)]
struct ClockFollower {
    running: bool,
    /// The next clock to send, counted from the top of the song.
    next_clock: i64,
}

impl ClockFollower {
    /// `beats` is where the song is now, in quarter notes; `jumped` says the
    /// playhead moved by itself rather than playing on.
    fn step(&mut self, beats: f64, playing: bool, jumped: bool) -> Vec<ClockEvent> {
        let mut events = Vec::new();
        if !playing {
            if self.running {
                self.running = false;
                events.push(ClockEvent::Stop);
            }
            return events;
        }
        let clocks = beats.max(0.0) * CLOCKS_PER_BEAT;
        if !self.running || jumped {
            if self.running {
                events.push(ClockEvent::Stop);
            }
            if clocks < 0.01 {
                events.push(ClockEvent::Start);
                self.next_clock = 0;
            } else {
                // The position is in sixteenths, so the receiver starts on
                // the next one and the first clock goes out when the song
                // gets there.
                let sixteenth = (clocks / CLOCKS_PER_SIXTEENTH as f64).ceil() as i64;
                events.push(ClockEvent::ContinueAt(sixteenth.min(0x3FFF) as u16));
                self.next_clock = sixteenth * CLOCKS_PER_SIXTEENTH;
            }
            self.running = true;
        }
        // A stalled thread catches up by skipping, not by a burst of clocks
        // that would make the receiver race.
        let due = clocks.floor() as i64;
        if due - self.next_clock > CLOCKS_PER_SIXTEENTH * 4 {
            self.next_clock = due;
        }
        while self.next_clock as f64 <= clocks + 1e-6 {
            events.push(ClockEvent::Tick);
            self.next_clock += 1;
        }
        events
    }
}

/// Where the playhead is between the audio blocks that move it: the last
/// position the engine published, moved on by the time since, so clocks go
/// out between blocks rather than in bunches at each one.
struct PlayheadEstimate {
    seen: u64,
    seen_at: Instant,
    estimate: f64,
}

impl PlayheadEstimate {
    fn new(position: u64) -> Self {
        Self {
            seen: position,
            seen_at: Instant::now(),
            estimate: position as f64,
        }
    }

    /// The estimate now, and whether the playhead jumped (moved further
    /// than playing could have taken it, either way).
    fn update(&mut self, position: u64, sample_rate: f64, now: Instant) -> (f64, bool) {
        let mut jumped = false;
        if position != self.seen {
            if (position as f64 - self.estimate).abs() > sample_rate * 0.15 {
                jumped = true;
                self.estimate = position as f64;
            }
            self.seen = position;
            self.seen_at = now;
        }
        // Never further than a block or two past what the engine said, and
        // never backwards while playing on.
        let ahead = now.saturating_duration_since(self.seen_at).as_secs_f64() * sample_rate;
        let projected = self.seen as f64 + ahead.min(sample_rate * 0.05);
        if jumped {
            self.estimate = projected;
        } else {
            self.estimate = self.estimate.max(projected);
        }
        (self.estimate, jumped)
    }
}

/// Spawn the MIDI clock dispatcher. Stays alive for the lifetime of the
/// process — the manager's ports decide whether anything actually hears the
/// ticks.
///
/// It reads the transport's shared atomics and takes the engine's lock
/// only to place the playhead in the song when playback starts or jumps;
/// it used to take that lock two thousand times a second.
pub fn spawn_dispatcher(engine: Arc<Mutex<DawEngine>>, state: Arc<MidiClockState>) {
    let transport = engine.lock().transport.clone();
    std::thread::spawn(move || {
        let mut follower = ClockFollower::default();
        let mut playhead: Option<PlayheadEstimate> = None;
        // Where the song is in beats, and the sample that is at.
        let mut beats_at = 0.0_f64;
        let mut beats_sample = 0.0_f64;
        loop {
            std::thread::sleep(Duration::from_micros(500));
            let enabled = state.enabled.load(Ordering::Relaxed);
            let playing = enabled && transport.is_playing();
            if !playing {
                playhead = None;
                for event in follower.step(0.0, false, false) {
                    event.send(&state.output.lock());
                }
                continue;
            }
            let sr = (transport.sample_rate.load(Ordering::Relaxed) as f64).max(1.0);
            let position = transport.position();
            let now = Instant::now();
            let (estimate, jumped) = match playhead.as_mut() {
                Some(p) => p.update(position, sr, now),
                None => {
                    playhead = Some(PlayheadEstimate::new(position));
                    (position as f64, true)
                }
            };
            if jumped || !follower.running {
                // Through the tempo map, once, where the playhead landed.
                let ticks = {
                    let eng = engine.lock();
                    let project = eng.project.lock();
                    project.tempo_map.samples_to_tick(estimate as u64, sr)
                };
                beats_at = ticks as f64 / hardwave_midi::PPQ as f64;
                beats_sample = estimate;
            } else {
                // Played on: the beats the tempo of the moment covers.
                let bpm = transport.bpm.load(Ordering::Relaxed).clamp(1.0, 999.0);
                beats_at += (estimate - beats_sample) / sr * bpm / 60.0;
                beats_sample = estimate;
            }
            let events = follower.step(beats_at, true, jumped);
            if !events.is_empty() {
                let out = state.output.lock();
                for event in events {
                    event.send(&out);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ClockEvent::*;

    fn ticks(events: &[ClockEvent]) -> usize {
        events.iter().filter(|e| **e == Tick).count()
    }

    #[test]
    fn from_the_top_it_is_start_and_a_clock_on_the_downbeat() {
        let mut f = ClockFollower::default();
        assert_eq!(f.step(0.0, true, false), vec![Start, Tick]);
        // A beat later, 24 clocks in all.
        assert_eq!(ticks(&f.step(1.0, true, false)), 24);
    }

    /// Play pressed in bar 17 used to send Start, so a drum machine began
    /// its pattern from the top.
    #[test]
    fn from_mid_song_it_is_the_song_position_and_continue() {
        let mut f = ClockFollower::default();
        // Bar 17 of 4/4 is beat 64, sixteenth 256.
        assert_eq!(f.step(64.0, true, false), vec![ContinueAt(256), Tick]);
        assert_eq!(song_position(256), [0xF2, 0, 2]);
    }

    #[test]
    fn between_sixteenths_the_first_clock_waits_for_the_next_one() {
        let mut f = ClockFollower::default();
        // A third of the way into the second sixteenth.
        let at = 0.25 + 0.25 / 3.0;
        assert_eq!(f.step(at, true, false), vec![ContinueAt(2)]);
        // Nothing until the song reaches sixteenth 2 (beat 0.5).
        assert!(f.step(0.49, true, false).is_empty());
        assert_eq!(f.step(0.5, true, false), vec![Tick]);
    }

    #[test]
    fn a_loop_jumping_back_is_stop_position_continue() {
        let mut f = ClockFollower::default();
        f.step(0.0, true, false);
        f.step(16.0, true, false);
        assert_eq!(f.step(4.0, true, true), vec![Stop, ContinueAt(16), Tick]);
    }

    #[test]
    fn stopping_sends_stop_once() {
        let mut f = ClockFollower::default();
        f.step(2.0, true, false);
        assert_eq!(f.step(0.0, false, false), vec![Stop]);
        assert!(f.step(0.0, false, false).is_empty());
    }

    #[test]
    fn a_stalled_thread_skips_instead_of_bursting() {
        let mut f = ClockFollower::default();
        f.step(0.0, true, false);
        assert!(ticks(&f.step(10.0, true, false)) < 30);
    }

    #[test]
    fn the_playhead_moves_on_between_blocks_and_notices_a_jump() {
        let t0 = Instant::now();
        let mut p = PlayheadEstimate::new(48_000);
        let (e, j) = p.update(48_000, 48_000.0, t0 + Duration::from_millis(5));
        assert!(!j);
        assert!((e - 48_240.0).abs() < 1.0, "5 ms on, got {e}");
        let (_, j) = p.update(48_512, 48_000.0, t0 + Duration::from_millis(11));
        assert!(!j, "the next block is playing on");
        let (e, j) = p.update(0, 48_000.0, t0 + Duration::from_millis(12));
        assert!(j, "a loop wrapping to the top is a jump");
        assert!(e < 1_000.0);
    }
}
