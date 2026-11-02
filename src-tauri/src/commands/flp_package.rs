//! FL Studio's zipped loop package: File > Export > Project data files
//! makes a .zip holding the .flp and every sample it plays, side by side.
//! It is how producers hand a project to someone else, and the import
//! only took a bare .flp, so a shared project meant unzipping it by hand
//! first.
//!
//! The package is unpacked into a new folder beside the zip, named after
//! it ("Song.zip" unpacks into "Song", or "Song (2)" when that is taken),
//! and stays there: the imported clips play the samples in it. The DAW
//! keeps no projects folder of its own and "Collect samples" needs a saved
//! song, so beside the zip is where the person already looks for it, as
//! the samples of an opened DAWproject go beside it.
//!
//! A zip comes from anyone, so the whole of it is checked before a single
//! file is written, and on any refusal nothing is unpacked:
//! - every entry's path stays inside the folder: no absolute path, no
//!   drive letter, no "..", no link, no Windows device name;
//! - at most 10,000 entries and 4 GB unpacked, and no FL project over
//!   512 MB (it is read into memory whole), so a small zip that unpacks
//!   to far more (a zip bomb) is stopped;
//! - each file is streamed to disk and may not unpack to more than the
//!   zip says it holds; if one does, or anything else goes wrong while
//!   writing, the new folder is removed again.
//!
//! Only the FL projects and the audio files the DAW plays are written;
//! anything else in the zip is named in the import report instead.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// What one package may hold.
#[derive(Clone, Copy)]
pub struct Limits {
    /// Entries in the zip, folders and skipped files included.
    pub entries: usize,
    /// Bytes written, all files together.
    pub total: u64,
    /// Bytes in one FL project.
    pub project: u64,
}

pub const LIMITS: Limits = Limits {
    entries: 10_000,
    total: 4 * 1024 * 1024 * 1024,
    project: 512 * 1024 * 1024,
};

/// A package unpacked into its folder.
#[derive(Debug)]
pub struct Unpacked {
    pub folder: PathBuf,
    /// The FL projects in it, the ones nearest the top first, then in the
    /// zip's order. Never empty.
    pub projects: Vec<PathBuf>,
    /// The audio files in it, in the zip's order.
    pub samples: Vec<PathBuf>,
    /// Entries left in the zip: neither a project nor a sample.
    pub skipped: Vec<String>,
}

