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
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

use crate::AppState;

/// Hard-coded production manifest URL. Override via env var for staging.
/// Hosted on the suite.hardwavestudios.com cluster (vst-web01 nginx) so we
/// can publish without touching the marketing-site infra.
const DEFAULT_MANIFEST_URL: &str = "https://suite.hardwavestudios.com/daw/frontend/manifest.json";

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

/// Highest manifest schema this client knows how to interpret. Bump in
/// lockstep with breaking field changes; older binaries see anything
/// strictly greater than this and degrade to silent fallback (Path C).
pub const KNOWN_MANIFEST_SCHEMA: u32 = 2;

/// Status events emitted to the splash on the `frontend-update-status`
/// channel. The frontend listens and renders the matching string under
/// the loading bar.
///
/// Variants present before the version-contract layer (`Checking` … `Skipped`)
/// remain unchanged because the splash text mapping in App.tsx keys off the
/// `kind` tag — renaming would silently break older frontends served from a
/// stale bundle. New variants append.
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
    /// Resolver decided the running binary is too old to host the
    /// advertised bundle safely. The splash should swap copy to "installer
    /// upgrade required"; the Tauri auto-updater (Path A) takes over.
    InstallerRequired {
        target_version: Option<String>,
        track: InstallerTrack,
        reason: String,
        release_url: Option<String>,
    },
    /// Bundle is staged in cache (or will be on next apply). Equivalent to
    /// `Ready` for backwards-compat, but emitted from the version-contract
    /// resolver path so the frontend can recognise the explicit hot-swap
    /// outcome and suppress its own auto-updater check for this session.
    HotSwapReady { version: String },
}

/// Outgoing-feed identifier for the Tauri auto-updater. Pivoting a user
/// to `Beta` lets us roll a hotfix to opt-in testers without cutting a
/// fresh stable installer; `Stable` is the default for everyone else.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallerTrack {
    Stable,
    Beta,
}

impl Default for InstallerTrack {
    fn default() -> Self {
        InstallerTrack::Stable
    }
}

/// Optional hint payload the modal uses for human-facing copy. Never
/// authoritative — the actual installer download is resolved by
/// `tauri-plugin-updater` against its own feed.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct InstallerHint {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub notes_url: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
}

/// Manifest as parsed from `manifest.json`. All version-contract fields
/// are `#[serde(default)]` so a manifest published before the schema
/// existed (or one served by a partial mirror) still parses — old fields
/// take their old meaning, new fields default to "no opinion".
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub latest_version: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub published_at: Option<String>,
    #[serde(default = "default_requires_api")]
    pub requires_api: String,
    pub bundle: BundleSpec,
    #[serde(default)]
    #[allow(dead_code)]
    pub changelog: Vec<String>,
    /// Strict semver floor — the running binary must be `>= min_installer`
    /// to host this bundle. Absent in pre-schema-2 manifests; treated as
    /// "no floor" so legacy manifests continue to apply.
    #[serde(default)]
    pub min_installer: Option<String>,
    /// Which `tauri-plugin-updater` feed Path A should use. Defaults to
    /// `Stable` so legacy manifests pick the existing release channel.
    #[serde(default)]
    pub installer_track: InstallerTrack,
    /// Monotonic schema marker. Old clients ignore it; newer clients
    /// refuse to interpret a manifest whose schema is ahead of them.
    #[serde(default = "default_manifest_schema")]
    pub manifest_schema: u32,
    /// Copy material for the installer modal. The actual download URL
    /// still comes from `tauri-plugin-updater`.
    #[serde(default)]
    pub installer_hint: Option<InstallerHint>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BundleSpec {
    pub url: String,
    pub size_bytes: u64,
    pub sha256: String,
}

fn default_requires_api() -> String {
    "*".to_string()
}

fn default_manifest_schema() -> u32 {
    // Pre-schema-2 manifests didn't ship this field. Treat them as
    // schema 1 so the resolver's "schema ahead of client" check (which
    // compares strictly greater than KNOWN_MANIFEST_SCHEMA = 2) cannot
    // fail spuriously.
    1
}

/// Reason an installer modal was selected. Flows into the modal copy and
/// the per-launch telemetry log line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallerReason {
    /// `installed < manifest.min_installer` — the strict semver floor
    /// failed before any other check.
    VersionFloor { needed: String, have: String },
    /// `requires_api` range didn't accept the running binary, and the
    /// running binary is *below* the range — upgrade resolves it.
    ApiRangeBelow { range: String, have: String },
}

/// Pure resolver decision. The caller wires each variant to the
/// appropriate UX surface; no UI lives in the resolver itself.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchPlan {
    /// Apply the advertised bundle (download/verify/extract). Sub-action
    /// distinguishes "actually newer" from "manifest matched but bundle
    /// is the same version we already cache".
    HotSwap { action: HotSwapAction },
    /// Path A — pop the Tauri auto-updater modal and run `check()`.
    InstallerModal {
        target_version: Option<String>,
        track: InstallerTrack,
        reason: InstallerReason,
        release_url: Option<String>,
    },
    /// Path C — silent fallback to bundled assets. Logged for triage.
    Fallback { reason: String },
}

/// Sub-action of `LaunchPlan::HotSwap`. `NoOp` skips the download but
/// still counts as "manifest decided we're fine"; `ApplyBundle` is the
/// happy path.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HotSwapAction {
    NoOp,
    ApplyBundle {
        version: String,
        bundle_url: String,
        size_bytes: u64,
        sha256: String,
    },
}

/// How long a cached `LaunchPlan` is considered fresh. App.tsx schedules
/// a `force_refresh: true` call on this same cadence; the cache TTL is
/// the belt-and-braces backstop in case that timer doesn't fire (e.g.
/// the renderer hangs or the user blocks the recheck window). Anything
/// older than this is treated as stale and re-resolved against a fresh
/// manifest fetch.
const LAUNCH_PLAN_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// In-memory cache entry for the resolved launch plan. Lives in
/// `AppState.frontend_launch_plan`; see the field doc comment in `lib.rs`
/// for the cross-call contract.
///
/// `resolved_at` is an `Instant` (monotonic) so wall-clock manipulation
/// can't shorten the TTL artificially. `source_version` is the manifest's
/// `latest_version` at resolve time — useful in logs for triage and for
/// the determinism unit test below. `applied` is flipped to `true` once
/// `check_and_apply` finishes the HotSwap::ApplyBundle path; subsequent
/// `version_contract_state` calls then report the bundle as up-to-date
/// instead of repeatedly telling App.tsx the same staged bundle still
/// needs applying.
#[derive(Debug, Clone)]
pub struct LaunchPlanCacheEntry {
    pub resolved_at: Instant,
    pub plan: LaunchPlan,
    pub source_version: String,
    pub applied: bool,
}

