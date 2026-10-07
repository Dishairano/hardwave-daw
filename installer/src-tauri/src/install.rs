//! Core install pipeline: fetch latest DAW release → download platform
//! installer → run silently → optional launch.
//!
//! Unlike Suite which downloads a portable zip and unpacks it, the DAW
//! ships native installers (NSIS .exe, .dmg, .AppImage) and we run
//! those silently behind the branded GUI. Net effect for the user: one
//! download, one branded window, full DAW installed at the end.

use crate::{InstallOptions, InstallProgress};
use base64::Engine as _;
use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[cfg(windows)]
/// Candidate executable names the post-install probe walks for. Tauri's
/// NSIS bundle uses `productName` (with spaces) for the binary; some
/// builds also leave a lower-case copy of the cargo `name`. List both
/// so detection works regardless of how the underlying installer
/// was bundled.
const DAW_EXE_CANDIDATES: &[&str] = &["Hardwave DAW.exe", "hardwave-daw.exe"];

/// Default install directory:
///   - Windows: %LocalAppData%\Programs\Hardwave\DAW
///   - macOS:   ~/Applications  (the .app installs here directly)
///   - Linux:   ~/.local/share/hardwave-daw
pub fn default_install_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default())
            .join("Programs")
            .join("Hardwave")
            .join("DAW")
    }
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().unwrap_or_default().join("Applications")
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        dirs::data_local_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
            .join("hardwave-daw")
    }
}

fn emit(app: &AppHandle, phase: &str, percent: u32, message: &str) {
    let _ = app.emit(
        "install://progress",
        InstallProgress {
            phase: phase.to_string(),
            percent,
            message: message.to_string(),
        },
    );
}

/// The public half of the key every Hardwave DAW update is signed with,
/// the same one the DAW's own updater trusts (src-tauri/tauri.conf.json;
/// a test keeps the two equal). The launcher runs what it downloads, so
/// it runs nothing that this key did not sign.
const UPDATER_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEYxQkQ4NkQyQTVCOUE0OApSV1JJbWxzcWJkZ2JEMGZPOHU0ZTVMeEtQMERwQTlxTEJYaDkxTGpNZVQzYkRSSlJHNUhsVnhJMgo=";

/// Whether `data` carries Hardwave's signature, given the `.sig` file
/// published beside it.
fn signed_by_hardwave(data: &[u8], sig_file: &str) -> Result<(), String> {
    let b64 = base64::engine::general_purpose::STANDARD;
    let key_text = String::from_utf8(b64.decode(UPDATER_PUBKEY).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let key = minisign_verify::PublicKey::decode(&key_text).map_err(|e| e.to_string())?;
    let sig_text = String::from_utf8(
        b64.decode(sig_file.trim())
            .map_err(|_| "the signature file is damaged".to_string())?,
    )
    .map_err(|_| "the signature file is damaged".to_string())?;
    let signature = minisign_verify::Signature::decode(&sig_text)
        .map_err(|_| "the signature file is damaged".to_string())?;
    key.verify(data, &signature, true)
        .map_err(|_| "the download is not signed by Hardwave".to_string())
}

/// Pretty installer asset suffix per platform. CI uploads with these
/// patterns to every release (see `.github/workflows/release.yml`).
fn target_asset_filter() -> &'static str {
    #[cfg(windows)]
    {
        "_x64-setup.exe"
    }
    #[cfg(target_os = "macos")]
    {
        ".dmg"
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        ".AppImage"
    }
}

