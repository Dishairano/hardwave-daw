//! Windows paths as people read them.
//!
//! Windows can name a file in two ways: `C:\Samples\kick.wav`, and the
//! "verbatim" form `\\?\C:\Samples\kick.wav` that `std::fs::canonicalize`
//! and the app's own install folder come back in. Both open the same
//! file, but the second showed up in the browser and in saved songs as
//! `\\?\C:\...`. Paths made here are given in the first form, and songs
//! saved with the second are read back in the first.

/// The longest path Windows takes without the verbatim prefix.
const MAX_PATH: usize = 260;

/// Names Windows keeps for devices: `NUL`, `CON.wav` and the like only
/// mean the file in the verbatim form.
fn is_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").trim_end();
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && upper.len() == 4
        && upper.as_bytes()[3].is_ascii_digit())
}

/// Whether every part of a path (after its drive or share) means the same
/// file without the verbatim prefix: no `.` or `..`, no name Windows
/// would trim or read as a device, and no character it refuses.
fn plain_parts_mean_the_same(parts: &str) -> bool {
    parts.split('\\').all(|c| {
        c != "."
            && c != ".."
            && !c.ends_with('.')
            && !c.ends_with(' ')
            && !is_device_name(c)
            && !c
                .chars()
                .any(|ch| ch.is_control() || "<>:\"|?*/".contains(ch))
    })
}

/// `path` without the `\\?\` prefix, when it names the same file without
/// it: `\\?\C:\x` reads `C:\x` and `\\?\UNC\server\share\x` reads
/// `\\server\share\x`. None for any other path, and for a verbatim one
/// that needs its prefix (too long, or with a part only the prefix
/// allows).
pub fn without_verbatim_prefix(path: &str) -> Option<String> {
    let rest = path.strip_prefix(r"\\?\")?;
    let (plain, parts) = match rest.get(..4) {
        Some(unc) if unc.eq_ignore_ascii_case(r"UNC\") => {
            let tail = &rest[4..];
            // A share needs a server and a share name.
            let mut names = tail.splitn(3, '\\');
            let (server, share) = (names.next()?, names.next()?);
            if server.is_empty() || share.is_empty() {
                return None;
            }
            (format!(r"\\{tail}"), tail)
        }
        _ => {
            let bytes = rest.as_bytes();
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && bytes[2] == b'\\';
            if !drive {
                return None;
            }
            (rest.to_string(), &rest[3..])
        }
    };
    let parts = parts.trim_end_matches('\\');
    if plain.len() >= MAX_PATH || (!parts.is_empty() && !plain_parts_mean_the_same(parts)) {
        return None;
    }
    Some(plain)
}

/// `path` as people read it: without the verbatim prefix where it means
/// the same file without it, else as it is.
pub fn plain(path: &str) -> String {
    without_verbatim_prefix(path).unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_path_loses_its_prefix() {
        assert_eq!(
            plain(r"\\?\C:\HWTest\wave\drums\Clap 1.wav"),
            r"C:\HWTest\wave\drums\Clap 1.wav"
        );
        assert_eq!(plain(r"\\?\d:\"), r"d:\");
    }

    #[test]
    fn a_share_keeps_its_two_slashes() {
        assert_eq!(
            plain(r"\\?\UNC\nas\samples\kick.wav"),
            r"\\nas\samples\kick.wav"
        );
        assert_eq!(without_verbatim_prefix(r"\\?\UNC\nas"), None);
    }

    #[test]
    fn other_paths_are_left_as_they_are() {
        for path in [
            r"C:\Samples\kick.wav",
            r"\\nas\samples\kick.wav",
            "/home/me/kick.wav",
            "Song Samples/kick.wav",
            r"\\?\Volume{1b3b1146-4076-11e1-84aa-806e6f6e6963}\kick.wav",
            r"\\?\C:",
            "",
        ] {
            assert_eq!(plain(path), path);
        }
    }

    #[test]
    fn a_path_that_needs_its_prefix_keeps_it() {
        let long = format!(r"\\?\C:\{}\kick.wav", "a".repeat(300));
        for path in [
            long.as_str(),
            r"\\?\C:\Samples\NUL",
            r"\\?\C:\Samples\con.wav",
            r"\\?\C:\Samples\COM1",
            r"\\?\C:\Samples\..\kick.wav",
            r"\\?\C:\Samples\kick.",
            r"\\?\C:\Samples\kick ",
            r"\\?\C:\Samples/kick.wav",
            r"\\?\C:\Sam:ples\kick.wav",
        ] {
            assert_eq!(without_verbatim_prefix(path), None, "{path}");
            assert_eq!(plain(path), path);
        }
        // A name that only starts like a device is an ordinary name.
        assert_eq!(
            plain(r"\\?\C:\Samples\Console.wav"),
            r"C:\Samples\Console.wav"
        );
    }
}
