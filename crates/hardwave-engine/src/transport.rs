//! Transport state — play/stop/record/loop, shared between audio thread and UI.

use atomic_float::AtomicF64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Transport state shared between the audio thread and the UI.
/// All fields are atomic — no locks needed on the audio thread.
#[derive(Clone)]
pub struct TransportState {
    pub playing: Arc<AtomicBool>,
    pub recording: Arc<AtomicBool>,
    pub looping: Arc<AtomicBool>,

    /// Current playhead position in samples.
    pub position_samples: Arc<AtomicU64>,

    /// BPM (may change via automation).
    pub bpm: Arc<AtomicF64>,

    /// Loop start/end in samples.
    pub loop_start: Arc<AtomicU64>,
    pub loop_end: Arc<AtomicU64>,

    /// Sample rate.
    pub sample_rate: Arc<AtomicU64>,

    /// Master output volume in dB. Read on the audio thread by MasterNode.
    pub master_volume_db: Arc<AtomicF64>,

    /// Time signature (numerator, denominator). Packed into a single atomic
    /// as two u32s so both halves update atomically.
    pub time_sig: Arc<AtomicU64>,

    /// Playback mode: 0 = Song, 1 = Pattern.
    pub pattern_mode: Arc<AtomicBool>,

    /// When true, live input on armed+monitored tracks is routed directly to
    /// the master bus, bypassing the track's FX chain and fader. Gives the
    /// lowest possible monitoring latency at the cost of hearing the input
    /// without processing.
    pub direct_monitoring: Arc<AtomicBool>,

    /// Plugin delay compensation toggle. When false, parallel-path alignment
    /// is skipped and the UI reports 0 latency — use for low-latency
    /// monitoring when absolute timing across tracks doesn't matter.
    pub pdc_enabled: Arc<AtomicBool>,
}

/// Pack a (numerator, denominator) time signature into a u64.
pub fn pack_time_sig(num: u32, den: u32) -> u64 {
    ((num as u64) << 32) | (den as u64)
}

/// Unpack a (numerator, denominator) time signature from a u64.
pub fn unpack_time_sig(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32)
}

impl Default for TransportState {
    fn default() -> Self {
        Self {
            playing: Arc::new(AtomicBool::new(false)),
            recording: Arc::new(AtomicBool::new(false)),
            looping: Arc::new(AtomicBool::new(false)),
            position_samples: Arc::new(AtomicU64::new(0)),
            bpm: Arc::new(AtomicF64::new(140.0)),
            loop_start: Arc::new(AtomicU64::new(0)),
            loop_end: Arc::new(AtomicU64::new(0)),
            sample_rate: Arc::new(AtomicU64::new(48000)),
            master_volume_db: Arc::new(AtomicF64::new(0.0)),
            time_sig: Arc::new(AtomicU64::new(pack_time_sig(4, 4))),
            pattern_mode: Arc::new(AtomicBool::new(false)),
            direct_monitoring: Arc::new(AtomicBool::new(false)),
            pdc_enabled: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl TransportState {
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    pub fn position(&self) -> u64 {
        self.position_samples.load(Ordering::Relaxed)
    }

    pub fn advance(&self, frames: u64) {
        let pos = self.position_samples.fetch_add(frames, Ordering::Relaxed) + frames;

        // Pattern mode: hard-loop on a 4-bar pattern at the current tempo/meter.
        // Song mode: use the user's loop region if looping is enabled.
        if self.pattern_mode.load(Ordering::Relaxed) {
            let sr = self.sample_rate.load(Ordering::Relaxed) as f64;
            let bpm = self.bpm.load(Ordering::Relaxed).max(1.0);
            let (num, _den) = unpack_time_sig(self.time_sig.load(Ordering::Relaxed));
            let beats_per_bar = num.max(1) as f64;
            let samples_per_beat = 60.0 / bpm * sr;
            let pattern_len = (samples_per_beat * beats_per_bar * 4.0) as u64;
            if pattern_len > 0 && pos >= pattern_len {
                self.position_samples
                    .store(pos % pattern_len, Ordering::Relaxed);
            }
        } else if self.looping.load(Ordering::Relaxed) {
            let loop_end = self.loop_end.load(Ordering::Relaxed);
            if loop_end > 0 && pos >= loop_end {
                let loop_start = self.loop_start.load(Ordering::Relaxed);
                self.position_samples.store(loop_start, Ordering::Relaxed);
            }
        }
    }

    pub fn set_position(&self, pos: u64) {
        self.position_samples.store(pos, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_wraps_position_in_song_mode() {
        let t = TransportState::default();
        t.looping.store(true, Ordering::Relaxed);
        t.loop_start.store(48000, Ordering::Relaxed);
        t.loop_end.store(96000, Ordering::Relaxed);
        t.playing.store(true, Ordering::Relaxed);
        t.pattern_mode.store(false, Ordering::Relaxed);
        // Set position just before loop end
        t.set_position(95000);
        // Advance past loop end
        t.advance(2000);
        let pos = t.position();
        assert!(
            (48000..96000).contains(&pos),
            "position should wrap to loop region, got {}",
            pos
        );
    }

    #[test]
    fn no_wrap_without_loop() {
        let t = TransportState::default();
        t.looping.store(false, Ordering::Relaxed);
        t.playing.store(true, Ordering::Relaxed);
        t.pattern_mode.store(false, Ordering::Relaxed);
        t.set_position(100000);
        t.advance(5000);
        assert_eq!(t.position(), 105000);
    }

    #[test]
    fn pattern_mode_wraps_at_4_bars() {
        let t = TransportState::default();
        t.pattern_mode.store(true, Ordering::Relaxed);
        t.bpm.store(120.0, Ordering::Relaxed);
        t.sample_rate.store(48000, Ordering::Relaxed);
        t.time_sig.store(pack_time_sig(4, 4), Ordering::Relaxed);
        // At 120 BPM, 4/4: 1 beat = 24000 samples, 4 bars = 16 beats = 384000 samples
        t.set_position(380000);
        t.advance(10000);
        let pos = t.position();
        assert!(
            pos < 384000,
            "pattern mode should wrap at 4-bar boundary, got {}",
            pos
        );
    }

    #[test]
    fn time_sig_pack_unpack() {
        for (num, den) in [(4, 4), (3, 4), (7, 8), (12, 16)] {
            let packed = pack_time_sig(num, den);
            let (n, d) = unpack_time_sig(packed);
            assert_eq!((n, d), (num, den));
        }
    }
}

/// Commands sent from UI thread to engine.
#[derive(Debug, Clone)]
pub enum TransportCommand {
    Play,
    Stop,
    Record,
    SetPosition(u64),
    SetBpm(f64),
    SetLoop(u64, u64),
    ToggleLoop,
    SetMasterVolume(f64),
    SetTimeSignature(u32, u32),
    SetPatternMode(bool),
}