/// Whether `path` is a zip by its name.
pub fn is_zip(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// One file the check let through, to be written.
struct Planned {
    index: usize,
    parts: Vec<String>,
    size: u64,
    project: bool,
}

/// Unpack the package at `zip` into a new folder beside it. `progress` is
/// told how many of the files to write are done.
pub fn unpack(
    zip: &Path,
    limits: Limits,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<Unpacked, String> {
    let file = std::fs::File::open(zip).map_err(|e| format!("Could not open that zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "That file is not a zip the DAW can read".to_string())?;
    if archive.len() > limits.entries {
        return Err(format!(
            "That zip holds {} files, more than an FL project package does (at most {}); nothing was unpacked",
            archive.len(),
            limits.entries
        ));
    }

    // Everything is checked before anything is written.
    let mut planned: Vec<Planned> = Vec::new();
    let mut skipped = Vec::new();
    let mut taken = std::collections::HashSet::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index_raw(index)
            .map_err(|e| format!("That zip is damaged ({e}); nothing was unpacked"))?;
        let name = entry.name().to_string();
        let parts = safe_parts(&name)
            .map_err(|why| format!("That zip is unsafe: \"{name}\" {why}. Nothing was unpacked"))?;
        if entry.is_symlink() {
            return Err(format!(
                "That zip is unsafe: \"{name}\" is a link, which could point anywhere on this computer. Nothing was unpacked"
            ));
        }
        if entry.is_dir() || parts.is_empty() {
            continue;
        }
        let extension = parts
            .last()
            .and_then(|p| Path::new(p).extension())
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let project = extension == "flp";
        let sample = hardwave_engine::source_paths::AUDIO_EXTENSIONS.contains(&extension.as_str());
        if !project && !sample {
            skipped.push(name);
            continue;
        }
        if entry.encrypted() {
            return Err(format!(
                "\"{name}\" in that zip is password-protected; unzip it yourself and import the .flp. Nothing was unpacked"
            ));
        }
        if !matches!(
            entry.compression(),
            zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
        ) {
            return Err(format!(
                "\"{name}\" in that zip is packed in a way the DAW cannot unpack ({}); zip it again as a normal zip. Nothing was unpacked",
                entry.compression()
            ));
        }
        let size = entry.size();
        if project && size > limits.project {
            return Err(format!(
                "\"{name}\" in that zip is larger than any FL project ({} MB); nothing was unpacked",
                size / (1024 * 1024)
            ));
        }
        total = total.saturating_add(size);
        if total > limits.total {
            return Err(format!(
                "That zip unpacks to more than {} GB; nothing was unpacked",
                limits.total / (1024 * 1024 * 1024)
            ));
        }
        // Two entries for one file would write over each other; on Windows
        // and macOS names differing only in case are the same file.
        if !taken.insert(parts.join("/").to_lowercase()) {
            return Err(format!(
                "That zip holds \"{name}\" twice; nothing was unpacked"
            ));
        }
        planned.push(Planned {
            index,
            parts,
            size,
            project,
        });
    }
    if !planned.iter().any(|p| p.project) {
        return Err("There is no FL Studio project (.flp) in that zip".into());
    }

    let folder = new_folder(zip)?;
    let written = (|| {
        let count = planned.len();
        let (mut projects, mut samples) = (Vec::new(), Vec::new());
        for (done, plan) in planned.iter().enumerate() {
            let target = plan
                .parts
                .iter()
                .fold(folder.clone(), |p, part| p.join(part));
            write_entry(&mut archive, plan, &target)?;
            if plan.project {
                projects.push((plan.parts.len(), target));
            } else {
                samples.push(target);
            }
            progress(done + 1, count);
        }
        // The one at the top is the song; one deeper is often FL's backup.
        projects.sort_by_key(|(depth, _)| *depth);
        Ok::<_, String>((projects.into_iter().map(|(_, p)| p).collect(), samples))
    })();
    match written {
        Ok((projects, samples)) => Ok(Unpacked {
            folder,
            projects,
            samples,
            skipped,
        }),
        Err(e) => {
            // The folder is new and holds only what was just written.
            let _ = std::fs::remove_dir_all(&folder);
            Err(format!("{e}. Nothing was unpacked"))
        }
    }
}

/// Stream one entry to `target`, never more than the zip says it holds.
fn write_entry(
    archive: &mut zip::ZipArchive<std::fs::File>,
    plan: &Planned,
    target: &Path,
) -> Result<(), String> {
    let name = plan.parts.join("/");
    let failed = |e: &dyn std::fmt::Display| format!("Could not unpack \"{name}\": {e}");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| failed(&e))?;
    }
    let mut entry = archive.by_index(plan.index).map_err(|e| failed(&e))?;
    // create_new: nothing already there is ever written over.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| failed(&e))?;
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    // A zip bomb says it holds little and unpacks to a lot: one byte past
    // what it says is enough to know.
    let copied =
        std::io::copy(&mut (&mut entry).take(plan.size + 1), &mut out).map_err(|e| failed(&e))?;
    out.flush().map_err(|e| failed(&e))?;
    if copied != plan.size {
        return Err(format!(
            "That zip is damaged or unsafe: \"{name}\" unpacks to {} than it says",
            if copied > plan.size { "more" } else { "less" }
        ));
    }
    Ok(())
}

/// The folders and file name an entry's path is made of, or why it may not
/// be written. Both slashes count as separators, whatever this computer
/// uses, so a path made on Windows is read as Windows would read it.
fn safe_parts(name: &str) -> Result<Vec<String>, &'static str> {
    if name.contains('\0') {
        return Err("has a broken name");
    }
    if name.starts_with(['/', '\\']) {
        return Err("starts at the top of the drive");
    }
    let mut parts = Vec::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return Err("climbs out of the folder it unpacks into"),
            _ => {}
        }
        // "C:" is a drive; "kick.wav:x" on Windows writes into a hidden
        // part of another file.
        if part.contains(':') {
            return Err("names a drive or a hidden part of a file");
        }
        if is_device_name(part) {
            return Err("is a Windows device name, not a file");
        }
        parts.push(part.to_string());
    }
    Ok(parts)
}

/// CON, NUL, COM1 and the rest open a device on Windows whatever their
/// extension ("nul.wav" too).
fn is_device_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && stem.as_bytes()[3].is_ascii_digit())
}

