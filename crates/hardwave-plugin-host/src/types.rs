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

    /// Hand parameter changes still waiting for the next block to the
    /// plug-in now, without processing audio. VST3 and CLAP plug-ins only
    /// hear a new value inside a block, and with the song stopped no block
    /// runs, so a knob set then read back its old value until play.
    /// Audio thread only, never during `process`. Native plug-ins set their
    /// values directly and have nothing to hand over.
    fn flush_parameters(&mut self) {}

    /// Get opaque plugin state for save.
    fn get_state(&self) -> Vec<u8>;

    /// A VST3 plug-in's class id, which names it in a `.vstpreset`. None
    /// for every other kind.
    fn vst3_class_id(&self) -> Option<[u8; 16]> {
        None
    }
    /// Restore plugin state from save.
    fn set_state(&mut self, state: &[u8]) -> Result<(), String>;

    fn latency_samples(&self) -> u32;

    /// A parameter value as people read it: "1.2 kHz", "-6.0 dB", "Low
    /// pass". Most parameters are stored 0 to 1, which said nothing on
    /// screen ("0.57 Hz"). `None` when the plug-in has no text for it.
    fn parameter_text(&self, _id: u32, _value: f64) -> Option<String> {
        None
    }

    /// For a parameter that picks one of a few choices (a filter mode, a
    /// wave shape), the choices in order. The value for choice `i` of `n`
    /// is `i / (n - 1)`.
    fn parameter_options(&self, _id: u32) -> Option<Vec<String>> {
        None
    }

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

    /// The window of a copy opened with `open_editor` was resized: see
    /// `PluginEditor::resize`.
    fn editor_resize(&mut self, _width: u32, _height: u32) -> Option<(u32, u32)> {
        None
    }

    /// See `PluginEditor::set_scale`.
    fn editor_set_scale(&mut self, _scale: f64) {}

    /// See `PluginEditor::set_resize_request`.
    fn set_editor_resize_request(&mut self, _request: ResizeRequest) {}

    /// The size the open editor asks for, in pixels, so its window can fit
    /// it. None when no editor is open or the plug-in does not say.
    fn editor_size(&self) -> Option<(u32, u32)> {
        None
    }

    /// This instance's own window, to be opened from the thread that owns
    /// windows while the instance itself plays on the audio thread.
    ///
    /// The host used to open a plug-in's window on a second copy of it,
    /// loaded only for the window. That copy never processed audio, so a
    /// plug-in whose interface shows the audio (an analyser, a loudness
    /// meter) waited for it forever, and anything it kept outside its
    /// parameters never reached the copy that played. Formats built for a
    /// separate interface thread (VST3's controller, CLAP's gui extension)
    /// hand this out; the rest keep the second copy.
    fn editor(&self) -> Option<std::sync::Arc<dyn PluginEditor>> {
        None
    }

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
    fn gui_edit_log(&self) -> Option<SharedEditLog> {
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

    /// How long this plug-in goes on sounding after its input falls
    /// silent: a reverb's decay, a delay's echoes, a synth's release.
    ///
    /// The host stops calling a plug-in whose input has been silent for
    /// longer than this and whose output has died away, and calls it
    /// again the moment sound, notes or a knob move arrive. Asked on the
    /// audio thread, so it must be cheap and must not allocate.
    fn tail(&self) -> Tail {
        Tail::Unknown
    }

    /// What the plug-in said about the block it just processed, beyond
    /// the audio: a CLAP plug-in's process status, or a VST3 plug-in
    /// flagging every output channel silent.
    fn process_hint(&self) -> ProcessHint {
        ProcessHint::None
    }

    /// Whether a change waits to reach the plug-in's audio side: a knob
    /// moved in its own window, or a flush it asked for. A sleeping
    /// plug-in (smart disable) is then called anyway, so the change is
    /// heard and reported. Audio thread: cheap, lock-free.
    fn has_pending_changes(&self) -> bool {
        false
    }

    /// Whether every input channel of the next block is exact silence, so
    /// a plug-in that can tell (VST3 silence flags, CLAP constant mask)
    /// may skip its own work on it. Set before `process`.
    fn set_input_silent(&mut self, _silent: bool) {}

    /// The plug-in was not called for this many samples because its input
    /// and output were silent. A plug-in with its own free-running clock
    /// (an LFO, a carrier) moves it on here, so it is where it would have
    /// been had it been called, and a song sounds the same whether it was
    /// asleep or not. Audio thread, cheap, no allocation.
    ///
    /// VST3 and CLAP have no way to be told this. Woken, such a plug-in is
    /// given the true song position (`set_transport`), so anything synced
    /// to the song lands right, but a free-running LFO inside it carries on
    /// from where it was when it fell asleep, at a different phase than
    /// had it run throughout. That is why a third-party plug-in is only
    /// left out on its own word or the producer's.
    fn skip_silence(&mut self, _num_samples: usize) {}
}

