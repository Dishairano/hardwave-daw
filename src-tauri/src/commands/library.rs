//! The sample library: one folder the DAW keeps for samples, where a
//! sample pack bought or downloaded as a .zip is installed in one step and
//! removed again in one step.
//!
//! The browser could browse any folder added to Places, but installing a
//! pack meant unzipping it by hand somewhere and adding that folder, and
//! there was no place the DAW called its own. The library is
//! Documents\Hardwave\Library unless the person picks another folder, and
//! the browser lists it in Places.
//!
//! A pack's zip comes from anyone, so extracting it is careful: every
//! entry's path must stay inside the pack's folder, only sound, MIDI,
//! preset, image and text files are written (never a program or a
//! script), and the sizes are bounded so a zip that unpacks to far more
//! than it claims (a zip bomb) is stopped and cleaned up.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The files a pack may hold that are written out.
const KEPT: &[&str] = &[
    "wav",
    "mp3",
    "flac",
    "aiff",
    "aif",
    "ogg",
    "m4a",
    "mid",
    "midi",
    "fxp",
    "fxb",
    "vstpreset",
    "txt",
    "md",
    "pdf",
    "rtf",
    "png",
    "jpg",
    "jpeg",
];
/// At most this much is unpacked from one pack.
const MAX_TOTAL: u64 = 32 * 1024 * 1024 * 1024;
/// And at most this many files.
const MAX_ENTRIES: usize = 200_000;

fn settings_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Hardwave").join("library.json"))
}

/// The library folder: the one picked, or Documents\Hardwave\Library.
pub fn library_dir() -> Result<PathBuf, String> {
    let picked = settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("folder").and_then(|f| f.as_str()).map(PathBuf::from));
    let dir = match picked {
        Some(dir) => dir,
        None => dirs::document_dir()
            .or_else(dirs::home_dir)
            .ok_or_else(|| "No documents folder on this system for the library".to_string())?
            .join("Hardwave")
            .join("Library"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make the library folder: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn get_library_folder() -> Result<String, String> {
    Ok(library_dir()?.to_string_lossy().into_owned())
}

/// Keep the library in another folder from now on. Packs already installed
/// stay where they are.
#[tauri::command]
pub fn set_library_folder(path: String) -> Result<String, String> {
    let dir = PathBuf::from(&path);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not use that folder: {e}"))?;
    let file = settings_path().ok_or("No data folder on this system")?;
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&file, serde_json::json!({ "folder": path }).to_string())
        .map_err(|e| format!("Could not keep the setting: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPack {
    /// The pack's folder in the library.
    pub folder: String,
    pub files: usize,
    /// Files left out: programs, scripts and anything else not a sample,
    /// and entries whose path would have left the pack's folder.
    pub skipped: usize,
}

/// A folder name for the pack from its zip's name.
fn pack_name(zip: &Path) -> String {
    let stem = zip
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sample pack");
    let clean: String = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_()&+,.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() {
        "Sample pack".into()
    } else {
        clean
    }
}

/// Unpack `zip` into a new folder under `library`, as described above.
pub fn install_into(library: &Path, zip: &Path) -> Result<InstalledPack, String> {
    let file = std::fs::File::open(zip).map_err(|e| format!("Could not open the pack: {e}"))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|_| "That file is not a zip the DAW can read".to_string())?;
    if archive.len() > MAX_ENTRIES {
        return Err("That pack holds more files than a sample pack does".into());
    }
    let name = pack_name(zip);
    let mut folder = library.join(&name);
    let mut n = 2;
    while folder.exists() {
        folder = library.join(format!("{name} ({n})"));
        n += 1;
    }
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("Could not make the pack's folder: {e}"))?;
    let result = (|| {
        let (mut files, mut skipped, mut total) = (0usize, 0usize, 0u64);
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("The pack is damaged: {e}"))?;
            if entry.is_dir() {
                continue;
            }
            // A path that is absolute, or climbs out with "..", is refused.
            let Some(relative) = entry.enclosed_name() else {
                skipped += 1;
                continue;
            };
            let kept = relative
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| KEPT.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if !kept {
                skipped += 1;
                continue;
            }
            let claimed = entry.size();
            if total.saturating_add(claimed) > MAX_TOTAL {
                return Err("That pack unpacks to more than the library takes in one go".into());
            }
            let target = folder.join(&relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Could not write the pack: {e}"))?;
            }
            let mut out = std::fs::File::create(&target)
                .map_err(|e| format!("Could not write the pack: {e}"))?;
            // Never more than the entry says it holds: a zip bomb claims
            // little and unpacks to a lot.
            let written = std::io::copy(&mut (&mut entry).take(claimed + 1), &mut out)
                .map_err(|e| format!("Could not write the pack: {e}"))?;
            if written > claimed {
                return Err(
                    "The pack is damaged or unsafe: a file unpacks to more than it says".into(),
                );
            }
            total += written;
            files += 1;
        }
        Ok((files, skipped))
    })();
    match result {
        Ok((files, skipped)) if files > 0 => Ok(InstalledPack {
            folder: folder.to_string_lossy().into_owned(),
            files,
            skipped,
        }),
        Ok(_) => {
            let _ = std::fs::remove_dir_all(&folder);
            Err("There are no samples in that zip".into())
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&folder);
            Err(e)
        }
    }
}

