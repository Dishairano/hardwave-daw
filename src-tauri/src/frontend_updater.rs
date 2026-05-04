//! Frontend updater — fetches a newer UI bundle from
//! https://hardwavestudios.com/daw/frontend/manifest.json on launch and
//! extracts it to the app's local data dir. The custom URI scheme handler
//! (registered in `lib.rs` — Commit 2) reads from this cache when present
//! and falls back to the bundled assets otherwise. Splash-driven UX:
//! status events are emitted to the frontend so the user sees what's
//! happening rather than a silent stall.
//!
//! Failure modes (no network / bad manifest / hash mismatch / timeout)
//! all degrade silently to the bundled assets — the producer never sees
//! an error mid-launch. Errors are logged for triage.
//!
//! See proposal at suite.hardwavestudios.com/frontend-updater-mockup/

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

/// Hard-coded production manifest URL. Override via env var for staging.
const DEFAULT_MANIFEST_URL: &str =
    "https://hardwavestudios.com/daw/frontend/manifest.json";

/// Total budget the splash will wait. Past this we abort and use the bundled
/// frontend; the next launch tries again. Keep tight so cold-start UX
/// stays snappy on weak connections.
const TOTAL_BUDGET_SECS: u64 = 5;

/// Per-request budget — the manifest fetch alone is small enough that 2 s
/// is generous. Download has its own bigger budget below.
const MANIFEST_TIMEOUT_SECS: u64 = 2;
const DOWNLOAD_TIMEOUT_SECS: u64 = 4;

/// Rust core's API contract version. Bumped whenever Tauri commands or
/// event payloads change in a way that would break older frontends. The
/// manifest's `requires_api` field is matched against this with semver
/// range syntax (e.g. `^1.5`).
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Status events emitted to the splash on the `frontend-update-status`
/// channel. The frontend listens and renders the matching string under
/// the loading bar.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Reaching out to the manifest endpoint.
    Checking,
    /// Bundle download in progress. `downloaded` and `total` are bytes.
    Downloading { downloaded: u64, total: u64 },
    /// SHA-256 verification step.
    Verifying,
    /// Extracting the verified zip to the cache dir.
    Applying,
    /// New bundle written to cache. Active on next launch.
    Ready { version: String },
    /// No newer bundle available, or running version is already latest.
    UpToDate,
    /// Update was found but the running Rust binary is too old/new.
    Incompatible {
        manifest_requires: String,
        running: String,
    },
    /// Anything that prevented an update — wrapped in plain English for
    /// the user. The frontend may show this briefly or swallow it; the
    /// app continues with the bundled fallback either way.
    Skipped { reason: String },
}

#[derive(Debug, Deserialize)]
struct Manifest {
    latest_version: String,
    #[serde(default)]
    #[allow(dead_code)]
    published_at: Option<String>,
    #[serde(default = "default_requires_api")]
    requires_api: String,
    bundle: BundleSpec,
    #[serde(default)]
    #[allow(dead_code)]
    changelog: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BundleSpec {
    url: String,
    size_bytes: u64,
    sha256: String,
}

fn default_requires_api() -> String {
    "*".to_string()
}

/// Reads the currently active frontend version from `<cache>/active.txt`.
/// Returns `None` if no cache exists yet — i.e. we're running the
/// bundled frontend that shipped with the installer.
pub fn read_active_version(cache_root: &Path) -> Option<String> {
    let marker = cache_root.join("active.txt");
    std::fs::read_to_string(marker)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn manifest_url() -> String {
    std::env::var("HARDWAVE_FRONTEND_MANIFEST_URL")
        .unwrap_or_else(|_| DEFAULT_MANIFEST_URL.to_string())
}

fn cache_root(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| format!("could not resolve app_local_data_dir: {e}"))?
        .join("frontend-cache");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create cache dir: {e}"))?;
    Ok(dir)
}

fn emit(app: &AppHandle, status: UpdateStatus) {
    if let Err(e) = app.emit("frontend-update-status", status) {
        log::warn!("could not emit frontend-update-status: {e}");
    }
}

fn version_is_newer(remote: &str, local: &str) -> bool {
    match (semver::Version::parse(remote), semver::Version::parse(local)) {
        (Ok(r), Ok(l)) => r > l,
        _ => remote != local, // fall back to string-inequality if parsing fails
    }
}

fn api_compatible(required: &str, running: &str) -> bool {
    let Ok(req) = semver::VersionReq::parse(required) else {
        return required == "*"; // tolerate the default
    };
    let Ok(run) = semver::Version::parse(running) else {
        return false;
    };
    req.matches(&run)
}

/// Main entry point — called once at app start, race-conditioned against
/// the splash screen's animation budget. Always returns `Ok(())`; failures
/// are logged and surfaced as `UpdateStatus::Skipped` events so the splash
/// can advance.
pub async fn check_and_apply(app: AppHandle) -> Result<(), String> {
    let result = tokio::time::timeout(
        Duration::from_secs(TOTAL_BUDGET_SECS),
        check_and_apply_inner(app.clone()),
    )
    .await;

    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            log::warn!("frontend update skipped: {e}");
            emit(&app, UpdateStatus::Skipped { reason: e });
            Ok(())
        }
        Err(_) => {
            log::warn!("frontend update timed out (budget {TOTAL_BUDGET_SECS}s)");
            emit(
                &app,
                UpdateStatus::Skipped {
                    reason: "timed out".to_string(),
                },
            );
            Ok(())
        }
    }
}

