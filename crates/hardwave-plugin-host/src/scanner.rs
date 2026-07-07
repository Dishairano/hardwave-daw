//! Plugin scanner — discovers VST3 and CLAP plugins on the system.

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScanDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedScan {
    plugins: Vec<PluginDescriptor>,
}

/// Progress callback invoked as the scanner walks directories.
/// `found` is the running count of plugins discovered; `current` is a human
/// label for what is being scanned (usually a path).
pub type ScanProgress = Box<dyn FnMut(usize, &str) + Send>;

// Subset of the VST3 SDK moduleinfo.json schema we read. Field names match
// the spec's JSON keys verbatim; unknown fields are ignored by serde.
#[derive(Debug, Deserialize)]
struct ModuleInfo {
    #[serde(rename = "Factory Info", default)]
    factory_info: Option<FactoryInfo>,
    #[serde(rename = "Classes", default)]
    classes: Vec<ModuleClass>,
}

#[derive(Debug, Default, Deserialize)]
struct FactoryInfo {
    #[serde(rename = "Vendor", default)]
    vendor: String,
}

#[derive(Debug, Deserialize)]
struct ModuleClass {
    #[serde(rename = "Category", default)]
    category: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Vendor", default)]
    vendor: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Sub Categories", default)]
    sub_categories: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PluginScanner {
    pub vst3_paths: Vec<PathBuf>,
    pub clap_paths: Vec<PathBuf>,
    pub custom_vst3_paths: Vec<PathBuf>,
    pub custom_clap_paths: Vec<PathBuf>,
    pub blocklist: HashSet<String>,
    cache: Vec<PluginDescriptor>,
    last_diff: ScanDiff,
}

impl PluginScanner {
    pub fn new() -> Self {
        Self {
            vst3_paths: default_vst3_paths(),
            clap_paths: default_clap_paths(),
            custom_vst3_paths: Vec::new(),
            custom_clap_paths: Vec::new(),
            blocklist: HashSet::new(),
            cache: Vec::new(),
            last_diff: ScanDiff::default(),
        }
    }

    /// Default path for the cache file.
    pub fn default_cache_path() -> Option<PathBuf> {
        let mut p = dirs::config_dir()?;
        p.push("hardwave");
        p.push("plugin-cache.json");
        Some(p)
    }

