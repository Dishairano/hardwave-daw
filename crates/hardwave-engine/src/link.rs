//! Ableton Link: tempo and start-stop shared with whatever else is on
//! the network.
//!
//! A producer rarely works alone with one program. A laptop running
//! Ableton, a phone running a drum app and a friend on the other side
//! of the table all agree on a tempo and a downbeat through Link, and
//! anything not in that session has to be nudged by hand all night.
//!
//! The tempo and the start and stop are shared both ways, and the peer
//! count is shown so it is obvious whether anyone is listening. The beat
//! is shared too: with others in the session, pressing play waits for the
//! session to reach the same place in the bar, and while playing the
//! playhead is kept on the session's beat (see [`PhaseFollower`]). Before,
//! two machines at the same tempo still played a beat or two apart.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rusty_link::{AblLink, SessionState};

/// What Link is doing, readable from anywhere.
#[derive(Default)]
pub struct LinkStatus {
    pub enabled: AtomicBool,
    pub peers: AtomicU64,
    /// Tempo of the session, as hundredths of a beat per minute so it
    /// can live in an atomic.
    pub tempo_centi: AtomicU64,
    pub start_stop_sync: AtomicBool,
}

impl LinkStatus {
    pub fn tempo(&self) -> f64 {
        self.tempo_centi.load(Ordering::Relaxed) as f64 / 100.0
    }
}

/// The session's clock, for the audio thread: Link and how late the
/// output is heard, while a session is joined.
#[derive(Default)]
pub struct LinkClock {
    slot: Mutex<Option<(Arc<AblLink>, Arc<hardwave_audio_io::StreamLatency>)>>,
}

impl LinkClock {
    fn attach(&self, link: Arc<AblLink>, latency: Arc<hardwave_audio_io::StreamLatency>) {
        *self.slot.lock() = Some((link, latency));
    }

    fn detach(&self) {
        *self.slot.lock() = None;
    }
}

/// What the playhead does this block to stay on the session's beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LinkAction {
    /// Nothing to do.
    Keep,
    /// Play was pressed, and the session has not reached the playhead's
    /// place in the bar yet: wait.
    Hold,
    /// Move the playhead by this many samples (earlier when negative).
    Shift(i64),
}

/// How far off the beat the playhead may be before it is nudged: about a
/// millisecond at 120 BPM.
const PHASE_TOLERANCE_BEATS: f64 = 0.002;
/// Off by more than this (a quarter beat) is not drift but a jump in the
/// session, followed at once.
const PHASE_JUMP_BEATS: f64 = 0.25;
/// The most a nudge moves the playhead in one block: small enough not to be
/// heard as a skip.
const MAX_NUDGE_SAMPLES: f64 = 16.0;

/// The difference between the session's place in the bar and ours, in
/// beats, the short way round: positive when the session is ahead.
fn phase_error(link_phase: f64, our_beats: f64, quantum: f64) -> f64 {
    let q = quantum.max(1e-9);
    let mut err = (link_phase - our_beats.rem_euclid(q)).rem_euclid(q);
    if err > q / 2.0 {
        err -= q;
    }
    err
}

/// What to do this block. `starting` is the first block after play was
/// pressed; `block_beats` is how many beats this block lasts.
fn decide(
    link_phase: f64,
    our_beats: f64,
    quantum: f64,
    starting: bool,
    block_beats: f64,
    samples_per_beat: f64,
) -> LinkAction {
    let err = phase_error(link_phase, our_beats, quantum);
    if starting {
        // Start in the block in which the session reaches our place in the
        // bar, moved by the fraction of the block it is off.
        return if err <= 0.0 && err > -block_beats.max(PHASE_TOLERANCE_BEATS) {
            LinkAction::Shift((err * samples_per_beat).round() as i64)
        } else {
            LinkAction::Hold
        };
    }
    if err.abs() > PHASE_JUMP_BEATS {
        LinkAction::Shift((err * samples_per_beat).round() as i64)
    } else if err.abs() > PHASE_TOLERANCE_BEATS {
        let nudge = (err * samples_per_beat).clamp(-MAX_NUDGE_SAMPLES, MAX_NUDGE_SAMPLES);
        LinkAction::Shift(nudge.round() as i64)
    } else {
        LinkAction::Keep
    }
}

