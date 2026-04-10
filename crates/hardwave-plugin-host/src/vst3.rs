//! VST3 plugin loading and hosting.
//!
//! Uses vst3-sys for the raw COM interface and libloading for dynamic library loading.
//! This is the foundation — full parameter/editor bridging will be built on top.

use crate::types::*;
use std::path::Path;

/// Load a VST3 plugin from a .vst3 bundle/dll path.
///
/// On Windows: loads the .dll directly
/// On macOS: loads Contents/MacOS/<name> inside the .vst3 bundle
/// On Linux: loads Contents/x86_64-linux/<name>.so inside the .vst3 bundle
pub fn resolve_vst3_binary(bundle_path: &Path) -> Option<std::path::PathBuf> {
    if bundle_path.is_file() {
        // Windows: .vst3 is the DLL itself
        return Some(bundle_path.to_path_buf());
    }

    if bundle_path.is_dir() {
        let name = bundle_path.file_stem()?.to_str()?;

        #[cfg(target_os = "macos")]
        {
            let binary = bundle_path.join("Contents/MacOS").join(name);
            if binary.exists() {
                return Some(binary);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let binary = bundle_path
                .join("Contents/x86_64-linux")
                .join(format!("{}.so", name));
            if binary.exists() {
                return Some(binary);
            }
        }

        #[cfg(target_os = "windows")]
        {
            let binary = bundle_path
                .join("Contents/x86_64-win")
                .join(format!("{}.vst3", name));
            if binary.exists() {
                return Some(binary);
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// VST3 plugin instance (stub — full implementation requires COM interface work)
// ---------------------------------------------------------------------------

/// Placeholder for a loaded VST3 plugin instance.
/// Full implementation will use vst3-sys IComponent + IEditController interfaces.
pub struct Vst3PluginInstance {
    descriptor: PluginDescriptor,
    active: bool,
    sample_rate: f64,
    // In full implementation:
    // library: libloading::Library,
    // factory: *mut vst3_sys::IPluginFactory,
    // component: *mut vst3_sys::IComponent,
    // controller: *mut vst3_sys::IEditController,
    // audio_processor: *mut vst3_sys::IAudioProcessor,
}

impl Vst3PluginInstance {
    pub fn load(descriptor: PluginDescriptor) -> Result<Self, String> {
        let _binary = resolve_vst3_binary(&descriptor.path).ok_or_else(|| {
            format!(
                "Could not resolve VST3 binary: {}",
                descriptor.path.display()
            )
        })?;

        log::info!(
            "Loading VST3: {} from {}",
            descriptor.name,
            descriptor.path.display()
        );

        // TODO: Full VST3 COM loading sequence:
        // 1. libloading::Library::new(binary)
        // 2. GetPluginFactory() -> IPluginFactory
        // 3. factory.createInstance(classId, IComponent::iid) -> IComponent
        // 4. component.initialize(hostContext)
        // 5. component.queryInterface(IAudioProcessor::iid) -> IAudioProcessor
        // 6. factory.createInstance(classId, IEditController::iid) -> IEditController
        // 7. controller.initialize(hostContext)

        Ok(Self {
            descriptor,
            active: false,
            sample_rate: 48000.0,
        })
    }
}

impl HostedPlugin for Vst3PluginInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, _max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate;
        self.active = true;
        // TODO: IAudioProcessor::setupProcessing() + setActive(true)
        Ok(())
    }

    fn deactivate(&mut self) {
        self.active = false;
        // TODO: IAudioProcessor::setActive(false)
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        // TODO: Full VST3 process call via IAudioProcessor::process()
        // For now, pass through audio unchanged
        for (ch, output) in outputs.iter_mut().enumerate() {
            if ch < inputs.len() {
                output.clear();
                output.extend_from_slice(&inputs[ch][..num_samples]);
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        0
    }
    fn get_parameter_info(&self, _index: u32) -> Option<ParameterInfo> {
        None
    }
    fn get_parameter_value(&self, _id: u32) -> f64 {
        0.0
    }
    fn set_parameter_value(&mut self, _id: u32, _value: f64) {}

    fn get_state(&self) -> Vec<u8> {
        Vec::new()
    }
    fn set_state(&mut self, _state: &[u8]) -> Result<(), String> {
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        0
    }

    fn open_editor(&mut self, _parent: raw_window_handle::RawWindowHandle) -> bool {
        false
    }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool {
        self.descriptor.has_editor
    }
}
