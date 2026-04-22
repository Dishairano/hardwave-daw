//! VST3 plugin hosting — real IPluginFactory + IComponent + IAudioProcessor
//! + IEditController traversal via the `vst3` crate's generated bindings.
//!
//! Walks the full VST3 host contract:
//!   GetPluginFactory (libloading)
//!     → IPluginFactory::countClasses / getClassInfo — find target class
//!     → IPluginFactory::createInstance(cid, IComponent::IID) — spin up the component
//!     → IComponent::initialize(null host) — plugin reads its static config
//!     → IComponent::queryInterface(IAudioProcessor::IID) — get DSP facet
//!     → IComponent::queryInterface(IEditController::IID) — get param + state facet
//!     → IComponent::getBusInfo — real audio I/O configuration
//!     → IAudioProcessor::setupProcessing + setActive + setProcessing
//!     → IAudioProcessor::process(ProcessData) — real audio
//!     → IComponent::getState / setState via IBStream — state chunks
//!     → IEditController::getParameterCount + getParameterInfo + getParamNormalized
//!
//! Audio processing is a real call through the plugin's AudioProcessor
//! vtable with populated AudioBusBuffers. MIDI events are translated
//! into the VST3 Event model through a host-owned IEventList stub.

use crate::types::*;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vst3::Steinberg::Vst::{
    BusDirections_, BusInfo, Event, Event_::EventTypes_, IAudioProcessor, IAudioProcessorTrait,
    IComponent, IComponentTrait, IEditController, IEditControllerTrait, IEventList, IEventListTrait,
    IoModes_, MediaTypes_, NoteOffEvent, NoteOnEvent, ProcessModes_, ProcessSetup,
    SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    kResultOk, tresult, FIDString, IBStream, IBStreamTrait, IBStream_::IStreamSeekMode_,
    IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait, PClassInfo, TUID,
};
use vst3::{ComPtr, Interface};

