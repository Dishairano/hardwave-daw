//! OSC (Open Sound Control) message parser + serializer. Supports
//! the common argument types (int, float, string) and addresses
//! with standard OSC 1.0 framing — 4-byte-aligned null-terminated
//! strings, big-endian integers/floats.
//!
//! Not a full OSC 1.0 implementation (no bundles, timestamps, or
//! blob / MIDI / color argument types) but covers the ~90% of
//! OSC traffic that real music software actually sends.

use std::io::Cursor;

/// One OSC argument value.
#[derive(Debug, Clone, PartialEq)]
pub enum OscArg {
    Int32(i32),
    Float32(f32),
    String(String),
}

/// An OSC message — address pattern + typed argument list.
#[derive(Debug, Clone, PartialEq)]
pub struct OscMessage {
    pub address: String,
    pub args: Vec<OscArg>,
}

/// Errors the parser and serializer can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscError {
    /// Message ended before a complete field was read.
    Truncated,
    /// Address pattern didn't start with `/`.
    InvalidAddress,
    /// Type tag string didn't start with `,`.
    InvalidTypeTag,
    /// Unknown type tag character.
    UnsupportedType(char),
    /// UTF-8 decode error on a string field.
    InvalidUtf8,
}

impl OscMessage {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            args: Vec::new(),
        }
    }

    pub fn push_int(mut self, value: i32) -> Self {
        self.args.push(OscArg::Int32(value));
        self
    }

    pub fn push_float(mut self, value: f32) -> Self {
        self.args.push(OscArg::Float32(value));
        self
    }

    pub fn push_string(mut self, value: impl Into<String>) -> Self {
        self.args.push(OscArg::String(value.into()));
        self
    }

    /// Encode this message as OSC 1.0 wire bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_osc_string(&mut out, &self.address);
        let mut tag = String::with_capacity(self.args.len() + 1);
        tag.push(',');
        for arg in &self.args {
            tag.push(match arg {
                OscArg::Int32(_) => 'i',
                OscArg::Float32(_) => 'f',
                OscArg::String(_) => 's',
            });
        }
        write_osc_string(&mut out, &tag);
        for arg in &self.args {
            match arg {
                OscArg::Int32(v) => out.extend_from_slice(&v.to_be_bytes()),
                OscArg::Float32(v) => out.extend_from_slice(&v.to_be_bytes()),
                OscArg::String(s) => write_osc_string(&mut out, s),
            }
        }
        out
    }

    /// Parse an OSC 1.0 wire-format message.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, OscError> {
        let mut cursor = Cursor::new(bytes);
        let address = read_osc_string(&mut cursor)?;
        if !address.starts_with('/') {
            return Err(OscError::InvalidAddress);
        }
        let tag = read_osc_string(&mut cursor)?;
        if !tag.starts_with(',') {
            return Err(OscError::InvalidTypeTag);
        }
        let mut args = Vec::with_capacity(tag.len() - 1);
        for ch in tag.chars().skip(1) {
            match ch {
                'i' => args.push(OscArg::Int32(read_i32_be(&mut cursor)?)),
                'f' => args.push(OscArg::Float32(read_f32_be(&mut cursor)?)),
                's' => args.push(OscArg::String(read_osc_string(&mut cursor)?)),
                other => return Err(OscError::UnsupportedType(other)),
            }
        }
        Ok(Self { address, args })
    }
}

fn pad4(len: usize) -> usize {
    (4 - (len % 4)) % 4
}

fn write_osc_string(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    let total = s.len() + 1;
    let pad = pad4(total);
    out.extend(std::iter::repeat_n(0, pad));
}

fn read_osc_string(cursor: &mut Cursor<&[u8]>) -> Result<String, OscError> {
    let data = cursor.get_ref();
    let pos = cursor.position() as usize;
    let slice = &data[pos..];
    let nul = slice
        .iter()
        .position(|&b| b == 0)
        .ok_or(OscError::Truncated)?;
    let s = std::str::from_utf8(&slice[..nul]).map_err(|_| OscError::InvalidUtf8)?;
    let owned = s.to_string();
    let total = nul + 1;
    let pad = pad4(total);
    let advance = total + pad;
    if pos + advance > data.len() {
        return Err(OscError::Truncated);
    }
    cursor.set_position((pos + advance) as u64);
    Ok(owned)
}

