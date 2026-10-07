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
    /// Libraries that had to be run to be read, and the stamp each had
    /// then. Unchanged ones are not run again on the next launch.
    #[serde(default)]
    described: Vec<(PathBuf, u64)>,
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

/// Reads the plug-ins a CLAP library declares. Reading them means
/// loading the library and running its code, so the DAW hands the
/// scanner one that does it in a separate process; without one, the
/// scanner names a CLAP by its file and never loads it.
pub type ClapDescriber =
    std::sync::Arc<dyn Fn(&Path) -> Option<Vec<crate::clap_ffi::ReadDescriptor>> + Send + Sync>;

/// Reads the classes in a VST3 bundle that has no moduleinfo.json, which
/// also means running it; the DAW hands over one that does it in a child.
pub type Vst3Describer =
    std::sync::Arc<dyn Fn(&Path) -> Option<Vec<crate::vst3::Vst3ClassSummary>> + Send + Sync>;

/// A cheap fingerprint of a file: its size and when it last changed.
fn file_stamp(path: &Path) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos() as u64;
    Some(modified.rotate_left(17) ^ meta.len())
}

/// How deep the scan follows folders inside a plug-in folder. Real
/// vendor layouts are a few levels; a loop of links is not.
const MAX_SCAN_DEPTH: usize = 8;

#[derive(Clone)]
pub struct PluginScanner {
    pub vst3_paths: Vec<PathBuf>,
    pub clap_paths: Vec<PathBuf>,
    pub custom_vst3_paths: Vec<PathBuf>,
    pub custom_clap_paths: Vec<PathBuf>,
    pub blocklist: HashSet<String>,
    cache: Vec<PluginDescriptor>,
    last_diff: ScanDiff,
    clap_describer: Option<ClapDescriber>,
    vst3_describer: Option<Vst3Describer>,
    /// Libraries read by running them, with the stamp they had then.
    described: std::collections::HashMap<PathBuf, u64>,
    /// What the last scan found per library, for reuse while scanning.
    prior_by_path: std::collections::HashMap<PathBuf, Vec<PluginDescriptor>>,
    /// The built-in plug-ins. A scan rebuilds the cache from disk and puts
    /// these back, so every caller gets them; a rescan from the plug-in
    /// picker used to drop them, and their slots lost their names.
    natives: Vec<PluginDescriptor>,
}