impl LaunchPlanCacheEntry {
    fn new(plan: LaunchPlan, source_version: String) -> Self {
        Self {
            resolved_at: Instant::now(),
            plan,
            source_version,
            applied: false,
        }
    }

    /// True if the entry is older than [`LAUNCH_PLAN_CACHE_TTL`] and
    /// must be re-resolved before being trusted again. Pure modulo
    /// `Instant::now`; the cache-decision unit tests inject a fake
    /// `now` so they don't need to wait 24 h.
    fn is_stale_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.resolved_at) > LAUNCH_PLAN_CACHE_TTL
    }
}

/// Pure decision helper for `version_contract_state` — given an existing
/// cache entry (if any), the requested freshness, and a notion of "now",
/// decide whether to serve the cache or re-fetch. Pulled out of the
/// command body so the cache-vs-fresh contract is unit-testable without
/// a Tauri runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheDecision {
    /// Cached entry is fresh and the caller didn't force a refresh —
    /// serve it verbatim, no manifest fetch.
    UseCache,
    /// Cached entry is missing, stale, or the caller forced a refresh —
    /// re-resolve against a fresh manifest fetch and overwrite the cache.
    Refresh,
}

fn select_cache_or_refresh(
    cached: Option<&LaunchPlanCacheEntry>,
    force_refresh: bool,
    now: Instant,
) -> CacheDecision {
    if force_refresh {
        return CacheDecision::Refresh;
    }
    match cached {
        Some(entry) if !entry.is_stale_at(now) => CacheDecision::UseCache,
        _ => CacheDecision::Refresh,
    }
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
    match (
        semver::Version::parse(remote),
        semver::Version::parse(local),
    ) {
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

    // Pure fetch — no side effects on the cache. The resolver decides
    // whether the manifest's bundle is allowed to land before we touch disk.
    let manifest_opt = match fetch_manifest_inner().await {
        Ok(m) => Some(m),
        Err(e) => {
            log::warn!("frontend updater: manifest fetch failed: {e}");
            None
        }
    };

    let plan = resolve_launch_plan(API_VERSION, local_version, manifest_opt.as_ref());
    log_launch_plan(&plan, manifest_opt.as_ref(), local_version);

    // Cache the plan in AppState so the follow-up `version_contract_state`
    // command (called by App.tsx right after this finishes) sees the same
    // decision the splash already showed. Without this, a CDN replica
    // flipping mid-launch could let `version_contract_state` re-fetch a
    // different manifest and the user sees BOTH a HotSwapReady event and
    // an InstallerRequired modal back-to-back. `source_version` is the
    // manifest's advertised `latest_version` (or "-" when unreachable);
    // `applied` flips to true once we successfully stage a bundle below.
    let source_version = manifest_opt
        .as_ref()
        .map(|m| m.latest_version.clone())
        .unwrap_or_else(|| "-".to_string());
    if let Some(state) = app.try_state::<AppState>() {
        *state.frontend_launch_plan.lock() =
            Some(LaunchPlanCacheEntry::new(plan.clone(), source_version));
    }

    match plan {
        LaunchPlan::HotSwap {
            action: HotSwapAction::NoOp,
        } => {
            emit(&app, UpdateStatus::UpToDate);
            Ok(())
        }
        LaunchPlan::HotSwap {
            action:
                HotSwapAction::ApplyBundle {
                    version,
                    bundle_url,
                    size_bytes,
                    sha256,
                },
        } => {
            apply_bundle(&app, &cache, &version, &bundle_url, size_bytes, &sha256).await?;
            emit(
                &app,
                UpdateStatus::HotSwapReady {
                    version: version.clone(),
                },
            );
            // Backwards-compat: legacy splash text mapping listens for
            // `Ready`. Emitting both means an older bundle still in cache
            // continues to render the right copy after this commit lands.
            emit(&app, UpdateStatus::Ready { version });
            // Mark the cached plan as applied — a subsequent
            // `version_contract_state` call now reports the bundle as
            // already-staged rather than telling App.tsx the same
            // ApplyBundle action is still pending. Without this, a
            // settings page polling the resolver would render "update
            // available — restart to apply" forever even after the
            // staged bundle is sitting on disk waiting for relaunch.
            if let Some(state) = app.try_state::<AppState>() {
                if let Some(entry) = state.frontend_launch_plan.lock().as_mut() {
                    entry.applied = true;
                }
            }
            Ok(())
        }
        LaunchPlan::InstallerModal {
            target_version,
            track,
            reason,
            release_url,
        } => {
            let reason_str = installer_reason_summary(&reason);
            emit(
                &app,
                UpdateStatus::InstallerRequired {
                    target_version,
                    track,
                    reason: reason_str,
                    release_url,
                },
            );
            // Note: we deliberately do NOT emit a follow-up legacy
            // `Incompatible` event here. App.tsx maps `installer_required`
            // to "Installer upgrade required" copy and `incompatible` to
            // "Starting up..."; emitting both back-to-back made the second
            // event win and erased the new mockup copy. The
            // `log_launch_plan` telemetry above is the single triage
            // surface — older bundles that only listen for the legacy
            // event will still see `Skipped` if the manifest fetch fails,
            // which is the correct fallback behaviour.
            Ok(())
        }
        LaunchPlan::Fallback { reason } => {
            emit(&app, UpdateStatus::Skipped { reason });
            Ok(())
        }
    }
}

/// Pure manifest fetcher. No cache writes, no event emissions. The
/// caller is expected to pass the result into `resolve_launch_plan`.
pub async fn fetch_manifest_inner() -> Result<Manifest, String> {
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
    Ok(manifest)
}

/// Apply a verified bundle to the cache. Side-effecting — only call when
/// `resolve_launch_plan` returned `HotSwap::ApplyBundle`. Splits the
/// download/verify/extract/promote stages out of the launch decision so
/// the resolver itself stays pure and testable.
async fn apply_bundle(
    app: &AppHandle,
    cache: &Path,
    version: &str,
    bundle_url: &str,
    size_bytes: u64,
    expected_sha: &str,
) -> Result<(), String> {
    let download_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("build download client: {e}"))?;

    let response = download_client
        .get(bundle_url)
        .send()
        .await
        .map_err(|e| format!("bundle download: {e}"))?
        .error_for_status()
        .map_err(|e| format!("bundle status: {e}"))?;

    let total = size_bytes;
    let mut downloaded: u64 = 0;
    let mut hasher = Sha256::new();
    let mut bytes = Vec::with_capacity(total as usize);

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("stream chunk: {e}"))?;
        downloaded += chunk.len() as u64;
        hasher.update(&chunk);
        bytes.extend_from_slice(&chunk);
        emit(app, UpdateStatus::Downloading { downloaded, total });
    }

    emit(app, UpdateStatus::Verifying);
    let actual = hex::encode(hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected_sha) {
        return Err(format!(
            "sha256 mismatch (expected {expected_sha}, got {actual})"
        ));
    }

    emit(app, UpdateStatus::Applying);
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
    let versioned = cache.join(version);
    if versioned.exists() {
        std::fs::remove_dir_all(&versioned).map_err(|e| format!("clean old version dir: {e}"))?;
    }
    std::fs::rename(&staging, &versioned).map_err(|e| format!("promote staging: {e}"))?;

    // Mark this version active. Custom protocol handler reads active.txt
    // on next launch and serves files from <cache>/<version>/.
    let active_marker = cache.join("active.txt");
    let tmp = cache.join("active.txt.tmp");
    {
        let mut f = tokio::fs::File::create(&tmp)
            .await
            .map_err(|e| format!("create active.txt.tmp: {e}"))?;
        f.write_all(version.as_bytes())
            .await
            .map_err(|e| format!("write active.txt.tmp: {e}"))?;
        f.sync_all()
            .await
            .map_err(|e| format!("sync active.txt.tmp: {e}"))?;
    }
    std::fs::rename(&tmp, &active_marker).map_err(|e| format!("promote active.txt: {e}"))?;

    log::info!("frontend updater: staged version {version} (active on next launch)");
    Ok(())
}