fn read_i32_be(cursor: &mut Cursor<&[u8]>) -> Result<i32, OscError> {
    let data = cursor.get_ref();
    let pos = cursor.position() as usize;
    if pos + 4 > data.len() {
        return Err(OscError::Truncated);
    }
    let v = i32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
    cursor.set_position((pos + 4) as u64);
    Ok(v)
}

fn read_f32_be(cursor: &mut Cursor<&[u8]>) -> Result<f32, OscError> {
    let data = cursor.get_ref();
    let pos = cursor.position() as usize;
    if pos + 4 > data.len() {
        return Err(OscError::Truncated);
    }
    let v = f32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
    cursor.set_position((pos + 4) as u64);
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_round_trips_through_bytes() {
        let msg = OscMessage::new("/hardwave/volume")
            .push_int(42)
            .push_float(0.75)
            .push_string("hello");
        let bytes = msg.to_bytes();
        let parsed = OscMessage::from_bytes(&bytes).expect("parse");
        assert_eq!(parsed, msg);
    }

    #[test]
    fn address_must_start_with_slash() {
        // Build an invalid message: address "foo" without leading slash.
        let mut bytes = Vec::new();
        write_osc_string(&mut bytes, "foo");
        write_osc_string(&mut bytes, ",i");
        bytes.extend_from_slice(&42_i32.to_be_bytes());
        let err = OscMessage::from_bytes(&bytes).unwrap_err();
        assert_eq!(err, OscError::InvalidAddress);
    }

    #[test]
    fn type_tag_must_start_with_comma() {
        let mut bytes = Vec::new();
        write_osc_string(&mut bytes, "/foo");
        write_osc_string(&mut bytes, "if"); // missing leading comma
        bytes.extend_from_slice(&42_i32.to_be_bytes());
        bytes.extend_from_slice(&0.5_f32.to_be_bytes());
        let err = OscMessage::from_bytes(&bytes).unwrap_err();
        assert_eq!(err, OscError::InvalidTypeTag);
    }

    #[test]
    fn unsupported_type_tag_returns_error() {
        let mut bytes = Vec::new();
        write_osc_string(&mut bytes, "/foo");
        write_osc_string(&mut bytes, ",z");
        let err = OscMessage::from_bytes(&bytes).unwrap_err();
        assert!(matches!(err, OscError::UnsupportedType('z')));
    }

    #[test]
    fn truncated_message_returns_error() {
        let mut bytes = Vec::new();
        write_osc_string(&mut bytes, "/foo");
        write_osc_string(&mut bytes, ",i");
        // Missing the 4-byte int payload.
        let err = OscMessage::from_bytes(&bytes).unwrap_err();
        assert_eq!(err, OscError::Truncated);
    }

    #[test]
    fn serialize_address_pads_to_4_byte_boundary() {
        // "/foo" is 4 bytes, then NUL + 3 pad = 8 total bytes.
        let msg = OscMessage::new("/foo");
        let bytes = msg.to_bytes();
        // Address block = 8, type tag ","\0\0\0 = 4. Total = 12.
        assert_eq!(bytes.len(), 12);
    }

    #[test]
    fn multiple_args_preserve_order() {
        let msg = OscMessage::new("/track/1/volume")
            .push_float(0.5)
            .push_int(127)
            .push_float(-0.25);
        let parsed = OscMessage::from_bytes(&msg.to_bytes()).unwrap();
        assert_eq!(parsed.args[0], OscArg::Float32(0.5));
        assert_eq!(parsed.args[1], OscArg::Int32(127));
        assert_eq!(parsed.args[2], OscArg::Float32(-0.25));
    }

    #[test]
    fn string_args_handle_varying_lengths() {
        for s in ["a", "ab", "abc", "abcd", "hello world"] {
            let msg = OscMessage::new("/test").push_string(s);
            let parsed = OscMessage::from_bytes(&msg.to_bytes()).unwrap();
            assert_eq!(parsed.args[0], OscArg::String(s.to_string()));
        }
    }
}
