//! The updates the DAW's two feeds offer are ours, and unpack to exactly what we signed.
//!
//! The updater signing key changed with v0.266.47 (incident 2026-11-12), so there are two
//! feeds (scripts/updater_feeds.py writes and checks them for every release):
//! - `latest.json`, read by every DAW up to v0.266.46. Those trust only the old key, so this
//!   feed always offers an update signed with it: the newest release up to v0.266.47, and
//!   v0.266.47 itself from then on.
//! - `latest-v2.json`, the endpoint in tauri.conf.json, read by builds that carry the new key:
//!   v0.266.47 and later, and test builds and release candidates made from the same code. Its
//!   update is signed with the key in tauri.conf.json, apart from v0.266.47's own feed, which
//!   was published before the signing secret switched and carries the old key. A release
//!   reading it is not offered v0.266.47 (it already runs v0.266.47 or later), but a test
//!   build or release candidate numbered below v0.266.47 is, and refuses its signature: such
//!   a build stays where it is until v0.266.48 (new key) is out, or is reinstalled by hand.
//!
//! For each feed this takes the update it offers through the same steps the updater does.
//! On Windows the updater downloads the `.nsis.zip` named in the feed when the running build
//! does not know it was installed with NSIS, checks it against its minisign key, and unpacks
//! it with `zip::ZipArchive::extract` (tauri-plugin-updater 2.14.1, zip 9). This does the
//! same signature check (including the version the signature was made for), the same
//! extract call on the same zip crate, and then checks the installer that came out of the
//! archive against the signature published for the bare installer, which only passes if the
//! unpacked file is byte for byte the one we signed.
//!
//! It downloads about 15 MB per feed, so it only runs when asked:
//! `hw-daw-cargo test -j 1 -p hardwave-daw --lib update_archive -- --ignored --nocapture`.
//! With `HW_UPDATE_FIXTURES` set to a folder holding the feeds and the files they name, it
//! reads them from there instead. Each test prints one line saying which feed it verified
//! and where from ("live" or the fixtures folder), or that it skipped the feed and why;
//! switch-secret.sh (HQ) reads those lines, so a skip can never pass for a check.

use std::io::Cursor;
use std::path::PathBuf;

use tauri::test::{mock_builder, MockRuntime};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The feed every DAW up to v0.266.46 reads.
const OLD_FEED: &str =
    "https://github.com/Dishairano/hardwave-daw/releases/latest/download/latest.json";

/// The key every DAW up to v0.266.46 trusts, and the one v0.266.47 is signed with.
const OLD_PUBKEY: &str = include_str!("../updater-key-until-v0.266.47.pub");

/// The last release signed with the old key, and the first to read `latest-v2.json`.
const BRIDGE: (u64, u64, u64) = (0, 266, 47);

fn app() -> tauri::App<MockRuntime> {
    mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(tauri::generate_context!())
        .expect("the app builds on the mock runtime")
}

/// The update endpoint the shipped app asks, from tauri.conf.json.
fn endpoint(app: &tauri::App<MockRuntime>) -> String {
    app.config().plugins.0["updater"]["endpoints"][0]
        .as_str()
        .expect("tauri.conf.json names an update endpoint")
        .to_string()
}

/// Where the feeds and archives come from: "live" (GitHub) or the fixtures folder.
fn source() -> String {
    match std::env::var("HW_UPDATE_FIXTURES") {
        Ok(dir) => format!("fixtures {dir}"),
        Err(_) => "live".to_string(),
    }
}

async fn fetch(url: &str) -> Vec<u8> {
    if let Ok(dir) = std::env::var("HW_UPDATE_FIXTURES") {
        let name = url.rsplit('/').next().unwrap().replace("%20", " ");
        let path = PathBuf::from(dir).join(name);
        return std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    }
    reqwest::get(url)
        .await
        .and_then(|r| r.error_for_status())
        .unwrap_or_else(|e| panic!("{url}: {e}"))
        .bytes()
        .await
        .unwrap_or_else(|e| panic!("{url}: {e}"))
        .to_vec()
}

async fn feed(url: &str) -> serde_json::Value {
    serde_json::from_slice(&fetch(url).await).unwrap_or_else(|e| panic!("{url} is JSON: {e}"))
}

/// The version a feed offers, as (major, minor, patch).
fn offered(feed: &serde_json::Value) -> (u64, u64, u64) {
    let version = feed["version"].as_str().expect("the feed names a version");
    let mut parts = version.trim_start_matches('v').split(['.', '-']).map(|p| {
        p.parse::<u64>()
            .unwrap_or_else(|_| panic!("version {version}"))
    });
    let mut next = || parts.next().unwrap_or_else(|| panic!("version {version}"));
    (next(), next(), next())
}