/// A new folder beside `zip`, named after it: "Song.zip" makes "Song", or
/// "Song (2)" and on when that is taken. Nothing already there is used.
fn new_folder(zip: &Path) -> Result<PathBuf, String> {
    let parent = zip
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let stem = zip
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = stem.trim().trim_matches('.').trim();
    let name = if stem.is_empty() { "FL project" } else { stem };
    for n in 1..10_000 {
        let folder = if n == 1 {
            parent.join(name)
        } else {
            parent.join(format!("{name} ({n})"))
        };
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "Could not make a folder for the project beside the zip ({}): {e}",
                    folder.display()
                ))
            }
        }
    }
    Err("Could not make a folder for the project beside the zip".into())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hw-flpzip-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            w.start_file(
                *name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn no_progress() -> impl FnMut(usize, usize) {
        |_, _| {}
    }

    /// Only the zip itself is left in `dir`: no folder was made.
    fn only_the_zip_in(dir: &Path) -> bool {
        std::fs::read_dir(dir).unwrap().count() == 1
    }

    #[test]
    fn a_flat_package_unpacks_beside_the_zip_and_again_beside_that() {
        let dir = temp("flat");
        let zip = dir.join("My Song.zip");
        make_zip(
            &zip,
            &[
                ("My Song.flp", b"FLhd"),
                ("Kick.wav", b"RIFF"),
                ("Snare.WAV", b"RIFF"),
                ("notes.txt", b"hi"),
                ("setup.exe", b"MZ"),
            ],
        );
        let p = unpack(&zip, LIMITS, &mut no_progress()).unwrap();
        assert_eq!(p.folder, dir.join("My Song"));
        assert_eq!(p.projects, vec![dir.join("My Song").join("My Song.flp")]);
        assert_eq!(p.samples.len(), 2);
        assert!(p.samples.iter().all(|s| s.is_file()));
        assert_eq!(p.skipped, vec!["notes.txt", "setup.exe"]);
        assert!(!p.folder.join("setup.exe").exists());
        // A folder already there is never used.
        let again = unpack(&zip, LIMITS, &mut no_progress()).unwrap();
        assert_eq!(again.folder, dir.join("My Song (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_project_at_the_top_is_the_one_opened() {
        let dir = temp("several");
        let zip = dir.join("Pack.zip");
        make_zip(
            &zip,
            &[
                ("Backup/Song (overwritten at 1h02).flp", b"FLhd"),
                ("Song.flp", b"FLhd"),
                ("Other.flp", b"FLhd"),
            ],
        );
        let p = unpack(&zip, LIMITS, &mut no_progress()).unwrap();
        let names: Vec<_> = p
            .projects
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            ["Song.flp", "Other.flp", "Song (overwritten at 1h02).flp"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_leaving_the_folder_refuses_the_whole_zip() {
        for evil in [
            "../evil.wav",
            "Samples/../../evil.wav",
            "..\\evil.wav",
            "Samples\\..\\..\\evil.wav",
            "/tmp/evil.wav",
            "\\Windows\\evil.wav",
            "C:\\Users\\Public\\evil.wav",
            "C:evil.wav",
            "kick.wav:hidden",
            "Samples/NUL.wav",
            "com1.wav",
        ] {
            let dir = temp("slip");
            let zip = dir.join("Song.zip");
            make_zip(
                &zip,
                &[
                    ("Song.flp", b"FLhd"),
                    ("Kick.wav", b"RIFF"),
                    (evil, b"RIFF"),
                ],
            );
            let e = unpack(&zip, LIMITS, &mut no_progress()).unwrap_err();
            assert!(
                e.contains("unsafe") && e.contains("Nothing was unpacked"),
                "{evil}: {e}"
            );
            assert!(only_the_zip_in(&dir), "{evil}: something was written");
            assert!(!dir.parent().unwrap().join("evil.wav").exists());
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn a_link_in_the_zip_refuses_the_whole_zip() {
        let dir = temp("link");
        let zip = dir.join("Song.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        w.start_file("Song.flp", opts).unwrap();
        w.write_all(b"FLhd").unwrap();
        w.add_symlink("Kick.wav", "/etc/passwd", opts).unwrap();
        w.finish().unwrap();
        let e = unpack(&zip, LIMITS, &mut no_progress()).unwrap_err();
        assert!(e.contains("is a link"), "{e}");
        assert!(only_the_zip_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn too_many_entries_or_too_many_bytes_refuse_the_whole_zip() {
        let dir = temp("caps");
        let zip = dir.join("Song.zip");
        let big = vec![0u8; 3000];
        make_zip(
            &zip,
            &[
                ("Song.flp", b"FLhd"),
                ("a.wav", &big),
                ("b.wav", &big),
                ("c.wav", &big),
            ],
        );
        let few = Limits {
            entries: 3,
            ..LIMITS
        };
        let e = unpack(&zip, few, &mut no_progress()).unwrap_err();
        assert!(e.contains("4 files"), "{e}");
        assert!(only_the_zip_in(&dir));

        let small = Limits {
            total: 8000,
            ..LIMITS
        };
        assert!(unpack(&zip, small, &mut no_progress()).is_err());
        assert!(only_the_zip_in(&dir));

        let tiny_project = Limits {
            project: 3,
            ..LIMITS
        };
        let e = unpack(&zip, tiny_project, &mut no_progress()).unwrap_err();
        assert!(e.contains("larger than any FL project"), "{e}");
        assert!(only_the_zip_in(&dir));

        // At the limits it unpacks.
        let exact = Limits {
            entries: 4,
            total: 9004,
            project: 4,
        };
        assert!(unpack(&zip, exact, &mut no_progress()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The sizes in the zip are what the checks go by, so a file that
    /// unpacks to more than its size says is caught while it is written,
    /// and what was already written goes again.
    #[test]
    fn a_file_unpacking_to_more_than_it_says_is_stopped_and_cleaned_up() {
        let dir = temp("bomb");
        let zip = dir.join("Song.zip");
        let loud = vec![7u8; 100_000];
        make_zip(&zip, &[("Song.flp", b"FLhd"), ("Bomb.wav", &loud)]);
        // Make the zip say Bomb.wav holds 1000 bytes: in its local header
        // and in the central directory.
        let mut bytes = std::fs::read(&zip).unwrap();
        let mut patched = 0;
        for at in 0..bytes.len().saturating_sub(4) {
            // The uncompressed size: 22 bytes into a local header, 24
            // into a central directory one.
            let offset = match &bytes[at..at + 4] {
                [0x50, 0x4b, 0x03, 0x04] => 22,
                [0x50, 0x4b, 0x01, 0x02] => 24,
                _ => continue,
            };
            let size = u32::from_le_bytes(bytes[at + offset..at + offset + 4].try_into().unwrap());
            if size == 100_000 {
                bytes[at + offset..at + offset + 4].copy_from_slice(&1000u32.to_le_bytes());
                patched += 1;
            }
        }
        assert_eq!(patched, 2);
        std::fs::write(&zip, &bytes).unwrap();
        let e = unpack(&zip, LIMITS, &mut no_progress()).unwrap_err();
        assert!(e.contains("Nothing was unpacked"), "{e}");
        assert!(only_the_zip_in(&dir), "the half-made folder was removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_without_a_project_writes_nothing() {
        let dir = temp("noflp");
        let zip = dir.join("Samples.zip");
        make_zip(&zip, &[("Kick.wav", b"RIFF")]);
        let e = unpack(&zip, LIMITS, &mut no_progress()).unwrap_err();
        assert!(e.contains("no FL Studio project"), "{e}");
        assert!(only_the_zip_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_same_file_twice_refuses_the_whole_zip() {
        let dir = temp("twice");
        let zip = dir.join("Song.zip");
        make_zip(
            &zip,
            &[
                ("Song.flp", b"FLhd"),
                ("Kick.wav", b"RIFF"),
                ("kick.WAV", b"RIFF"),
            ],
        );
        assert!(unpack(&zip, LIMITS, &mut no_progress()).is_err());
        assert!(only_the_zip_in(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_names_are_known_by_their_stem() {
        for name in ["NUL", "nul.wav", "Con.flp", "COM1.wav", "lpt9", "AUX .wav"] {
            assert!(is_device_name(name), "{name}");
        }
        for name in ["Null.wav", "COM.wav", "Console.wav", "COMA.wav", "Kick.wav"] {
            assert!(!is_device_name(name), "{name}");
        }
    }
}