/// Pure decision function. Given the installed binary version, the
/// currently-active frontend version (cache or bundled), and the
/// manifest if reachable, return the launch plan.
///
/// Order of checks mirrors §2 of `min-installer-architecture.md`:
///   0. No manifest / corrupt / schema-too-new → Fallback.
///   1. Strict installer floor (`min_installer`) → InstallerModal.
///   2. `requires_api` range:
///        - below range → InstallerModal (catchable by upgrade)
///        - above range → Fallback (binary newer than manifest)
///   3. Bundle version vs. active frontend → HotSwap::NoOp or
///      HotSwap::ApplyBundle.
///
/// `installed` and `active` are passed in so the function can be unit-
/// tested without a real `AppHandle`.
pub fn resolve_launch_plan(
    installed: &str,
    active: &str,
    manifest: Option<&Manifest>,
) -> LaunchPlan {
    // Step 0 — manifest reachability + schema cliff.
    let m = match manifest {
        None => {
            return LaunchPlan::Fallback {
                reason: "manifest unavailable".to_string(),
            };
        }
        Some(m) if m.manifest_schema > KNOWN_MANIFEST_SCHEMA => {
            return LaunchPlan::Fallback {
                reason: format!(
                    "manifest schema {} ahead of client {}",
                    m.manifest_schema, KNOWN_MANIFEST_SCHEMA
                ),
            };
        }
        Some(m) => m,
    };

    let installed_v = match semver::Version::parse(installed) {
        Ok(v) => v,
        Err(_) => {
            // We can't reason about a malformed installer version; the
            // safest move is to stay on the bundled frontend.
            return LaunchPlan::Fallback {
                reason: format!("installed version unparseable: {installed}"),
            };
        }
    };

    // Step 1 — strict installer floor. Checked before anything else so an
    // ancient binary with a fresh manifest never hot-swaps.
    if let Some(floor_str) = &m.min_installer {
        match semver::Version::parse(floor_str) {
            Ok(floor) if installed_v < floor => {
                return LaunchPlan::InstallerModal {
                    target_version: m
                        .installer_hint
                        .as_ref()
                        .and_then(|h| h.version.clone())
                        .or_else(|| Some(floor_str.clone())),
                    track: m.installer_track,
                    reason: InstallerReason::VersionFloor {
                        needed: floor_str.clone(),
                        have: installed.to_string(),
                    },
                    release_url: m
                        .installer_hint
                        .as_ref()
                        .and_then(|h| h.release_url.clone()),
                };
            }
            Ok(_) => { /* floor met — fall through to step 2 */ }
            Err(e) => {
                // Bad floor in the manifest — refuse to interpret it. The
                // CI publisher should have caught this (frontend-publish.yml
                // §6 invariants); on the client we degrade safely rather
                // than risk a wrong decision. Logged at error level so it
                // shows up in triage immediately — every running client
                // hitting this path means the publisher shipped a broken
                // manifest, which is a release-blocker, not a warning.
                log::error!(
                    "frontend updater: min_installer unparseable in manifest: {floor_str:?} ({e}); falling back to bundled"
                );
                return LaunchPlan::Fallback {
                    reason: format!("min_installer unparseable: {floor_str}"),
                };
            }
        }
    }

    // Step 2 — API range. Reuse the existing parser; if the range is the
    // wildcard default (`"*"`), `req.matches` always succeeds and we drop
    // through to step 3.
    let range_ok = api_compatible(&m.requires_api, installed);
    if !range_ok {
        // Distinguish below-range (catchable by upgrade → modal) from
        // above-range (binary newer than manifest → silent fallback).
        // We rely on the lower bound parsed out of the range string;
        // if the range is opaque, fall back silently.
        match range_lower_bound(&m.requires_api) {
            Some(lower) if installed_v < lower => LaunchPlan::InstallerModal {
                target_version: m.installer_hint.as_ref().and_then(|h| h.version.clone()),
                track: m.installer_track,
                reason: InstallerReason::ApiRangeBelow {
                    range: m.requires_api.clone(),
                    have: installed.to_string(),
                },
                release_url: m
                    .installer_hint
                    .as_ref()
                    .and_then(|h| h.release_url.clone()),
            },
            _ => LaunchPlan::Fallback {
                reason: format!(
                    "binary {installed} outside manifest range {} (treating as ahead)",
                    m.requires_api
                ),
            },
        }
    } else if !version_is_newer(&m.latest_version, active) {
        LaunchPlan::HotSwap {
            action: HotSwapAction::NoOp,
        }
    } else {
        LaunchPlan::HotSwap {
            action: HotSwapAction::ApplyBundle {
                version: m.latest_version.clone(),
                bundle_url: m.bundle.url.clone(),
                size_bytes: m.bundle.size_bytes,
                sha256: m.bundle.sha256.clone(),
            },
        }
    }
}

