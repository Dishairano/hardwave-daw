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

    /// How many dB this plug-in is currently pulling the signal down by,
    /// as a non-positive number, or `None` when it does not reduce gain.
    ///
    /// Compressors and limiters already work this out every sample and
    /// threw it away, so a mixer could not show what a compressor was
    /// doing. Dynamics plug-ins override it; everything else does not.
    fn gain_reduction_db(&self) -> Option<f32> {
        None
    }

    /// Open the plugin's native editor window, parented to the given handle.
    fn open_editor(&mut self, parent_handle: raw_window_handle::RawWindowHandle) -> bool;
    fn close_editor(&mut self);
    fn has_editor(&self) -> bool;

    /// Returns the shared parameter queue used by the plug-in's GUI to
    /// emit knob movements.
    ///
    /// VST3 and CLAP plug-ins return `Some` so a separate editor instance
    /// and the chain instance can be wired to the same queue: the editor
    /// captures GUI edits (VST3 via `IComponentHandler::performEdit`, CLAP
    /// via the `clap.host-params` `request_flush` → `params.flush` path)
    /// and pushes them into the queue; the chain's audio thread drains it
    /// at the start of each block and applies the change. Without this
    /// shared queue, GUI knob movements in the floating editor are silent
    /// because the audio chain holds a different queue allocation.
    ///
    /// Native plug-ins use direct parameter setters and have no
    /// asynchronous GUI→audio queue, so they return `None`.
    fn pending_params(&self) -> Option<SharedParamQueue> {
        None
    }

    /// A copy of every edit made in the plug-in's own window.
    ///
    /// `pending_params` is drained by the plug-in itself on the audio
    /// path, so by the time the app looks it is empty: knobs inside a
    /// plug-in window could be heard but never recorded. This second
    /// queue is written at the same moment and drained only by the app,
    /// which is what lets automation record a knob the plug-in owns.
    fn gui_edit_log(&self) -> Option<SharedParamQueue> {
        None
    }

    /// The presets that ship inside the plug-in, in the order it lists
    /// them. Empty when the plug-in publishes none, or when the format
    /// gives the host no way to ask.
    fn factory_presets(&self) -> Vec<String> {
        Vec::new()
    }

    /// Play one of those presets, by its place in `factory_presets`.
    fn load_factory_preset(&mut self, _index: usize) -> Result<(), String> {
        Err("this plug-in has no presets of its own that the host can read".into())
    }

    /// Where the song is, told to the plug-in before each block.
    ///
    /// Without it a plug-in can only work in milliseconds: a stutter
    /// ran at its own rate and drifted away from the music, and a
    /// delay could not be set in beats. Plug-ins that do not care
    /// ignore it.
    fn set_transport(&mut self, _transport: TransportInfo) {}
}

/// Where the song is, as the host sees it at the start of a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransportInfo {
    pub playing: bool,
    /// Beats per minute.
    pub tempo: f64,
    /// How far into the song the block starts, in beats.
    pub position_beats: f64,
    /// Beats in a bar and what a beat is, as the song's time signature
    /// says.
    pub time_sig: (u32, u32),
    pub sample_rate: f64,
}

impl Default for TransportInfo {
    fn default() -> Self {
        Self {
            playing: false,
            tempo: 120.0,
            position_beats: 0.0,
            time_sig: (4, 4),
            sample_rate: 48_000.0,
        }
    }
}

impl TransportInfo {
    /// How many samples one beat lasts.
    pub fn samples_per_beat(&self) -> f64 {
        if self.tempo <= 0.0 {
            return self.sample_rate;
        }
        self.sample_rate * 60.0 / self.tempo
    }

    /// Where the block starts inside the bar, 0 to 1.
    pub fn phase_in_bar(&self) -> f64 {
        let beats_per_bar = (self.time_sig.0.max(1)) as f64;
        (self.position_beats / beats_per_bar).rem_euclid(1.0)
    }
}

/// Shared queue for GUI → audio parameter edits, used by both VST3 and
/// CLAP hosts. The editor pushes `(param_id, normalized_value)` entries;
/// the audio thread drains them under `try_lock`/`lock` at the start of
/// each `process` block and applies them to the chain instance.
pub type SharedParamQueue = std::sync::Arc<parking_lot::Mutex<Vec<(u32, f64)>>>;
