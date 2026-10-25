//! Following an external master: MIDI clock (tempo, Start, Continue from
//! the song position it sends, Stop) and MIDI timecode (chasing its
//! position: playing while its quarter frames come in, locating on its
//! full-frame message, stopping when they stop).

use hardwave_engine::DawEngine;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct MidiClockSyncState {
    pub enabled: Arc<AtomicBool>,
    /// Most recent BPM observed on the external clock, or `None` if no
    /// clock master has been seen. Updated whether or not sync is enabled
    /// so the UI can show what *would* be slaved.
    pub last_bpm: Arc<Mutex<Option<f64>>>,
    /// True once any 0xF8 tick has been seen since the sync state was
    /// created. Lets the UI distinguish "no master present" from "master
    /// present, sync disabled".
    pub ticks_seen: Arc<AtomicBool>,
    /// Follow MIDI timecode from a master (a video app, a tape machine,
    /// another DAW).
    pub mtc_enabled: Arc<AtomicBool>,
    /// The master's timecode at the top of the song, in seconds: video work
    /// usually starts at 01:00:00:00.
    pub mtc_offset: Arc<Mutex<f64>>,
    /// The master's last position and frame rate, for the settings page.
    pub mtc_seen: Arc<Mutex<Option<(f64, f64)>>>,
}

impl MidiClockSyncState {
    pub fn new() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            last_bpm: Arc::new(Mutex::new(None)),
            ticks_seen: Arc::new(AtomicBool::new(false)),
            mtc_enabled: Arc::new(AtomicBool::new(false)),
            mtc_offset: Arc::new(Mutex::new(0.0)),
            mtc_seen: Arc::new(Mutex::new(None)),
        }
    }
}

impl Default for MidiClockSyncState {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the sync dispatcher. It drains the input manager's clock-sync
/// snapshot every ~10 ms and, when enabled, writes the estimated BPM to
/// `transport.bpm` and reacts to Start / Continue / Stop messages.
pub fn spawn_dispatcher(
    engine: Arc<Mutex<DawEngine>>,
    midi_input: Arc<Mutex<hardwave_midi::MidiInputManager>>,
    state: Arc<MidiClockSyncState>,
) {
    let transport = engine.lock().transport.clone();
    std::thread::spawn(move || {
        let mut chase = MtcChase::default();
        loop {
            std::thread::sleep(Duration::from_millis(10));
            // The input manager's own lock only, never while holding the engine's.
            let snapshot = midi_input.lock().take_clock_sync_snapshot();

            if let Some(bpm) = snapshot.bpm_estimate {
                *state.last_bpm.lock() = Some(bpm);
            }
            if snapshot.ticks_received {
                state.ticks_seen.store(true, Ordering::Relaxed);
            }
            if let (Some((at, _)), Some(fps)) = (snapshot.mtc_running_at, snapshot.mtc_fps) {
                *state.mtc_seen.lock() = Some((at, fps));
            }

            if state.enabled.load(Ordering::Relaxed) {
                if let Some(bpm) = snapshot.bpm_estimate {
                    transport.bpm.store(bpm, Ordering::Relaxed);
                }
                // A jump arrives as Stop, the position and Continue in one
                // burst; applied as separate flags in a fixed order it ended
                // stopped. The position goes first, then whichever of Start,
                // Continue and Stop came last.
                // Continue carries on from the song position the master
                // sent; it used to carry on from wherever the playhead was.
                // Start is the top of the song, whatever position came first.
                if snapshot.last_transport == Some(0xFA) {
                    transport.set_position(0);
                } else if let Some(sixteenths) = snapshot.song_position {
                    let ticks = sixteenths as u64 * hardwave_midi::PPQ / 4;
                    let sr = transport.sample_rate.load(Ordering::Relaxed) as f64;
                    let samples = {
                        let eng = engine.lock();
                        let project = eng.project.lock();
                        project.tempo_map.tick_to_samples(ticks, sr)
                    };
                    transport.set_position(samples);
                }
                match snapshot.last_transport {
                    Some(0xFA) | Some(0xFB) => transport.playing.store(true, Ordering::Relaxed),
                    Some(0xFC) => transport.playing.store(false, Ordering::Relaxed),
                    _ => {}
                }
            }

            if state.mtc_enabled.load(Ordering::Relaxed) {
                let sr = transport.sample_rate.load(Ordering::Relaxed) as f64;
                let offset = *state.mtc_offset.lock();
                let action = chase.step(
                    &snapshot,
                    transport.is_playing(),
                    transport.position() as f64 / sr.max(1.0),
                    offset,
                    std::time::Instant::now(),
                );
                if let Some(seconds) = action.locate {
                    transport.set_position((seconds.max(0.0) * sr) as u64);
                }
                if let Some(play) = action.play {
                    transport.playing.store(play, Ordering::Relaxed);
                }
            } else {
                chase = MtcChase::default();
            }
        }
    });
}

/// What following MIDI timecode does this round.
#[derive(Debug, Default, PartialEq)]
struct ChaseAction {
    /// Move the playhead here, in seconds into the song.
    locate: Option<f64>,
    /// Start (true) or stop (false) playback.
    play: Option<bool>,
}

/// Follows a timecode master: plays while its quarter frames come in,
/// moves the playhead when it drifts or jumps, locates on a full-frame
/// message and stops when the frames stop.
#[derive(Default)]
struct MtcChase {
    /// Playback was started by the master, so it is also stopped by it.
    following: bool,
}

/// How far the playhead may sit from the master before it is moved: about
/// three frames. A take of moving it every round would stutter.
const CHASE_TOLERANCE_SECS: f64 = 0.1;
/// Quarter frames come four times a frame; a master silent for this long
/// has stopped.
const CHASE_TIMEOUT: Duration = Duration::from_millis(250);

impl MtcChase {
    fn step(
        &mut self,
        snapshot: &hardwave_midi::input::ClockSyncSnapshot,
        playing: bool,
        position_secs: f64,
        offset: f64,
        now: std::time::Instant,
    ) -> ChaseAction {
        let mut action = ChaseAction::default();
        if let Some(at) = snapshot.mtc_locate {
            action.locate = Some(at - offset);
        }
        if let Some((at, read_at)) = snapshot.mtc_running_at {
            let target = at + now.saturating_duration_since(read_at).as_secs_f64() - offset;
            if target < 0.0 {
                // The master is rolling up to the song (a pre-roll): wait at
                // the top. Playing from 0 straight away and being pulled
                // back every frame looped the first moment of the song for
                // the whole pre-roll.
                if playing && self.following {
                    action.play = Some(false);
                    self.following = false;
                }
                if position_secs > 0.0 {
                    action.locate = Some(0.0);
                }
                return action;
            }
            if !playing {
                action.locate = Some(target);
                action.play = Some(true);
                self.following = true;
            } else if (position_secs - target).abs() > CHASE_TOLERANCE_SECS {
                action.locate = Some(target);
            }
        }
        let silent = snapshot
            .mtc_last_frame
            .is_none_or(|t| now.saturating_duration_since(t) > CHASE_TIMEOUT);
        if self.following && playing && silent && snapshot.mtc_running_at.is_none() {
            action.play = Some(false);
            self.following = false;
        }
        action
    }
}

#[cfg(test)]
mod chase_tests {
    use super::*;
    use hardwave_midi::input::ClockSyncSnapshot;
    use std::time::Instant;

