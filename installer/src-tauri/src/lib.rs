//! Hardwave DAW custom installer.
//!
//! A Tauri-based launcher-style installer with a branded GUI that drives
//! the DAW's existing platform installers. It:
//!   1. Resolves the latest DAW version from the GitHub release tag
//!      (with an optional Hardwave updater endpoint as primary source).
//!   2. Lets the user pick an install location and shortcut preferences.
//!   3. Downloads the platform-specific installer (.exe / .dmg /
//!      .AppImage) from the GitHub release assets.
//!   4. Runs that installer silently while showing branded progress.
//!   5. Launches the DAW.
//!
//! Shortcuts and the uninstall registry entry are handled by the
//! underlying NSIS installer on Windows, by the .app bundle on macOS,
//! and by the .desktop file on Linux.
//!
//! On Windows the same program is the DAW's uninstaller: the installer
//! carries it as `binaries\hardwave-daw-uninstaller.exe` and the Windows
//! Apps entry starts it with `--uninstall` (see uninstall.rs).

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, State};

mod install;
mod uninstall;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersion {
    pub version: String,
    pub notes: String,
    pub pub_date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    pub phase: String, // "downloading" | "extracting" | "shortcuts" | "registering" | "done"
    pub percent: u32,  // 0-100
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOptions {
    pub install_dir: String,
    pub create_desktop_shortcut: bool,
    pub create_start_menu_shortcut: bool,
    pub launch_after: bool,
}

#[derive(Default)]
pub struct InstallState {
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

// ─── Commands ────────────────────────────────────────────────────────────────

/// Probe the Hardwave updater endpoint to find the latest version and the
/// portable-zip download URL.
#[tauri::command]
async fn fetch_latest_version() -> Result<LatestVersion, String> {
    let client = reqwest::Client::builder()
        .user_agent("HardwaveDawInstaller/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // Primary: Hardwave API. Fallback: GitHub latest release tag.
    let primary_res = client
        .get("https://hardwavestudios.com/api/updates/hardwave-daw/latest")
        .send()
        .await;

    if let Ok(res) = primary_res {
        if res.status().is_success() {
            if let Ok(body) = res.json::<serde_json::Value>().await {
                if let Some(v) = body.get("version").and_then(|v| v.as_str()) {
                    return Ok(LatestVersion {
                        version: v.trim_start_matches('v').to_string(),
                        notes: body
                            .get("notes")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        pub_date: body
                            .get("pub_date")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    });
                }
            }
        }
    }

    // Fallback: GitHub API, the latest full release. Release candidates
    // are tagged with a dash and published as prereleases, which this
    // endpoint leaves out on purpose: they are for testers.
    let gh: serde_json::Value = client
        .get("https://api.github.com/repos/Dishairano/hardwave-daw/releases/latest")
        .send()
        .await
        .map_err(|e| format!("GitHub API request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub API returned error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Parsing GitHub response failed: {e}"))?;

    let tag = gh
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or("GitHub release missing tag_name")?;
    let body = gh.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let pub_date = gh
        .get("published_at")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok(LatestVersion {
        version: tag.trim_start_matches('v').to_string(),
        notes: body.to_string(),
        pub_date,
    })
}

/// Returns the default per-user install directory.
#[tauri::command]
fn default_install_dir() -> String {
    install::default_install_dir().to_string_lossy().to_string()
}

/// Starts the full install pipeline. Emits `install://progress` events.
#[tauri::command]
async fn start_install(
    app: AppHandle,
    state: State<'_, InstallState>,
    options: InstallOptions,
) -> Result<String, String> {
    state
        .cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let cancel = state.cancel.clone();

    install::run(app, cancel, options).await
}

/// Cancel an in-progress install.
#[tauri::command]
fn cancel_install(state: State<'_, InstallState>) {
    state
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Launch the installed DAW at the given path. When `exe_path` resolves
/// to a directory (the install succeeded but the post-install probe
/// couldn't pin down the exact binary), we open the OS file manager at
/// that location so the user can find + run the DAW manually instead
/// of getting a generic "Launch failed".
#[tauri::command]
fn launch_installed(exe_path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(&exe_path);
    if !path.exists() {
        return Err(format!("Path not found: {exe_path}"));
    }
    let is_dir = path.is_dir();

    #[cfg(windows)]
    {
        if is_dir {
            // Open File Explorer at the install root.
            std::process::Command::new("explorer")
                .arg(&path)
                .spawn()
                .map_err(|e| format!("Could not open install folder: {e}"))?;
        } else {
            std::process::Command::new(&path)
                .spawn()
                .map_err(|e| format!("Launch failed: {e}"))?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        // `open` works for both directories (Finder window) and .app bundles.
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Launch failed: {e}"))?;
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        if is_dir {
            // xdg-open handles directories on every desktop env.
            std::process::Command::new("xdg-open")
                .arg(&path)
                .spawn()
                .map_err(|e| format!("Could not open install folder: {e}"))?;
        } else {
            std::process::Command::new(&path)
                .spawn()
                .map_err(|e| format!("Launch failed: {e}"))?;
        }
    }
    Ok(())
}

/// Quit the installer window.
#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

/// Which window this run is: "install" or "uninstall".
#[tauri::command]
fn launch_mode(mode: State<'_, LaunchMode>) -> &'static str {
    if mode.uninstall {
        "uninstall"
    } else {
        "install"
    }
}

/// The steps the removal window lists.
#[tauri::command]
fn uninstall_steps() -> Vec<&'static str> {
    uninstall::STEPS.to_vec()
}

#[derive(Debug, Clone, Serialize)]
struct UninstallProgress {
    step: usize,
    message: String,
}

/// Remove the DAW. Emits `uninstall://progress` per step. The error is
/// "daw-running" when the DAW is open, "not-installed" when there is no
/// install to remove, or a sentence.
#[tauri::command]
async fn start_uninstall(
    app: AppHandle,
    mode: State<'_, LaunchMode>,
    remove_settings: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use tauri::Emitter;
        let Some(dir) = uninstall::target() else {
            return Err(uninstall::Stop::NotInstalled.code());
        };
        let removed = mode.removed.clone();
        tauri::async_runtime::spawn_blocking(move || {
            uninstall::remove(&dir, remove_settings, |step, message| {
                let _ = app.emit(
                    "uninstall://progress",
                    UninstallProgress {
                        step,
                        message: message.to_string(),
                    },
                );
            })
            .map(|()| {
                // This program and the folders go when the window closes.
                *removed.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
            })
            .map_err(|stop| stop.code())
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (app, mode, remove_settings);
        Err(uninstall::Stop::NotInstalled.code())
    }
}

/// How this run was started, and the folder it removed, if it did.
struct LaunchMode {
    uninstall: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    removed: Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
}

// ─── Entry ───────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let exe = std::env::current_exe().ok();
    let args = uninstall::parse_args(exe.as_deref(), std::env::args());
    #[cfg(windows)]
    if args.uninstall && args.quiet {
        // For scripts: no window, only the app (never the settings).
        let code = match uninstall::target() {
            Some(dir) => match uninstall::remove(&dir, false, |_, _| {}) {
                Ok(()) => {
                    uninstall::clean_up_after(&dir);
                    0
                }
                Err(stop) => {
                    eprintln!("hardwave-daw-uninstaller: {}", stop.code());
                    if stop == uninstall::Stop::DawRunning {
                        2
                    } else {
                        1
                    }
                }
            },
            None => {
                eprintln!("hardwave-daw-uninstaller: not-installed");
                1
            }
        };
        std::process::exit(code);
    }
    #[cfg(not(windows))]
    if args.uninstall {
        eprintln!("hardwave-daw-uninstaller: removing is for Windows; on this system remove the app as usual");
        return;
    }
    let uninstalling = args.uninstall;
    let removed = Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(InstallState::default())
        .manage(LaunchMode {
            uninstall: uninstalling,
            removed: removed.clone(),
        })
        .setup(move |app| {
            if uninstalling {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.set_title("Remove Hardwave DAW");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            fetch_latest_version,
            default_install_dir,
            start_install,
            cancel_install,
            launch_installed,
            quit,
            launch_mode,
            uninstall_steps,
            start_uninstall
        ])
        .build(tauri::generate_context!())
        .expect("error while running hardwave-daw-installer");
    app.run(move |_, event| {
        if let tauri::RunEvent::Exit = event {
            #[cfg(windows)]
            if let Some(dir) = removed.lock().unwrap_or_else(|e| e.into_inner()).take() {
                uninstall::clean_up_after(&dir);
            }
        }
    });
}
