//! The `.vstpreset` file: the preset format every VST3 host reads and
//! writes, so a preset made in Cubase, Studio One or the plug-in maker's
//! own library loads here, and one saved here loads there.
//!
//! Layout (all little-endian): a 48-byte header of `VST3`, a version, the
//! plug-in's class id as 32 hex characters and the offset of the chunk
//! list; then the chunks; then the list itself, `List` and a count, and per
//! chunk a 4-letter id, offset and size. `Comp` is the plug-in's state,
//! `Cont` its editor's, `Info` an optional XML description.

/// What a `.vstpreset` carries that a host uses.
#[derive(Debug, Clone, PartialEq)]
pub struct VstPreset {
    /// The class id as written in the file: 32 hex characters.
    pub class_id: String,
    /// The plug-in's state, as its `getState` gives it.
    pub component: Vec<u8>,
    /// The editor's state, when the file has one.
    pub controller: Option<Vec<u8>>,
}

const HEADER: usize = 4 + 4 + 32 + 8;

/// Read a `.vstpreset`. Every offset and size is checked against the file,
/// so a broken or hostile file is refused rather than read past its end.
pub fn parse(bytes: &[u8]) -> Result<VstPreset, String> {
    let bad = || "not a .vstpreset file".to_string();
    if bytes.len() < HEADER || &bytes[0..4] != b"VST3" {
        return Err(bad());
    }
    let class_id = std::str::from_utf8(&bytes[8..40])
        .map_err(|_| bad())?
        .to_ascii_uppercase();
    if !class_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let list_at = u64::from_le_bytes(bytes[40..48].try_into().unwrap()) as usize;
    let list = bytes.get(list_at..).ok_or_else(bad)?;
    if list.len() < 8 || &list[0..4] != b"List" {
        return Err(bad());
    }
    let count = u32::from_le_bytes(list[4..8].try_into().unwrap()) as usize;
    if count > 64 || list.len() < 8 + count * 20 {
        return Err(bad());
    }
    let mut component = None;
    let mut controller = None;
    for i in 0..count {
        let e = &list[8 + i * 20..8 + (i + 1) * 20];
        let offset = u64::from_le_bytes(e[4..12].try_into().unwrap()) as usize;
        let size = u64::from_le_bytes(e[12..20].try_into().unwrap()) as usize;
        let end = offset.checked_add(size).ok_or_else(bad)?;
        let data = bytes.get(offset..end).ok_or_else(bad)?.to_vec();
        match &e[0..4] {
            b"Comp" => component = Some(data),
            b"Cont" => controller = Some(data),
            _ => {}
        }
    }
    Ok(VstPreset {
        class_id,
        component: component.ok_or("the preset holds no plug-in state")?,
        controller,
    })
}

/// Write a `.vstpreset` for the plug-in with `class_id` (32 hex characters).
pub fn write(class_id: &str, component: &[u8], controller: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + component.len() + 64);
    out.extend_from_slice(b"VST3");
    out.extend_from_slice(&1u32.to_le_bytes());
    let mut id = class_id.as_bytes().to_vec();
    id.resize(32, b'0');
    out.extend_from_slice(&id);
    out.extend_from_slice(&0u64.to_le_bytes());
    let mut entries: Vec<(&[u8; 4], usize, usize)> = Vec::new();
    entries.push((b"Comp", out.len(), component.len()));
    out.extend_from_slice(component);
    if let Some(c) = controller {
        entries.push((b"Cont", out.len(), c.len()));
        out.extend_from_slice(c);
    }
    let list_at = out.len() as u64;
    out[40..48].copy_from_slice(&list_at.to_le_bytes());
    out.extend_from_slice(b"List");
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (id, offset, size) in entries {
        out.extend_from_slice(id);
        out.extend_from_slice(&(offset as u64).to_le_bytes());
        out.extend_from_slice(&(size as u64).to_le_bytes());
    }
    out
}

/// A class id as the SDK prints it. On Windows a class id is laid out like
/// a COM GUID (its first three fields little-endian), elsewhere it is plain
/// big-endian, and the printed form reads the four 32-bit words through
/// that layout. `com` picks the Windows layout.
pub fn class_id_string(tuid: &[u8; 16], com: bool) -> String {
    let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
    let (l1, l2) = if com {
        let d1 = u32::from_le_bytes([tuid[0], tuid[1], tuid[2], tuid[3]]);
        let d2 = u16::from_le_bytes([tuid[4], tuid[5]]) as u32;
        let d3 = u16::from_le_bytes([tuid[6], tuid[7]]) as u32;
        (d1, (d2 << 16) | d3)
    } else {
        (be(&tuid[0..4]), be(&tuid[4..8]))
    };
    format!(
        "{l1:08X}{l2:08X}{:08X}{:08X}",
        be(&tuid[8..12]),
        be(&tuid[12..16])
    )
}

/// Whether a preset's class id is this plug-in's, in either layout: a
/// preset written on a Mac loads on Windows.
pub fn is_for(preset_class: &str, tuid: &[u8; 16]) -> bool {
    let p = preset_class.to_ascii_uppercase();
    p == class_id_string(tuid, true) || p == class_id_string(tuid, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_preset_reads_back() {
        let file = write("0123456789ABCDEF0123456789ABCDEF", &[1, 2, 3], Some(&[9]));
        let p = parse(&file).unwrap();
        assert_eq!(p.class_id, "0123456789ABCDEF0123456789ABCDEF");
        assert_eq!(p.component, vec![1, 2, 3]);
        assert_eq!(p.controller, Some(vec![9]));
    }

    /// A broken file is refused, never read past its end.
    #[test]
    fn a_broken_file_is_refused() {
        let mut file = write("0123456789ABCDEF0123456789ABCDEF", &[1, 2, 3], None);
        assert!(parse(&file[..20]).is_err());
        let n = file.len();
        // The chunk's size pointing far past the end.
        file[n - 8..].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(parse(&file).is_err());
        assert!(parse(b"RIFF....").is_err());
    }

    /// The SDK's printed form of a class id, both layouts.
    #[test]
    fn class_ids_print_as_the_sdk_does() {
        let tuid = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        assert_eq!(
            class_id_string(&tuid, false),
            "0102030405060708090A0B0C0D0E0F10"
        );
        assert_eq!(
            class_id_string(&tuid, true),
            "0403020106050807090A0B0C0D0E0F10"
        );
        assert!(is_for("0403020106050807090a0b0c0d0e0f10", &tuid));
        assert!(!is_for("FFFFFFFF06050807090A0B0C0D0E0F10", &tuid));
    }
}