    /// Load cached scan results from disk (does not trigger a fresh scan).
    pub fn load_cache_from_disk(&mut self, path: &Path) -> Result<usize, String> {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.cache.clear();
                return Ok(0);
            }
            Err(e) => return Err(format!("read cache: {e}")),
        };
        let cached: CachedScan =
            serde_json::from_slice(&bytes).map_err(|e| format!("parse cache: {e}"))?;
        self.cache = cached.plugins;
        Ok(self.cache.len())
    }

    /// Persist current scan results to disk.
    pub fn save_cache_to_disk(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create cache dir: {e}"))?;
        }
        let cached = CachedScan {
            plugins: self.cache.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&cached).map_err(|e| format!("encode cache: {e}"))?;
        std::fs::write(path, bytes).map_err(|e| format!("write cache: {e}"))?;
        Ok(())
    }

    /// Scan all configured paths for plugins. Computes a diff against the prior cache.
    pub fn scan(&mut self) -> &[PluginDescriptor] {
        self.scan_with_progress(None)
    }

    /// Same as [`scan`] but invokes `progress` as each directory is walked.
    pub fn scan_with_progress(
        &mut self,
        mut progress: Option<ScanProgress>,
    ) -> &[PluginDescriptor] {
        let prior: HashSet<String> = self.cache.iter().map(|p| p.id.clone()).collect();
        self.cache.clear();

        let vst3_paths: Vec<PathBuf> = self
            .vst3_paths
            .iter()
            .chain(self.custom_vst3_paths.iter())
            .cloned()
            .collect();
        let clap_paths: Vec<PathBuf> = self
            .clap_paths
            .iter()
            .chain(self.custom_clap_paths.iter())
            .cloned()
            .collect();

        for path in &vst3_paths {
            if path.exists() {
                if let Some(cb) = progress.as_mut() {
                    cb(self.cache.len(), &path.display().to_string());
                }
                self.scan_vst3_dir(path, progress.as_mut());
            }
        }

        for path in &clap_paths {
            if path.exists() {
                if let Some(cb) = progress.as_mut() {
                    cb(self.cache.len(), &path.display().to_string());
                }
                self.scan_clap_dir(path, progress.as_mut());
            }
        }

        // Apply blocklist.
        if !self.blocklist.is_empty() {
            self.cache.retain(|p| !self.blocklist.contains(&p.id));
        }

        // Compute diff vs prior cache.
        let current: HashSet<String> = self.cache.iter().map(|p| p.id.clone()).collect();
        let added: Vec<String> = current.difference(&prior).cloned().collect();
        let removed: Vec<String> = prior.difference(&current).cloned().collect();
        self.last_diff = ScanDiff { added, removed };

        log::info!(
            "Plugin scan complete: {} plugins (+{} added, -{} removed vs previous)",
            self.cache.len(),
            self.last_diff.added.len(),
            self.last_diff.removed.len(),
        );
        &self.cache
    }

    /// Returns the diff from the most recent scan.
    pub fn last_diff(&self) -> &ScanDiff {
        &self.last_diff
    }

    /// Get cached scan results.
    pub fn plugins(&self) -> &[PluginDescriptor] {
        &self.cache
    }

    /// Find a plugin by ID.
    pub fn find(&self, id: &str) -> Option<&PluginDescriptor> {
        self.cache.iter().find(|p| p.id == id)
    }

    /// Inject a native (in-process) plugin descriptor into the cache.
    /// Used for bundled plugins that don't live in a VST3 / CLAP
    /// directory — the `add_plugin_to_track` command looks them up
    /// through `find(id)` just like external plugins.
    pub fn register_native(&mut self, descriptor: PluginDescriptor) {
        if let Some(existing) = self.cache.iter_mut().find(|p| p.id == descriptor.id) {
            *existing = descriptor;
        } else {
            self.cache.push(descriptor);
        }
    }

    /// Bulk-register a list of native descriptors. Called at engine
    /// init so the native plugin catalog is available before any
    /// directory scan has run.
    pub fn register_natives<I>(&mut self, descriptors: I)
    where
        I: IntoIterator<Item = PluginDescriptor>,
    {
        for d in descriptors {
            self.register_native(d);
        }
    }

    fn scan_vst3_dir(&mut self, dir: &Path, mut progress: Option<&mut ScanProgress>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext == "vst3" {
                if path.is_dir() || path.is_file() {
                    log::debug!("Found VST3: {}", path.display());
                    let descriptors = parse_vst3_bundle(&path);
                    if let Some(cb) = progress.as_deref_mut() {
                        cb(self.cache.len(), &path.display().to_string());
                    }
                    for d in descriptors {
                        self.cache.push(d);
                    }
                }

                // Don't recurse into .vst3 bundles — moduleinfo parsing already
                // enumerates their classes, and their Contents/ subtree can
                // legitimately contain unrelated .vst3-named resources.
            } else if path.is_dir() {
                // Scan nested non-bundle directories (e.g. vendor subfolders).
                self.scan_vst3_dir(&path, progress.as_deref_mut());
            }
        }
    }

    fn scan_clap_dir(&mut self, dir: &Path, mut progress: Option<&mut ScanProgress>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext == "clap" {
                log::debug!("Found CLAP: {}", path.display());
                if let Some(cb) = progress.as_deref_mut() {
                    cb(self.cache.len(), &path.display().to_string());
                }
                for d in parse_clap_library(&path) {
                    self.cache.push(d);
                }
            } else if path.is_dir() {
                self.scan_clap_dir(&path, progress.as_deref_mut());
            }
        }
    }
}

/// Load a `.clap` shared library, read its plugin descriptors, and convert
/// them to [`PluginDescriptor`]. Falls back to a filename-only stub if the
/// library cannot be loaded or exposes no plugins — so broken CLAPs still
/// appear in the browser (marked Unknown) where they can be blocklisted.
fn parse_clap_library(library_path: &Path) -> Vec<PluginDescriptor> {
    let fallback_name = library_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    match crate::clap_ffi::read_clap_descriptors(library_path) {
        Some(list) if !list.is_empty() => list
            .into_iter()
            .map(|d| {
                let name = if d.name.is_empty() {
                    fallback_name.clone()
                } else {
                    d.name
                };
                let vendor = if d.vendor.is_empty() {
                    "Unknown".into()
                } else {
                    d.vendor
                };
                let version = if d.version.is_empty() {
                    "0.0.0".into()
                } else {
                    d.version
                };
                let id = if d.id.is_empty() {
                    format!("clap:{}", name.to_lowercase().replace(' ', "-"))
                } else {
                    format!("clap:{}", d.id)
                };
                let (category, has_midi_input) = classify_clap(&d.features);
                PluginDescriptor {
                    id,
                    name,
                    vendor,
                    version,
                    format: PluginFormat::Clap,
                    path: library_path.to_path_buf(),
                    category,
                    num_inputs: 2,
                    num_outputs: 2,
                    has_midi_input,
                    has_editor: true,
                }
            })
            .collect(),
        _ => vec![PluginDescriptor {
            id: format!("clap:{}", fallback_name.to_lowercase().replace(' ', "-")),
            name: fallback_name,
            vendor: "Unknown".into(),
            version: "0.0.0".into(),
            format: PluginFormat::Clap,
            path: library_path.to_path_buf(),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: true,
        }],
    }
}