async fn check_and_apply_inner(app: AppHandle) -> Result<(), String> {
    emit(&app, UpdateStatus::Checking);

    let cache = cache_root(&app)?;
    let active = read_active_version(&cache);
    let local_version = active.as_deref().unwrap_or(API_VERSION);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(MANIFEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let manifest: Manifest = client
        .get(manifest_url())
        .send()
        .await
        .map_err(|e| format!("manifest request: {e}"))?
        .error_for_status()
        .map_err(|e| format!("manifest status: {e}"))?
        .json()
        .await
        .map_err(|e| format!("manifest parse: {e}"))?;

    if !version_is_newer(&manifest.latest_version, local_version) {
        emit(&app, UpdateStatus::UpToDate);
        return Ok(());
    }

    if !api_compatible(&manifest.requires_api, API_VERSION) {
        emit(
            &app,
            UpdateStatus::Incompatible {
                manifest_requires: manifest.requires_api.clone(),
                running: API_VERSION.to_string(),
            },
        );
        return Ok(());
    }

    // Download bundle, streaming so we can emit progress.
    let download_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("build download client: {e}"))?;

    let response = download_client
        .get(&manifest.bundle.url)
        .send()
        .await
        .map_err(|e| format!("bundle download: {e}"))?
        .error_for_status()
        .map_err(|e| format!("bundle status: {e}"))?;

    let total = manifest.bundle.size_bytes;
    let mut downloaded: u64 = 0;
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(total as usize);

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream chunk: {e}"))?;
        downloaded += chunk.len() as u64;
        hasher.update(&chunk);
        bytes.extend_from_slice(&chunk);
        emit(
            &app,
            UpdateStatus::Downloading { downloaded, total },
        );
    }

    emit(&app, UpdateStatus::Verifying);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(&manifest.bundle.sha256) {
        return Err(format!(
            "sha256 mismatch (expected {}, got {actual})",
            manifest.bundle.sha256
        ));
    }

    emit(&app, UpdateStatus::Applying);
    let staging = cache.join("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| format!("clean staging: {e}"))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("create staging: {e}"))?;

    {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .map_err(|e| format!("zip open: {e}"))?;
        archive
            .extract(&staging)
            .map_err(|e| format!("zip extract: {e}"))?;
    }

    // Atomically promote staging -> versioned dir.
    let versioned = cache.join(&manifest.latest_version);
    if versioned.exists() {
        std::fs::remove_dir_all(&versioned).map_err(|e| format!("clean old version dir: {e}"))?;
    }
    std::fs::rename(&staging, &versioned).map_err(|e| format!("promote staging: {e}"))?;

    // Mark this version active. Custom protocol handler (Commit 2) reads
    // active.txt on next launch and serves files from <cache>/<version>/.
    let active_marker = cache.join("active.txt");
    let tmp = cache.join("active.txt.tmp");
    {
        let mut f = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| format!("create active.txt.tmp: {e}"))?;
        f.write_all(manifest.latest_version.as_bytes())
            .await
            .map_err(|e| format!("write active.txt.tmp: {e}"))?;
        f.sync_all()
            .await
            .map_err(|e| format!("sync active.txt.tmp: {e}"))?;
    }
    std::fs::rename(&tmp, &active_marker).map_err(|e| format!("promote active.txt: {e}"))?;

    emit(
        &app,
        UpdateStatus::Ready {
            version: manifest.latest_version.clone(),
        },
    );
    log::info!(
        "frontend updater: staged version {} (active on next launch)",
        manifest.latest_version
    );
    Ok(())
}

/// Tauri command bridge — called by the splash screen JS at app launch.
/// Always resolves so the splash never blocks on a runtime error.
#[tauri::command]
pub async fn frontend_update_check_and_apply(app: AppHandle) -> Result<(), String> {
    check_and_apply(app).await
}

/// Returns the active frontend version (if any) plus the API version of
/// the running Rust core. Used by the settings page when we add it later.
#[tauri::command]
pub fn frontend_update_status(app: AppHandle) -> Result<FrontendVersionInfo, String> {
    let cache = cache_root(&app)?;
    Ok(FrontendVersionInfo {
        api_version: API_VERSION.to_string(),
        active_frontend_version: read_active_version(&cache),
    })
}

#[derive(Debug, Serialize)]
pub struct FrontendVersionInfo {
    pub api_version: String,
    pub active_frontend_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_remote_wins() {
        assert!(version_is_newer("1.5.4", "1.5.3"));
        assert!(!version_is_newer("1.5.3", "1.5.4"));
        assert!(!version_is_newer("1.5.3", "1.5.3"));
    }

    #[test]
    fn fallback_to_string_compare_when_unparseable() {
        // If either side isn't valid semver, we still notice difference.
        assert!(version_is_newer("not-a-version", "0.0.0"));
        assert!(!version_is_newer("same", "same"));
    }

    #[test]
    fn api_compat_basic() {
        assert!(api_compatible("^1.5", "1.5.0"));
        assert!(api_compatible("^1.5", "1.5.99"));
        assert!(!api_compatible("^1.5", "1.4.0"));
        assert!(!api_compatible("^1.5", "2.0.0"));
        // Default wildcard accepts anything.
        assert!(api_compatible("*", "0.157.28"));
    }
}
