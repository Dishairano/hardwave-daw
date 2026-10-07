//! Which files a project may make the DAW open.
//!
//! A project names its samples by path, and a project can come from
//! anyone: a download, Workspace, a room partner. Opening it must not
//! become a way to read files that are not samples, to step out of the
//! song's folder, or to make Windows connect to a stranger's server
//! (opening \\server\share\kick.wav sends the person's Windows sign-in
//! to that server before a single byte of audio arrives).
//!
//! So a path from a project is used only when it names an audio file,
//! and:
//! - a relative path stays inside the project's folder;
//! - an absolute path is on a local drive, or on a network server the
//!   person has picked a file from themselves on this machine;
//! - it never names a device.

use std::path::{Component, Path, PathBuf};

/// The audio files a clip can play.
pub const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "flac", "aiff", "aif", "ogg", "m4a"];
/// The video files a song can be scored to.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mov", "m4v", "mkv"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    NotAudio,
    LeavesTheProjectFolder,
    /// A network server nobody on this machine chose; the name is kept
    /// so the window can say which one.
    UntrustedServer(String),
    Device,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::NotAudio => write!(f, "not a file the DAW plays"),
            Refusal::LeavesTheProjectFolder => write!(f, "points outside the project's folder"),
            Refusal::UntrustedServer(host) => write!(
                f,
                "on the network server {host}, which this computer has not used for samples; relink it to load it"
            ),
            Refusal::Device => write!(f, "not a file"),
        }
    }
}

fn has_extension(path: &str, allowed: &[&str]) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| allowed.iter().any(|a| a.eq_ignore_ascii_case(ext)))
}

/// The server in a Windows network path (`\\server\share\...` or
/// `//server/share/...`), lowercased, or None when the path is not one.
pub fn network_server(path: &str) -> Option<String> {
    let rest = path
        .strip_prefix(r"\\?\UNC\")
        .or_else(|| path.strip_prefix(r"\\"))
        .or_else(|| path.strip_prefix("//"))?;
    let host = rest.split(['\\', '/']).next().unwrap_or("");
    (!host.is_empty() && host != "." && host != "?").then(|| host.to_ascii_lowercase())
}

fn is_device(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with(r"\\.\")
        || lower.starts_with("//./")
        || lower.starts_with(r"\\?\globalroot")
        || lower.starts_with("/dev/")
        || lower.starts_with("/proc/")
        || lower.starts_with("/sys/")
}

/// Where a project's sample really is, or why it may not be opened.
pub fn check(
    file: &str,
    project_dir: Option<&Path>,
    trusted_servers: &[String],
) -> Result<PathBuf, Refusal> {
    check_with(file, project_dir, trusted_servers, AUDIO_EXTENSIONS)
}

/// The same rules for a project's video.
pub fn check_video(
    file: &str,
    project_dir: Option<&Path>,
    trusted_servers: &[String],
) -> Result<PathBuf, Refusal> {
    check_with(file, project_dir, trusted_servers, VIDEO_EXTENSIONS)
}

fn check_with(
    file: &str,
    project_dir: Option<&Path>,
    trusted_servers: &[String],
    extensions: &[&str],
) -> Result<PathBuf, Refusal> {
    if !has_extension(file, extensions) {
        return Err(Refusal::NotAudio);
    }
    if is_device(file) {
        return Err(Refusal::Device);
    }
    if let Some(server) = network_server(file) {
        return if trusted_servers
            .iter()
            .any(|t| t.eq_ignore_ascii_case(&server))
        {
            Ok(PathBuf::from(file))
        } else {
            Err(Refusal::UntrustedServer(server))
        };
    }
    let path = Path::new(file);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    // A relative path is the project's own folder and below. On
    // Windows "C:kick.wav" or "\kick.wav" is neither absolute nor
    // relative to the project, so only plain names are accepted.
    if file.contains(':')
        || file.starts_with('\\')
        || file.starts_with('/')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || file.split(['\\', '/']).any(|part| part == "..")
    {
        return Err(Refusal::LeavesTheProjectFolder);
    }
    Ok(match project_dir {
        Some(dir) => dir.join(path),
        None => path.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(file: &str) -> PathBuf {
        check(file, Some(Path::new("/songs/Raw Drop")), &[]).unwrap()
    }

    fn refused(file: &str) -> Refusal {
        check(
            file,
            Some(Path::new("/songs/Raw Drop")),
            &["nas".to_string()],
        )
        .unwrap_err()
    }

    #[test]
    fn samples_beside_the_song_and_on_local_drives_open() {
        assert_eq!(
            ok("Raw Drop Samples/Kick.wav"),
            PathBuf::from("/songs/Raw Drop/Raw Drop Samples/Kick.wav")
        );
        assert_eq!(
            ok("/home/p/Samples/Kick.WAV"),
            PathBuf::from("/home/p/Samples/Kick.WAV")
        );
        assert!(check(r"\\NAS\samples\kick.wav", None, &["nas".to_string()]).is_ok());
    }

    #[test]
    fn a_project_cannot_reach_for_files_that_are_not_samples() {
        assert_eq!(refused("../../.ssh/id_ed25519"), Refusal::NotAudio);
        assert_eq!(
            refused(r"..\..\AppData\Roaming\x\credentials.json"),
            Refusal::NotAudio
        );
        assert_eq!(
            refused("/home/p/.config/hardwave/auth_token"),
            Refusal::NotAudio
        );
    }

    #[test]
    fn a_relative_sample_stays_in_the_project_folder() {
        assert_eq!(
            refused("../../../secret.wav"),
            Refusal::LeavesTheProjectFolder
        );
        assert_eq!(
            refused(r"Samples\..\..\x.wav"),
            Refusal::LeavesTheProjectFolder
        );
        assert_eq!(refused("C:x.wav"), Refusal::LeavesTheProjectFolder);
        assert_eq!(refused(r"\x.wav"), Refusal::LeavesTheProjectFolder);
    }

    #[test]
    fn a_strangers_server_is_not_contacted() {
        assert_eq!(
            refused(r"\\attacker.example\share\kick.wav"),
            Refusal::UntrustedServer("attacker.example".into())
        );
        assert_eq!(
            refused("//attacker.example/share/kick.wav"),
            Refusal::UntrustedServer("attacker.example".into())
        );
        assert_eq!(
            refused(r"\\?\UNC\attacker.example\s\kick.wav"),
            Refusal::UntrustedServer("attacker.example".into())
        );
        assert_eq!(
            refused(r"\\attacker.example@SSL\x\kick.wav"),
            Refusal::UntrustedServer("attacker.example@ssl".into())
        );
    }

    #[test]
    fn a_video_follows_the_same_rules() {
        assert!(check_video("/films/cut.mp4", None, &[]).is_ok());
        assert_eq!(
            check_video(r"\\x.example\s\cut.mp4", None, &[]),
            Err(Refusal::UntrustedServer("x.example".into()))
        );
        assert_eq!(
            check_video("/home/p/notes.txt", None, &[]),
            Err(Refusal::NotAudio)
        );
    }

    #[test]
    fn devices_are_not_files() {
        assert_eq!(refused(r"\\.\pipe\x.wav"), Refusal::Device);
        assert_eq!(refused("/dev/stdin.wav"), Refusal::Device);
        assert_eq!(refused(r"\\?\GLOBALROOT\Device\x.wav"), Refusal::Device);
    }
}