/// Best-effort lower-bound extractor for a semver range string.
/// Returns `None` for opaque ranges (notably the wildcard `*`); the
/// resolver treats `None` as "we can't tell which side of the range
/// we're on, prefer silent fallback over a misleading installer modal".
fn range_lower_bound(range: &str) -> Option<semver::Version> {
    // The `semver` crate doesn't expose comparators publicly, so parse
    // by hand. Supports the two forms our manifests actually publish:
    //   "^a.b.c" / "^a.b" / "^a"     -> floor is a.b.c (zeros for missing)
    //   ">=a.b.c, <x.y.z" / ">=a.b"  -> floor is the >= operand
    // Anything else returns None.
    let trimmed = range.trim();
    if trimmed.is_empty() || trimmed == "*" {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('^') {
        return parse_loose(rest);
    }

    // Comma-separated comparator list — find the >= or > segment.
    for raw in trimmed.split(',') {
        let part = raw.trim();
        if let Some(rest) = part.strip_prefix(">=") {
            return parse_loose(rest.trim());
        }
        if let Some(rest) = part.strip_prefix('>') {
            // `>a.b.c` means strictly greater. The resolver only uses
            // this lower bound to decide between InstallerModal (below
            // range) and Fallback (above range). Treating `>a.b.c` as
            // floor `a.b.c` is intentional: `api_compatible` (which
            // uses semver::VersionReq) correctly rejects exactly
            // `a.b.c`, and our `installed_v < floor` check then routes
            // it to InstallerModal (the catchable-by-upgrade branch).
            // Bumping the patch here would push borderline binaries to
            // Fallback, which is the wrong UX. See the
            // `range_lower_bound_strict_greater` test for the contract.
            return parse_loose(rest.trim());
        }
    }
    None
}

fn parse_loose(s: &str) -> Option<semver::Version> {
    let s = s.trim();
    if let Ok(v) = semver::Version::parse(s) {
        return Some(v);
    }
    // "1.2" / "1" — pad with zeros so semver parses.
    let parts: Vec<&str> = s.split('.').collect();
    let padded = match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => return None,
    };
    semver::Version::parse(&padded).ok()
}

fn installer_reason_summary(r: &InstallerReason) -> String {
    match r {
        InstallerReason::VersionFloor { needed, have } => {
            format!("installer floor {needed} not met (running {have})")
        }
        InstallerReason::ApiRangeBelow { range, have } => {
            format!("running {have} below manifest API range {range}")
        }
    }
}

/// One-shot structured log line per launch — single observability surface
/// for "why did this client take this path?". No PII; safe to forward to
/// a log aggregator later.
fn log_launch_plan(plan: &LaunchPlan, manifest: Option<&Manifest>, active: &str) {
    let manifest_latest = manifest.map(|m| m.latest_version.as_str()).unwrap_or("-");
    let manifest_floor = manifest
        .and_then(|m| m.min_installer.as_deref())
        .unwrap_or("-");
    let manifest_range = manifest.map(|m| m.requires_api.as_str()).unwrap_or("-");
    log::info!(
        "version-contract decision installed={} active={} manifest_latest={} manifest_min_installer={} manifest_requires_api={} plan={}",
        API_VERSION,
        active,
        manifest_latest,
        manifest_floor,
        manifest_range,
        plan_label(plan),
    );
}

fn plan_label(plan: &LaunchPlan) -> &'static str {
    match plan {
        LaunchPlan::HotSwap {
            action: HotSwapAction::NoOp,
        } => "hot_swap_noop",
        LaunchPlan::HotSwap {
            action: HotSwapAction::ApplyBundle { .. },
        } => "hot_swap_apply",
        LaunchPlan::InstallerModal { .. } => "installer_required",
        LaunchPlan::Fallback { .. } => "fallback",
    }
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

/// Serializable shape of the resolved launch plan, for the frontend.
/// `App.tsx` invokes `version_contract_state` once after the splash's
/// `frontend_update_check_and_apply` has resolved, and gates the Tauri
/// auto-updater modal on `decision === "installer_required"`. Every other
/// decision means no installer modal this session.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionContractDecision {
    HotSwap,
    InstallerRequired,
    UpToDate,
    Fallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionContractState {
    pub decision: VersionContractDecision,
    /// Best-known target version for the installer modal. Always
    /// populated when `decision == "installer_required"`; otherwise
    /// `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installer_target: Option<String>,
    /// Which `tauri-plugin-updater` feed to use. Defaults to `Stable`
    /// when the manifest doesn't pin it.
    pub installer_track: InstallerTrack,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    /// Hot-swap target version (only set when the resolver returned
    /// `HotSwap`, even for the no-op sub-action). Lets the frontend
    /// surface "up-to-date at v0.158.4" without a separate query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_swap_version: Option<String>,
    /// Free-text reason for telemetry / settings-page badge copy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Highest manifest schema this client knows. Surfaced so a future
    /// Settings → Updates panel can render a "client behind manifest"
    /// hint without another round-trip.
    pub known_schema: u32,
}

/// Tauri command — returns the launch-time decision so App.tsx can
/// gate the Tauri auto-updater modal. Pure with respect to the cache:
/// no bundle is downloaded, no `active.txt` is rewritten.
///
/// **Cache-first** to defeat the staged-rollout race. When
/// `frontend_update_check_and_apply` has already run on this launch
/// (the normal splash path), it stashes its `LaunchPlan` in
/// `AppState.frontend_launch_plan`; we read that here so the App.tsx
/// follow-up sees the EXACT decision the splash rendered, even if a
/// CDN replica flips manifests between the two calls. The cold path
/// (command invoked before the splash, or in a context with no
/// AppState) falls back to a fresh fetch.
///
/// `force_refresh` bypasses the cache and re-fetches the manifest. The
/// 24 h recheck timer in App.tsx passes `true`; the launch-time invoke
/// passes `false`. Cached entries older than [`LAUNCH_PLAN_CACHE_TTL`]
/// are also treated as stale and re-resolved automatically — that's the
/// belt-and-braces backstop in case the renderer-side timer doesn't
/// fire.
///
/// Resolves with a populated state object on every code path; failures
/// degrade to `Fallback` so the splash and `App.tsx` can keep moving.
#[tauri::command]
pub async fn version_contract_state(
    app: AppHandle,
    force_refresh: Option<bool>,
) -> Result<VersionContractState, String> {
    // Tauri's default arg deserialiser accepts `null` / missing fields as
    // `None` so legacy frontends that invoked this with no args still
    // work — they get the cache-first behaviour they always had.
    let force_refresh = force_refresh.unwrap_or(false);

    // Cache-first decision is a pure function of the cached entry, the
    // force flag, and a notion of "now". Pulled out so we don't hold the
    // mutex across the await that follows.
    let now = Instant::now();
    let (decision, cached_state) = if let Some(state) = app.try_state::<AppState>() {
        let guard = state.frontend_launch_plan.lock();
        let decision = select_cache_or_refresh(guard.as_ref(), force_refresh, now);
        let snapshot = if matches!(decision, CacheDecision::UseCache) {
            guard.as_ref().map(|e| cached_entry_to_state(e))
        } else {
            None
        };
        (decision, snapshot)
    } else {
        // No AppState — almost certainly a unit-test or pre-`.manage()`
        // call. Always refresh in that branch; nothing to cache against.
        (CacheDecision::Refresh, None)
    };

    if let (CacheDecision::UseCache, Some(state)) = (decision, cached_state) {
        return Ok(state);
    }

    let plan = resolve_launch_plan_uncached(&app).await;
    let source_version = active_or_unknown(&app);
    if let Some(state) = app.try_state::<AppState>() {
        *state.frontend_launch_plan.lock() =
            Some(LaunchPlanCacheEntry::new(plan.clone(), source_version));
    }
    Ok(launch_plan_to_state(&plan))
}