/// Load a VST3 plugin from a .vst3 bundle/dll path.
///
/// On Windows: loads the .dll directly
/// On macOS: loads Contents/MacOS/<name> inside the .vst3 bundle
/// On Linux: loads Contents/x86_64-linux/<name>.so inside the .vst3 bundle
pub fn resolve_vst3_binary(bundle_path: &Path) -> Option<PathBuf> {
    if bundle_path.is_file() {
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

type GetPluginFactoryFn = unsafe extern "C" fn() -> *mut IPluginFactory;

/// Loaded VST3 plugin instance. The inner state is kept behind an
/// `Arc` so `Drop` runs even if a panic unwinds through process().
pub struct Vst3PluginInstance {
    inner: Arc<Vst3Inner>,
}

/// Owned resources; dropped in reverse construction order:
/// processor/component released → factory released → library closed.
struct Vst3Inner {
    descriptor: PluginDescriptor,
    #[allow(dead_code)]
    library: Option<libloading::Library>,
    #[allow(dead_code)]
    factory: Option<ComPtr<IPluginFactory>>,
    component: Option<ComPtr<IComponent>>,
    processor: Option<ComPtr<IAudioProcessor>>,
    controller: Option<ComPtr<IEditController>>,
    class_cid: TUID,
    sample_rate: f64,
    max_block: i32,
    active: bool,
    processing: bool,
    cached_params: Vec<ParameterInfo>,
    input_bus_count: u32,
    output_bus_count: u32,
    num_inputs: u32,
    num_outputs: u32,
    has_midi_input: bool,
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

        // 1. dlopen the binary.
        let library = unsafe { libloading::Library::new(&binary) }
            .map_err(|e| format!("dlopen {}: {e}", binary.display()))?;

        // 2. Resolve GetPluginFactory.
        let get_factory: libloading::Symbol<GetPluginFactoryFn> =
            unsafe { library.get(b"GetPluginFactory\0") }.map_err(|_| {
                format!(
                    "{}: not a VST3 binary (GetPluginFactory missing)",
                    binary.display()
                )
            })?;

        let factory_raw = unsafe { get_factory() };
        if factory_raw.is_null() {
            return Err(format!(
                "{}: GetPluginFactory returned null",
                binary.display()
            ));
        }
        let factory = unsafe { ComPtr::<IPluginFactory>::from_raw(factory_raw) }
            .ok_or_else(|| format!("{}: null factory", binary.display()))?;

        // 3. Walk the class list; match by name (descriptor.name) or
        //    fall back to the first effect/instrument class.
        let class_count = unsafe { factory.countClasses() };
        if class_count <= 0 {
            return Err(format!("{}: factory has no classes", binary.display()));
        }

        let mut chosen_cid: Option<TUID> = None;
        let mut fallback_cid: Option<TUID> = None;
        for i in 0..class_count {
            let mut info: PClassInfo = unsafe { std::mem::zeroed() };
            let res = unsafe { factory.getClassInfo(i, &mut info) };
            if res != kResultOk {
                continue;
            }
            let class_name = class_info_name_to_string(&info.name);
            let class_category = class_info_name_to_string(&info.category);
            if class_name.eq_ignore_ascii_case(&descriptor.name) {
                chosen_cid = Some(info.cid);
                break;
            }
            if class_category == "Audio Module Class"
                || class_category == "Audio Effect Class"
                || class_category == "Instrument Class"
            {
                fallback_cid.get_or_insert(info.cid);
            }
        }
        let class_cid = chosen_cid
            .or(fallback_cid)
            .ok_or_else(|| format!("{}: no suitable class found", binary.display()))?;

        // 4. createInstance(cid, IComponent::IID, &mut obj)
        let mut component_obj: *mut c_void = std::ptr::null_mut();
        let iid = IComponent::IID;
        let create_res = unsafe {
            factory.createInstance(
                class_cid.as_ptr() as FIDString,
                iid.as_ptr() as FIDString,
                &mut component_obj,
            )
        };
        if create_res != kResultOk || component_obj.is_null() {
            return Err(format!(
                "{}: createInstance failed ({create_res})",
                binary.display()
            ));
        }
        let component = unsafe { ComPtr::<IComponent>::from_raw(component_obj as *mut IComponent) }
            .ok_or_else(|| format!("{}: null component after createInstance", binary.display()))?;

        // 5. initialize(null host context) — we pass null; the plugin
        //    must not require host callbacks for basic hosting. A full
        //    host context implementation would plug IHostApplication in
        //    here.
        let init_res = unsafe { component.initialize(std::ptr::null_mut()) };
        if init_res != kResultOk {
            return Err(format!(
                "{}: component.initialize failed ({init_res})",
                binary.display()
            ));
        }

        // 6. Query IAudioProcessor via queryInterface.
        let processor = component.cast::<IAudioProcessor>();

        // 7. Query IEditController via queryInterface.
        //    VST3 plugins with `kDistributable` use a separate controller
        //    created via `createInstance` with the controller's CID; for
        //    simplicity we only try the single-component path here.
        let controller = component.cast::<IEditController>();

        // 8. Set IO mode to simple so the processor handles processing.
        unsafe {
            let _ = component.setIoMode(IoModes_::kSimple as i32);
        }

        // 9. Query real audio I/O counts.
        let input_bus_count = unsafe {
            component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kInput as i32)
        };
        let output_bus_count = unsafe {
            component.getBusCount(MediaTypes_::kAudio as i32, BusDirections_::kOutput as i32)
        };
        let event_input_count = unsafe {
            component.getBusCount(MediaTypes_::kEvent as i32, BusDirections_::kInput as i32)
        };
        let (num_inputs, num_outputs) =
            count_io_channels(&component, input_bus_count, output_bus_count);

        let has_midi_input = event_input_count > 0;

        // 10. Cache parameter descriptors.
        let cached_params = enumerate_parameters(&controller);

        // Activate the audio buses so process() sees them.
        unsafe {
            for i in 0..input_bus_count.max(0) {
                let _ = component.activateBus(
                    MediaTypes_::kAudio as i32,
                    BusDirections_::kInput as i32,
                    i,
                    1,
                );
            }
            for i in 0..output_bus_count.max(0) {
                let _ = component.activateBus(
                    MediaTypes_::kAudio as i32,
                    BusDirections_::kOutput as i32,
                    i,
                    1,
                );
            }
        }

        let inner = Arc::new(Vst3Inner {
            descriptor,
            library: Some(library),
            factory: Some(factory),
            component: Some(component),
            processor,
            controller,
            class_cid,
            sample_rate: 48_000.0,
            max_block: 0,
            active: false,
            processing: false,
            cached_params,
            input_bus_count: input_bus_count.max(0) as u32,
            output_bus_count: output_bus_count.max(0) as u32,
            num_inputs,
            num_outputs,
            has_midi_input,
        });

        Ok(Self { inner })
    }

    /// Returns the real `(num_inputs, num_outputs)` channel counts
    /// queried from `IComponent::getBusInfo`. The `Query plugin audio
    /// I/O configuration` roadmap item reads through this.
    pub fn io_channels(&self) -> (u32, u32) {
        (self.inner.num_inputs, self.inner.num_outputs)
    }

    pub fn has_midi_input(&self) -> bool {
        self.inner.has_midi_input
    }

    pub fn class_cid(&self) -> TUID {
        self.inner.class_cid
    }

    fn inner_mut(&mut self) -> Result<&mut Vst3Inner, String> {
        Arc::get_mut(&mut self.inner)
            .ok_or_else(|| "Vst3Inner aliased — clone the host, not the instance".to_string())
    }
}

