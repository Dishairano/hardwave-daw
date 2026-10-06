//! Telling a 32-bit plug-in from a 64-bit one.
//!
//! A 64-bit process cannot load a 32-bit plug-in, and what the user
//! sees when it tries is a plug-in that "does not work" with no
//! reason given. Plenty of free plug-ins from the 2000s were never
//! rebuilt, and hard dance is full of them.
//!
//! The architecture is in the file's own header, so it can be read
//! without loading anything.

use std::path::Path;

/// What a plug-in binary was built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryArch {
    X86,
    X86_64,
    Arm64,
    /// A file we can read but do not recognise, or one we cannot read.
    Unknown,
}

impl BinaryArch {
    /// Whether this process can load it directly.
    pub fn matches_host(&self) -> bool {
        match self {
            BinaryArch::X86 => cfg!(target_arch = "x86"),
            BinaryArch::X86_64 => cfg!(target_arch = "x86_64"),
            BinaryArch::Arm64 => cfg!(target_arch = "aarch64"),
            // An unknown one is tried rather than refused: being wrong
            // about the header should not keep a working plug-in out.
            BinaryArch::Unknown => true,
        }
    }
}

/// Read the architecture out of the first bytes of a binary.
pub fn read_arch(bytes: &[u8]) -> BinaryArch {
    // Windows: MZ, then the PE header's offset at 0x3C, then the
    // machine word two bytes after "PE\0\0".
    if bytes.len() > 0x40 && bytes[0] == b'M' && bytes[1] == b'Z' {
        let offset =
            u32::from_le_bytes([bytes[0x3C], bytes[0x3D], bytes[0x3E], bytes[0x3F]]) as usize;
        if bytes.len() >= offset + 6 && &bytes[offset..offset + 4] == b"PE\0\0" {
            let machine = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]);
            return match machine {
                0x014C => BinaryArch::X86,
                0x8664 => BinaryArch::X86_64,
                0xAA64 => BinaryArch::Arm64,
                _ => BinaryArch::Unknown,
            };
        }
        return BinaryArch::Unknown;
    }
    // ELF: the class byte says 32 or 64, the machine word says which.
    if bytes.len() > 20 && &bytes[0..4] == b"\x7FELF" {
        let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
        return match machine {
            0x03 => BinaryArch::X86,
            0x3E => BinaryArch::X86_64,
            0xB7 => BinaryArch::Arm64,
            _ => BinaryArch::Unknown,
        };
    }
    BinaryArch::Unknown
}

/// The architecture of a plug-in on disk.
///
/// A VST3 or CLAP can be a folder with the binary inside it, which is
/// how every VST3 on Windows is shipped now, so the bundle layout is
/// walked rather than the folder being read as a file.
pub fn plugin_arch(path: &Path) -> BinaryArch {
    let binary = resolve_binary(path);
    let Ok(mut file) = std::fs::File::open(binary) else {
        return BinaryArch::Unknown;
    };
    use std::io::Read;
    let mut head = vec![0u8; 1024];
    let Ok(read) = file.read(&mut head) else {
        return BinaryArch::Unknown;
    };
    head.truncate(read);
    read_arch(&head)
}

/// The actual binary inside a plug-in, bundle or not.
pub fn resolve_binary(path: &Path) -> std::path::PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    // A VST3 bundle: Contents/<arch>/<name>.vst3
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    for arch in [
        "x86_64-win",
        "x86-win",
        "arm64-win",
        "x86_64-linux",
        "MacOS",
    ] {
        let candidate = path.join("Contents").join(arch).join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x100];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        let offset: u32 = 0x80;
        bytes[0x3C..0x40].copy_from_slice(&offset.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn a_thirty_two_bit_windows_plug_in_is_recognised() {
        assert_eq!(read_arch(&pe(0x014C)), BinaryArch::X86);
    }

    #[test]
    fn a_sixty_four_bit_windows_plug_in_is_recognised() {
        assert_eq!(read_arch(&pe(0x8664)), BinaryArch::X86_64);
        assert_eq!(read_arch(&pe(0xAA64)), BinaryArch::Arm64);
    }

    #[test]
    fn an_elf_is_read_as_well() {
        let mut bytes = vec![0u8; 64];
        bytes[0..4].copy_from_slice(b"\x7FELF");
        bytes[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
        assert_eq!(read_arch(&bytes), BinaryArch::X86_64);
    }

    #[test]
    fn something_that_is_not_a_binary_is_unknown_and_still_tried() {
        assert_eq!(read_arch(b"this is a text file"), BinaryArch::Unknown);
        assert_eq!(read_arch(&[]), BinaryArch::Unknown);
        assert!(
            BinaryArch::Unknown.matches_host(),
            "being wrong about the header must not keep a working plug-in out"
        );
    }

    #[test]
    fn a_header_that_points_past_the_end_does_not_panic() {
        let mut bytes = pe(0x8664);
        bytes[0x3C..0x40].copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
        assert_eq!(read_arch(&bytes), BinaryArch::Unknown);
    }

    #[test]
    fn this_process_matches_its_own_architecture() {
        #[cfg(target_arch = "x86_64")]
        {
            assert!(BinaryArch::X86_64.matches_host());
            assert!(!BinaryArch::X86.matches_host());
        }
        #[cfg(target_arch = "aarch64")]
        {
            assert!(BinaryArch::Arm64.matches_host());
            assert!(!BinaryArch::X86.matches_host());
        }
    }
}
