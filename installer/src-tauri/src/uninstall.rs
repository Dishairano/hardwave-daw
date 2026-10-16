//! Removing the DAW from Windows.
//!
//! The NSIS installer puts this program in the install folder as
//! `binaries\hardwave-daw-uninstaller.exe` and points the Windows Apps
//! entry at it: `--uninstall` opens the window, `--uninstall --quiet` is
//! for scripts. However Windows starts it, it is this window, and nothing
//! is removed before the person presses the button.
//!
//! It removes nothing NSIS knows how to remove. After the confirmation it
//! runs NSIS's own `uninstall.exe` silently (the app, its built-in
//! plug-ins, the shortcuts, the Windows Apps entry), then deletes what that
//! leaves behind: NSIS's uninstaller and, once this program has exited,
//! itself and the folders, when empty. The DAW's settings go only when
//! asked, and only the settings: presets, recordings, bounces and scripts
//! live in the same folders and are the person's work.
//!
//! The folder it removes is always the one it sits in; nothing on the
//! command line can point it anywhere else.

use std::path::{Path, PathBuf};

/// The uninstaller's file name inside `<install dir>\binaries`.
pub const UNINSTALLER_NAME: &str = "hardwave-daw-uninstaller.exe";
/// The DAW's own program in the install folder.
pub const DAW_EXE: &str = "hardwave-daw.exe";
/// NSIS's uninstaller in the install folder.
const NSIS_UNINSTALLER: &str = "uninstall.exe";
/// The DAW's Tauri identifier, which names its WebView profile folder.
const DAW_BUNDLE_ID: &str = "com.hardwave.daw";

/// What the command line asked for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Args {
    pub uninstall: bool,
    pub quiet: bool,
}

/// `exe` is this program's own path: run as the uninstaller (a
/// double-click on it in the install folder), it is the removal window
/// whatever the command line says.
pub fn parse_args(exe: Option<&Path>, args: impl IntoIterator<Item = String>) -> Args {
    let mut out = Args {
        uninstall: exe
            .and_then(|e| e.file_name())
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(UNINSTALLER_NAME)),
        quiet: false,
    };
    for arg in args.into_iter().skip(1) {
        match arg.as_str() {
            "--uninstall" => out.uninstall = true,
            "--quiet" => out.quiet = true,
            _ => {}
        }
    }
    out
}

/// The install folder of an uninstaller at `exe`, which lives in
/// `<install dir>\binaries`.
pub fn install_dir_of(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    let named_binaries = bin
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("binaries"));
    named_binaries.then(|| bin.parent().map(Path::to_path_buf))?
}

/// Whether there is an install to remove in `dir`: NSIS's uninstaller is
/// there. Not the DAW itself, which a broken install can be missing: the
/// Windows Apps entry still has to be removable.
pub fn can_remove(dir: &Path) -> bool {
    dir.join(NSIS_UNINSTALLER).is_file()
}

/// The DAW's settings: its window's stored settings and recent files (the
/// WebView profile under the local app data folder) and its own settings
/// files in the roaming `hardwave` folder. That folder also holds the
/// person's recordings, bounces, scripts and the plug-ins' logs, and the
/// roaming `com.hardwave.daw` folder holds their presets: those stay.
pub fn settings_paths(local_app_data: &Path, roaming_app_data: &Path) -> Vec<PathBuf> {
    let hardwave = roaming_app_data.join("hardwave");
    vec![
        local_app_data.join(DAW_BUNDLE_ID),
        hardwave.join("audio-prefs.json"),
        hardwave.join("midi_mappings.json"),
        hardwave.join("plugin-cache.json"),
    ]
}

/// The steps of a removal, as the window lists them.
pub const STEPS: [&str; 4] = [
    "Close Hardwave DAW",
    "Remove the app",
    "Remove the shortcuts",
    "Remove it from Windows Apps",
];

/// Why a removal stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The DAW is open; the person saves and closes it, then tries again.
    DawRunning,
    /// No DAW install where one was expected.
    NotInstalled,
    /// Something failed; the text says what.
    Failed(String),
}

impl Stop {
    /// What the window and the quiet run's error output say.
    pub fn code(&self) -> String {
        match self {
            Stop::DawRunning => "daw-running".to_string(),
            Stop::NotInstalled => "not-installed".to_string(),
            Stop::Failed(why) => why.clone(),
        }
    }
}