/// How long a plug-in goes on sounding after its input falls silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    /// The plug-in does not say. The host assumes a few seconds and also
    /// waits for the output to die away.
    Unknown,
    /// This many samples at the rate the plug-in runs at.
    Samples(u32),
    /// It may go on sounding forever with silence in (a delay at full
    /// feedback, a drone): never stop calling it.
    Infinite,
}

/// What a plug-in said about its last block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessHint {
    /// Nothing beyond the audio.
    #[default]
    None,
    /// Keep calling it, whatever the input (CLAP_PROCESS_CONTINUE).
    Continue,
    /// Keep calling it while its output is not quiet
    /// (CLAP_PROCESS_CONTINUE_IF_NOT_QUIET).
    ContinueIfNotQuiet,
    /// Go by its tail (CLAP_PROCESS_TAIL).
    Tail,
    /// It needs nothing more until input or events arrive
    /// (CLAP_PROCESS_SLEEP).
    Sleep,
    /// It flagged every output channel silent (VST3 silence flags). A hint
    /// only: the buffer is what is heard.
    OutputSilent,
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
    /// The same position in samples.
    pub position_samples: u64,
}

impl Default for TransportInfo {
    fn default() -> Self {
        Self {
            playing: false,
            tempo: 120.0,
            position_beats: 0.0,
            time_sig: (4, 4),
            sample_rate: 48_000.0,
            position_samples: 0,
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
/// CLAP hosts. The editor pushes `(param_id, normalized_value)` changes;
/// the instance that plays takes them at the start of each block without
/// ever waiting (see [`crate::param_queue`]).
pub type SharedParamQueue = std::sync::Arc<crate::param_queue::ParamChangeQueue>;

/// The app's copy of the edits made in a plug-in's own window (and of the
/// changes a plug-in reports from its audio side), drained by the app for
/// automation recording. The same lock-free queue as above, so a push from
/// a window or from the audio thread never waits and never drops a move:
/// it keeps the newest value per parameter until the app takes it.
pub type SharedEditLog = std::sync::Arc<crate::param_queue::ParamChangeQueue>;

/// A plug-in's own window on the instance that plays. Every call is made on
/// the thread that owns the app's windows; the instance closes the window
/// itself when it goes away, after which these do nothing.
pub trait PluginEditor: Send + Sync {
    fn open(&self, parent: raw_window_handle::RawWindowHandle) -> bool;
    fn size(&self) -> Option<(u32, u32)>;
    fn close(&self);

    /// The window was resized to `width` by `height` (physical pixels):
    /// the size the editor takes, which a plug-in may round to what it can
    /// draw, or None when it cannot be resized.
    fn resize(&self, _width: u32, _height: u32) -> Option<(u32, u32)> {
        None
    }

    /// The screen's scale (1.0 at 100%, 1.5 at 150%), so the editor draws
    /// sharp and at the size the rest of the screen is.
    fn set_scale(&self, _scale: f64) {}

    /// Called when the plug-in asks for a new size itself (a panel that
    /// opens, a zoom menu of its own), before it is opened.
    fn set_resize_request(&self, _request: ResizeRequest) {}

    /// An editor that shows itself, in a window of its own rather than
    /// in one the host gives it (a plug-in running in its own process):
    /// opens it and says whether it did. None for the usual kind, which
    /// needs `open` with the host's window.
    fn open_own_window(&self) -> Option<bool> {
        None
    }
}

/// What a plug-in calls to ask its window for a new size, in physical
/// pixels; returns whether the window took it.
pub type ResizeRequest = std::sync::Arc<dyn Fn(u32, u32) -> bool + Send + Sync>;
