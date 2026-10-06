//! The frames a sandboxed plug-in and the DAW send each other.
//!
//! A plug-in that crashes during playback takes the DAW with it,
//! because it runs in our own process. A sandboxed one runs in a child
//! process instead, and everything that used to be a function call
//! becomes a frame down a pipe: here is a block of audio, move that
//! parameter, give me your state.
//!
//! Deliberately dull: a tag, a length, and the bytes. A plug-in
//! process that goes wrong should fail to answer, not confuse the host
//! with a half-understood message.

use std::io::{Read, Write};

/// Frames the DAW sends to the child.
pub const REQ_AUDIO: u32 = 1;
pub const REQ_SET_PARAM: u32 = 2;
pub const REQ_SET_STATE: u32 = 3;
pub const REQ_GET_STATE: u32 = 4;
pub const REQ_ACTIVATE: u32 = 5;
pub const REQ_SHUTDOWN: u32 = 6;

/// Frames the child sends back.
pub const RES_AUDIO: u32 = 101;
pub const RES_STATE: u32 = 102;
pub const RES_OK: u32 = 103;
pub const RES_ERROR: u32 = 104;

/// A block bigger than this is a mistake or a corrupted stream, and
/// allocating on it would be the crash we are trying to avoid.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// One message: what it is, and its bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub kind: u32,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(kind: u32, payload: Vec<u8>) -> Self {
        Self { kind, payload }
    }

    /// A stereo block, as interleaved samples.
    pub fn audio(left: &[f32], right: &[f32]) -> Self {
        let frames = left.len().min(right.len());
        let mut payload = Vec::with_capacity(frames * 8);
        for i in 0..frames {
            payload.extend_from_slice(&left[i].to_le_bytes());
            payload.extend_from_slice(&right[i].to_le_bytes());
        }
        Self::new(REQ_AUDIO, payload)
    }

    /// Read a stereo block back out of a frame, into buffers that are
    /// already the right size. Returns how many frames were written.
    pub fn read_audio(&self, left: &mut [f32], right: &mut [f32]) -> usize {
        let frames = (self.payload.len() / 8).min(left.len()).min(right.len());
        for i in 0..frames {
            let base = i * 8;
            left[i] = f32::from_le_bytes([
                self.payload[base],
                self.payload[base + 1],
                self.payload[base + 2],
                self.payload[base + 3],
            ]);
            right[i] = f32::from_le_bytes([
                self.payload[base + 4],
                self.payload[base + 5],
                self.payload[base + 6],
                self.payload[base + 7],
            ]);
        }
        frames
    }

    /// A parameter move: which one, and where to.
    pub fn set_param(id: u32, value: f64) -> Self {
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&id.to_le_bytes());
        payload.extend_from_slice(&value.to_le_bytes());
        Self::new(REQ_SET_PARAM, payload)
    }

    pub fn read_set_param(&self) -> Option<(u32, f64)> {
        if self.payload.len() < 12 {
            return None;
        }
        let id = u32::from_le_bytes(self.payload[0..4].try_into().ok()?);
        let value = f64::from_le_bytes(self.payload[4..12].try_into().ok()?);
        Some((id, value))
    }

    /// Sample rate and the biggest block the child should expect.
    pub fn activate(sample_rate: f64, max_block: u32) -> Self {
        let mut payload = Vec::with_capacity(12);
        payload.extend_from_slice(&sample_rate.to_le_bytes());
        payload.extend_from_slice(&max_block.to_le_bytes());
        Self::new(REQ_ACTIVATE, payload)
    }

    pub fn read_activate(&self) -> Option<(f64, u32)> {
        if self.payload.len() < 12 {
            return None;
        }
        let sample_rate = f64::from_le_bytes(self.payload[0..8].try_into().ok()?);
        let max_block = u32::from_le_bytes(self.payload[8..12].try_into().ok()?);
        Some((sample_rate, max_block))
    }

    pub fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        out.write_all(&self.kind.to_le_bytes())?;
        out.write_all(&(self.payload.len() as u32).to_le_bytes())?;
        out.write_all(&self.payload)?;
        out.flush()
    }

    /// Read one frame. An end of stream is `Ok(None)`, which is how a
    /// child that has gone away is noticed.
    pub fn read(input: &mut impl Read) -> std::io::Result<Option<Frame>> {
        let mut header = [0u8; 8];
        match input.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let kind = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if len > MAX_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("a frame of {len} bytes is not a frame we sent"),
            ));
        }
        let mut payload = vec![0u8; len];
        match input.read_exact(&mut payload) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        Ok(Some(Frame { kind, payload }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_survives_the_round_trip() {
        let frame = Frame::new(REQ_SET_STATE, vec![1, 2, 3, 4, 5]);
        let mut bytes = Vec::new();
        frame.write(&mut bytes).expect("write");
        let back = Frame::read(&mut bytes.as_slice())
            .expect("read")
            .expect("some");
        assert_eq!(back, frame);
    }

    #[test]
    fn audio_comes_back_as_the_same_samples() {
        let left: Vec<f32> = (0..128).map(|n| (n as f32 * 0.01).sin()).collect();
        let right: Vec<f32> = left.iter().map(|s| -s).collect();
        let frame = Frame::audio(&left, &right);
        let mut out_l = vec![0.0f32; 128];
        let mut out_r = vec![0.0f32; 128];
        let frames = frame.read_audio(&mut out_l, &mut out_r);
        assert_eq!(frames, 128);
        assert_eq!(out_l, left);
        assert_eq!(out_r, right);
    }

    #[test]
    fn a_parameter_move_survives_the_round_trip() {
        let frame = Frame::set_param(7, 0.25);
        assert_eq!(frame.read_set_param(), Some((7, 0.25)));
    }

    #[test]
    fn an_activate_carries_the_rate_and_the_block_size() {
        let frame = Frame::activate(44_100.0, 512);
        assert_eq!(frame.read_activate(), Some((44_100.0, 512)));
    }

    #[test]
    fn several_frames_read_back_in_order() {
        let mut bytes = Vec::new();
        Frame::set_param(1, 0.5).write(&mut bytes).unwrap();
        Frame::new(RES_OK, Vec::new()).write(&mut bytes).unwrap();
        Frame::audio(&[1.0, 2.0], &[3.0, 4.0])
            .write(&mut bytes)
            .unwrap();

        let mut cursor = bytes.as_slice();
        assert_eq!(
            Frame::read(&mut cursor).unwrap().unwrap().kind,
            REQ_SET_PARAM
        );
        assert_eq!(Frame::read(&mut cursor).unwrap().unwrap().kind, RES_OK);
        assert_eq!(Frame::read(&mut cursor).unwrap().unwrap().kind, REQ_AUDIO);
        assert_eq!(Frame::read(&mut cursor).unwrap(), None);
    }

    #[test]
    fn a_stream_that_stops_half_way_is_an_end_not_an_error() {
        let mut bytes = Vec::new();
        Frame::new(REQ_SET_STATE, vec![9; 32])
            .write(&mut bytes)
            .unwrap();
        bytes.truncate(20); // cut mid-payload
        let mut cursor = bytes.as_slice();
        assert_eq!(Frame::read(&mut cursor).unwrap(), None);
    }

    #[test]
    fn a_frame_claiming_to_be_enormous_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&REQ_AUDIO.to_le_bytes());
        bytes.extend_from_slice(&(MAX_FRAME_BYTES as u32 + 1).to_le_bytes());
        let mut cursor = bytes.as_slice();
        assert!(Frame::read(&mut cursor).is_err());
    }
}
