//! CLAP plugin instance — opens the binary via `libloading`,
//! resolves `clap_entry`, creates a `clap_plugin_t` via the plugin
//! factory, drives its lifecycle (`init`, `activate`,
//! `start_processing`, `process`, `stop_processing`, `deactivate`,
//! `destroy`), and exposes parameters + state via the `clap.params`
//! and `clap.state` extensions.
//!
//! Audio processing is a real call through to the plugin's `process`
//! function pointer with a populated `ClapProcess` block. MIDI events
//! from the host side are translated into CLAP note-on / note-off /
//! raw-MIDI events via a minimal in-memory queue.

use crate::clap_ffi::{
    build_static_host, ClapAudioBuffer, ClapEventHeader, ClapEventMidi, ClapEventNote,
    ClapInputEvents, ClapIstream, ClapOstream, ClapOutputEvents, ClapParamInfo, ClapPlugin,
    ClapPluginEntry, ClapPluginFactory, ClapPluginParams, ClapPluginState, ClapProcess,
    CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
    CLAP_EXT_PARAMS, CLAP_EXT_STATE, CLAP_PROCESS_ERROR,
};
use crate::types::*;
use std::ffi::{c_void, CString};

pub struct ClapPluginInstance {
    descriptor: PluginDescriptor,
    active: bool,
    processing: bool,
    sample_rate: f64,
    max_block: u32,
    library: Option<libloading::Library>,
    entry: *const ClapPluginEntry,
    plugin: *const ClapPlugin,
    // Host context must outlive the plugin; keep owned even though the
    // field isn't read directly after create_plugin returns.
    #[allow(dead_code)]
    host: Box<crate::clap_ffi::ClapHost>,
    cached_params: Vec<ParameterInfo>,
    initialized: bool,
}

// SAFETY: all pointers point into the plugin binary (lifetime bound
// by `library`) or into host-owned boxes (`host`, `Box<ClapPlugin>`).
// The CLAP contract allows the host to call the plugin's function
// table from a single audio thread; we uphold that at the call sites.
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
        let path_c = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| "CLAP path contains interior NUL".to_string())?;
        let init_ok = unsafe { ((*entry).init)(path_c.as_ptr()) };
        if !init_ok {
            return Err(format!(
                "{}: clap_entry.init returned false",
                path.display()
            ));
        }
        let factory_id =
            CString::new("clap.plugin-factory").map_err(|_| "invalid factory id".to_string())?;
        let factory_ptr = unsafe { ((*entry).get_factory)(factory_id.as_ptr()) };
        if factory_ptr.is_null() {
            unsafe { ((*entry).deinit)() };
            return Err(format!("{}: no plugin factory", path.display()));
        }
        let factory = factory_ptr as *const ClapPluginFactory;
        let host = Box::new(build_static_host());
        let plugin_id_c = CString::new(descriptor.id.as_bytes())
            .map_err(|_| "plugin id contains interior NUL".to_string())?;
        let plugin = unsafe {
            ((*factory).create_plugin)(factory, &*host as *const _, plugin_id_c.as_ptr())
        };
        if plugin.is_null() {
            unsafe { ((*entry).deinit)() };
            return Err(format!(
                "{}: create_plugin returned null for id {}",
                path.display(),
                descriptor.id
            ));
        }
        // Initialize the plugin.
        let init_ok = unsafe { ((*plugin).init)(plugin) };
        if !init_ok {
            unsafe {
                ((*plugin).destroy)(plugin);
                ((*entry).deinit)();
            }
            return Err(format!(
                "{}: clap_plugin.init returned false",
                path.display()
            ));
        }

        let mut me = Self {
            descriptor,
            active: false,
            processing: false,
            sample_rate: 48_000.0,
            max_block: 0,
            library: Some(lib),
            entry,
            plugin,
            host,
            cached_params: Vec::new(),
            initialized: true,
        };
        me.refresh_params();
        Ok(me)
    }

    fn params_ext(&self) -> Option<*const ClapPluginParams> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_PARAMS[..CLAP_EXT_PARAMS.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginParams)
        }
    }

    fn state_ext(&self) -> Option<*const ClapPluginState> {
        if self.plugin.is_null() {
            return None;
        }
        let id = CString::new(&CLAP_EXT_STATE[..CLAP_EXT_STATE.len() - 1]).ok()?;
        let ptr = unsafe { ((*self.plugin).get_extension)(self.plugin, id.as_ptr()) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as *const ClapPluginState)
        }
    }

    fn refresh_params(&mut self) {
        self.cached_params.clear();
        let Some(params) = self.params_ext() else {
            return;
        };
        let count = unsafe { ((*params).count)(self.plugin) };
        for i in 0..count {
            let mut info: ClapParamInfo = unsafe { std::mem::zeroed() };
            let ok = unsafe { ((*params).get_info)(self.plugin, i, &mut info) };
            if !ok {
                continue;
            }
            let name = clap_fixed_string_to_rust(&info.name);
            self.cached_params.push(ParameterInfo {
                id: info.id,
                name,
                default_value: info.default_value,
                min: info.min_value,
                max: info.max_value,
                unit: String::new(),
                automatable: (info.flags & 1) != 0, // CLAP_PARAM_IS_AUTOMATABLE
            });
        }
    }
}