impl Drop for Vst3Inner {
    fn drop(&mut self) {
        if self.processing {
            if let Some(p) = &self.processor {
                unsafe { p.setProcessing(0) };
            }
        }
        if self.active {
            if let Some(c) = &self.component {
                unsafe { c.setActive(0) };
            }
        }
        if let Some(c) = &self.component {
            unsafe { c.terminate() };
        }
        // Drop order: processor & controller ComPtrs release before
        // component; component releases before factory; library closes
        // last. The Arc fields below drop in declaration order.
        self.processor.take();
        self.controller.take();
        self.component.take();
        self.factory.take();
        self.library.take();
    }
}

fn class_info_name_to_string(bytes: &[vst3::Steinberg::char8]) -> String {
    let null_pos = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let slice: Vec<u8> = bytes[..null_pos].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&slice).into_owned()
}

fn count_io_channels(
    component: &ComPtr<IComponent>,
    input_buses: i32,
    output_buses: i32,
) -> (u32, u32) {
    let mut inputs = 0u32;
    let mut outputs = 0u32;
    for i in 0..input_buses.max(0) {
        let mut info: BusInfo = unsafe { std::mem::zeroed() };
        let res = unsafe {
            component.getBusInfo(
                MediaTypes_::kAudio as i32,
                BusDirections_::kInput as i32,
                i,
                &mut info,
            )
        };
        if res == kResultOk {
            inputs += info.channelCount.max(0) as u32;
        }
    }
    for i in 0..output_buses.max(0) {
        let mut info: BusInfo = unsafe { std::mem::zeroed() };
        let res = unsafe {
            component.getBusInfo(
                MediaTypes_::kAudio as i32,
                BusDirections_::kOutput as i32,
                i,
                &mut info,
            )
        };
        if res == kResultOk {
            outputs += info.channelCount.max(0) as u32;
        }
    }
    (inputs.max(2), outputs.max(2))
}