pub async fn run(
    app: AppHandle,
    cancel: Arc<AtomicBool>,
    opts: InstallOptions,
) -> Result<String, String> {
    let install_dir = PathBuf::from(&opts.install_dir);

    // 1. Resolve the latest release's platform-specific installer URL,
    //    and its signature.
    emit(&app, "downloading", 0, "Finding latest Hardwave DAW…");
    let (download_url, signature_url) = resolve_download_url().await?;

    // 2. Prepare install dir (NSIS will create its own subtree on
    // Windows — directory just needs to exist).
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("Cannot create install directory: {e}"))?;

    // 3. Download the platform installer into a fresh folder of its own,
    //    which nothing else on the machine could have prepared, and
    //    check its signature before it may run.
    let temp_dir = tempfile::Builder::new()
        .prefix("hardwave-daw-installer-")
        .tempdir()
        .map_err(|e| format!("Cannot create temp directory: {e}"))?;
    let installer_path = temp_dir
        .path()
        .join(format!("hardwave-daw-installer{}", installer_suffix()));
    download_with_progress(&app, &cancel, &download_url, &installer_path).await?;

    if cancel.load(Ordering::SeqCst) {
        return Err("Install cancelled".into());
    }

    emit(&app, "downloading", 68, "Checking the download…");
    let signature = fetch_text(&signature_url).await?;
    let bytes = tokio::fs::read(&installer_path)
        .await
        .map_err(|e| format!("Cannot read the download: {e}"))?;
    if let Err(why) = signed_by_hardwave(&bytes, &signature) {
        return Err(format!(
            "The download could not be verified ({why}), so nothing was installed. \
             Try again, or download from https://hardwavestudios.com/daw"
        ));
    }
    drop(bytes);

    // 4. Run the platform installer silently.
    emit(&app, "installing", 70, "Installing Hardwave DAW…");
    let exe_path = run_platform_installer(&installer_path, &install_dir, &app).await?;
    drop(temp_dir);

    // NSIS handles its own desktop / start-menu shortcuts on Windows;
    // we honour the user's checkbox by trusting NSIS defaults rather
    // than writing custom shortcuts on top. macOS / Linux paths above
    // already place the binary somewhere the OS launcher can find it.
    let _ = opts.create_desktop_shortcut;
    let _ = opts.create_start_menu_shortcut;

    emit(&app, "done", 100, "Installation complete");

    if opts.launch_after {
        #[cfg(windows)]
        {
            let _ = std::process::Command::new(&exe_path).spawn();
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(&exe_path).spawn();
        }
        #[cfg(all(not(windows), not(target_os = "macos")))]
        {
            let _ = std::process::Command::new(&exe_path).spawn();
        }
    }

    Ok(exe_path.to_string_lossy().to_string())
}

fn installer_suffix() -> &'static str {
    #[cfg(windows)]
    {
        ".exe"
    }
    #[cfg(target_os = "macos")]
    {
        ".dmg"
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        ".AppImage"
    }
}