#[cfg(windows)]
pub use windows_impl::*;

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    /// The install folder this program sits in, when there is an install
    /// to remove there.
    pub fn target() -> Option<PathBuf> {
        let dir = install_dir_of(&std::env::current_exe().ok()?)?;
        can_remove(&dir).then_some(dir)
    }

    /// Whether the DAW from this install folder is running, for this
    /// person. Another account's DAW, or another copy elsewhere, does not
    /// stop this one from being removed.
    pub fn daw_running(dir: &Path) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let want = dir.join(DAW_EXE).to_string_lossy().to_lowercase();
        // SAFETY: plain Win32 calls with buffers sized as the calls ask;
        // every handle opened here is closed here.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return false;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = false;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more && !found {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(0);
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if name.eq_ignore_ascii_case(DAW_EXE) {
                    let process =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                    if process != 0 {
                        let mut buf = [0u16; 1024];
                        let mut size = buf.len() as u32;
                        if QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) != 0
                        {
                            let path = String::from_utf16_lossy(&buf[..size as usize]);
                            found = path.to_lowercase() == want;
                        }
                        CloseHandle(process);
                    }
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            found
        }
    }

    /// Remove the DAW from `dir`. `step` hears each step as it starts (its
    /// index in [`STEPS`]) and a line for the log.
    pub fn remove(
        dir: &Path,
        remove_settings: bool,
        mut step: impl FnMut(usize, &str),
    ) -> Result<(), Stop> {
        if !can_remove(dir) {
            return Err(Stop::NotInstalled);
        }
        step(0, "Checking that Hardwave DAW is closed");
        if daw_running(dir) {
            return Err(Stop::DawRunning);
        }

        // NSIS removes the app, its plug-ins, the shortcuts and the Windows
        // Apps entry. `_?=` keeps it running in place, so this waits for
        // it rather than for a copy it would start in %TEMP%.
        step(1, &format!("Removing {}", dir.display()));
        let status = std::process::Command::new(dir.join(NSIS_UNINSTALLER))
            .arg("/S")
            .arg(format!("_?={}", dir.display()))
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| Stop::Failed(format!("Could not start the removal: {e}")))?;
        if !status.success() {
            return Err(Stop::Failed(format!(
                "The removal stopped (code {}). Nothing more was changed.",
                status.code().unwrap_or(-1)
            )));
        }
        step(2, "Removed the shortcuts");
        // What NSIS leaves when it runs in place: its own uninstaller.
        // This program and the folders go once it has exited
        // (`clean_up_after`).
        let _ = std::fs::remove_file(dir.join(NSIS_UNINSTALLER));

        step(3, "Removed it from Windows Apps");
        if remove_settings {
            if let (Some(local), Some(roaming)) = (dirs::data_local_dir(), dirs::data_dir()) {
                for path in settings_paths(&local, &roaming) {
                    let _ = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                }
            }
        }
        Ok(())
    }

    /// After a removal: once this program has exited, delete it and the
    /// install folders it leaves, each only when empty (`rmdir` without
    /// `/s`). Windows cannot delete a running program, so a cmd that waits
    /// three seconds does it. It waits with `ping`: `timeout` needs a
    /// console and, started without one, quits at once, so the delete ran
    /// while this program was still open and left it behind.
    pub fn clean_up_after(dir: &Path) {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        if !exe.starts_with(dir) {
            return;
        }
        let cmd = format!(
            "ping -n 4 127.0.0.1 > nul & del /f /q \"{}\" & rmdir \"{}\" & rmdir \"{}\"",
            exe.display(),
            dir.join("binaries").display(),
            dir.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &cmd])
            .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Args {
        let launcher = Path::new("/downloads/HardwaveDAW-Setup.exe");
        parse_args(
            Some(launcher),
            std::iter::once("setup".to_string()).chain(list.iter().map(|s| s.to_string())),
        )
    }

    #[test]
    fn the_command_line_says_what_to_do() {
        assert_eq!(args(&[]), Args::default());
        assert_eq!(
            args(&["--uninstall", "--quiet"]),
            Args {
                uninstall: true,
                quiet: true
            }
        );
        // Run as the uninstaller, it is the removal window without asking.
        let own = Path::new("/apps/Hardwave DAW/binaries").join(UNINSTALLER_NAME);
        assert!(parse_args(Some(&own), ["x".to_string()]).uninstall);
    }

    #[test]
    fn the_install_folder_is_the_one_above_binaries() {
        let dir = Path::new("/apps/Hardwave DAW");
        assert_eq!(
            install_dir_of(&dir.join("binaries").join(UNINSTALLER_NAME)),
            Some(dir.to_path_buf())
        );
        // A copy anywhere else has no install folder of its own.
        assert_eq!(install_dir_of(&dir.join(UNINSTALLER_NAME)), None);
    }

    #[test]
    fn an_install_is_removable_while_its_uninstaller_is_there() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!can_remove(dir.path()));
        // A broken install without the DAW still has to leave Windows Apps.
        std::fs::write(dir.path().join("uninstall.exe"), b"").unwrap();
        assert!(can_remove(dir.path()));
    }

    #[test]
    fn settings_leave_presets_recordings_and_scripts_alone() {
        let local = Path::new("/local");
        let roaming = Path::new("/roaming");
        let paths = settings_paths(local, roaming);
        assert!(paths.contains(&local.join("com.hardwave.daw")));
        assert!(paths.contains(&roaming.join("hardwave").join("audio-prefs.json")));
        // The person's work, in the same folders: never on the list.
        for kept in [
            roaming.join("com.hardwave.daw"),
            roaming.join("hardwave"),
            roaming.join("hardwave").join("Recordings"),
            roaming.join("hardwave").join("Bounces"),
            roaming.join("hardwave").join("daw"),
        ] {
            assert!(
                !paths.iter().any(|p| kept.starts_with(p)),
                "{} would go",
                kept.display()
            );
        }
    }
}
