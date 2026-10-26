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
    /// First channel of the pair recorded from, counting from zero.
    #[serde(default)]
    pub input_channel_offset: u16,
    /// First channel of the pair the mix goes out of.
    #[serde(default)]
    pub output_channel_offset: u16,
    /// Threads sharing the audio work beside the audio thread. Zero is
    /// the audio thread alone, which is what every build did before.
    #[serde(default)]
    pub worker_threads: usize,
    /// Join an Ableton Link session at startup.
    #[serde(default)]
    pub link_enabled: bool,
    /// Plug-ins to run in a process of their own, by id.
    ///
    /// Sandboxing costs a block of latency, so it is a choice per
    /// plug-in rather than something done to all of them: switch it on
    /// for the one that keeps falling over.
    #[serde(default)]
    pub sandboxed_plugins: Vec<String>,
    /// Listen for OSC, so a phone running TouchOSC can drive the DAW.
    #[serde(default)]
    pub osc_enabled: bool,
    /// The port to listen on. Zero means the default, 9000.
    #[serde(default)]
    pub osc_port: u16,
    /// Network servers the person picked samples from; a project may
    /// only make the DAW open files on these.
    #[serde(default)]
    pub trusted_sample_servers: Vec<String>,
    /// The audio driver (ASIO, WASAPI, JACK...) picked in the settings.
    /// It was not kept, so an ASIO user was back on the default driver at
    /// every launch, with the default device of that driver.
    #[serde(default)]
    pub audio_host: Option<String>,
    /// Let go of the audio device while another program is in front and
    /// the song is stopped (see audio_focus.rs). Off unless switched on.
    #[serde(default)]
    pub release_device_in_background: bool,
    /// Takes as 24-bit WAVs instead of 32-bit float (see TakeFormat).
    #[serde(default)]
    pub take_bits_24: bool,
    /// One folder for every take; none keeps them with the song.
    #[serde(default)]
    pub take_folder: Option<String>,
    /// The output pair the metronome goes out of, first channel counting
    /// from zero; none keeps it in the mix.
    #[serde(default)]
    pub click_channel_offset: Option<u16>,
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
            input_channel_offset: 0,
            output_channel_offset: 0,
            worker_threads: 0,
            link_enabled: false,
            sandboxed_plugins: Vec::new(),
            osc_enabled: false,
            osc_port: 0,
            trusted_sample_servers: Vec::new(),
            audio_host: None,
            release_device_in_background: false,
            take_bits_24: false,
            take_folder: None,
            click_channel_offset: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AudioPrefs;

    #[test]
    fn a_prefs_file_from_before_the_driver_was_kept_still_loads() {
        let old = r#"{"output_device":"Speakers","sample_rate":44100,"buffer_size":256}"#;
        let prefs: AudioPrefs = serde_json::from_str(old).expect("loads");
        assert_eq!(prefs.audio_host, None);
        assert_eq!(prefs.sample_rate, 44100);
        assert!(
            !prefs.release_device_in_background,
            "off unless switched on"
        );
    }

    #[test]
    fn the_driver_is_kept() {
        let mut prefs = AudioPrefs::factory();
        prefs.audio_host = Some("ASIO".into());
        let back: AudioPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).expect("round trip");
        assert_eq!(back.audio_host.as_deref(), Some("ASIO"));
    }

    #[test]
    fn the_take_format_and_folder_are_kept() {
        let mut prefs = AudioPrefs::factory();
        prefs.take_bits_24 = true;
        prefs.take_folder = Some("D:/Takes".into());
        let back: AudioPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).expect("round trip");
        assert!(back.take_bits_24);
        assert_eq!(back.take_folder.as_deref(), Some("D:/Takes"));
    }
}