/// Run the downloaded platform installer; return the path of the
/// installed Hardwave DAW binary so the launcher knows what to start.
async fn run_platform_installer(
    installer_path: &Path,
    install_dir: &Path,
    _app: &AppHandle,
) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        // NSIS supports `/S` for silent install. `/D=<path>` is the
        // install destination override (must be the LAST argument and
        // must NOT have quotes — NSIS parses it specially).
        let status = std::process::Command::new(installer_path)
            .arg("/S")
            .arg(format!("/D={}", install_dir.display()))
            .status()
            .map_err(|e| format!("Failed to run installer: {e}"))?;
        if !status.success() {
            return Err(format!(
                "Installer exited with code {}",
                status.code().unwrap_or(-1)
            ));
        }
        // Walk well-known Tauri-NSIS install layouts plus the per-user
        // and per-machine fallbacks so we find the binary even when
        // the inner installer ignored /D= (perMachine bundles often do)
        // or used a different binary name. Returning a path lets the
        // launcher's `launch_after` step actually launch; if every
        // candidate misses, fall back to the user's chosen install_dir
        // — the install genuinely succeeded, the launcher just can't
        // auto-start the binary, and that is recoverable from the
        // Done screen.
        let mut search_roots: Vec<PathBuf> = vec![install_dir.to_path_buf()];
        if let Some(local) = dirs::data_local_dir() {
            search_roots.push(local.join("Programs").join("Hardwave").join("DAW"));
            search_roots.push(local.join("Programs").join("Hardwave DAW"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            search_roots.push(PathBuf::from(&pf).join("Hardwave").join("DAW"));
            search_roots.push(PathBuf::from(&pf).join("Hardwave DAW"));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            search_roots.push(PathBuf::from(&pf86).join("Hardwave").join("DAW"));
            search_roots.push(PathBuf::from(&pf86).join("Hardwave DAW"));
        }
        for root in &search_roots {
            for name in DAW_EXE_CANDIDATES {
                let direct = root.join(name);
                if direct.exists() {
                    return Ok(direct);
                }
            }
            // One level deep — Tauri sometimes nests inside a versioned subdir.
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if !p.is_dir() {
                        continue;
                    }
                    for name in DAW_EXE_CANDIDATES {
                        let candidate = p.join(name);
                        if candidate.exists() {
                            return Ok(candidate);
                        }
                    }
                }
            }
        }
        // Soft-fail: install really did finish; we just couldn't find
        // the exe to launch. Return the install_dir as the path so the
        // Done screen can show "files installed at <path>" and offer a
        // manual-launch link via the OS file explorer.
        eprintln!(
            "post-install probe could not locate Hardwave DAW.exe under any of {:?}; returning install_dir as best-effort path so the Done screen still works",
            search_roots
        );
        Ok(install_dir.to_path_buf())
    }
    #[cfg(target_os = "macos")]
    {
        // Mount the .dmg, copy the .app to the chosen install dir
        // (defaults to ~/Applications), unmount.
        let mount_point =
            std::env::temp_dir().join(format!("hardwave-daw-mnt-{}", std::process::id()));
        std::fs::create_dir_all(&mount_point).ok();
        let attach = std::process::Command::new("hdiutil")
            .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
            .arg(&mount_point)
            .arg(installer_path)
            .status()
            .map_err(|e| format!("hdiutil attach failed: {e}"))?;
        if !attach.success() {
            return Err("Failed to mount .dmg".into());
        }
        let mut app_src: Option<PathBuf> = None;
        if let Ok(entries) = std::fs::read_dir(&mount_point) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("app") {
                    app_src = Some(p);
                    break;
                }
            }
        }
        let app_src = app_src.ok_or("No .app inside mounted .dmg")?;
        let app_name = app_src.file_name().ok_or("Bad .app name on mounted .dmg")?;
        let app_dst = install_dir.join(app_name);
        // rsync keeps perms / symlinks / extended attrs that fs::copy_dir loses.
        let copy = std::process::Command::new("rsync")
            .args(["-a", "--delete"])
            .arg(format!("{}/", app_src.display()))
            .arg(&app_dst)
            .status();
        let _ = std::process::Command::new("hdiutil")
            .args(["detach", "-quiet"])
            .arg(&mount_point)
            .status();
        let _ = std::fs::remove_dir(&mount_point);
        match copy {
            Ok(s) if s.success() => Ok(app_dst),
            Ok(s) => Err(format!("rsync exited {}", s.code().unwrap_or(-1))),
            Err(e) => Err(format!("rsync failed: {e}")),
        }
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        // Linux: copy the AppImage to <install_dir>/Hardwave-DAW.AppImage,
        // chmod +x, write a .desktop file under
        // ~/.local/share/applications so the menu picks it up.
        std::fs::create_dir_all(install_dir).ok();
        let dest = install_dir.join("Hardwave-DAW.AppImage");
        std::fs::copy(installer_path, &dest)
            .map_err(|e| format!("Failed to copy AppImage: {e}"))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).ok();
        if let Some(home) = dirs::home_dir() {
            let apps = home.join(".local/share/applications");
            std::fs::create_dir_all(&apps).ok();
            let entry = apps.join("hardwave-daw.desktop");
            let body = format!(
                "[Desktop Entry]\nName=Hardwave DAW\nExec={}\nIcon=hardwave-daw\nType=Application\nCategories=AudioVideo;Audio;\n",
                dest.display()
            );
            let _ = std::fs::write(entry, body);
        }
        Ok(dest)
    }
}

/// Resolve the platform-specific installer asset URL on the latest
/// Hardwave DAW GitHub release. The asset filter lives in
/// `target_asset_filter()` and matches the suffix the DAW's release.yml
/// uploads.
async fn resolve_download_url() -> Result<(String, String), String> {
    let client = reqwest::Client::builder()
        .user_agent("HardwaveDawInstaller/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    // The latest full release only: a release candidate is for testers
    // and carries a dash in its tag, which GitHub marks as prerelease.
    let gh: serde_json::Value = client
        .get("https://api.github.com/repos/Dishairano/hardwave-daw/releases/latest")
        .send()
        .await
        .map_err(|e| format!("GitHub API failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub API error: {e}"))?
        .json()
        .await
        .map_err(|e| format!("GitHub API parse failed: {e}"))?;

    let assets = gh
        .get("assets")
        .and_then(|v| v.as_array())
        .ok_or("No assets on latest release")?;
    let suffix = target_asset_filter();
    let url_of = |wanted: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a.get("name")?.as_str()?;
            if wanted(name) {
                a.get("browser_download_url")?.as_str().map(str::to_string)
            } else {
                None
            }
        })
    };
    let installer = url_of(&|name| name.ends_with(suffix));
    let signature = url_of(&|name| name.ends_with(&format!("{suffix}.sig")));
    match (installer, signature) {
        (Some(installer), Some(signature)) => Ok((installer, signature)),
        _ => Err(format!(
            "Could not find a signed installer ending in {suffix} on the latest Hardwave DAW release. \
             Please re-run the installer, or download manually from \
             https://github.com/Dishairano/hardwave-daw/releases/latest"
        )),
    }
}