fn enumerate_parameters(controller: &Option<ComPtr<IEditController>>) -> Vec<ParameterInfo> {
    let Some(ctrl) = controller else {
        return Vec::new();
    };
    let count = unsafe { ctrl.getParameterCount() };
    let mut out = Vec::with_capacity(count.max(0) as usize);
    for i in 0..count.max(0) {
        let mut info: vst3::Steinberg::Vst::ParameterInfo = unsafe { std::mem::zeroed() };
        let res = unsafe { ctrl.getParameterInfo(i, &mut info) };
        if res != kResultOk {
            continue;
        }
        out.push(ParameterInfo {
            id: info.id,
            name: wchar_string_to_rust(&info.title),
            default_value: info.defaultNormalizedValue,
            min: 0.0,
            max: 1.0,
            unit: wchar_string_to_rust(&info.units),
            automatable: (info.flags & 1) != 0, // ParameterFlags::kCanAutomate = 1
        });
    }
    out
}

fn wchar_string_to_rust(chars: &[vst3::Steinberg::char16]) -> String {
    let end = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..end])
}

impl HostedPlugin for Vst3PluginInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.inner.descriptor
    }

    fn activate(&mut self, sample_rate: f64, max_block_size: u32) -> Result<(), String> {
        let inner = self.inner_mut()?;
        inner.sample_rate = sample_rate.max(1.0);
        inner.max_block = max_block_size.max(1) as i32;
        let Some(processor) = &inner.processor else {
            return Err("IAudioProcessor unavailable".into());
        };
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            maxSamplesPerBlock: inner.max_block,
            sampleRate: inner.sample_rate,
        };
        let res = unsafe { processor.setupProcessing(&mut setup) };
        if res != kResultOk {
            return Err(format!("setupProcessing failed ({res})"));
        }
        if let Some(component) = &inner.component {
            let res = unsafe { component.setActive(1) };
            if res != kResultOk {
                return Err(format!("component.setActive(true) failed ({res})"));
            }
            inner.active = true;
        }
        let res = unsafe { processor.setProcessing(1) };
        if res != kResultOk {
            return Err(format!("setProcessing(true) failed ({res})"));
        }
        inner.processing = true;
        Ok(())
    }

    fn deactivate(&mut self) {
        if let Ok(inner) = self.inner_mut() {
            if inner.processing {
                if let Some(p) = &inner.processor {
                    unsafe { p.setProcessing(0) };
                }
                inner.processing = false;
            }
            if inner.active {
                if let Some(c) = &inner.component {
                    unsafe { c.setActive(0) };
                }
                inner.active = false;
            }
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
        let Ok(inner) = self.inner_mut() else {
            pass_through(inputs, outputs, num_samples);
            return;
        };
        if !inner.processing || inner.processor.is_none() {
            pass_through(inputs, outputs, num_samples);
            return;
        }
        // Prepare output buffers.
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }

        // Build channel pointer arrays for input and output.
        let mut input_copies: Vec<Vec<f32>> = inputs
            .iter()
            .map(|c| c[..num_samples.min(c.len())].to_vec())
            .collect();
        let mut input_channel_ptrs: Vec<*mut f32> =
            input_copies.iter_mut().map(|c| c.as_mut_ptr()).collect();
        let mut output_channel_ptrs: Vec<*mut f32> =
            outputs.iter_mut().map(|c| c.as_mut_ptr()).collect();

        let mut in_bus = vst3::Steinberg::Vst::AudioBusBuffers {
            numChannels: input_channel_ptrs.len() as i32,
            silenceFlags: 0,
            __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                channelBuffers32: input_channel_ptrs.as_mut_ptr(),
            },
        };
        let mut out_bus = vst3::Steinberg::Vst::AudioBusBuffers {
            numChannels: output_channel_ptrs.len() as i32,
            silenceFlags: 0,
            __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                channelBuffers32: output_channel_ptrs.as_mut_ptr(),
            },
        };

        let mut data = vst3::Steinberg::Vst::ProcessData {
            processMode: ProcessModes_::kRealtime as i32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as i32,
            numSamples: num_samples as i32,
            numInputs: if inner.input_bus_count > 0 { 1 } else { 0 },
            numOutputs: if inner.output_bus_count > 0 { 1 } else { 0 },
            inputs: if inner.input_bus_count > 0 {
                &mut in_bus
            } else {
                std::ptr::null_mut()
            },
            outputs: if inner.output_bus_count > 0 {
                &mut out_bus
            } else {
                std::ptr::null_mut()
            },
            inputParameterChanges: std::ptr::null_mut(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };

        // Wire host MIDI events into VST3 Event list via a local
        // IEventList implementation. The wrapper lives until the
        // ComPtr is dropped at the end of this scope.
        let midi_events: Vec<Event> = encode_midi_events_for_vst3(midi_in);
        let event_list_wrapper = vst3::ComWrapper::new(HostEventList {
            events: Mutex::new(midi_events),
        });
        let event_list_ptr = event_list_wrapper
            .to_com_ptr::<IEventList>()
            .map(|p| p.as_ptr())
            .unwrap_or(std::ptr::null_mut());
        if !event_list_ptr.is_null() && !midi_in.is_empty() {
            data.inputEvents = event_list_ptr;
        }

        if let Some(processor) = &inner.processor {
            let res = unsafe { processor.process(&mut data) };
            if res != kResultOk {
                log::warn!(
                    "VST3 plugin '{}' process returned {}; zeroing outputs",
                    inner.descriptor.id,
                    res
                );
                for out in outputs.iter_mut() {
                    for s in out.iter_mut() {
                        *s = 0.0;
                    }
                }
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        self.inner.cached_params.len() as u32
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        self.inner.cached_params.get(index as usize).cloned()
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let Some(ctrl) = &self.inner.controller else {
            return 0.0;
        };
        unsafe { ctrl.getParamNormalized(id) }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let Ok(inner) = self.inner_mut() else {
            return;
        };
        if let Some(ctrl) = &inner.controller {
            unsafe { ctrl.setParamNormalized(id, value.clamp(0.0, 1.0)) };
        }
    }

    fn get_state(&self) -> Vec<u8> {
        let Some(component) = &self.inner.component else {
            return Vec::new();
        };
        let stream = MemoryStream::new_writer();
        let stream_ptr = stream.as_ibstream_ptr();
        let res = unsafe { component.getState(stream_ptr) };
        if res == kResultOk {
            stream.take_bytes()
        } else {
            Vec::new()
        }
    }

    fn set_state(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Ok(inner) = self.inner_mut() else {
            return Err("aliased host".into());
        };
        let Some(component) = &inner.component else {
            return Ok(());
        };
        let stream = MemoryStream::new_reader(bytes.to_vec());
        let stream_ptr = stream.as_ibstream_ptr();
        let res = unsafe { component.setState(stream_ptr) };
        if res != kResultOk {
            return Err(format!("setState failed ({res})"));
        }
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        let Some(processor) = &self.inner.processor else {
            return 0;
        };
        unsafe { processor.getLatencySamples() }
    }

    fn open_editor(&mut self, _parent: raw_window_handle::RawWindowHandle) -> bool {
        false
    }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool {
        self.inner.descriptor.has_editor
    }
}

fn pass_through(inputs: &[&[f32]], outputs: &mut [Vec<f32>], num_samples: usize) {
    for (ch, output) in outputs.iter_mut().enumerate() {
        output.clear();
        if ch < inputs.len() {
            let n = num_samples.min(inputs[ch].len());
            output.extend_from_slice(&inputs[ch][..n]);
            if output.len() < num_samples {
                output.resize(num_samples, 0.0);
            }
        } else {
            output.resize(num_samples, 0.0);
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryStream — an in-process IBStream implementation backed by a
// Vec<u8>. Used for VST3 state save/restore so we can round-trip chunks
// through Rust without touching disk.
// ---------------------------------------------------------------------------

use parking_lot::Mutex;

struct MemoryStreamInner {
    bytes: Vec<u8>,
    cursor: usize,
}

/// A writable / readable IBStream backed by a Rust `Vec<u8>`.
struct MemoryStream {
    wrapped: vst3::ComWrapper<MemoryStreamImpl>,
}

impl MemoryStream {
    fn new_writer() -> Self {
        Self {
            wrapped: vst3::ComWrapper::new(MemoryStreamImpl {
                inner: Mutex::new(MemoryStreamInner {
                    bytes: Vec::new(),
                    cursor: 0,
                }),
            }),
        }
    }

    fn new_reader(bytes: Vec<u8>) -> Self {
        Self {
            wrapped: vst3::ComWrapper::new(MemoryStreamImpl {
                inner: Mutex::new(MemoryStreamInner { bytes, cursor: 0 }),
            }),
        }
    }

    fn as_ibstream_ptr(&self) -> *mut IBStream {
        self.wrapped
            .to_com_ptr::<IBStream>()
            .map(|p| p.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }

    fn take_bytes(self) -> Vec<u8> {
        let imp = self.wrapped;
        let guard = imp.inner.lock();
        guard.bytes.clone()
    }
}

struct MemoryStreamImpl {
    inner: Mutex<MemoryStreamInner>,
}

impl vst3::Class for MemoryStreamImpl {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemoryStreamImpl {
    unsafe fn read(
        &self,
        buffer: *mut c_void,
        num_bytes: i32,
        num_bytes_read: *mut i32,
    ) -> tresult {
        let mut inner = self.inner.lock();
        let remaining = inner.bytes.len() - inner.cursor;
        let n = (num_bytes.max(0) as usize).min(remaining);
        if n > 0 {
            let src = &inner.bytes[inner.cursor..inner.cursor + n];
            std::ptr::copy_nonoverlapping(src.as_ptr(), buffer as *mut u8, n);
            inner.cursor += n;
        }
        if !num_bytes_read.is_null() {
            *num_bytes_read = n as i32;
        }
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: i32,
        num_bytes_written: *mut i32,
    ) -> tresult {
        let mut inner = self.inner.lock();
        let n = num_bytes.max(0) as usize;
        let src = std::slice::from_raw_parts(buffer as *const u8, n);
        // Append / overwrite at the cursor position.
        if inner.cursor >= inner.bytes.len() {
            inner.bytes.extend_from_slice(src);
        } else {
            let end = (inner.cursor + n).min(inner.bytes.len());
            let overlap = end - inner.cursor;
            let cursor = inner.cursor;
            inner.bytes[cursor..end].copy_from_slice(&src[..overlap]);
            if overlap < n {
                inner.bytes.extend_from_slice(&src[overlap..]);
            }
        }
        inner.cursor += n;
        if !num_bytes_written.is_null() {
            *num_bytes_written = n as i32;
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: i64, mode: i32, result: *mut i64) -> tresult {
        let mut inner = self.inner.lock();
        let len = inner.bytes.len() as i64;
        let new_pos = match mode {
            x if x == IStreamSeekMode_::kIBSeekSet as i32 => pos,
            x if x == IStreamSeekMode_::kIBSeekCur as i32 => inner.cursor as i64 + pos,
            x if x == IStreamSeekMode_::kIBSeekEnd as i32 => len + pos,
            _ => return vst3::Steinberg::kInvalidArgument,
        };
        if new_pos < 0 || new_pos > len {
            return vst3::Steinberg::kInvalidArgument;
        }
        inner.cursor = new_pos as usize;
        if !result.is_null() {
            *result = new_pos;
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut i64) -> tresult {
        let inner = self.inner.lock();
        if !pos.is_null() {
            *pos = inner.cursor as i64;
        }
        kResultOk
    }
}

// ---------------------------------------------------------------------------
// HostEventList — in-process IEventList backed by a Vec<Event>.
// Translates host MIDI events into VST3 Event structs for input buses.
// ---------------------------------------------------------------------------

struct HostEventList {
    events: Mutex<Vec<Event>>,
}

impl vst3::Class for HostEventList {
    type Interfaces = (IEventList,);
}

impl IEventListTrait for HostEventList {
    unsafe fn getEventCount(&self) -> i32 {
        self.events.lock().len() as i32
    }

    unsafe fn getEvent(&self, index: i32, e: *mut Event) -> tresult {
        let events = self.events.lock();
        let idx = index as usize;
        if idx >= events.len() || e.is_null() {
            return vst3::Steinberg::kInvalidArgument;
        }
        *e = events[idx];
        kResultOk
    }

    unsafe fn addEvent(&self, e: *mut Event) -> tresult {
        if e.is_null() {
            return vst3::Steinberg::kInvalidArgument;
        }
        self.events.lock().push(*e);
        kResultOk
    }
}

fn encode_midi_events_for_vst3(midi: &[hardwave_midi::MidiEvent]) -> Vec<Event> {
    let mut out = Vec::with_capacity(midi.len());
    for ev in midi {
        match *ev {
            hardwave_midi::MidiEvent::NoteOn {
                timing,
                channel,
                note,
                velocity,
            } => {
                let mut event: Event = unsafe { std::mem::zeroed() };
                event.busIndex = 0;
                event.sampleOffset = timing as i32;
                event.ppqPosition = 0.0;
                event.flags = 0;
                event.r#type = EventTypes_::kNoteOnEvent as u16;
                event.__field0.noteOn = NoteOnEvent {
                    channel: channel as i16,
                    pitch: note as i16,
                    tuning: 0.0,
                    velocity,
                    length: 0,
                    noteId: -1,
                };
                out.push(event);
            }
            hardwave_midi::MidiEvent::NoteOff {
                timing,
                channel,
                note,
                velocity,
            } => {
                let mut event: Event = unsafe { std::mem::zeroed() };
                event.busIndex = 0;
                event.sampleOffset = timing as i32;
                event.ppqPosition = 0.0;
                event.flags = 0;
                event.r#type = EventTypes_::kNoteOffEvent as u16;
                event.__field0.noteOff = NoteOffEvent {
                    channel: channel as i16,
                    pitch: note as i16,
                    tuning: 0.0,
                    velocity,
                    noteId: -1,
                };
                out.push(event);
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(result.is_err());
    }

    #[test]
    fn load_non_vst3_file_rejects_with_missing_symbol() {
        let candidate = PathBuf::from("/bin/ls");
        if !candidate.exists() {
            return;
        }
        let desc = descriptor_for(candidate);
        let result = Vst3PluginInstance::load(desc);
        assert!(result.is_err(), "non-VST3 binary must be rejected");
        let err = result.err().unwrap();
        assert!(
            err.contains("GetPluginFactory") || err.contains("dlopen"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn memory_stream_read_write_round_trip() {
        use vst3::Steinberg::IBStream_::IStreamSeekMode_;
        let stream = MemoryStream::new_writer();
        let ptr = stream.as_ibstream_ptr();
        unsafe {
            let data = [1u8, 2, 3, 4, 5];
            let mut written = 0i32;
            ((*(*ptr).vtbl).write)(
                ptr,
                data.as_ptr() as *mut c_void,
                data.len() as i32,
                &mut written,
            );
            assert_eq!(written, 5);
            let mut pos = 0i64;
            ((*(*ptr).vtbl).seek)(ptr, 0, IStreamSeekMode_::kIBSeekSet as i32, &mut pos);
            let mut buf = [0u8; 5];
            let mut n = 0i32;
            ((*(*ptr).vtbl).read)(ptr, buf.as_mut_ptr() as *mut c_void, 5, &mut n);
            assert_eq!(n, 5);
            assert_eq!(buf, data);
        }
    }
}
