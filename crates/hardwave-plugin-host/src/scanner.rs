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
                self.scan_vst3_dir(path);
            }
        }

        for path in &clap_paths {
            if path.exists() {
                self.scan_clap_dir(path);
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

    fn scan_vst3_dir(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext == "vst3" {
                // VST3 bundles are directories on macOS/Linux, .dll on Windows
                if path.is_dir() || path.is_file() {
                    log::debug!("Found VST3: {}", path.display());
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string();

                    self.cache.push(PluginDescriptor {
                        id: format!("vst3:{}", name.to_lowercase().replace(' ', "-")),
                        name: name.clone(),
                        vendor: "Unknown".into(),
                        version: "1.0.0".into(),
                        format: PluginFormat::Vst3,
                        path: path.clone(),
                        category: PluginCategory::Effect,
                        num_inputs: 2,
                        num_outputs: 2,
                        has_midi_input: false,
                        has_editor: true,
                    });
                }

                // Recurse into VST3 bundles
                if path.is_dir() {
                    self.scan_vst3_dir(&path);
                }
            }
        }
    }

    fn scan_clap_dir(&mut self, dir: &Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext == "clap" {
                log::debug!("Found CLAP: {}", path.display());
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();

                self.cache.push(PluginDescriptor {
                    id: format!("clap:{}", name.to_lowercase().replace(' ', "-")),
                    name,
                    vendor: "Unknown".into(),
                    version: "1.0.0".into(),
                    format: PluginFormat::Clap,
                    path: path.clone(),
                    category: PluginCategory::Effect,
                    num_inputs: 2,
                    num_outputs: 2,
                    has_midi_input: false,
                    has_editor: true,
                });
            }
        }
    }
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
