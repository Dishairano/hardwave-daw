//! MIDI Time Code in: the quarter frames a master sends while it plays, and
//! the full-frame message it sends to say where it is when it locates.

/// A position as timecode, at the master's frame rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    /// Frames per second: 24, 25, 29.97 (drop frame) or 30.
    pub fps: f64,
}

impl Timecode {
    /// The rate bits as MTC sends them.
    fn rate(bits: u8) -> f64 {
        match bits & 3 {
            0 => 24.0,
            1 => 25.0,
            2 => 30_000.0 / 1001.0,
            _ => 30.0,
        }
    }

    /// Seconds from 00:00:00:00. Drop-frame timecode skips frame numbers,
    /// not time, so its frames are counted the way the master counts them.
    pub fn seconds(&self) -> f64 {
        let whole = self.hours as f64 * 3600.0 + self.minutes as f64 * 60.0 + self.seconds as f64;
        if (self.fps - 29.97).abs() < 0.01 {
            // Two frame numbers are dropped every minute except each tenth.
            let total_minutes = self.hours as i64 * 60 + self.minutes as i64;
            let dropped = 2 * (total_minutes - total_minutes / 10);
            let nominal = (whole * 30.0) as i64 + self.frames as i64 - dropped;
            nominal as f64 / self.fps
        } else {
            whole + self.frames as f64 / self.fps
        }
    }

    /// `frames` frames later.
    pub fn plus_frames(&self, frames: f64) -> f64 {
        self.seconds() + frames / self.fps
    }

    /// The full-frame message: `F0 7F <device> 01 01 hr mn sc fr F7`.
    pub fn from_full_frame(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0xF0, 0x7F, _, 0x01, 0x01, hr, mn, sc, fr, 0xF7, ..] => Some(Self {
                hours: hr & 0x1F,
                minutes: mn & 0x3F,
                seconds: sc & 0x3F,
                frames: fr & 0x1F,
                fps: Self::rate(hr >> 5),
            }),
            _ => None,
        }
    }
}

/// Puts the eight quarter frames of a timecode back together.
#[derive(Debug, Default, Clone)]
pub struct QuarterFrames {
    pieces: [u8; 8],
    seen: u8,
}

impl QuarterFrames {
    /// One quarter frame's data byte. Returns the position the master is at
    /// now, in seconds, and its frame rate, once all eight pieces have come
    /// in, in order.
    ///
    /// The eight pieces take two frames to send and describe where the
    /// master was when the first went out, so the position now is two
    /// frames on from what they spell.
    pub fn push(&mut self, data: u8) -> Option<(f64, f64)> {
        let piece = (data >> 4) & 7;
        if piece == 0 {
            self.seen = 0;
        }
        self.pieces[piece as usize] = data & 0x0F;
        self.seen |= 1 << piece;
        if piece != 7 || self.seen != 0xFF {
            return None;
        }
        self.seen = 0;
        let p = &self.pieces;
        let tc = Timecode {
            frames: p[0] | (p[1] & 1) << 4,
            seconds: p[2] | (p[3] & 3) << 4,
            minutes: p[4] | (p[5] & 3) << 4,
            hours: p[6] | (p[7] & 1) << 4,
            fps: Timecode::rate(p[7] >> 1),
        };
        Some((tc.plus_frames(2.0), tc.fps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quarter frames for 01:02:03:04 at 25 fps.
    fn quarter_frames() -> [u8; 8] {
        let (h, m, s, f, rate) = (1u8, 2u8, 3u8, 4u8, 1u8);
        [
            f & 0xF,
            0x10 | (f >> 4),
            0x20 | (s & 0xF),
            0x30 | (s >> 4),
            0x40 | (m & 0xF),
            0x50 | (m >> 4),
            0x60 | (h & 0xF),
            0x70 | (h >> 4) | (rate << 1),
        ]
    }

    #[test]
    fn eight_quarter_frames_spell_the_time_two_frames_on() {
        let mut qf = QuarterFrames::default();
        let pieces = quarter_frames();
        for d in &pieces[..7] {
            assert_eq!(qf.push(*d), None);
        }
        let (at, fps) = qf.push(pieces[7]).expect("complete");
        assert_eq!(fps, 25.0);
        let expected = 3600.0 + 120.0 + 3.0 + (4.0 + 2.0) / 25.0;
        assert!((at - expected).abs() < 1e-9, "got {at}");
    }

    #[test]
    fn a_cycle_missing_a_piece_gives_nothing() {
        let mut qf = QuarterFrames::default();
        let pieces = quarter_frames();
        for (i, d) in pieces.iter().enumerate() {
            if i != 3 {
                assert_eq!(qf.push(*d), None);
            }
        }
    }

    #[test]
    fn the_full_frame_message_locates() {
        let tc = Timecode::from_full_frame(&[
            0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 0x00, 0x1E, 0x0A, 0xF7,
        ])
        .expect("full frame");
        assert_eq!(
            (tc.hours, tc.minutes, tc.seconds, tc.frames),
            (1, 0, 30, 10)
        );
        assert_eq!(tc.fps, 30.0, "rate bits 11 are 30 fps");
        assert!((tc.seconds() - (3630.0 + 10.0 / 30.0)).abs() < 1e-9);
        assert!(Timecode::from_full_frame(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]).is_none());
    }

    #[test]
    fn drop_frame_counts_the_frames_the_master_counts() {
        // 00:01:00;02 is the first frame number after the drop: 1800 frames.
        let tc = Timecode {
            hours: 0,
            minutes: 1,
            seconds: 0,
            frames: 2,
            fps: 30_000.0 / 1001.0,
        };
        assert!((tc.seconds() - 1800.0 / tc.fps).abs() < 1e-9);
    }
}
