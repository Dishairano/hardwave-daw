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
// VST3 plugin instance
// ---------------------------------------------------------------------------

/// A loaded VST3 plugin instance.
///
/// The current implementation opens the plugin's dynamic library via
/// `libloading` and verifies the `GetPluginFactory` export exists — that's
/// enough to reject obviously-invalid bundles up front. Full COM interface
/// traversal (createInstance → IAudioProcessor / IEditController) is the
/// next integration tier; until then, processing is a pass-through.
pub struct Vst3PluginInstance {
    descriptor: PluginDescriptor,
    active: bool,
    sample_rate: f64,
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    _library: Option<libloading::Library>,
}

impl Vst3PluginInstance {
    pub fn load(descriptor: PluginDescriptor) -> Result<Self, String> {
        let binary = resolve_vst3_binary(&descriptor.path).ok_or_else(|| {
            format!(
                "Could not resolve VST3 binary: {}",
                descriptor.path.display()
            )
        })?;

        log::info!(
            "Loading VST3: {} from {}",
            descriptor.name,
            binary.display()
        );

        #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
        let library = {
            let lib = unsafe { libloading::Library::new(&binary) }
                .map_err(|e| format!("dlopen {}: {e}", binary.display()))?;
            // Probe for the VST3 factory entry point. Any valid VST3
            // binary exports `GetPluginFactory`; a missing symbol is a
            // hard reject so we fail fast before the audio thread tries
            // to instantiate a broken plugin.
            let probe: Result<
                libloading::Symbol<unsafe extern "C" fn() -> *mut core::ffi::c_void>,
                _,
            > = unsafe { lib.get(b"GetPluginFactory\0") };
            if probe.is_err() {
                return Err(format!(
                    "{}: not a VST3 binary (GetPluginFactory missing)",
                    binary.display()
                ));
            }
            Some(lib)
        };

        Ok(Self {
            descriptor,
            active: false,
            sample_rate: 48000.0,
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
            _library: library,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn descriptor_for(path: PathBuf) -> PluginDescriptor {
        PluginDescriptor {
            id: "test.bogus.vst3".into(),
            name: "Bogus".into(),
            vendor: "Test".into(),
            version: "0.0.0".into(),
            format: PluginFormat::Vst3,
            path,
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    #[test]
    fn load_missing_path_errors_cleanly() {
        let desc = descriptor_for(PathBuf::from("/definitely/does/not/exist.vst3"));
        let result = Vst3PluginInstance::load(desc);
        assert!(result.is_err(), "missing binary should fail");
    }

    #[test]
    fn load_non_vst3_file_rejects_with_missing_symbol() {
        // Point at a real file that isn't a VST3 — /bin/ls exists on
        // Linux / macOS and is a valid binary dlopen can open, but it
        // doesn't export `GetPluginFactory` so the loader rejects it.
        let candidate = PathBuf::from("/bin/ls");
        if !candidate.exists() {
            return; // skip on platforms without /bin/ls
        }
        let desc = descriptor_for(candidate);
        let result = Vst3PluginInstance::load(desc);
        assert!(result.is_err(), "non-VST3 binary should be rejected");
        let err = result.err().unwrap();
        assert!(
            err.contains("GetPluginFactory") || err.contains("dlopen"),
            "unexpected error: {err}"
        );
    }
}
