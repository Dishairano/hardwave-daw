//! Filesystem listing for the browser panel's Places tree.
//!
//! Before this the browser only showed files added one at a time through the
//! Open dialog, so a sample pack could not be browsed at all. This lists one
//! directory level on demand. The UI asks again whenever a folder is opened,
//! so a large library is never walked up front.

use serde::Serialize;
use std::cmp::Ordering;
use std::path::Path;

/// Extensions the engine's decoder (symphonia with all codecs) can import.
/// Only these are listed, so every file the browser shows can go on a track.
const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "aiff", "aif", "ogg", "m4a"];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BrowserEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// File size in bytes; 0 for folders.
    pub size_bytes: u64,
}

/// List one folder for the browser. Runs on a blocking thread so a slow
/// network drive or a huge folder cannot stall the UI.
#[tauri::command]
pub async fn list_directory(path: String) -> Result<Vec<BrowserEntry>, String> {
    let dir = path.clone();
    tauri::async_runtime::spawn_blocking(move || list_directory_entries(Path::new(&dir)))
        .await
        .map_err(|e| format!("Listing {path} failed: {e}"))?
}

/// Subfolders first, then audio files, each group sorted by name ignoring
/// case. Hidden entries are skipped. An entry that cannot be read (a broken
/// symlink, a permission error on one file) is skipped instead of failing
/// the whole folder; only a folder that cannot be opened is an error.
pub fn list_directory_entries(dir: &Path) -> Result<Vec<BrowserEntry>, String> {
    let read = std::fs::read_dir(dir).map_err(|e| format!("Can't open {}: {e}", dir.display()))?;
    let mut entries: Vec<BrowserEntry> = read
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            // `fs::metadata` follows symlinks, so a linked sample folder
            // browses like a real one. A broken link has no metadata.
            let meta = std::fs::metadata(&path).ok()?;
            if is_hidden(&name, &meta) {
                return None;
            }
            let is_dir = meta.is_dir();
            if !is_dir && !is_audio_file(&path) {
                return None;
            }
            Some(BrowserEntry {
                name,
                path: path.to_string_lossy().into_owned(),
                is_dir,
                size_bytes: if is_dir { 0 } else { meta.len() },
            })
        })
        .collect();
    entries.sort_by(compare_entries);
    Ok(entries)
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(ext)))
}

#[cfg(windows)]
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    name.starts_with('.')
        || (meta.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

fn compare_entries(a: &BrowserEntry, b: &BrowserEntry) -> Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        .then_with(|| a.name.cmp(&b.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(entries: &[BrowserEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn lists_folders_first_then_audio_sorted_ignoring_case() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("snares")).unwrap();
        fs::create_dir(dir.path().join("Kicks")).unwrap();
        fs::write(dir.path().join("b loop.WAV"), b"x").unwrap();
        fs::write(dir.path().join("A Loop.flac"), b"xyz").unwrap();

        let entries = list_directory_entries(dir.path()).unwrap();

        assert_eq!(
            names(&entries),
            ["Kicks", "snares", "A Loop.flac", "b loop.WAV"]
        );
        assert!(entries[0].is_dir && entries[1].is_dir);
        assert!(!entries[2].is_dir && !entries[3].is_dir);
        assert_eq!(entries[0].size_bytes, 0);
        assert_eq!(entries[2].size_bytes, 3);
        assert_eq!(
            entries[2].path,
            dir.path().join("A Loop.flac").to_string_lossy()
        );
    }

    #[test]
    fn skips_hidden_entries_and_files_the_engine_cannot_import() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".cache")).unwrap();
        fs::write(dir.path().join(".DS_Store"), b"x").unwrap();
        fs::write(dir.path().join("._kick.wav"), b"x").unwrap();
        fs::write(dir.path().join("readme.txt"), b"x").unwrap();
        fs::write(dir.path().join("song.hwp"), b"x").unwrap();
        fs::write(dir.path().join("kick.wav"), b"x").unwrap();

        let entries = list_directory_entries(dir.path()).unwrap();

        assert_eq!(names(&entries), ["kick.wav"]);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let err = list_directory_entries(&dir.path().join("gone")).unwrap_err();
        assert!(err.contains("gone"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn follows_symlinked_folders_and_skips_broken_links() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let listed = dir.path().join("listed");
        fs::create_dir(&listed).unwrap();
        std::os::unix::fs::symlink(&real, listed.join("linked")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("nowhere"), listed.join("broken.wav")).unwrap();

        let entries = list_directory_entries(&listed).unwrap();

        assert_eq!(names(&entries), ["linked"]);
        assert!(entries[0].is_dir);
    }
}
