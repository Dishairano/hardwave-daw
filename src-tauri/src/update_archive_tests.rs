//! The update the DAW installs is ours, and it unpacks to exactly what we signed.
//!
//! On Windows the updater downloads the `.nsis.zip` named in `latest.json`
//! when the running build does not know it was installed with NSIS, checks
//! it against the minisign key in tauri.conf.json, and unpacks it with
//! `zip::ZipArchive::extract` (tauri-plugin-updater 2.14.1, zip 9). This
//! takes the latest real release through the same steps: the updater's own
//! signature check (including the version the signature was made for), the
//! same extract call on the same zip crate, and then the installer that came
//! out of the archive against the signature published for the bare
//! installer, which only passes if the unpacked file is byte for byte the
//! one we signed.
//!
//! It downloads about 15 MB, so it only runs when asked:
//! `hw-daw-cargo test -j 1 -p hardwave-daw --lib update_archive -- --ignored`.
//! With `HW_UPDATE_FIXTURES` set to a folder holding `latest.json` and the
//! files it names, it reads them from there instead.

use std::io::Cursor;
use std::path::PathBuf;

use tauri::test::{mock_builder, MockRuntime};
use tauri_plugin_updater::{Update, UpdaterExt};

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

/// The update `latest.json` announces for one platform entry, as the
/// updater sees it. Restoring a manifest makes the updater require the
/// signed version to match the announced one.
fn update_for(app: &tauri::App<MockRuntime>, manifest: &serde_json::Value, target: &str) -> Update {
    app.updater_builder()
        .target(target)
        .version_comparator(|_, _| true)
        .build()
        .expect("the updater builds from tauri.conf.json")
        .restore_update(manifest.clone())
        .expect("latest.json reads")
        .unwrap_or_else(|| panic!("latest.json has no {target} entry"))
}

#[test]
#[ignore = "downloads the latest release (about 15 MB); run with --ignored"]
fn the_latest_update_archive_is_ours_and_unpacks_to_the_installer_we_signed() {
    tauri::async_runtime::block_on(async {
        let app = app();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fetch(&endpoint(&app)).await).expect("latest.json is JSON");

        let zipped = update_for(&app, &manifest, "windows-x86_64");
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

        update_for(&app, &manifest, "windows-x86_64-nsis")
            .verify(&installer)
            .expect("the unpacked installer is byte for byte the installer we signed");
    });
}