/// The update a feed announces for one platform entry, as an updater trusting `pubkey`
/// (the key in tauri.conf.json when None) sees it. Restoring a manifest makes the updater
/// require the signed version to match the announced one.
fn update_for(
    app: &tauri::App<MockRuntime>,
    manifest: &serde_json::Value,
    target: &str,
    pubkey: Option<&str>,
) -> Update {
    let mut builder = app
        .updater_builder()
        .target(target)
        .version_comparator(|_, _| true);
    if let Some(pubkey) = pubkey {
        builder = builder.pubkey(pubkey.trim());
    }
    builder
        .build()
        .expect("the updater builds from tauri.conf.json")
        .restore_update(manifest.clone())
        .expect("the feed reads")
        .unwrap_or_else(|| panic!("the feed has no {target} entry"))
}

/// The feed's Windows update is signed with `pubkey` and unpacks to the installer we signed.
async fn check_update(
    app: &tauri::App<MockRuntime>,
    manifest: &serde_json::Value,
    pubkey: Option<&str>,
) {
    let zipped = update_for(app, manifest, "windows-x86_64", pubkey);
    assert!(
        zipped.download_url.path().ends_with(".nsis.zip"),
        "{}",
        zipped.download_url
    );
    let archive = fetch(zipped.download_url.as_str()).await;
    zipped
        .verify(&archive)
        .expect("the archive carries our signature, made for the version announced");

    // One flipped bit anywhere and the updater refuses it.
    let mut tampered = archive.clone();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 1;
    assert!(zipped.verify(&tampered).is_err());

    // Unpacked exactly as the updater does on Windows.
    let dir = tempfile::tempdir().unwrap();
    zip::ZipArchive::new(Cursor::new(&archive))
        .expect("the archive opens with zip 9")
        .extract(dir.path())
        .expect("the archive unpacks with zip 9");
    let installers: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "exe"))
        .collect();
    assert_eq!(installers.len(), 1, "{installers:?}");
    let installer = std::fs::read(&installers[0]).unwrap();
    assert_eq!(
        &installer[..2],
        b"MZ",
        "the unpacked file is a Windows program"
    );

    update_for(app, manifest, "windows-x86_64-nsis", pubkey)
        .verify(&installer)
        .expect("the unpacked installer is byte for byte the installer we signed");
}

#[test]
#[ignore = "downloads the latest release (about 15 MB); run with --ignored"]
fn the_old_feed_offers_an_update_old_installs_accept() {
    tauri::async_runtime::block_on(async {
        let app = app();
        let manifest = feed(OLD_FEED).await;
        assert!(
            offered(&manifest) <= BRIDGE,
            "latest.json offers {}: DAWs up to v0.266.46 cannot verify anything after v0.266.47",
            manifest["version"]
        );
        check_update(&app, &manifest, Some(OLD_PUBKEY)).await;
        println!(
            "update_archive: verified latest.json ({}): offers {}, signed with the old key, \
             unpacks to the installer we signed",
            source(),
            manifest["version"].as_str().unwrap_or_default()
        );
    });
}

#[test]
#[ignore = "downloads the latest release (about 15 MB); run with --ignored"]
fn the_new_feed_offers_an_update_signed_with_the_key_its_readers_trust() {
    tauri::async_runtime::block_on(async {
        let app = app();
        assert!(
            endpoint(&app).ends_with("/latest/download/latest-v2.json"),
            "from v0.266.47 on the app reads latest-v2.json, not {}",
            endpoint(&app)
        );
        let newest = offered(&feed(OLD_FEED).await);
        if newest < BRIDGE {
            println!(
                "update_archive: SKIPPED latest-v2.json ({}): NOT VERIFIED. The newest release is \
                 {newest:?}, older than v0.266.47, the first release that publishes latest-v2.json",
                source()
            );
            return;
        }
        let manifest = feed(&endpoint(&app)).await;
        let bridge = offered(&manifest) == BRIDGE;
        let pubkey = bridge.then_some(OLD_PUBKEY);
        check_update(&app, &manifest, pubkey).await;
        println!(
            "update_archive: verified latest-v2.json ({}): offers {}, signed with {}, \
             unpacks to the installer we signed",
            source(),
            manifest["version"].as_str().unwrap_or_default(),
            if bridge {
                "the old key"
            } else {
                "the key in tauri.conf.json"
            }
        );
    });
}