fn clap_fixed_string_to_rust(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

impl Drop for ClapPluginInstance {
    fn drop(&mut self) {
        if self.processing && !self.plugin.is_null() {
            unsafe { ((*self.plugin).stop_processing)(self.plugin) };
        }
        if self.active && !self.plugin.is_null() {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
        }
        if self.initialized && !self.plugin.is_null() {
            unsafe { ((*self.plugin).destroy)(self.plugin) };
        }
        if !self.entry.is_null() {
            unsafe { ((*self.entry).deinit)() };
        }
        self.library.take();
    }
}

impl HostedPlugin for ClapPluginInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate.max(1.0);
        self.max_block = max_block_size.max(1);
        let ok =
            unsafe { ((*self.plugin).activate)(self.plugin, self.sample_rate, 1, self.max_block) };
        if !ok {
            return Err("clap_plugin.activate returned false".into());
        }
        self.active = true;
        let start_ok = unsafe { ((*self.plugin).start_processing)(self.plugin) };
        if !start_ok {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
            self.active = false;
            return Err("clap_plugin.start_processing returned false".into());
        }
        self.processing = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        if self.processing {
            unsafe { ((*self.plugin).stop_processing)(self.plugin) };
            self.processing = false;
        }
        if self.active {
            unsafe { ((*self.plugin).deactivate)(self.plugin) };
            self.active = false;
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        midi_in: &[hardwave_midi::MidiEvent],
        _midi_out: &mut Vec<hardwave_midi::MidiEvent>,
        num_samples: usize,
    ) {
        if !self.processing || self.plugin.is_null() || num_samples == 0 {
            // Fail open — pass audio through if the plugin isn't ready.
            for (ch, output) in outputs.iter_mut().enumerate() {
                if ch < inputs.len() {
                    output.clear();
                    output.extend_from_slice(&inputs[ch][..num_samples.min(inputs[ch].len())]);
                }
            }
            return;
        }
        // Build input channel pointers. CLAP wants `*mut *mut f32`.
        let channel_count = inputs.len().min(outputs.len()).max(1) as u32;
        let mut input_scratch: Vec<Vec<f32>> = inputs
            .iter()
            .map(|c| c[..num_samples.min(c.len())].to_vec())
            .collect();
        // Ensure outputs have enough capacity.
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        let mut input_ptrs: Vec<*mut f32> =
            input_scratch.iter_mut().map(|v| v.as_mut_ptr()).collect();
        let mut output_ptrs: Vec<*mut f32> = outputs.iter_mut().map(|v| v.as_mut_ptr()).collect();
        let audio_in = ClapAudioBuffer {
            data32: input_ptrs.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count,
            latency: 0,
            constant_mask: 0,
        };
        let mut audio_out = ClapAudioBuffer {
            data32: output_ptrs.as_mut_ptr(),
            data64: std::ptr::null_mut(),
            channel_count,
            latency: 0,
            constant_mask: 0,
        };

        let events = encode_midi_events(midi_in);
        let events_ctx = Box::into_raw(Box::new(events));

        let input_events = ClapInputEvents {
            ctx: events_ctx as *mut c_void,
            size: event_queue_size,
            get: event_queue_get,
        };
        let output_events = ClapOutputEvents {
            ctx: std::ptr::null_mut(),
            try_push: event_queue_push_noop,
        };

        let process = ClapProcess {
            steady_time: -1,
            frames_count: num_samples as u32,
            transport: std::ptr::null(),
            audio_inputs: &audio_in,
            audio_outputs: &mut audio_out,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };

        let status = unsafe { ((*self.plugin).process)(self.plugin, &process) };

        // Release the event queue box.
        unsafe {
            let _: Box<EncodedEventQueue> = Box::from_raw(events_ctx);
        }

        if status == CLAP_PROCESS_ERROR {
            log::warn!(
                "CLAP plugin '{}' returned CLAP_PROCESS_ERROR",
                self.descriptor.id
            );
            for out in outputs.iter_mut() {
                for s in out.iter_mut() {
                    *s = 0.0;
                }
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        self.cached_params.len() as u32
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        self.cached_params.get(index as usize).cloned()
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let Some(params) = self.params_ext() else {
            return 0.0;
        };
        let mut out = 0.0_f64;
        let ok = unsafe { ((*params).get_value)(self.plugin, id, &mut out) };
        if ok {
            out
        } else {
            0.0
        }
    }

    fn set_parameter_value(&mut self, _id: u32, _value: f64) {
        // CLAP sets parameters via events in `process` or `params.flush`
        // — the audio-graph event path submits a param-value event on
        // the next block, not through this synchronous host call. We
        // leave this as a no-op rather than synthesize a fake event
        // that would bypass the plugin's own synchronization.
    }

    fn get_state(&self) -> Vec<u8> {
        let Some(state) = self.state_ext() else {
            return Vec::new();
        };
        let buf: Box<StateWriter> = Box::new(StateWriter { bytes: Vec::new() });
        let ctx = Box::into_raw(buf);
        let stream = ClapOstream {
            ctx: ctx as *mut c_void,
            write: state_writer_write,
        };
        let ok = unsafe { ((*state).save)(self.plugin, &stream) };
        let written = unsafe { Box::from_raw(ctx) };
        if ok {
            written.bytes
        } else {
            Vec::new()
        }
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Some(state) = self.state_ext() else {
            return Ok(());
        };
        let reader: Box<StateReader> = Box::new(StateReader {
            bytes: bytes.to_vec(),
            cursor: 0,
        });
        let ctx = Box::into_raw(reader);
        let stream = ClapIstream {
            ctx: ctx as *mut c_void,
            read: state_reader_read,
        };
        let ok = unsafe { ((*state).load)(self.plugin, &stream) };
        let _reader = unsafe { Box::from_raw(ctx) };
        if ok {
            Ok(())
        } else {
            Err("clap_plugin_state.load returned false".into())
        }
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

// ---------------------------------------------------------------------------
// Event queue bridging: host-side MidiEvent list → CLAP event queue
// ---------------------------------------------------------------------------

enum EncodedEvent {
    NoteOn(ClapEventNote),
    NoteOff(ClapEventNote),
    Midi(ClapEventMidi),
}

struct EncodedEventQueue {
    events: Vec<EncodedEvent>,
}

fn encode_midi_events(midi: &[hardwave_midi::MidiEvent]) -> EncodedEventQueue {
    let mut events = Vec::with_capacity(midi.len());
    for ev in midi {
        match *ev {
            hardwave_midi::MidiEvent::NoteOn {
                timing,
                channel,
                note,
                velocity,
            } => {
                events.push(EncodedEvent::NoteOn(ClapEventNote {
                    header: ClapEventHeader {
                        size: std::mem::size_of::<ClapEventNote>() as u32,
                        time: timing,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        event_type: CLAP_EVENT_NOTE_ON,
                        flags: 0,
                    },
                    note_id: -1,
                    port_index: 0,
                    channel: channel as i16,
                    key: note as i16,
                    velocity: velocity as f64,
                }));
            }
            hardwave_midi::MidiEvent::NoteOff {
                timing,
                channel,
                note,
                velocity,
            } => {
                events.push(EncodedEvent::NoteOff(ClapEventNote {
                    header: ClapEventHeader {
                        size: std::mem::size_of::<ClapEventNote>() as u32,
                        time: timing,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        event_type: CLAP_EVENT_NOTE_OFF,
                        flags: 0,
                    },
                    note_id: -1,
                    port_index: 0,
                    channel: channel as i16,
                    key: note as i16,
                    velocity: velocity as f64,
                }));
            }
            hardwave_midi::MidiEvent::ControlChange {
                timing,
                channel,
                cc,
                value,
            } => {
                let cc_value = (value.clamp(0.0, 1.0) * 127.0) as u8;
                events.push(EncodedEvent::Midi(ClapEventMidi {
                    header: ClapEventHeader {
                        size: std::mem::size_of::<ClapEventMidi>() as u32,
                        time: timing,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        event_type: CLAP_EVENT_MIDI,
                        flags: 0,
                    },
                    port_index: 0,
                    data: [0xB0 | (channel & 0x0F), cc, cc_value],
                }));
            }
            _ => {}
        }
    }
    EncodedEventQueue { events }
}

unsafe extern "C" fn event_queue_size(list: *const ClapInputEvents) -> u32 {
    let q = &*((*list).ctx as *const EncodedEventQueue);
    q.events.len() as u32
}

unsafe extern "C" fn event_queue_get(
    list: *const ClapInputEvents,
    index: u32,
) -> *const ClapEventHeader {
    let q = &*((*list).ctx as *const EncodedEventQueue);
    match q.events.get(index as usize) {
        Some(EncodedEvent::NoteOn(ev)) | Some(EncodedEvent::NoteOff(ev)) => {
            &ev.header as *const ClapEventHeader
        }
        Some(EncodedEvent::Midi(ev)) => &ev.header as *const ClapEventHeader,
        None => std::ptr::null(),
    }
}

unsafe extern "C" fn event_queue_push_noop(
    _list: *const ClapOutputEvents,
    _event: *const ClapEventHeader,
) -> bool {
    true
}

// ---------------------------------------------------------------------------
// State stream bridges — host-owned Vec<u8> for save/load.
// ---------------------------------------------------------------------------

struct StateWriter {
    bytes: Vec<u8>,
}

struct StateReader {
    bytes: Vec<u8>,
    cursor: usize,
}

unsafe extern "C" fn state_writer_write(
    stream: *const ClapOstream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    let writer = &mut *((*stream).ctx as *mut StateWriter);
    let src = std::slice::from_raw_parts(buffer as *const u8, size as usize);
    writer.bytes.extend_from_slice(src);
    size as i64
}

unsafe extern "C" fn state_reader_read(
    stream: *const ClapIstream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    let reader = &mut *((*stream).ctx as *mut StateReader);
    let remaining = reader.bytes.len() - reader.cursor;
    let n = (size as usize).min(remaining);
    if n == 0 {
        return 0;
    }
    let dst = std::slice::from_raw_parts_mut(buffer as *mut u8, n);
    dst.copy_from_slice(&reader.bytes[reader.cursor..reader.cursor + n]);
    reader.cursor += n;
    n as i64
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
    }
}

