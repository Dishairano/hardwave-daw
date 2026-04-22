//! CLAP plugin instance loader. Mirrors `Vst3PluginInstance::load` —
//! opens the `.clap` dynamic library via `libloading`, resolves
//! `clap_entry`, calls `init(path)`, and keeps the handle live for
//! the plugin's lifetime. On drop, `deinit()` is called and the
//! library unloads.
//!
//! Audio processing, MIDI, parameter, and state calls are still
//! TODO — they need the CLAP plugin-factory + `clap_plugin_t`
//! function-table wiring that lives above this loader. But having a
//! real library handle means the rest of the host can trust that
//! a `ClapPluginInstance` corresponds to a binary that the OS could
//! actually load.

use crate::clap_ffi::{ClapPluginEntry, ClapPluginFactory};
use crate::types::*;
use std::ffi::CString;

pub struct ClapPluginInstance {
    descriptor: PluginDescriptor,
    active: bool,
    sample_rate: f64,
    library: Option<libloading::Library>,
    entry: *const ClapPluginEntry,
}

// SAFETY: `clap_entry` function tables are required by the CLAP spec
// to be thread-safe. We never mutate state through the pointer; the
// pointer itself is owned by the shared library whose lifetime is
// bounded by `library`.
unsafe impl Send for ClapPluginInstance {}

impl ClapPluginInstance {
    pub fn load(descriptor: PluginDescriptor) -> Result<Self, String> {
        let path = descriptor.path.clone();
        if !path.exists() {
            return Err(format!("CLAP binary not found: {}", path.display()));
        }
        let lib = unsafe { libloading::Library::new(&path) }
            .map_err(|e| format!("dlopen {}: {e}", path.display()))?;
        let entry_sym: libloading::Symbol<*const ClapPluginEntry> = unsafe {
            lib.get(b"clap_entry\0")
        }
        .map_err(|_| format!("{}: not a CLAP binary (clap_entry missing)", path.display()))?;
        let entry = *entry_sym;
        if entry.is_null() {
            return Err(format!("{}: clap_entry symbol is null", path.display()));
        }
        // Call init(path) — per CLAP contract the plugin is allowed to
        // return `false` for any reason; we surface that as an error.
        let path_c = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "CLAP path contains interior NUL".to_string())?;
        let init_ok = unsafe { ((*entry).init)(path_c.as_ptr()) };
        if !init_ok {
            return Err(format!(
                "{}: clap_entry.init returned false",
                path.display()
            ));
        }
        Ok(Self {
            descriptor,
            active: false,
            sample_rate: 48_000.0,
            library: Some(lib),
            entry,
        })
    }

    /// Fetch the plugin factory — the call site every downstream CLAP
    /// API walks through. Returns `None` if the plugin doesn't advertise
    /// the `clap.plugin-factory` id.
    pub fn plugin_factory(&self) -> Option<*const ClapPluginFactory> {
        if self.entry.is_null() {
            return None;
        }
        let factory_id = CString::new("clap.plugin-factory").ok()?;
        let ptr = unsafe { ((*self.entry).get_factory)(factory_id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginFactory)
        }
    }
}

impl Drop for ClapPluginInstance {
    fn drop(&mut self) {
        if !self.entry.is_null() {
            unsafe { ((*self.entry).deinit)() };
        }
        // Library drops here; `entry` pointer into it becomes invalid.
        self.library.take();
    }
}

impl HostedPlugin for ClapPluginInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, _max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate;
        self.active = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        self.active = false;
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        _midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        // TODO: walk the clap_plugin_t function table.
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
            id: "test.bogus.clap".into(),
            name: "Bogus".into(),
            vendor: "Test".into(),
            version: "0.0.0".into(),
            format: PluginFormat::Clap,
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
        let desc = descriptor_for(PathBuf::from("/definitely/does/not/exist.clap"));
        let result = ClapPluginInstance::load(desc);
        assert!(result.is_err());
    }

    #[test]
    fn load_non_clap_file_rejects_on_missing_entry() {
        let candidate = PathBuf::from("/bin/ls");
        if !candidate.exists() {
            return;
        }
        let desc = descriptor_for(candidate);
        let result = ClapPluginInstance::load(desc);
        assert!(result.is_err(), "non-CLAP binary must be rejected");
        let err = result.err().unwrap();
        assert!(
            err.contains("clap_entry") || err.contains("dlopen"),
            "unexpected error: {err}"
        );
    }
}
