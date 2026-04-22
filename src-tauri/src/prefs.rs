use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Shared config directory for all prefs files (`~/.config/hardwave/` on
/// Linux, the OS-specific equivalent elsewhere). Returns `None` if the
/// platform has no config dir.
pub fn prefs_dir() -> Option<PathBuf> {
    let mut p = dirs::config_dir()?;
    p.push("hardwave");
    Some(p)
}

/// Audio device preferences persisted between sessions. Matches the fields the
/// Audio Settings dialog lets users configure. Every field is optional on
/// deserialize (via #[serde(default)]) so older prefs files load cleanly after
/// schema additions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AudioPrefs {
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    #[serde(default = "default_buffer_size")]
    pub buffer_size: u32,
    #[serde(default)]
    pub wasapi_exclusive: bool,
    #[serde(default)]
    pub input_device: Option<String>,
    #[serde(default = "default_input_channels")]
    pub input_channels: u16,
}

fn default_sample_rate() -> u32 {
    48000
}
fn default_buffer_size() -> u32 {
    512
}
fn default_input_channels() -> u16 {
    2
}

impl AudioPrefs {
    pub fn default_path() -> Option<PathBuf> {
        let mut p = dirs::config_dir()?;
        p.push("hardwave");
        p.push("audio-prefs.json");
        Some(p)
    }

    pub fn load() -> Self {
        let Some(path) = Self::default_path() else {
            return Self::factory();
        };
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Self>(&bytes).unwrap_or_else(|e| {
                log::warn!("Failed to parse {}: {e}", path.display());
                Self::factory()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::factory(),
            Err(e) => {
                log::warn!("Failed to read {}: {e}", path.display());
                Self::factory()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = Self::default_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(&path, bytes) {
                    log::warn!("Failed to write {}: {e}", path.display());
                }
            }
            Err(e) => log::warn!("Failed to serialize audio prefs: {e}"),
        }
    }

    fn factory() -> Self {
        Self {
            output_device: None,
            sample_rate: default_sample_rate(),
            buffer_size: default_buffer_size(),
            wasapi_exclusive: false,
            input_device: None,
            input_channels: default_input_channels(),
        }
    }
}