/// Keeps the playhead on the session's beat, on the audio thread.
pub struct PhaseFollower {
    state: SessionState,
    starting: bool,
    was_playing: bool,
}

impl Default for PhaseFollower {
    fn default() -> Self {
        Self {
            state: SessionState::new(),
            starting: false,
            was_playing: false,
        }
    }
}

impl PhaseFollower {
    /// One audio block. `wants_to_play` is whether the transport is playing
    /// or waiting to; `our_beats` gives the playhead in quarter notes (asked
    /// only when there is a session to follow, and None when it cannot be
    /// read this block); `quantum` is the bar in quarter notes. Does nothing
    /// without others in the session.
    #[allow(clippy::too_many_arguments)]
    pub fn block(
        &mut self,
        clock: &LinkClock,
        wants_to_play: bool,
        our_beats: impl FnOnce() -> Option<f64>,
        quantum: f64,
        bpm: f64,
        sample_rate: f64,
        frames: usize,
    ) -> LinkAction {
        if !wants_to_play {
            self.was_playing = false;
            self.starting = false;
            return LinkAction::Keep;
        }
        if !self.was_playing {
            self.was_playing = true;
            self.starting = true;
        }
        // try_lock: the audio thread waits for nobody. A missed block is
        // put right by the next one.
        let Some(slot) = clock.slot.try_lock() else {
            return LinkAction::Keep;
        };
        let Some((link, latency)) = slot.as_ref() else {
            self.starting = false;
            return LinkAction::Keep;
        };
        if link.num_peers() == 0 {
            self.starting = false;
            return LinkAction::Keep;
        }
        let Some(our_beats) = our_beats() else {
            return LinkAction::Keep;
        };
        link.capture_audio_session_state(&mut self.state);
        // The beat the session is on when this block is heard.
        let heard_at = link.clock_micros() + (latency.output_nanos() / 1000) as i64;
        let link_phase = self.state.phase_at_time(heard_at, quantum);
        let samples_per_beat = sample_rate * 60.0 / bpm.max(1.0);
        let block_beats = frames as f64 / samples_per_beat.max(1.0);
        let action = decide(
            link_phase,
            our_beats,
            quantum,
            self.starting,
            block_beats,
            samples_per_beat,
        );
        if !matches!(action, LinkAction::Hold) {
            self.starting = false;
        }
        action
    }
}

/// The session, and the thread that keeps it and the transport in step.
pub struct LinkSession {
    link: Arc<AblLink>,
    pub status: Arc<LinkStatus>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    clock: Arc<LinkClock>,
}