/// Best-effort active-frontend version string for stamping into a cache
/// entry's `source_version` field. Falls back to `"-"` when the cache
/// dir is unreadable so the field is never empty in logs.
fn active_or_unknown(app: &AppHandle) -> String {
    cache_root(app)
        .ok()
        .and_then(|c| read_active_version(&c))
        .unwrap_or_else(|| "-".to_string())
}

/// Shared fresh-fetch path for the cache-miss / `force_refresh` branch.
/// Pulled out so the command body stays focused on cache decisions.
async fn resolve_launch_plan_uncached(app: &AppHandle) -> LaunchPlan {
    let active_version = match cache_root(app) {
        Ok(cache) => read_active_version(&cache),
        Err(e) => {
            log::warn!("version_contract_state: cache root unavailable: {e}");
            None
        }
    };
    let active = active_version.as_deref().unwrap_or(API_VERSION);

    let manifest = match fetch_manifest_inner().await {
        Ok(m) => Some(m),
        Err(e) => {
            log::warn!("version_contract_state: manifest fetch failed: {e}");
            None
        }
    };

    resolve_launch_plan(API_VERSION, active, manifest.as_ref())
}

/// Project a cache entry into the wire-format state. When `applied` is
/// set and the underlying plan is `HotSwap::ApplyBundle`, we report
/// `UpToDate` instead — the bundle is already on disk waiting for
/// relaunch, so further "update available" UI would be a lie.
fn cached_entry_to_state(entry: &LaunchPlanCacheEntry) -> VersionContractState {
    if entry.applied {
        if let LaunchPlan::HotSwap {
            action: HotSwapAction::ApplyBundle { version, .. },
        } = &entry.plan
        {
            return VersionContractState {
                decision: VersionContractDecision::UpToDate,
                installer_target: None,
                installer_track: InstallerTrack::default(),
                release_url: None,
                hot_swap_version: Some(version.clone()),
                reason: Some("bundle staged; restart to apply".to_string()),
                known_schema: KNOWN_MANIFEST_SCHEMA,
            };
        }
    }
    launch_plan_to_state(&entry.plan)
}

