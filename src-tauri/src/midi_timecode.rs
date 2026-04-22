//! MIDI Time Code output — emits Quarter Frame messages (status 0xF1) at
//! 4 × fps while the transport is playing and MTC send is enabled. The
//! output manager is shared with MidiClockState so both clock and timecode
//! stream to the same set of configured MIDI output ports.
//!
//! The encoded timecode is a snapshot taken at piece 0 of each 8-message
//! cycle and cycles through pieces 0..=7 over two frames of wall time, so
//! by the time piece 7 is sent the encoded timecode is two frames old —
//! standard MTC behavior that receivers already compensate for.

use hardwave_engine::DawEngine;
use hardwave_midi::MidiOutputManager;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Frame-rate codes accepted by `set_fps`. 24/25/30 non-drop are fully
/// supported; 29 is reserved for future 29.97 drop-frame support.
pub const MTC_FPS_24: u32 = 24;
pub const MTC_FPS_25: u32 = 25;
pub const MTC_FPS_30: u32 = 30;

pub struct MidiTimecodeState {
    pub enabled: Arc<AtomicBool>,
    pub fps: Arc<AtomicU32>,
    pub output: Arc<Mutex<MidiOutputManager>>,
}

impl MidiTimecodeState {
    /// Build a state that broadcasts through an already-existing output
    /// manager (shared with MidiClockState so both features stream to the
    /// same port set).
    pub fn with_output(output: Arc<Mutex<MidiOutputManager>>) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
            fps: Arc::new(AtomicU32::new(MTC_FPS_30)),
            output,
        }
    }

    pub fn fps_as_f64(&self) -> f64 {
        match self.fps.load(Ordering::Relaxed) {
            MTC_FPS_24 => 24.0,
            MTC_FPS_25 => 25.0,
            _ => 30.0,
        }
    }

    /// SMPTE type bits encoded in piece 7 (the two high bits of its data
    /// nibble). 24fps=00, 25fps=01, 30fps non-drop=11.
    pub fn smpte_type_bits(&self) -> u8 {
        match self.fps.load(Ordering::Relaxed) {
            MTC_FPS_24 => 0b00,
            MTC_FPS_25 => 0b01,
            _ => 0b11,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmpteTime {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
}

/// Convert a playback position in seconds to SMPTE hours/minutes/seconds/
/// frames at the given integer frame rate. Hours wrap at 24 so very long
/// sessions don't overflow the 5-bit hour field.
pub fn seconds_to_smpte(secs: f64, fps: u32) -> SmpteTime {
    let fps_f = fps.max(1) as f64;
    let total_frames = (secs.max(0.0) * fps_f).floor() as u64;
    let frames = (total_frames % fps as u64) as u8;
    let total_seconds = total_frames / fps as u64;
    let seconds = (total_seconds % 60) as u8;
    let total_minutes = total_seconds / 60;
    let minutes = (total_minutes % 60) as u8;
    let hours = ((total_minutes / 60) % 24) as u8;
    SmpteTime {
        hours,
        minutes,
        seconds,
        frames,
    }
}

/// Build the quarter-frame data byte for a given piece (0..=7), timecode
/// snapshot, and SMPTE type bits. Status byte 0xF1 is emitted separately.
pub fn quarter_frame_data(piece: u8, t: SmpteTime, smpte_bits: u8) -> u8 {
    let piece = piece & 0x07;
    let nibble = match piece {
        0 => t.frames & 0x0F,
        1 => (t.frames >> 4) & 0x01,
        2 => t.seconds & 0x0F,
        3 => (t.seconds >> 4) & 0x03,
        4 => t.minutes & 0x0F,
        5 => (t.minutes >> 4) & 0x03,
        6 => t.hours & 0x0F,
        7 => ((t.hours >> 4) & 0x01) | ((smpte_bits & 0x03) << 1),
        _ => 0,
    };
    (piece << 4) | (nibble & 0x0F)
}

/// Spawn the MTC dispatcher. Sleeps ~500 µs per iteration and emits one
/// quarter frame whenever the scheduled deadline passes. Snapshots the
/// timecode at piece 0 and re-uses it for pieces 1..=7 so receivers see a
/// consistent 8-message cycle encoding a single SMPTE position.
pub fn spawn_dispatcher(engine: Arc<Mutex<DawEngine>>, state: Arc<MidiTimecodeState>) {
    std::thread::spawn(move || {
        let mut next_at = Instant::now();
        let mut piece: u8 = 0;
        let mut snapshot = SmpteTime {
            hours: 0,
            minutes: 0,
            seconds: 0,
            frames: 0,
        };
        loop {
            std::thread::sleep(Duration::from_micros(500));
            let enabled = state.enabled.load(Ordering::Relaxed);
            let (is_playing, position_samples, sample_rate) = {
                let eng = engine.lock();
                let t = &eng.transport;
                (
                    t.is_playing(),
                    t.position(),
                    t.sample_rate.load(Ordering::Relaxed),
                )
            };
            if !enabled || !is_playing {
                // Reset phase so a fresh play from 0 always starts at piece 0.
                piece = 0;
                next_at = Instant::now();
                continue;
            }
            let now = Instant::now();
            if now < next_at {
                continue;
            }
            if piece == 0 {
                let sr = sample_rate.max(1) as f64;
                let secs = position_samples as f64 / sr;
                snapshot = seconds_to_smpte(secs, state.fps.load(Ordering::Relaxed));
            }
            let data = quarter_frame_data(piece, snapshot, state.smpte_type_bits());
            {
                let out = state.output.lock();
                out.broadcast(&[0xF1, data]);
            }
            let fps_f = state.fps_as_f64();
            let interval = Duration::from_secs_f64(1.0 / (4.0 * fps_f));
            next_at += interval;
            if now.saturating_duration_since(next_at) > Duration::from_millis(100) {
                next_at = now + interval;
            }
            piece = (piece + 1) & 0x07;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_to_smpte_at_30fps_wraps_correctly() {
        let t = seconds_to_smpte(3661.5, 30);
        assert_eq!(t.hours, 1);
        assert_eq!(t.minutes, 1);
        assert_eq!(t.seconds, 1);
        assert_eq!(t.frames, 15);
    }

    #[test]
    fn quarter_frame_piece_0_encodes_low_nibble_of_frames() {
        let t = SmpteTime {
            hours: 0,
            minutes: 0,
            seconds: 0,
            frames: 0x0B,
        };
        let d = quarter_frame_data(0, t, 0b11);
        assert_eq!(d, 0x0B);
    }

    #[test]
    fn quarter_frame_piece_7_encodes_smpte_type() {
        let t = SmpteTime {
            hours: 0,
            minutes: 0,
            seconds: 0,
            frames: 0,
        };
        // piece 7, hours high nibble = 0, smpte type = 30 non-drop (11)
        let d = quarter_frame_data(7, t, 0b11);
        assert_eq!(d, 0x70 | 0b0110);
    }

    #[test]
    fn seconds_to_smpte_clamps_negative() {
        let t = seconds_to_smpte(-1.0, 30);
        assert_eq!(t.hours, 0);
        assert_eq!(t.minutes, 0);
        assert_eq!(t.seconds, 0);
        assert_eq!(t.frames, 0);
    }
}