async fn fetch_text(url: &str) -> Result<String, String> {
    reqwest::Client::builder()
        .user_agent("HardwaveDawInstaller/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Could not fetch the signature: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Could not fetch the signature: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Could not fetch the signature: {e}"))
}

async fn download_with_progress(
    app: &AppHandle,
    cancel: &Arc<AtomicBool>,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .user_agent("HardwaveDawInstaller/0.1")
        .build()
        .map_err(|e| e.to_string())?;

    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download HTTP error: {e}"))?;

    let total = res.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Cannot create {dest:?}: {e}"))?;

    let mut stream = res.bytes_stream();
    let mut downloaded: u64 = 0;
    let mut last_pct: u32 = u32::MAX;

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err("Install cancelled".into());
        }
        let bytes = chunk.map_err(|e| format!("Download stream error: {e}"))?;
        file.write_all(&bytes)
            .await
            .map_err(|e| format!("Write error: {e}"))?;
        downloaded += bytes.len() as u64;

        if total > 0 {
            // 0-65% download, 65-95% silent install, 95-100% finish.
            let pct = ((downloaded as f64 / total as f64) * 65.0) as u32;
            if pct != last_pct {
                last_pct = pct;
                let msg = format!(
                    "Downloading… {} / {}",
                    human_bytes(downloaded),
                    human_bytes(total)
                );
                emit(app, "downloading", pct, &msg);
            }
        }
    }
    file.flush().await.ok();
    Ok(())
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = n as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_trusts_the_same_key_as_the_daw() {
        let conf = include_str!("../../../src-tauri/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(conf).unwrap();
        assert_eq!(
            conf["plugins"]["updater"]["pubkey"].as_str(),
            Some(UPDATER_PUBKEY)
        );
    }

    /// Run by hand with a published installer and its .sig:
    /// HW_INSTALLER=... HW_INSTALLER_SIG=... cargo test -- --ignored
    #[test]
    #[ignore = "needs a published installer on disk"]
    fn a_published_installer_verifies() {
        let data = std::fs::read(std::env::var("HW_INSTALLER").unwrap()).unwrap();
        let sig = std::fs::read_to_string(std::env::var("HW_INSTALLER_SIG").unwrap()).unwrap();
        signed_by_hardwave(&data, &sig).unwrap();
        let mut tampered = data.clone();
        tampered[1000] ^= 1;
        assert!(
            signed_by_hardwave(&tampered, &sig).is_err(),
            "one changed byte is refused"
        );
    }

    #[test]
    fn anything_not_signed_by_hardwave_is_refused() {
        assert!(signed_by_hardwave(b"some installer", "not a signature").is_err());
        // A well-formed signature from some other key.
        let other = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduIHNlY3JldCBrZXkKUlVReGM3UXhLV0JCMktJanZhUmdOcU5sOWVvQkhwZkZ1T2ZGbWJYV3RSR1NIUURHVXJrN3lHaVpPa2RGRWxZRXdSbG1Pd01aU3FFZkhvMTJ4bmtodkFKWXJvVEJCcDdNYmdNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzAwMDAwMDAwCWZpbGU6eC5leGUKK3l0bXlKK3Z6dGV0d0RyUFpZTnRIRkRBQjVacTdIS0N2Y2c1VjVzTVVHb1lQSU1oMzBUdWZ0OVhlQjlVc0RhWkxrb2hhQ1h3d3VJaG9BdTRJUkFmQ0E9PQo=";
        assert!(signed_by_hardwave(b"some installer", other).is_err());
    }
}