impl std::fmt::Debug for PluginScanner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginScanner")
            .field("vst3_paths", &self.vst3_paths)
            .field("clap_paths", &self.clap_paths)
            .field("plugins", &self.cache.len())
            .finish()
    }
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
            clap_describer: None,
            vst3_describer: None,
            described: Default::default(),
            prior_by_path: Default::default(),
            natives: Vec::new(),
        }
    }

    /// How CLAP libraries are read; see [`ClapDescriber`].
    pub fn set_clap_describer(&mut self, describer: ClapDescriber) {
        self.clap_describer = Some(describer);
    }

    /// How VST3 bundles without a moduleinfo.json are read; see
    /// [`Vst3Describer`].
    pub fn set_vst3_describer(&mut self, describer: Vst3Describer) {
        self.vst3_describer = Some(describer);
    }

    /// What the last scan found in this library, if it was read by
    /// running it and has not changed since.
    fn unchanged(&self, library: &Path, stamp: Option<u64>) -> Option<Vec<PluginDescriptor>> {
        let stamp = stamp?;
        if self.described.get(library) != Some(&stamp) {
            return None;
        }
        self.prior_by_path
            .get(library)
            .filter(|found| !found.is_empty())
            .cloned()
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
        self.described = cached.described.into_iter().collect();
        Ok(self.cache.len())
    }

    /// Persist current scan results to disk.
    pub fn save_cache_to_disk(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create cache dir: {e}"))?;
        }
        let cached = CachedScan {
            plugins: self.cache.clone(),
            described: self
                .described
                .iter()
                .map(|(path, stamp)| (path.clone(), *stamp))
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&cached).map_err(|e| format!("encode cache: {e}"))?;
        std::fs::write(path, bytes).map_err(|e| format!("write cache: {e}"))?;
        Ok(())
    }

    /// Scan `shared` without holding its lock for the walk. A scan runs
    /// plug-in code in child processes and can take many seconds; holding
    /// the lock that long stalled everything that names a plug-in, the
    /// track list among them, so the window stopped responding at launch.
    /// The scan works on a copy and the result is swapped in at the end.
    /// Scans are one at a time.
    pub fn scan_shared(
        shared: &parking_lot::Mutex<PluginScanner>,
        progress: Option<ScanProgress>,
    ) -> Vec<PluginDescriptor> {
        static ONE_AT_A_TIME: parking_lot::Mutex<()> = parking_lot::const_mutex(());
        let _turn = ONE_AT_A_TIME.lock();
        let mut working = shared.lock().clone();
        let found = working.scan_with_progress(progress).to_vec();
        let mut live = shared.lock();
        // Settings changed while scanning (a folder added, a plug-in
        // blocked) stay as they are now; only what the scan found moves.
        live.cache = working.cache;
        live.last_diff = working.last_diff;
        live.described = working.described;
        if !live.blocklist.is_empty() {
            let blocked = live.blocklist.clone();
            live.cache.retain(|p| !blocked.contains(&p.id));
        }
        found
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
        self.prior_by_path.clear();
        for p in self.cache.drain(..) {
            self.prior_by_path
                .entry(p.path.clone())
                .or_default()
                .push(p);
        }

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
                let mut seen = HashSet::new();
                self.scan_vst3_dir(path, progress.as_mut(), 0, &mut seen);
            }
        }

        for path in &clap_paths {
            if path.exists() {
                if let Some(cb) = progress.as_mut() {
                    cb(self.cache.len(), &path.display().to_string());
                }
                let mut seen = HashSet::new();
                self.scan_clap_dir(path, progress.as_mut(), 0, &mut seen);
            }
        }

        self.prior_by_path.clear();
        let present: HashSet<&PathBuf> = self.cache.iter().map(|p| &p.path).collect();
        self.described.retain(|path, _| present.contains(path));

        // The built-ins are not on disk; put them back.
        for native in self.natives.clone() {
            if !self.cache.iter().any(|p| p.id == native.id) {
                self.cache.push(native);
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
        match self.natives.iter_mut().find(|p| p.id == descriptor.id) {
            Some(existing) => *existing = descriptor.clone(),
            None => self.natives.push(descriptor.clone()),
        }
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

    fn scan_vst3_dir(
        &mut self,
        dir: &Path,
        mut progress: Option<&mut ScanProgress>,
        depth: usize,
        seen: &mut HashSet<PathBuf>,
    ) {
        if !first_visit(dir, depth, seen) {
            return;
        }
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
                    let descriptors = match self.unchanged(&path, vst3_stamp(&path)) {
                        Some(found) => found,
                        None => {
                            let (found, ran) =
                                parse_vst3_bundle(&path, self.vst3_describer.as_ref());
                            if ran {
                                if let Some(stamp) = vst3_stamp(&path) {
                                    self.described.insert(path.clone(), stamp);
                                }
                            }
                            found
                        }
                    };
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
                self.scan_vst3_dir(&path, progress.as_deref_mut(), depth + 1, seen);
            }
        }
    }

    fn scan_clap_dir(
        &mut self,
        dir: &Path,
        mut progress: Option<&mut ScanProgress>,
        depth: usize,
        seen: &mut HashSet<PathBuf>,
    ) {
        if !first_visit(dir, depth, seen) {
            return;
        }
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
                // Read through the describer, so whatever the library does
                // when it loads happens in another process.
                // A library read before and unchanged since is not run again.
                let found = match self.unchanged(&path, file_stamp(&path)) {
                    Some(found) => found,
                    None => {
                        let (found, ran) = parse_clap_library(&path, self.clap_describer.as_ref());
                        if ran {
                            if let Some(stamp) = file_stamp(&path) {
                                self.described.insert(path.clone(), stamp);
                            }
                        }
                        found
                    }
                };
                self.cache.extend(found);
            } else if path.is_dir() {
                self.scan_clap_dir(&path, progress.as_deref_mut(), depth + 1, seen);
            }
        }
    }
}

/// Load a `.clap` shared library, read its plugin descriptors, and convert
/// them to [`PluginDescriptor`]. Falls back to a filename-only stub if the
/// library cannot be loaded or exposes no plugins — so broken CLAPs still
/// appear in the browser (marked Unknown) where they can be blocklisted.
/// Describe the plug-in classes in one bundle or library, without loading any
/// of its code. Used by the crash probe, which needs a descriptor before it
/// can try to instantiate.
pub fn describe_vst3(bundle_path: &Path) -> Vec<PluginDescriptor> {
    parse_vst3_bundle(bundle_path, None).0
}

/// A VST3's stamp is its binary's: the bundle folder's own time does not
/// change when the plug-in inside it is replaced.
fn vst3_stamp(bundle_path: &Path) -> Option<u64> {
    file_stamp(&crate::vst3::resolve_vst3_binary(bundle_path)?)
}

/// Describe a CLAP by loading it here, in this process. Only for the
/// processes that exist to take that risk: the crash probe and the
/// describe child. The DAW itself goes through its describer.
pub fn describe_clap(library_path: &Path) -> Vec<PluginDescriptor> {
    let here: ClapDescriber = std::sync::Arc::new(crate::clap_ffi::read_clap_descriptors);
    parse_clap_library(library_path, Some(&here)).0
}

/// Whether to walk into a folder: not too deep, and not one already
/// walked through another link.
fn first_visit(dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>) -> bool {
    if depth > MAX_SCAN_DEPTH {
        return false;
    }
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    seen.insert(real)
}