impl LinkSession {
    /// Join the network and keep the tempo in step with `transport`.
    ///
    /// The work runs on a thread of its own at fifty times a second,
    /// not on the audio thread: Link takes its own locks, and the audio
    /// thread may not wait for anybody.
    pub fn start(
        transport: crate::TransportState,
        initial_bpm: f64,
        latency: Arc<hardwave_audio_io::StreamLatency>,
    ) -> Self {
        let link = Arc::new(AblLink::new(initial_bpm));
        link.enable(true);
        link.enable_start_stop_sync(true);
        transport.link.attach(Arc::clone(&link), latency);
        let clock = Arc::clone(&transport.link);

        let status = Arc::new(LinkStatus::default());
        status.enabled.store(true, Ordering::Relaxed);
        status.start_stop_sync.store(true, Ordering::Relaxed);
        status
            .tempo_centi
            .store((initial_bpm * 100.0) as u64, Ordering::Relaxed);

        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let link = Arc::clone(&link);
            let status = Arc::clone(&status);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("hardwave-link".into())
                .spawn(move || {
                    let mut state = SessionState::new();
                    let mut last_sent_bpm = initial_bpm;
                    let mut last_sent_playing = transport.playing.load(Ordering::Relaxed);
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        link.capture_app_session_state(&mut state);
                        status.peers.store(link.num_peers(), Ordering::Relaxed);

                        // Their tempo becomes ours, unless we are the
                        // ones who changed it.
                        let their_bpm = state.tempo();
                        let our_bpm = transport.bpm.load(Ordering::Relaxed);
                        status
                            .tempo_centi
                            .store((their_bpm * 100.0) as u64, Ordering::Relaxed);
                        if (our_bpm - last_sent_bpm).abs() > 0.001 {
                            // We moved: tell the session.
                            state.set_tempo(our_bpm, link.clock_micros());
                            link.commit_app_session_state(&state);
                            last_sent_bpm = our_bpm;
                        } else if (their_bpm - our_bpm).abs() > 0.001 {
                            transport.bpm.store(their_bpm, Ordering::Relaxed);
                            last_sent_bpm = their_bpm;
                        }

                        // Start and stop, both ways.
                        let our_playing = transport.playing.load(Ordering::Relaxed);
                        let their_playing = state.is_playing();
                        if our_playing != last_sent_playing {
                            state.set_is_playing(our_playing, link.clock_micros());
                            link.commit_app_session_state(&state);
                            last_sent_playing = our_playing;
                        } else if their_playing != our_playing {
                            transport.playing.store(their_playing, Ordering::Relaxed);
                            last_sent_playing = their_playing;
                        }
                    }
                    link.enable(false);
                })
                .expect("link thread")
        };

        Self {
            link,
            status,
            stop,
            worker: Some(worker),
            clock,
        }
    }

    pub fn peers(&self) -> u64 {
        self.link.num_peers()
    }
}

impl Drop for LinkSession {
    fn drop(&mut self) {
        self.clock.detach();
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.status.enabled.store(false, Ordering::Relaxed);
        self.status.peers.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPB: f64 = 24_000.0; // samples a beat at 120 BPM, 48 kHz

    #[test]
    fn the_error_goes_the_short_way_round_the_bar() {
        assert!((phase_error(0.5, 4.0, 4.0) - 0.5).abs() < 1e-9);
        assert!(
            (phase_error(3.9, 8.1, 4.0) + 0.2).abs() < 1e-9,
            "just behind the barline"
        );
        assert!(
            (phase_error(0.1, 7.9, 4.0) - 0.2).abs() < 1e-9,
            "just past it"
        );
    }

    /// Two machines at the same tempo still played a beat or two apart:
    /// play started wherever it was pressed.
    #[test]
    fn play_waits_for_the_session_to_reach_the_bar() {
        // We start on beat 1 (phase 0); the session is on beat 3: wait.
        assert_eq!(decide(2.0, 0.0, 4.0, true, 0.02, SPB), LinkAction::Hold);
        // Still more than a block (0.02 beats) before the downbeat: wait.
        assert_eq!(decide(3.95, 0.0, 4.0, true, 0.02, SPB), LinkAction::Hold);
        // The session reaches the downbeat inside this block: start, moved
        // back by the hundredth of a beat it is off.
        assert_eq!(
            decide(3.99, 0.0, 4.0, true, 0.02, SPB),
            LinkAction::Shift(-240)
        );
    }

    #[test]
    fn drift_is_nudged_a_little_at_a_time_and_a_jump_followed() {
        assert_eq!(decide(1.001, 1.0, 4.0, false, 0.02, SPB), LinkAction::Keep);
        assert_eq!(
            decide(1.01, 1.0, 4.0, false, 0.02, SPB),
            LinkAction::Shift(16)
        );
        assert_eq!(
            decide(0.99, 1.0, 4.0, false, 0.02, SPB),
            LinkAction::Shift(-16)
        );
        assert_eq!(
            decide(2.0, 1.0, 4.0, false, 0.02, SPB),
            LinkAction::Shift(24_000)
        );
    }

    #[test]
    fn alone_in_the_session_nothing_is_held() {
        let clock = LinkClock::default();
        let mut f = PhaseFollower::default();
        assert_eq!(
            f.block(&clock, true, || Some(0.0), 4.0, 120.0, 48_000.0, 512),
            LinkAction::Keep
        );
    }
}