/// Install a sample pack from its .zip into the library.
#[tauri::command]
pub async fn install_sample_pack(path: String) -> Result<InstalledPack, String> {
    let library = library_dir()?;
    tauri::async_runtime::spawn_blocking(move || install_into(&library, Path::new(&path)))
        .await
        .map_err(|e| format!("installing stopped: {e}"))?
}

/// Whether `folder` is a pack in the library: a folder directly inside it.
fn is_pack(library: &Path, folder: &Path) -> bool {
    match (library.canonicalize(), folder.canonicalize()) {
        (Ok(lib), Ok(f)) => f.is_dir() && f.parent() == Some(lib.as_path()),
        _ => false,
    }
}

/// Remove a pack from the library, to the recycle bin, so it can come back.
/// Only a folder directly inside the library qualifies.
#[tauri::command]
pub fn remove_sample_pack(folder: String) -> Result<(), String> {
    let library = library_dir()?;
    let path = PathBuf::from(&folder);
    if !is_pack(&library, &path) {
        return Err("Only a pack in the library can be removed here".into());
    }
    trash::delete(&path).map_err(|e| format!("Could not move the pack to the recycle bin: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hw-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    /// Samples come out in their folders; a program and a path climbing
    /// out of the pack do not.
    #[test]
    fn a_pack_installs_its_samples_and_nothing_else() {
        let dir = temp("install");
        let zip = dir.join("Raw Kicks Vol 1.zip");
        make_zip(
            &zip,
            &[
                ("Kicks/kick 01.wav", b"RIFF"),
                ("Kicks/kick 02.WAV", b"RIFF"),
                ("readme.txt", b"hi"),
                ("setup.exe", b"MZ"),
                ("../../evil.wav", b"RIFF"),
            ],
        );
        let lib = dir.join("Library");
        std::fs::create_dir_all(&lib).unwrap();
        let pack = install_into(&lib, &zip).unwrap();
        assert_eq!((pack.files, pack.skipped), (3, 2));
        let folder = PathBuf::from(&pack.folder);
        assert_eq!(folder.file_name().unwrap(), "Raw Kicks Vol 1");
        assert!(folder.join("Kicks").join("kick 02.WAV").exists());
        assert!(!folder.join("setup.exe").exists());
        assert!(!dir.join("evil.wav").exists());
        // The same pack again goes beside it.
        let again = install_into(&lib, &zip).unwrap();
        assert!(again.folder.ends_with("Raw Kicks Vol 1 (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A zip with no samples leaves nothing behind.
    #[test]
    fn a_zip_without_samples_leaves_no_folder() {
        let dir = temp("empty");
        let zip = dir.join("tools.zip");
        make_zip(&zip, &[("tool.exe", b"MZ"), ("run.bat", b"x")]);
        let lib = dir.join("Library");
        std::fs::create_dir_all(&lib).unwrap();
        assert!(install_into(&lib, &zip).is_err());
        assert_eq!(std::fs::read_dir(&lib).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only a folder directly in the library counts as a pack.
    #[test]
    fn only_a_pack_in_the_library_can_be_removed() {
        let dir = temp("remove");
        let lib = dir.join("Library");
        std::fs::create_dir_all(lib.join("Pack").join("Sub")).unwrap();
        assert!(is_pack(&lib, &lib.join("Pack")));
        assert!(!is_pack(&lib, &lib.join("Pack").join("Sub")));
        assert!(!is_pack(&lib, &lib));
        assert!(!is_pack(&lib, &dir));
        assert!(!is_pack(&lib, &lib.join("Pack").join("..").join("..")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