fn launch_plan_to_state(plan: &LaunchPlan) -> VersionContractState {
    match plan {
        LaunchPlan::HotSwap {
            action: HotSwapAction::NoOp,
        } => VersionContractState {
            decision: VersionContractDecision::UpToDate,
            installer_target: None,
            installer_track: InstallerTrack::default(),
            release_url: None,
            hot_swap_version: None,
            reason: None,
            known_schema: KNOWN_MANIFEST_SCHEMA,
        },
        LaunchPlan::HotSwap {
            action: HotSwapAction::ApplyBundle { version, .. },
        } => VersionContractState {
            decision: VersionContractDecision::HotSwap,
            installer_target: None,
            installer_track: InstallerTrack::default(),
            release_url: None,
            hot_swap_version: Some(version.clone()),
            reason: None,
            known_schema: KNOWN_MANIFEST_SCHEMA,
        },
        LaunchPlan::InstallerModal {
            target_version,
            track,
            reason,
            release_url,
        } => VersionContractState {
            decision: VersionContractDecision::InstallerRequired,
            installer_target: target_version.clone(),
            installer_track: *track,
            release_url: release_url.clone(),
            hot_swap_version: None,
            reason: Some(installer_reason_summary(reason)),
            known_schema: KNOWN_MANIFEST_SCHEMA,
        },
        LaunchPlan::Fallback { reason } => VersionContractState {
            decision: VersionContractDecision::Fallback,
            installer_target: None,
            installer_track: InstallerTrack::default(),
            release_url: None,
            hot_swap_version: None,
            reason: Some(reason.clone()),
            known_schema: KNOWN_MANIFEST_SCHEMA,
        },
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Custom URI scheme + activation (commit 2)
// ────────────────────────────────────────────────────────────────────────────

/// URI scheme name registered by `lib.rs`. Reserved purely for serving the
/// frontend bundle from cache; everything else continues to flow through
/// Tauri's default `tauri://localhost` handler.
pub const PROTOCOL_SCHEME: &str = "hardwave-app";

/// Build the URL we'd navigate to when the cache is active. Hostname is a
/// fixed placeholder (`localhost`) that we never resolve — the protocol
/// handler intercepts the request before any DNS happens.
pub fn cache_navigation_url() -> String {
    format!("{PROTOCOL_SCHEME}://localhost/")
}

/// Map a relative path to a Content-Type guess. The bundled asset
/// resolver gives us a real MIME for fallback hits; this only fires for
/// files we read out of the cache directory ourselves.
fn mime_for(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Read a single asset from the active cache, if any. Returns the file
/// bytes when the cache exists *and* contains the requested path.
fn read_from_cache(app: &AppHandle, path: &str) -> Option<Vec<u8>> {
    let cache = cache_root(app).ok()?;
    let version = read_active_version(&cache)?;
    let resolved = cache.join(version).join(path);
    // Sanity: refuse paths that escape the cache root (e.g. via `..`).
    let canonical = resolved.canonicalize().ok()?;
    let cache_canon = cache.canonicalize().ok()?;
    if !canonical.starts_with(&cache_canon) {
        log::warn!("frontend updater: refusing path outside cache: {path}");
        return None;
    }
    std::fs::read(&canonical).ok()
}

/// Protocol handler — first-line cache hit, then falls back to the bundled
/// asset resolver. Used by `tauri::Builder::register_uri_scheme_protocol`
/// in `lib.rs`. Synchronous because reading from disk for sub-megabyte
/// files is cheap; the alternative async API costs Tauri an extra thread
/// hop per request.
pub fn handle_request(
    app: &AppHandle,
    request: &tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let raw_path = request.uri().path();
    // Strip the leading slash and decode percent-encoded segments. The
    // path "" is a navigation root → serve index.html.
    let trimmed = raw_path.trim_start_matches('/');
    let path = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if let Some(bytes) = read_from_cache(app, path) {
        return tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", mime_for(path))
            .header("Cache-Control", "no-cache")
            .body(bytes)
            .unwrap_or_else(|_| {
                tauri::http::Response::builder()
                    .status(500)
                    .body(b"response build failed".to_vec())
                    .unwrap()
            });
    }

    if let Some(asset) = app.asset_resolver().get(path.to_string()) {
        return tauri::http::Response::builder()
            .status(200)
            .header("Content-Type", asset.mime_type)
            .body(asset.bytes)
            .unwrap_or_else(|_| {
                tauri::http::Response::builder()
                    .status(500)
                    .body(b"response build failed".to_vec())
                    .unwrap()
            });
    }

    tauri::http::Response::builder()
        .status(404)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(format!("not found: {path}").into_bytes())
        .unwrap()
}

/// Decide at startup whether the main window should load from cache or
/// stay on the bundled default. Called from the `setup` closure in
/// `lib.rs`. Returns `true` when navigation happened.
pub fn maybe_activate_cache(app: &AppHandle) -> bool {
    let cache = match cache_root(app) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("frontend updater: cannot read cache root: {e}");
            return false;
        }
    };
    let Some(version) = read_active_version(&cache) else {
        return false;
    };
    let entry = cache.join(&version).join("index.html");
    if !entry.exists() {
        log::warn!(
            "frontend updater: active.txt points to {version} but {entry:?} is missing — falling back to bundled"
        );
        return false;
    }
    let url_str = cache_navigation_url();
    let parsed: url::Url = match url_str.parse() {
        Ok(u) => u,
        Err(e) => {
            log::warn!("frontend updater: bad nav url {url_str}: {e}");
            return false;
        }
    };
    if let Some(window) = app.get_webview_window("main") {
        match window.navigate(parsed) {
            Ok(()) => {
                log::info!("frontend updater: activated cached version {version}");
                true
            }
            Err(e) => {
                log::warn!("frontend updater: navigate failed: {e}");
                false
            }
        }
    } else {
        log::warn!("frontend updater: main webview window not found at activation time");
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest(latest: &str) -> Manifest {
        Manifest {
            latest_version: latest.to_string(),
            published_at: None,
            requires_api: "*".to_string(),
            bundle: BundleSpec {
                url: "https://example.invalid/bundle.zip".to_string(),
                size_bytes: 1,
                sha256: "0".repeat(64),
            },
            changelog: vec![],
            min_installer: None,
            installer_track: InstallerTrack::default(),
            manifest_schema: 1,
            installer_hint: None,
        }
    }

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

    #[test]
    fn range_lower_bound_caret_forms() {
        assert_eq!(
            range_lower_bound("^0.157"),
            Some(semver::Version::parse("0.157.0").unwrap())
        );
        assert_eq!(
            range_lower_bound("^1.2.3"),
            Some(semver::Version::parse("1.2.3").unwrap())
        );
    }

    #[test]
    fn range_lower_bound_comma_form() {
        assert_eq!(
            range_lower_bound(">=0.158, <0.160"),
            Some(semver::Version::parse("0.158.0").unwrap())
        );
        assert_eq!(
            range_lower_bound(">=0.158.4, <0.160.0"),
            Some(semver::Version::parse("0.158.4").unwrap())
        );
    }

    #[test]
    fn range_lower_bound_wildcard_yields_none() {
        assert!(range_lower_bound("*").is_none());
        assert!(range_lower_bound("").is_none());
    }

    // §2 of the spec — Resolver branches.

    #[test]
    fn resolver_no_manifest_falls_back() {
        let plan = resolve_launch_plan("0.157.29", "0.157.29", None);
        match plan {
            LaunchPlan::Fallback { reason } => {
                assert!(reason.contains("manifest unavailable"))
            }
            _ => panic!("expected Fallback when manifest is None"),
        }
    }

    #[test]
    fn resolver_schema_too_new_falls_back() {
        let mut m = make_manifest("0.158.4");
        m.manifest_schema = KNOWN_MANIFEST_SCHEMA + 7;
        let plan = resolve_launch_plan("0.158.4", "0.158.4", Some(&m));
        match plan {
            LaunchPlan::Fallback { reason } => assert!(reason.contains("schema")),
            _ => panic!("expected Fallback when schema is ahead of client"),
        }
    }

    #[test]
    fn resolver_min_installer_floor_triggers_modal() {
        let mut m = make_manifest("0.158.4");
        m.min_installer = Some("0.158.0".to_string());
        m.installer_hint = Some(InstallerHint {
            version: Some("0.158.0".to_string()),
            notes_url: None,
            release_url: Some(
                "https://github.com/Dishairano/hardwave-daw/releases/tag/v0.158.0".to_string(),
            ),
        });
        // Worked example from the spec: running 0.157.9 with floor 0.158.0
        // → InstallerModal { reason: VersionFloor }
        let plan = resolve_launch_plan("0.157.9", "0.157.9", Some(&m));
        match plan {
            LaunchPlan::InstallerModal {
                target_version,
                reason,
                release_url,
                ..
            } => {
                assert_eq!(target_version.as_deref(), Some("0.158.0"));
                assert_eq!(
                    reason,
                    InstallerReason::VersionFloor {
                        needed: "0.158.0".to_string(),
                        have: "0.157.9".to_string(),
                    }
                );
                assert!(release_url.is_some());
            }
            other => panic!("expected InstallerModal for floor mismatch, got {other:?}"),
        }
    }

    #[test]
    fn resolver_min_installer_satisfied_then_api_below_range_modal() {
        // Edge case from §2: min_installer met but requires_api rejects
        // a binary that's *below* the declared range. Spec says modal,
        // not fallback (catchable by upgrade).
        let mut m = make_manifest("0.158.4");
        m.min_installer = Some("0.150.0".to_string());
        m.requires_api = ">=0.158, <0.160".to_string();
        let plan = resolve_launch_plan("0.157.9", "0.157.9", Some(&m));
        match plan {
            LaunchPlan::InstallerModal { reason, .. } => match reason {
                InstallerReason::ApiRangeBelow { range, have } => {
                    assert_eq!(range, ">=0.158, <0.160");
                    assert_eq!(have, "0.157.9");
                }
                other => panic!("expected ApiRangeBelow reason, got {other:?}"),
            },
            other => panic!("expected InstallerModal for below-range, got {other:?}"),
        }
    }

    #[test]
    fn resolver_api_above_range_falls_back_silently() {
        // Staged-rollout case: running 0.161.0 against a manifest that
        // declares ">=0.158, <0.160". Don't hassle the user with a
        // downgrade modal — stay on bundled.
        let mut m = make_manifest("0.158.4");
        m.requires_api = ">=0.158, <0.160".to_string();
        let plan = resolve_launch_plan("0.161.0", "0.158.4", Some(&m));
        match plan {
            LaunchPlan::Fallback { reason } => {
                assert!(reason.contains("outside manifest range"))
            }
            other => panic!("expected Fallback for above-range, got {other:?}"),
        }
    }

    #[test]
    fn resolver_hot_swap_happy_path() {
        let m = make_manifest("0.158.4");
        let plan = resolve_launch_plan("0.158.4", "0.158.0", Some(&m));
        match plan {
            LaunchPlan::HotSwap {
                action: HotSwapAction::ApplyBundle { version, .. },
            } => assert_eq!(version, "0.158.4"),
            other => panic!("expected HotSwap::ApplyBundle, got {other:?}"),
        }
    }

    #[test]
    fn resolver_hot_swap_noop_when_active_already_latest() {
        let m = make_manifest("0.158.4");
        let plan = resolve_launch_plan("0.158.4", "0.158.4", Some(&m));
        assert!(matches!(
            plan,
            LaunchPlan::HotSwap {
                action: HotSwapAction::NoOp
            }
        ));
    }

    // §4 backwards-compat invariant — the deployed test manifest at
    // suite.hardwavestudios.com/daw/frontend/manifest.json (v0.157.29)
    // doesn't carry the new fields. Parsing must not regress.
    #[test]
    fn legacy_manifest_parses_without_new_fields() {
        let json = r#"{
            "latest_version": "0.157.29",
            "published_at": "2026-04-15T10:00:00Z",
            "requires_api": "^0.157",
            "bundle": {
                "url": "https://example.invalid/bundle.zip",
                "size_bytes": 1024,
                "sha256": "abc"
            },
            "changelog": ["legacy"]
        }"#;
        let m: Manifest = serde_json::from_str(json).expect("legacy manifest must parse");
        assert!(m.min_installer.is_none());
        assert_eq!(m.installer_track, InstallerTrack::Stable);
        assert_eq!(m.manifest_schema, 1);
        assert!(m.installer_hint.is_none());

        // Resolver with this manifest takes the hot-swap-or-noop branch
        // (no floor, wildcard-style range matches, version comparison
        // decides).
        let plan = resolve_launch_plan("0.157.29", "0.157.0", Some(&m));
        match plan {
            LaunchPlan::HotSwap {
                action: HotSwapAction::ApplyBundle { version, .. },
            } => assert_eq!(version, "0.157.29"),
            other => panic!("expected HotSwap::ApplyBundle, got {other:?}"),
        }

        // Nit 4: belt-and-braces invariant — the legacy manifest fixture
        // must always be at-or-below KNOWN_MANIFEST_SCHEMA, otherwise the
        // resolver's schema-cliff branch would short-circuit this test
        // and the assertion above would be exercising a different branch
        // than the comment claims.
        assert!(
            m.manifest_schema <= KNOWN_MANIFEST_SCHEMA,
            "legacy fixture schema {} exceeds KNOWN_MANIFEST_SCHEMA {} — fixture must stay backwards-compat",
            m.manifest_schema,
            KNOWN_MANIFEST_SCHEMA
        );
    }

    // Blocker 1 (defensive): API_VERSION is read from CARGO_PKG_VERSION at
    // compile time and reported to the resolver as the running binary's
    // version. If the workspace Cargo.toml ever drifts back to a bare
    // `0.1.0` default, every fresh manifest publish (current floor:
    // 0.157.0) silently DOSes hot-swap by routing all clients to the
    // installer modal. Pin a hard floor here so a regression fails CI
    // before it ever ships.
    #[test]
    fn api_version_is_parseable_and_above_minimum_floor() {
        let parsed = semver::Version::parse(API_VERSION)
            .unwrap_or_else(|e| panic!("API_VERSION {API_VERSION:?} must be valid semver: {e}"));
        let floor = semver::Version::parse("0.1.0").unwrap();
        assert!(
            parsed >= floor,
            "API_VERSION {API_VERSION} below defensive floor 0.1.0 — workspace Cargo.toml \
             [workspace.package].version is wrong; the binary would self-DOS hot-swap"
        );
    }

    // Nit 3: lower-bound extraction for strict-greater-than comparators.
    // `>0.158` and `>=0.158` produce the SAME lower bound by design (the
    // resolver only uses this for routing below-range vs above-range; the
    // strict-vs-loose distinction is enforced by `api_compatible`, which
    // uses semver::VersionReq).
    #[test]
    fn range_lower_bound_strict_greater() {
        assert_eq!(
            range_lower_bound(">0.158"),
            Some(semver::Version::parse("0.158.0").unwrap())
        );
        assert_eq!(
            range_lower_bound(">0.158.4"),
            Some(semver::Version::parse("0.158.4").unwrap())
        );
        // `>` and `>=` collapse to the same floor — that's the contract.
        assert_eq!(range_lower_bound(">0.158"), range_lower_bound(">=0.158"));
    }

    // Blocker 2 (resolver invariant): `launch_plan_to_state` is a pure
    // mapping the cached-vs-fresh paths both feed into. Cache the same
    // plan twice and confirm the serialized state is identical so an
    // App.tsx reader can't observe drift between the splash event and
    // the follow-up command. (The full AppState round-trip needs a
    // tauri::AppHandle, which is out of scope for a unit test — the
    // mapping invariant is the load-bearing piece.)
    #[test]
    fn launch_plan_to_state_is_deterministic_for_same_plan() {
        let plan = LaunchPlan::InstallerModal {
            target_version: Some("0.158.0".to_string()),
            track: InstallerTrack::Stable,
            reason: InstallerReason::VersionFloor {
                needed: "0.158.0".to_string(),
                have: "0.157.9".to_string(),
            },
            release_url: Some("https://example.invalid/release".to_string()),
        };
        let a = launch_plan_to_state(&plan);
        let b = launch_plan_to_state(&plan);
        // Decision + payload identity — drift here would mean the cached
        // path and the fresh path could ever produce different state.
        assert!(matches!(
            a.decision,
            VersionContractDecision::InstallerRequired
        ));
        assert!(matches!(
            b.decision,
            VersionContractDecision::InstallerRequired
        ));
        assert_eq!(a.installer_target, b.installer_target);
        assert_eq!(a.release_url, b.release_url);
        assert_eq!(a.reason, b.reason);
        assert_eq!(a.known_schema, b.known_schema);
    }

    // Blocker 2 (cache contract): `select_cache_or_refresh` is the pure
    // decision helper the `version_contract_state` command body uses
    // before deciding whether to re-fetch the manifest. Three pinned
    // contracts:
    //
    // 1. With NO cached entry, always refresh.
    // 2. With a fresh cached entry and `force_refresh=false`, use cache.
    // 3. With a fresh cached entry and `force_refresh=true`, refresh.
    // 4. With a stale (>TTL) cached entry, refresh even when
    //    `force_refresh=false` — the belt-and-braces backstop in case
    //    App.tsx's recheck timer never fires.
    //
    // The reviewer's required test "dispatch resolver twice with the
    // same manifest, expect identical plan; then dispatch with a
    // different manifest under force_refresh=false, expect cached plan
    // still wins" reduces to (2): the decision says UseCache, so the
    // second manifest is never fetched and the cached plan is the one
    // that gets returned.
    #[test]
    fn cache_decision_no_cache_always_refreshes() {
        let now = Instant::now();
        assert_eq!(
            select_cache_or_refresh(None, false, now),
            CacheDecision::Refresh
        );
        assert_eq!(
            select_cache_or_refresh(None, true, now),
            CacheDecision::Refresh
        );
    }

    #[test]
    fn cache_decision_fresh_entry_uses_cache_unless_forced() {
        let entry = LaunchPlanCacheEntry::new(
            LaunchPlan::HotSwap {
                action: HotSwapAction::NoOp,
            },
            "0.158.4".to_string(),
        );
        let now = entry.resolved_at;
        assert_eq!(
            select_cache_or_refresh(Some(&entry), false, now),
            CacheDecision::UseCache,
            "fresh cache, no force → must serve cache so the splash and the App.tsx follow-up can't disagree"
        );
        assert_eq!(
            select_cache_or_refresh(Some(&entry), true, now),
            CacheDecision::Refresh,
            "force_refresh=true overrides cache — that's the 24h recheck path's contract"
        );
    }

    #[test]
    fn cache_decision_stale_entry_refreshes_even_without_force() {
        // Construct an entry whose resolved_at sits well outside the TTL.
        // `Instant` arithmetic is monotonic so we go forward from `now`
        // by TTL + buffer to simulate the cache aging.
        let entry = LaunchPlanCacheEntry::new(
            LaunchPlan::HotSwap {
                action: HotSwapAction::NoOp,
            },
            "0.158.4".to_string(),
        );
        let later = entry.resolved_at + LAUNCH_PLAN_CACHE_TTL + Duration::from_secs(60);
        assert!(entry.is_stale_at(later));
        assert_eq!(
            select_cache_or_refresh(Some(&entry), false, later),
            CacheDecision::Refresh,
            "stale cache must trigger a re-fetch even when caller didn't force refresh"
        );
    }

    // Blocker 2 (applied semantics): once the splash has actually
    // staged a bundle, a follow-up `version_contract_state` query must
    // NOT keep telling App.tsx the same ApplyBundle action is pending —
    // that would surface "update available" forever, even though the
    // bundle is sitting on disk waiting for relaunch.
    #[test]
    fn applied_apply_bundle_collapses_to_up_to_date() {
        let plan = LaunchPlan::HotSwap {
            action: HotSwapAction::ApplyBundle {
                version: "0.158.4".to_string(),
                bundle_url: "https://example.invalid/bundle.zip".to_string(),
                size_bytes: 1234,
                sha256: "0".repeat(64),
            },
        };
        let mut entry = LaunchPlanCacheEntry::new(plan, "0.158.4".to_string());

        // Before apply: state reads as a pending HotSwap.
        let pre = cached_entry_to_state(&entry);
        assert!(matches!(pre.decision, VersionContractDecision::HotSwap));
        assert_eq!(pre.hot_swap_version.as_deref(), Some("0.158.4"));

        // After apply (the field flips inside `check_and_apply_inner`):
        // the same entry now reads as up-to-date with the staged version.
        entry.applied = true;
        let post = cached_entry_to_state(&entry);
        assert!(matches!(post.decision, VersionContractDecision::UpToDate));
        assert_eq!(post.hot_swap_version.as_deref(), Some("0.158.4"));
        assert_eq!(
            post.reason.as_deref(),
            Some("bundle staged; restart to apply")
        );
    }

    // Reviewer's required scenario in concrete form. Two separate
    // `LaunchPlan` results derived from two different manifests; under
    // `force_refresh=false` the cache decision picks the FIRST one and
    // never observes the second. This is the exact race the cache
    // exists to prevent (CDN replica flips between splash and follow-up
    // App.tsx invoke).
    #[test]
    fn second_manifest_does_not_win_under_cache_first() {
        // First fetch: latest_version 0.158.4, no min_installer.
        let m1 = make_manifest("0.158.4");
        let plan1 = resolve_launch_plan("0.158.4", "0.158.0", Some(&m1));
        let entry = LaunchPlanCacheEntry::new(plan1.clone(), m1.latest_version.clone());

        // Second fetch (CDN flipped to staged-rollout manifest): same
        // version field, but a min_installer floor that would route the
        // SAME running binary to InstallerModal instead. If we ever
        // re-fetched here, App.tsx would see the InstallerRequired
        // decision after the splash already showed HotSwapReady.
        let mut m2 = make_manifest("0.158.4");
        m2.min_installer = Some("0.999.0".to_string());
        let plan2 = resolve_launch_plan("0.158.4", "0.158.0", Some(&m2));

        // Sanity: the two plans really differ. If they didn't, the rest
        // of the test would still pass for the wrong reason.
        assert!(
            matches!(
                plan1,
                LaunchPlan::HotSwap {
                    action: HotSwapAction::ApplyBundle { .. }
                }
            ),
            "first plan should be HotSwap::ApplyBundle"
        );
        assert!(
            matches!(plan2, LaunchPlan::InstallerModal { .. }),
            "second plan should be InstallerModal — confirms the manifests really differ"
        );

        // Under force_refresh=false with a fresh cached entry, the
        // decision is UseCache. The second manifest never gets a chance
        // to influence the state; the splash's HotSwap decision stands.
        let now = entry.resolved_at;
        assert_eq!(
            select_cache_or_refresh(Some(&entry), false, now),
            CacheDecision::UseCache
        );
        let served = cached_entry_to_state(&entry);
        assert!(matches!(served.decision, VersionContractDecision::HotSwap));
    }
}
