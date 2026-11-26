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

use crate::types::{ParameterInfo, TransportInfo};
use hardwave_midi::{MidiEvent, NoteExpressionKind};

/// Frames the DAW sends to the child.
pub const REQ_AUDIO: u32 = 1;
pub const REQ_SET_PARAM: u32 = 2;
pub const REQ_SET_STATE: u32 = 3;
pub const REQ_GET_STATE: u32 = 4;
pub const REQ_ACTIVATE: u32 = 5;
pub const REQ_SHUTDOWN: u32 = 6;
/// A block with the notes that fall in it and where the song is: what an
/// instrument needs, and what a tempo-synced effect needs.
pub const REQ_PROCESS: u32 = 7;
/// Ask for the parameter list, answered with `RES_PARAMS`.
pub const REQ_PARAMS: u32 = 8;
/// Show the plug-in's own window, or put it away.
pub const REQ_OPEN_EDITOR: u32 = 9;
pub const REQ_CLOSE_EDITOR: u32 = 10;
/// A question about the plug-in that is answered at once with
/// `RES_ANSWER`: how a value reads, a parameter's choices, its presets.
/// Never sent from the audio thread.
pub const REQ_ASK: u32 = 11;
/// Play one of the plug-in's own presets, by its place in the list.
pub const REQ_LOAD_PRESET: u32 = 12;

/// Frames the child sends back.
pub const RES_AUDIO: u32 = 101;
pub const RES_STATE: u32 = 102;
pub const RES_OK: u32 = 103;
pub const RES_ERROR: u32 = 104;
/// The parameter list, with where each one stands.
pub const RES_PARAMS: u32 = 105;
/// Knobs moved in the plug-in's own window, sent before the block they
/// were noticed in, so the host can record them and show where they are.
pub const RES_PARAM_MOVES: u32 = 106;
/// The plug-in's window opened (payload 1) or closed (payload 0), which
/// it can do by itself when the person closes it.
pub const RES_EDITOR: u32 = 107;
/// What the host needs to know about the plug-in beyond its parameters:
/// the delay it adds, and the id a `.vstpreset` names it by. Sent when
/// the plug-in has loaded and again whenever its delay changes.
pub const RES_INFO: u32 = 108;
/// The answer to a `REQ_ASK`.
pub const RES_ANSWER: u32 = 109;

/// What a plug-in in its own process says about itself (`RES_INFO`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChildInfo {
    /// The delay the plug-in itself adds, in samples.
    pub latency: u32,
    /// A VST3 plug-in's class id.
    pub class_id: Option<[u8; 16]>,
}

/// A question for the plug-in's process (`REQ_ASK`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Ask {
    /// How this value of this parameter reads ("1.2 kHz").
    Text(u32, f64),
    /// The choices of a parameter that picks one of a few.
    Options(u32),
    /// The presets that ship inside the plug-in.
    Presets,
}

/// The answer to an [`Ask`] (`RES_ANSWER`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Answer {
    Text(Option<String>),
    Options(Option<Vec<String>>),
    Presets(Vec<String>),
}

/// The most notes and controller moves one block carries. More in one
/// block than this is not playing, it is a stuck stream.
pub const MAX_BLOCK_EVENTS: usize = 1024;

/// Bytes per MIDI event in a process frame.
const EVENT_BYTES: usize = 12;
/// Bytes of the process frame before its events.
const PROCESS_HEADER: usize = 44;

fn put_event(out: &mut Vec<u8>, ev: &MidiEvent) {
    let (kind, channel, data, timing, value) = match *ev {
        MidiEvent::NoteOn {
            timing,
            channel,
            note,
            velocity,
        } => (1u8, channel, note, timing, velocity),
        MidiEvent::NoteOff {
            timing,
            channel,
            note,
            velocity,
        } => (2, channel, note, timing, velocity),
        MidiEvent::ControlChange {
            timing,
            channel,
            cc,
            value,
        } => (3, channel, cc, timing, value),
        MidiEvent::PitchBend {
            timing,
            channel,
            value,
        } => (4, channel, 0, timing, value),
        MidiEvent::Aftertouch {
            timing,
            channel,
            note,
            pressure,
        } => (5, channel, note, timing, pressure),
        MidiEvent::ChannelPressure {
            timing,
            channel,
            pressure,
        } => (6, channel, 0, timing, pressure),
        MidiEvent::NoteExpression {
            timing,
            channel,
            note,
            kind,
            value,
        } => {
            let which = match kind {
                NoteExpressionKind::Tuning => 0,
                NoteExpressionKind::Pan => 1,
            };
            out.extend_from_slice(&[7, channel, note, which]);
            out.extend_from_slice(&timing.to_le_bytes());
            out.extend_from_slice(&value.to_le_bytes());
            return;
        }
    };
    out.extend_from_slice(&[kind, channel, data, 0]);
    out.extend_from_slice(&timing.to_le_bytes());
    out.extend_from_slice(&value.to_le_bytes());
}