fn classify_clap(features: &[String]) -> (PluginCategory, bool) {
    // CLAP feature strings are lowercase dotted identifiers. See
    // "clap/plugin-features.h" in the CLAP SDK for the full list.
    let mut is_instrument = false;
    let mut is_analyzer = false;
    let mut has_midi_input = false;
    for f in features {
        match f.as_str() {
            "instrument" | "synthesizer" | "drum" | "drum-machine" | "sampler" => {
                is_instrument = true;
                has_midi_input = true;
            }
            "analyzer" => is_analyzer = true,
            "note-effect" | "note-detector" => has_midi_input = true,
            _ => {}
        }
    }
    let cat = if is_instrument {
        PluginCategory::Instrument
    } else if is_analyzer {
        PluginCategory::Analyzer
    } else {
        PluginCategory::Effect
    };
    (cat, has_midi_input)
}

/// Read a VST3 bundle and return one descriptor per audio-module class.
/// Prefers `Contents/moduleinfo.json` (VST3 SDK ≥ 3.7) over the filename
/// fallback so vendor, version, category, and I/O counts are accurate.
fn parse_vst3_bundle(bundle_path: &Path) -> Vec<PluginDescriptor> {
    let fallback_name = bundle_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    if let Some(info) = read_moduleinfo(bundle_path) {
        let factory_vendor = info
            .factory_info
            .as_ref()
            .map(|f| f.vendor.clone())
            .unwrap_or_default();

        let audio_classes: Vec<_> = info
            .classes
            .into_iter()
            .filter(|c| c.category == "Audio Module Class")
            .collect();

        if !audio_classes.is_empty() {
            return audio_classes
                .into_iter()
                .map(|c| {
                    let name = if c.name.is_empty() {
                        fallback_name.clone()
                    } else {
                        c.name
                    };
                    let vendor = if !c.vendor.is_empty() {
                        c.vendor
                    } else if !factory_vendor.is_empty() {
                        factory_vendor.clone()
                    } else {
                        "Unknown".into()
                    };
                    let version = if c.version.is_empty() {
                        "0.0.0".into()
                    } else {
                        c.version
                    };
                    let (category, has_midi_input) = classify_vst3(&c.sub_categories);
                    PluginDescriptor {
                        id: format!("vst3:{}", name.to_lowercase().replace(' ', "-")),
                        name,
                        vendor,
                        version,
                        format: PluginFormat::Vst3,
                        path: bundle_path.to_path_buf(),
                        category,
                        num_inputs: 2,
                        num_outputs: 2,
                        has_midi_input,
                        has_editor: true,
                    }
                })
                .collect();
        }
    }

    vec![PluginDescriptor {
        id: format!("vst3:{}", fallback_name.to_lowercase().replace(' ', "-")),
        name: fallback_name,
        vendor: "Unknown".into(),
        version: "1.0.0".into(),
        format: PluginFormat::Vst3,
        path: bundle_path.to_path_buf(),
        category: PluginCategory::Effect,
        num_inputs: 2,
        num_outputs: 2,
        has_midi_input: false,
        has_editor: true,
    }]
}

fn read_moduleinfo(bundle_path: &Path) -> Option<ModuleInfo> {
    let candidates = [
        bundle_path.join("Contents/moduleinfo.json"),
        bundle_path.join("Contents/Resources/moduleinfo.json"),
    ];
    for candidate in candidates {
        if let Ok(bytes) = std::fs::read(&candidate) {
            // Strip BOM if present — some VST3 moduleinfo.json files ship with one.
            let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
            match serde_json::from_slice::<ModuleInfo>(bytes) {
                Ok(info) => return Some(info),
                Err(e) => log::debug!("Failed to parse {}: {e}", candidate.display()),
            }
        }
    }
    None
}