fn parse_clap_library(
    library_path: &Path,
    describer: Option<&ClapDescriber>,
) -> (Vec<PluginDescriptor>, bool) {
    let fallback_name = library_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    match describer.and_then(|describe| describe(library_path)) {
        Some(list) if !list.is_empty() => (
            list.into_iter()
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
            true,
        ),
        _ => (
            vec![PluginDescriptor {
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
            false,
        ),
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
/// The bool says whether the bundle had to be run to be read.
fn parse_vst3_bundle(
    bundle_path: &Path,
    describer: Option<&Vst3Describer>,
) -> (Vec<PluginDescriptor>, bool) {
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
            return (
                audio_classes
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
                    .collect(),
                false,
            );
        }
    }

    // No moduleinfo.json: ask the module itself, through the describer.
    // The first class keeps the id the file name gave it, so slots saved
    // before this still find their plug-in; only its name and vendor are
    // now the real ones.
    let stem_id = format!("vst3:{}", fallback_name.to_lowercase().replace(' ', "-"));
    if let Some(classes) = describer.and_then(|describe| describe(bundle_path)) {
        if !classes.is_empty() {
            let found = classes
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    let name = if c.name.is_empty() {
                        fallback_name.clone()
                    } else {
                        c.name
                    };
                    let (category, has_midi_input) = classify_vst3(&c.sub_categories);
                    PluginDescriptor {
                        id: if i == 0 {
                            stem_id.clone()
                        } else {
                            format!("vst3:{}", name.to_lowercase().replace(' ', "-"))
                        },
                        name,
                        vendor: if c.vendor.is_empty() {
                            "Unknown".into()
                        } else {
                            c.vendor
                        },
                        version: if c.version.is_empty() {
                            "1.0.0".into()
                        } else {
                            c.version
                        },
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
            return (found, true);
        }
    }

    (
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
        }],
        false,
    )
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
    fn a_vst3_without_moduleinfo_gets_its_real_name_and_keeps_its_id() {
        let dir = tmp("vst3-describe");
        // A single-file VST3, as nih-plug bundles look to the scanner when
        // they ship no moduleinfo.json.
        std::fs::write(dir.join("hardwave-wettboi.vst3"), b"not a real module").unwrap();
        let mut s = scanner_over(&dir);
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&calls);
        s.set_vst3_describer(std::sync::Arc::new(move |_path: &Path| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Some(vec![crate::vst3::Vst3ClassSummary {
                name: "WettBoi".into(),
                vendor: "Hardwave Studios".into(),
                version: "0.4.0".into(),
                sub_categories: vec!["Fx".into()],
            }])
        }));
        s.scan();
        let p = s
            .find("vst3:hardwave-wettboi")
            .expect("the id from the file name stays");
        assert_eq!(p.name, "WettBoi");
        assert_eq!(p.vendor, "Hardwave Studios");

        // An unchanged module is not run again on the next scan, including
        // after a restart that reads the scan back from disk.
        s.scan();
        let cache = dir.join("cache.json");
        s.save_cache_to_disk(&cache).unwrap();
        let mut again = scanner_over(&dir);
        again.load_cache_from_disk(&cache).unwrap();
        let counted = std::sync::Arc::clone(&calls);
        again.set_vst3_describer(std::sync::Arc::new(move |_path: &Path| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            None
        }));
        again.scan();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(again.find("vst3:hardwave-wettboi").unwrap().name, "WettBoi");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rescan_keeps_the_built_in_plugins() {
        let dir = tmp("natives-rescan");
        let mut s = scanner_over(&dir);
        s.register_native(PluginDescriptor {
            id: "native:kicksynth".into(),
            name: "KickSynth".into(),
            vendor: "Hardwave".into(),
            version: "1.0.0".into(),
            format: PluginFormat::Vst3,
            path: PathBuf::new(),
            category: PluginCategory::Instrument,
            num_inputs: 0,
            num_outputs: 2,
            has_midi_input: true,
            has_editor: true,
        });
        s.scan();
        assert!(
            s.find("native:kicksynth").is_some(),
            "a rescan must not drop the built-ins"
        );
        assert!(s.last_diff().removed.is_empty() && s.last_diff().added.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
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

#[cfg(test)]
mod hostile_folders {
    use super::*;

    fn scanner_for(dir: &Path) -> PluginScanner {
        let mut s = PluginScanner::new();
        s.vst3_paths = vec![dir.to_path_buf()];
        s.clap_paths = vec![dir.to_path_buf()];
        s
    }

    #[test]
    fn a_clap_is_listed_by_name_without_being_loaded_when_nothing_may_read_it() {
        let dir = std::env::temp_dir().join(format!("hw-scan-clap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Not a library at all: loading it would fail, but it must not
        // even be tried in this process.
        std::fs::write(dir.join("Trap.clap"), b"not a library").unwrap();
        let mut scanner = scanner_for(&dir);
        let found = scanner.scan().to_vec();
        let trap = found
            .iter()
            .find(|d| d.path.ends_with("Trap.clap"))
            .expect("listed");
        assert_eq!(trap.name, "Trap");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_loop_of_links_does_not_hang_the_scan() {
        let dir = std::env::temp_dir().join(format!("hw-scan-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a")).unwrap();
        std::os::unix::fs::symlink(&dir, dir.join("a").join("back")).unwrap();
        std::os::unix::fs::symlink(dir.join("a"), dir.join("again")).unwrap();
        let started = std::time::Instant::now();
        let mut scanner = scanner_for(&dir);
        scanner.scan();
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