fn get_event(b: &[u8]) -> Option<MidiEvent> {
    let (kind, channel, data) = (b[0], b[1], b[2]);
    let timing = u32::from_le_bytes(b[4..8].try_into().ok()?);
    let value = f32::from_le_bytes(b[8..12].try_into().ok()?);
    Some(match kind {
        1 => MidiEvent::NoteOn {
            timing,
            channel,
            note: data,
            velocity: value,
        },
        2 => MidiEvent::NoteOff {
            timing,
            channel,
            note: data,
            velocity: value,
        },
        3 => MidiEvent::ControlChange {
            timing,
            channel,
            cc: data,
            value,
        },
        4 => MidiEvent::PitchBend {
            timing,
            channel,
            value,
        },
        5 => MidiEvent::Aftertouch {
            timing,
            channel,
            note: data,
            pressure: value,
        },
        6 => MidiEvent::ChannelPressure {
            timing,
            channel,
            pressure: value,
        },
        7 => MidiEvent::NoteExpression {
            timing,
            channel,
            note: data,
            kind: match b[3] {
                0 => NoteExpressionKind::Tuning,
                1 => NoteExpressionKind::Pan,
                _ => return None,
            },
            value,
        },
        _ => return None,
    })
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le_u64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn le_f64(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// A block bigger than this is a mistake or a corrupted stream, and
/// allocating on it would be the crash we are trying to avoid.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// Every frame starts with this. A plug-in that prints to the stream,
/// or a stream that slipped, shows up as a frame without it, which ends
/// the connection there and then, instead of being read as a length and
/// waited on.
pub const FRAME_MAGIC: u32 = 0x4857_4652; // "HWFR"

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

    /// A block for the plug-in: its audio, the MIDI that falls in it, and
    /// the transport at its start. Written into `payload`, which is reused
    /// block after block so a steady stream allocates nothing.
    ///
    /// A sidechain, when the slot has one routed, follows the main audio
    /// as a second stereo pair of the same length. Without it a compressor
    /// keyed from the kick heard no key once it ran in its own process.
    pub fn write_process(
        payload: &mut Vec<u8>,
        left: &[f32],
        right: &[f32],
        sidechain: Option<(&[f32], &[f32])>,
        midi: &[MidiEvent],
        transport: &TransportInfo,
    ) {
        let frames = left.len().min(right.len());
        let events = midi.len().min(MAX_BLOCK_EVENTS);
        payload.clear();
        payload.extend_from_slice(&(frames as u32).to_le_bytes());
        payload.extend_from_slice(&(events as u32).to_le_bytes());
        payload.extend_from_slice(&u32::from(transport.playing).to_le_bytes());
        payload.extend_from_slice(&transport.tempo.to_le_bytes());
        payload.extend_from_slice(&transport.position_beats.to_le_bytes());
        payload.extend_from_slice(&transport.time_sig.0.to_le_bytes());
        payload.extend_from_slice(&transport.time_sig.1.to_le_bytes());
        payload.extend_from_slice(&transport.position_samples.to_le_bytes());
        for ev in &midi[..events] {
            put_event(payload, ev);
        }
        for i in 0..frames {
            payload.extend_from_slice(&left[i].to_le_bytes());
            payload.extend_from_slice(&right[i].to_le_bytes());
        }
        if let Some((key_left, key_right)) = sidechain {
            for i in 0..frames {
                let l = key_left.get(i).copied().unwrap_or(0.0);
                let r = key_right.get(i).copied().unwrap_or(l);
                payload.extend_from_slice(&l.to_le_bytes());
                payload.extend_from_slice(&r.to_le_bytes());
            }
        }
    }

    /// Read a process frame into buffers the right size and a MIDI list
    /// with room. Returns the frames written, the transport and whether a
    /// sidechain came with it (read into `sidechain`, when there is room),
    /// or None for a frame that does not add up. The sample rate is the
    /// one the plug-in was activated at, so it is not carried.
    pub fn read_process(
        &self,
        left: &mut [f32],
        right: &mut [f32],
        sidechain: Option<(&mut [f32], &mut [f32])>,
        midi: &mut Vec<MidiEvent>,
        sample_rate: f64,
    ) -> Option<(usize, TransportInfo, bool)> {
        let b = &self.payload;
        let frames = le_u32(b, 0)? as usize;
        let events = le_u32(b, 4)? as usize;
        if events > MAX_BLOCK_EVENTS {
            return None;
        }
        let transport = TransportInfo {
            playing: le_u32(b, 8)? != 0,
            tempo: le_f64(b, 12)?,
            position_beats: le_f64(b, 20)?,
            time_sig: (le_u32(b, 28)?, le_u32(b, 32)?),
            sample_rate,
            position_samples: le_u64(b, 36)?,
        };
        let audio_at = PROCESS_HEADER + events * EVENT_BYTES;
        let plain = audio_at.checked_add(frames.checked_mul(8)?)?;
        let keyed = audio_at.checked_add(frames.checked_mul(16)?)?;
        let has_key = frames > 0 && b.len() == keyed;
        if (b.len() != plain && !has_key) || frames > left.len() || frames > right.len() {
            return None;
        }
        midi.clear();
        for e in 0..events {
            let at = PROCESS_HEADER + e * EVENT_BYTES;
            if let Some(ev) = get_event(&b[at..at + EVENT_BYTES]) {
                midi.push(ev);
            }
        }
        for i in 0..frames {
            let at = audio_at + i * 8;
            left[i] = f32::from_le_bytes(b[at..at + 4].try_into().ok()?);
            right[i] = f32::from_le_bytes(b[at + 4..at + 8].try_into().ok()?);
        }
        let mut key_read = false;
        if has_key {
            if let Some((key_left, key_right)) = sidechain {
                if frames <= key_left.len() && frames <= key_right.len() {
                    for i in 0..frames {
                        let at = plain + i * 8;
                        key_left[i] = f32::from_le_bytes(b[at..at + 4].try_into().ok()?);
                        key_right[i] = f32::from_le_bytes(b[at + 4..at + 8].try_into().ok()?);
                    }
                    key_read = true;
                }
            }
        }
        Some((frames, transport, key_read))
    }

    /// What the plug-in says about itself.
    pub fn info(info: &ChildInfo) -> Self {
        Self::new(RES_INFO, serde_json::to_vec(info).unwrap_or_default())
    }

    pub fn read_info(&self) -> Option<ChildInfo> {
        serde_json::from_slice(&self.payload).ok()
    }

    /// A question for the plug-in's process.
    pub fn ask(ask: &Ask) -> Self {
        Self::new(REQ_ASK, serde_json::to_vec(ask).unwrap_or_default())
    }

    pub fn read_ask(&self) -> Option<Ask> {
        serde_json::from_slice(&self.payload).ok()
    }

    pub fn answer(answer: &Answer) -> Self {
        Self::new(RES_ANSWER, serde_json::to_vec(answer).unwrap_or_default())
    }

    pub fn read_answer(&self) -> Option<Answer> {
        serde_json::from_slice(&self.payload).ok()
    }

    /// Play the preset at this place in the plug-in's own list.
    pub fn load_preset(index: u32) -> Self {
        Self::new(REQ_LOAD_PRESET, index.to_le_bytes().to_vec())
    }

    pub fn read_preset_index(&self) -> Option<u32> {
        le_u32(&self.payload, 0)
    }

    /// The parameter list, each with where it stands now.
    pub fn params(list: &[(ParameterInfo, f64)]) -> Self {
        Self::new(RES_PARAMS, serde_json::to_vec(list).unwrap_or_default())
    }

    pub fn read_params(&self) -> Option<Vec<(ParameterInfo, f64)>> {
        serde_json::from_slice(&self.payload).ok()
    }

    /// Knobs moved in the plug-in's window: which, and to where.
    pub fn param_moves(moves: &[(u32, f64)]) -> Self {
        let mut payload = Vec::with_capacity(moves.len() * 12);
        for (id, value) in moves {
            payload.extend_from_slice(&id.to_le_bytes());
            payload.extend_from_slice(&value.to_le_bytes());
        }
        Self::new(RES_PARAM_MOVES, payload)
    }

    pub fn read_param_moves(&self) -> Vec<(u32, f64)> {
        self.payload
            .as_chunks::<12>()
            .0
            .iter()
            .filter_map(|c| {
                Some((
                    u32::from_le_bytes(c[0..4].try_into().ok()?),
                    f64::from_le_bytes(c[4..12].try_into().ok()?),
                ))
            })
            .collect()
    }

    /// Show the plug-in's window. `owner` is the host's main window, as a
    /// number, so the child can put its window in front of it.
    pub fn open_editor(owner: u64) -> Self {
        Self::new(REQ_OPEN_EDITOR, owner.to_le_bytes().to_vec())
    }

    pub fn read_owner(&self) -> u64 {
        le_u64(&self.payload, 0).unwrap_or(0)
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
        out.write_all(&FRAME_MAGIC.to_le_bytes())?;
        out.write_all(&self.kind.to_le_bytes())?;
        out.write_all(&(self.payload.len() as u32).to_le_bytes())?;
        out.write_all(&self.payload)?;
        out.flush()
    }

    /// Read one frame. An end of stream is `Ok(None)`, which is how a
    /// child that has gone away is noticed.
    pub fn read(input: &mut impl Read) -> std::io::Result<Option<Frame>> {
        let mut header = [0u8; 12];
        match input.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
        if magic != FRAME_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "something other than a frame arrived on the stream",
            ));
        }
        let kind = u32::from_le_bytes(header[4..8].try_into().unwrap());
        let len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
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
    fn printed_text_in_the_stream_ends_it_instead_of_being_waited_on() {
        let mut bytes = b"Loading presets...\n".to_vec();
        Frame::new(RES_OK, Vec::new()).write(&mut bytes).unwrap();
        let mut cursor = bytes.as_slice();
        assert!(Frame::read(&mut cursor).is_err());
    }

    #[test]
    fn a_frame_claiming_to_be_enormous_is_refused() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
        bytes.extend_from_slice(&REQ_AUDIO.to_le_bytes());
        bytes.extend_from_slice(&(MAX_FRAME_BYTES as u32 + 1).to_le_bytes());
        let mut cursor = bytes.as_slice();
        assert!(Frame::read(&mut cursor).is_err());
    }

    #[test]
    fn a_block_carries_its_notes_and_the_transport() {
        let left: Vec<f32> = (0..64).map(|n| n as f32 / 64.0).collect();
        let right: Vec<f32> = left.iter().map(|s| -s).collect();
        let midi = [
            MidiEvent::NoteOn {
                timing: 3,
                channel: 1,
                note: 60,
                velocity: 0.75,
            },
            MidiEvent::ControlChange {
                timing: 10,
                channel: 0,
                cc: 74,
                value: 0.5,
            },
            MidiEvent::PitchBend {
                timing: 20,
                channel: 2,
                value: -0.25,
            },
            MidiEvent::NoteExpression {
                timing: 3,
                channel: 1,
                note: 60,
                kind: NoteExpressionKind::Pan,
                value: -0.5,
            },
            MidiEvent::NoteOff {
                timing: 63,
                channel: 1,
                note: 60,
                velocity: 0.0,
            },
        ];
        let transport = TransportInfo {
            playing: true,
            tempo: 150.0,
            position_beats: 33.5,
            time_sig: (7, 8),
            sample_rate: 44_100.0,
            position_samples: 123_456_789,
        };
        let mut payload = Vec::new();
        Frame::write_process(&mut payload, &left, &right, None, &midi, &transport);
        let frame = Frame::new(REQ_PROCESS, payload);
        let (mut l, mut r, mut events) = (vec![0.0; 64], vec![0.0; 64], Vec::with_capacity(8));
        let (frames, back, keyed) = frame
            .read_process(&mut l, &mut r, None, &mut events, 44_100.0)
            .expect("reads");
        assert_eq!(frames, 64);
        assert!(!keyed, "no sidechain was sent");
        assert_eq!((l, r), (left, right));
        assert_eq!(back, transport);
        assert_eq!(format!("{events:?}"), format!("{midi:?}"));
    }

    #[test]
    fn a_process_frame_that_does_not_add_up_is_refused() {
        let mut payload = Vec::new();
        Frame::write_process(
            &mut payload,
            &[0.0; 16],
            &[0.0; 16],
            None,
            &[],
            &TransportInfo::default(),
        );
        payload.pop();
        let frame = Frame::new(REQ_PROCESS, payload);
        let (mut l, mut r, mut events) = (vec![0.0; 16], vec![0.0; 16], Vec::new());
        assert!(frame
            .read_process(&mut l, &mut r, None, &mut events, 48_000.0)
            .is_none());
        // Nor a block bigger than the buffers it is read into.
        let mut payload = Vec::new();
        Frame::write_process(
            &mut payload,
            &[0.0; 32],
            &[0.0; 32],
            None,
            &[],
            &TransportInfo::default(),
        );
        let frame = Frame::new(REQ_PROCESS, payload);
        assert!(frame
            .read_process(&mut l, &mut r, None, &mut events, 48_000.0)
            .is_none());
    }

    #[test]
    fn a_block_carries_its_sidechain_beside_the_main_audio() {
        let left: Vec<f32> = (0..32).map(|n| n as f32).collect();
        let right: Vec<f32> = left.iter().map(|s| -s).collect();
        let key_left: Vec<f32> = (0..32).map(|n| 100.0 + n as f32).collect();
        let key_right: Vec<f32> = key_left.iter().map(|s| -s).collect();
        let mut payload = Vec::new();
        Frame::write_process(
            &mut payload,
            &left,
            &right,
            Some((&key_left, &key_right)),
            &[],
            &TransportInfo::default(),
        );
        let frame = Frame::new(REQ_PROCESS, payload);
        let (mut l, mut r) = (vec![0.0; 32], vec![0.0; 32]);
        let (mut kl, mut kr) = (vec![0.0; 32], vec![0.0; 32]);
        let mut events = Vec::new();
        let (frames, _, keyed) = frame
            .read_process(
                &mut l,
                &mut r,
                Some((&mut kl, &mut kr)),
                &mut events,
                48_000.0,
            )
            .expect("reads");
        assert_eq!(frames, 32);
        assert!(keyed, "the sidechain came with it");
        assert_eq!((l, r), (left.clone(), right.clone()));
        assert_eq!((kl, kr), (key_left, key_right));
        // A reader with no room for a sidechain still gets the main audio.
        let (mut l, mut r) = (vec![0.0; 32], vec![0.0; 32]);
        let (frames, _, keyed) = frame
            .read_process(&mut l, &mut r, None, &mut events, 48_000.0)
            .expect("reads");
        assert_eq!((frames, keyed), (32, false));
        assert_eq!((l, r), (left, right));
    }

    #[test]
    fn what_the_plug_in_says_about_itself_and_its_answers_survive_the_round_trip() {
        let info = ChildInfo {
            latency: 512,
            class_id: Some([7; 16]),
        };
        assert_eq!(Frame::info(&info).read_info(), Some(info));
        for ask in [Ask::Text(3, 0.25), Ask::Options(9), Ask::Presets] {
            assert_eq!(Frame::ask(&ask).read_ask(), Some(ask));
        }
        for answer in [
            Answer::Text(Some("1.2 kHz".into())),
            Answer::Options(None),
            Answer::Presets(vec!["Init".into(), "Screech".into()]),
        ] {
            assert_eq!(Frame::answer(&answer).read_answer(), Some(answer));
        }
        assert_eq!(Frame::load_preset(41).read_preset_index(), Some(41));
        // Bytes that are not an answer are no answer, not a panic.
        assert_eq!(Frame::new(RES_ANSWER, vec![1, 2, 3]).read_answer(), None);
    }

    #[test]
    fn the_parameter_list_and_knob_moves_survive_the_round_trip() {
        let info = ParameterInfo {
            id: 12,
            name: "Cutoff".into(),
            default_value: 0.5,
            min: 0.0,
            max: 1.0,
            unit: "Hz".into(),
            automatable: true,
        };
        let back = Frame::params(&[(info.clone(), 0.3)])
            .read_params()
            .expect("list");
        assert_eq!(back.len(), 1);
        assert_eq!(
            (back[0].0.id, back[0].0.name.as_str(), back[0].1),
            (12, "Cutoff", 0.3)
        );
        let moves = [(12, 0.9), (3, 0.125)];
        assert_eq!(
            Frame::param_moves(&moves).read_param_moves(),
            moves.to_vec()
        );
        assert_eq!(
            Frame::open_editor(0xDEAD_BEEF_0042).read_owner(),
            0xDEAD_BEEF_0042
        );
    }
}