fn classify_vst3(sub_categories: &[String]) -> (PluginCategory, bool) {
    let mut is_instrument = false;
    let mut is_analyzer = false;
    let mut has_midi_input = false;
    for sub in sub_categories {
        let lower = sub.to_ascii_lowercase();
        if lower.contains("instrument") || lower.contains("synth") || lower.contains("drum") {
            is_instrument = true;
        }
        if lower.contains("analyzer") {
            is_analyzer = true;
        }
        // VST3 convention: the "Note Expression" / "Instrument" categories
        // indicate the plugin consumes MIDI/note events.
        if is_instrument || lower.contains("note") || lower.contains("midi") {
            has_midi_input = true;
        }
    }
    let cat = if is_instrument {
        PluginCategory::Instrument
    } else if is_analyzer {
        PluginCategory::Analyzer
    } else {
        PluginCategory::Effect
    };
    (cat, has_midi_input)
}

impl Default for PluginScanner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Platform-specific default paths
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn default_vst3_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from(r"C:\Program Files\Common Files\VST3"),
        PathBuf::from(r"C:\Program Files (x86)\Common Files\VST3"),
    ]
}

#[cfg(target_os = "macos")]
fn default_vst3_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/Library/Audio/Plug-Ins/VST3")];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Library/Audio/Plug-Ins/VST3"));
    }
    paths
}

#[cfg(target_os = "linux")]
fn default_vst3_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/usr/lib/vst3"),
        PathBuf::from("/usr/local/lib/vst3"),
    ];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".vst3"));
    }
    paths
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn default_vst3_paths() -> Vec<PathBuf> {
    vec![]
}

#[cfg(target_os = "windows")]
fn default_clap_paths() -> Vec<PathBuf> {
    vec![PathBuf::from(r"C:\Program Files\Common Files\CLAP")]
}

#[cfg(target_os = "macos")]
fn default_clap_paths() -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::from("/Library/Audio/Plug-Ins/CLAP")];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("Library/Audio/Plug-Ins/CLAP"));
    }
    paths
}

