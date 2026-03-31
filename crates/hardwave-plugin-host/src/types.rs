use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Plugin format
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginFormat {
    Vst3,
    Clap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginCategory {
    Effect,
    Instrument,
    Analyzer,
    Other,
}

// ---------------------------------------------------------------------------
// Plugin descriptor (scan result)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub format: PluginFormat,
    pub path: PathBuf,
    pub category: PluginCategory,
    pub num_inputs: u32,
    pub num_outputs: u32,
    pub has_midi_input: bool,
    pub has_editor: bool,
}

// ---------------------------------------------------------------------------
// Plugin parameter info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub default_value: f64,
    pub min: f64,
    pub max: f64,
    pub unit: String,
    pub automatable: bool,
}

// ---------------------------------------------------------------------------
// Hosted plugin trait
// ---------------------------------------------------------------------------

/// Trait that all hosted plugins (VST3, CLAP, native) implement.
pub trait HostedPlugin: Send {
    fn descriptor(&self) -> &PluginDescriptor;

    fn activate(&mut self, sample_rate: f64, max_block_size: u32) -> Result<(), String>;
    fn deactivate(&mut self);

    /// Process audio. `inputs` and `outputs` are channel arrays of f32 slices.
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    );

    fn get_parameter_count(&self) -> u32;
    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo>;
    fn get_parameter_value(&self, id: u32) -> f64;
    fn set_parameter_value(&mut self, id: u32, value: f64);

    /// Get opaque plugin state for save.
    fn get_state(&self) -> Vec<u8>;
    /// Restore plugin state from save.
    fn set_state(&mut self, state: &[u8]) -> Result<(), String>;

    fn latency_samples(&self) -> u32;

    /// Open the plugin's native editor window, parented to the given handle.
    fn open_editor(&mut self, parent_handle: raw_window_handle::RawWindowHandle) -> bool;
    fn close_editor(&mut self);
    fn has_editor(&self) -> bool;
}