    fn running(at: f64, read_at: Instant) -> ClockSyncSnapshot {
        ClockSyncSnapshot {
            mtc_running_at: Some((at, read_at)),
            mtc_last_frame: Some(read_at),
            mtc_fps: Some(25.0),
            ..Default::default()
        }
    }

    #[test]
    fn a_master_starting_puts_the_song_where_it_is_and_plays() {
        let now = Instant::now();
        let mut c = MtcChase::default();
        // The master reads 01:00:12 and the song starts at 01:00:00.
        let a = c.step(&running(3612.0, now), false, 0.0, 3600.0, now);
        assert_eq!(a.play, Some(true));
        assert!((a.locate.unwrap() - 12.0).abs() < 1e-9);
    }

    #[test]
    fn close_enough_is_left_alone_and_a_jump_is_followed() {
        let now = Instant::now();
        let mut c = MtcChase { following: true };
        assert_eq!(
            c.step(&running(12.0, now), true, 12.05, 0.0, now),
            ChaseAction::default()
        );
        let a = c.step(&running(40.0, now), true, 12.1, 0.0, now);
        assert_eq!(a.locate, Some(40.0));
        assert_eq!(a.play, None);
    }

    #[test]
    fn the_master_falling_silent_stops_the_song_it_started() {
        let then = Instant::now();
        let now = then + Duration::from_millis(400);
        let mut c = MtcChase { following: true };
        let quiet = ClockSyncSnapshot {
            mtc_last_frame: Some(then),
            ..Default::default()
        };
        assert_eq!(c.step(&quiet, true, 20.0, 0.0, now).play, Some(false));
        // Playback someone started by hand is not the master's to stop.
        let mut c = MtcChase::default();
        assert_eq!(c.step(&quiet, true, 20.0, 0.0, now).play, None);
    }

    /// A master rolling from 00:59:50 to a song starting at 01:00:00: the
    /// song waits at the top until the master gets there, then plays.
    #[test]
    fn a_pre_roll_waits_at_the_top_then_plays() {
        let now = Instant::now();
        let mut c = MtcChase::default();
        let a = c.step(&running(3590.0, now), false, 0.0, 3600.0, now);
        assert_eq!(a, ChaseAction::default(), "nothing yet, already at the top");
        let a = c.step(&running(3600.04, now), false, 0.0, 3600.0, now);
        assert_eq!(a.play, Some(true));
        assert!((a.locate.unwrap() - 0.04).abs() < 1e-9);
    }

    #[test]
    fn a_full_frame_message_locates_without_playing() {
        let now = Instant::now();
        let mut c = MtcChase::default();
        let locate = ClockSyncSnapshot {
            mtc_locate: Some(3630.0),
            ..Default::default()
        };
        let a = c.step(&locate, false, 0.0, 3600.0, now);
        assert_eq!(
            a,
            ChaseAction {
                locate: Some(30.0),
                play: None
            }
        );
    }
}