#[cfg(target_os = "linux")]
fn default_clap_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/usr/lib/clap"),
        PathBuf::from("/usr/local/lib/clap"),
    ];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".clap"));
    }
    paths
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn default_clap_paths() -> Vec<PathBuf> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hw-scan-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A directory-style .vst3 bundle with a moduleinfo.json, like every
    /// VST3-SDK-3.7+ plugin ships. No binary needed — the scanner reads
    /// metadata only at discovery time.
    fn write_fake_vst3(dir: &Path, bundle: &str, json: &str) -> PathBuf {
        let b = dir.join(bundle);
        std::fs::create_dir_all(b.join("Contents")).unwrap();
        std::fs::write(b.join("Contents/moduleinfo.json"), json).unwrap();
        b
    }

    const KICK_MODULEINFO: &str = r#"{
      "Factory Info": { "Vendor": "Hardwave Test" },
      "Classes": [
        {
          "Category": "Audio Module Class",
          "Name": "Test Kick",
          "Vendor": "",
          "Version": "1.2.3",
          "Sub Categories": ["Instrument", "Synth"]
        },
        { "Category": "Component Controller Class", "Name": "Ignored", "Vendor": "", "Version": "", "Sub Categories": [] }
      ]
    }"#;

    fn scanner_over(dir: &Path) -> PluginScanner {
        let mut s = PluginScanner::new();
        // Isolate from the host machine: only the temp dir is scanned.
        s.vst3_paths.clear();
        s.clap_paths.clear();
        s.custom_vst3_paths = vec![dir.to_path_buf()];
        s
    }

    #[test]
    fn scan_reads_moduleinfo_metadata() {
        let dir = tmp("moduleinfo");
        write_fake_vst3(&dir, "TestKick.vst3", KICK_MODULEINFO);

        let mut s = scanner_over(&dir);
        let found = s.scan().to_vec();
        assert_eq!(found.len(), 1, "one Audio Module Class → one descriptor");
        let p = &found[0];
        assert_eq!(p.id, "vst3:test-kick");
        assert_eq!(p.name, "Test Kick");
        assert_eq!(
            p.vendor, "Hardwave Test",
            "falls back to Factory Info vendor"
        );
        assert_eq!(p.version, "1.2.3");
        assert_eq!(p.format, PluginFormat::Vst3);
        assert_eq!(p.category, PluginCategory::Instrument);
        assert!(p.has_midi_input, "instruments consume notes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_finds_bundles_in_vendor_subfolders_but_not_inside_bundles() {
        let dir = tmp("nesting");
        // Vendor subfolder → must recurse.
        write_fake_vst3(&dir.join("SomeVendor"), "Nested.vst3", KICK_MODULEINFO);
        // A resource *inside* a bundle that is itself named .vst3 —
        // must NOT be scanned as a separate plugin.
        let outer = write_fake_vst3(&dir, "Outer.vst3", KICK_MODULEINFO);
        write_fake_vst3(
            &outer.join("Contents"),
            "inner-resource.vst3",
            KICK_MODULEINFO,
        );

        let mut s = scanner_over(&dir);
        let names: Vec<String> = s
            .scan()
            .iter()
            .map(|p| p.path.display().to_string())
            .collect();
        assert_eq!(
            names.len(),
            2,
            "vendor-nested + outer, not the inner resource: {names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("inner-resource")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn blocklist_removes_plugin_and_diff_tracks_changes() {
        let dir = tmp("blocklist");
        write_fake_vst3(&dir, "TestKick.vst3", KICK_MODULEINFO);

        let mut s = scanner_over(&dir);
        s.scan();
        assert_eq!(s.plugins().len(), 1);
        assert_eq!(s.last_diff().added.len(), 1, "first scan reports the add");

        s.blocklist.insert("vst3:test-kick".into());
        s.scan();
        assert!(s.plugins().is_empty(), "blocklisted plugin is filtered");
        assert_eq!(
            s.last_diff().removed,
            vec!["vst3:test-kick".to_string()],
            "diff reports the disappearance"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let dir = tmp("cache");
        write_fake_vst3(&dir, "TestKick.vst3", KICK_MODULEINFO);
        let cache_file = dir.join("scan-cache.json");

        let mut s = scanner_over(&dir);
        s.scan();
        s.save_cache_to_disk(&cache_file).expect("save cache");

        let mut fresh = PluginScanner::new();
        let n = fresh.load_cache_from_disk(&cache_file).expect("load cache");
        assert_eq!(n, 1);
        let p = fresh
            .find("vst3:test-kick")
            .expect("cached descriptor findable");
        assert_eq!(p.name, "Test Kick");
        assert_eq!(p.version, "1.2.3");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn register_native_upserts_by_id() {
        let mut s = PluginScanner::new();
        let mk = |version: &str| PluginDescriptor {
            id: "native:kicksynth".into(),
            name: "KickSynth".into(),
            vendor: "Hardwave".into(),
            version: version.into(),
            // No dedicated Native variant — engine registers natives with
            // an existing format tag; upsert semantics don't depend on it.
            format: PluginFormat::Vst3,
            path: PathBuf::new(),
            category: PluginCategory::Instrument,
            num_inputs: 0,
            num_outputs: 2,
            has_midi_input: true,
            has_editor: true,
        };
        s.register_native(mk("1.0.0"));
        s.register_native(mk("1.1.0"));
        assert_eq!(s.plugins().len(), 1, "same id replaces, not duplicates");
        assert_eq!(s.find("native:kicksynth").unwrap().version, "1.1.0");
    }

    #[test]
    fn moduleinfo_with_bom_parses() {
        let dir = tmp("bom");
        let with_bom = format!("\u{feff}{KICK_MODULEINFO}");
        write_fake_vst3(&dir, "Bommed.vst3", &with_bom);
        let mut s = scanner_over(&dir);
        assert_eq!(
            s.scan().len(),
            1,
            "BOM-prefixed moduleinfo.json still parses"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn classify_clap_features() {
        assert_eq!(
            classify_clap(&["instrument".into()]),
            (PluginCategory::Instrument, true)
        );
        assert_eq!(
            classify_clap(&["analyzer".into()]),
            (PluginCategory::Analyzer, false)
        );
        assert_eq!(
            classify_clap(&["note-effect".into()]),
            (PluginCategory::Effect, true),
            "note effects take MIDI but stay effects"
        );
        assert_eq!(classify_clap(&[]), (PluginCategory::Effect, false));
    }

    #[test]
    fn classify_vst3_subcategories() {
        let v = |s: &str| vec![s.to_string()];
        assert_eq!(
            classify_vst3(&v("Instrument|Synth")),
            (PluginCategory::Instrument, true)
        );
        assert_eq!(
            classify_vst3(&v("Fx|Analyzer")),
            (PluginCategory::Analyzer, false)
        );
        assert_eq!(
            classify_vst3(&v("Fx|Dynamics")),
            (PluginCategory::Effect, false)
        );
    }
}
