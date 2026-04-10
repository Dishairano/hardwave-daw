//! Plugin scanner — discovers VST3 and CLAP plugins on the system.

use crate::types::*;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct PluginScanner {
    pub vst3_paths: Vec<PathBuf>,
    pub clap_paths: Vec<PathBuf>,
    cache: Vec<PluginDescriptor>,
}

impl PluginScanner {
    pub fn new() -> Self {
        Self {
            vst3_paths: default_vst3_paths(),
            clap_paths: default_clap_paths(),
            cache: Vec::new(),
        }
    }

    /// Scan all configured paths for plugins.
    pub fn scan(&mut self) -> &[PluginDescriptor] {
        self.cache.clear();

        let vst3_paths: Vec<PathBuf> = self.vst3_paths.clone();
        let clap_paths: Vec<PathBuf> = self.clap_paths.clone();

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

        log::info!("Plugin scan complete: {} plugins found", self.cache.len());
        &self.cache
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
                    // In production, this would load the plugin in a subprocess to extract metadata.
                    // For now, create a descriptor from the filename.
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
