//! Ableton Link: tempo and start-stop shared with whatever else is on
//! the network.
//!
//! A producer rarely works alone with one program. A laptop running
//! Ableton, a phone running a drum app and a friend on the other side
//! of the table all agree on a tempo and a downbeat through Link, and
//! anything not in that session has to be nudged by hand all night.
//!
//! What is here is the useful half: the tempo and the start and stop
//! are shared both ways, and the peer count is shown so it is obvious
//! whether anyone is listening. Beat-accurate launch quantisation is
//! not: that needs the transport to be driven from Link's beat clock
//! rather than its own, which is a change to how playback is timed
//! and not something to bolt on beside it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

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

/// The session, and the thread that keeps it and the transport in step.
pub struct LinkSession {
    link: Arc<AblLink>,
    pub status: Arc<LinkStatus>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl LinkSession {
    /// Join the network and keep the tempo in step with `transport`.
    ///
    /// The work runs on a thread of its own at fifty times a second,
    /// not on the audio thread: Link takes its own locks, and the audio
    /// thread may not wait for anybody.
    pub fn start(transport: crate::TransportState, initial_bpm: f64) -> Self {
        let link = Arc::new(AblLink::new(initial_bpm));
        link.enable(true);
        link.enable_start_stop_sync(true);

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
        }
    }

    pub fn peers(&self) -> u64 {
        self.link.num_peers()
    }
}

impl Drop for LinkSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.status.enabled.store(false, Ordering::Relaxed);
        self.status.peers.store(0, Ordering::Relaxed);
    }
}
