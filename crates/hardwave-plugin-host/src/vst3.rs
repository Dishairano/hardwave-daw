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

// The `vst3` crate generates its enum constants from the C++ SDK headers, and
// their integer type follows the platform's C++ ABI: already i32 on Windows
// MSVC, a different width elsewhere. So `Foo_::kBar as i32` is a real cast on
// Linux and a no-op on Windows, where clippy then flags every one of them as
// unnecessary. Dropping the casts breaks the Linux build, and cfg-ing 22 call
// sites would be far worse than the lint. CI clippy only ran on Linux, so
// these first showed up when the gate started running on Windows.
#![allow(clippy::unnecessary_cast)]

use crate::types::*;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use vst3::Steinberg::Vst::{
    BusDirections_, BusInfo, ControllerNumbers_, Event, Event_::EventTypes_, IAudioProcessor,
    IAudioProcessorTrait, IComponent, IComponentHandler, IComponentHandlerTrait, IComponentTrait,
    IEditController, IEditControllerTrait, IEventList, IEventListTrait, IMidiMapping,
    IMidiMappingTrait, IParamValueQueue, IParamValueQueueTrait, IParameterChanges,
    IParameterChangesTrait, IUnitInfo, IUnitInfoTrait, IoModes_, MediaTypes_, NoteOffEvent,
    NoteOnEvent, ParamID, ParamValue, PolyPressureEvent, ProcessContext,
    ProcessContext_::StatesAndFlags_, ProcessModes_, ProcessSetup, ProgramListInfo,
    SymbolicSampleSizes_, ViewType,
};
use vst3::Steinberg::{
    kResultOk, tresult, FIDString, IBStream, IBStreamTrait, IBStream_::IStreamSeekMode_, IPlugView,
    IPlugViewTrait, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait, PClassInfo, TUID,
};

#[cfg(target_os = "windows")]
use vst3::Steinberg::kPlatformTypeHWND;
#[cfg(target_os = "macos")]
use vst3::Steinberg::kPlatformTypeNSView;
#[cfg(target_os = "linux")]
use vst3::Steinberg::kPlatformTypeX11EmbedWindowID;
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
    plug_view: Option<ComPtr<IPlugView>>,
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
    /// Cross-thread queue for parameter changes pushed by the plugin's
    /// own GUI (knobs, sliders) via the IComponentHandler we register
    /// on the controller. The audio thread drains this at the start of
    /// each [`process`] block and applies the changes via
    /// `setParamNormalized` so the plugin's audio side stays in sync
    /// with the GUI's display state. Without this, knob movements in
    /// the floating editor window were silent — the host never heard
    /// about them and the live audio kept rendering from stale values.
    pending_params: crate::types::SharedParamQueue,
    /// Every GUI edit again, drained by the app for automation recording.
    gui_edit_log: crate::types::SharedParamQueue,
    /// Holds the IComponentHandler ComWrapper alive for the lifetime
    /// of the plugin instance. Dropping the wrapper would invalidate
    /// the pointer the controller still holds.
    #[allow(dead_code)]
    component_handler: Option<vst3::ComWrapper<HardwaveComponentHandler>>,
    /// Where the song is, for the plug-in's ProcessContext. A null context
    /// left every tempo-synced delay, LFO, arp and gate without a tempo.
    transport: TransportInfo,
    /// Values the host set since the last block (automation, a controller,
    /// our parameter sheet). The controller hears them at once; the audio
    /// processor only hears what arrives in a block's parameter changes, so
    /// they wait here. Before this the processor of any plug-in with its
    /// own controller never heard automation at all.
    host_param_changes: Vec<(u32, f64)>,
    /// MIDI controllers to parameters, as the plug-in's IMidiMapping says,
    /// per (channel, controller); pitch bend is controller 129 and channel
    /// pressure 128. VST3 has no CC events: a plug-in hears CC and pitch
    /// bend only as these parameters, so they were dropped before.
    midi_map: std::collections::HashMap<(u8, u16), ParamID>,
    /// Made once and refilled every block, so processing does not allocate.
    in_changes: vst3::ComWrapper<HostParameterChanges>,
    in_changes_ptr: Option<ComPtr<IParameterChanges>>,
    events: vst3::ComWrapper<HostEventList>,
    events_ptr: Option<ComPtr<IEventList>>,
    /// Input copies kept between blocks.
    input_scratch: Vec<Vec<f32>>,
}

/// Bridges the plugin's editor → host parameter notifications back to
/// our audio thread. The plugin's GUI invokes `performEdit(id, value)`
/// when the user moves a knob; we capture (id, value) into the
/// shared `pending_params` queue. The matching `Vst3PluginInstance`
/// drains that queue from `process()` and forwards each entry to
/// `controller.setParamNormalized` so the audio side hears the change
/// on the next block.
struct HardwaveComponentHandler {
    pending: crate::types::SharedParamQueue,
    /// The same edits again, for the app rather than the audio thread.
    /// Nothing on the audio path drains this, so a knob moved in the
    /// plug-in's own window can be recorded as automation.
    log: crate::types::SharedParamQueue,
}

impl vst3::Class for HardwaveComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for HardwaveComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        // Touch-events delimit a gesture but don't carry value data;
        // we simply acknowledge so the plugin's automation path keeps
        // marching. A future commit will bookend automation regions
        // here.
        kResultOk
    }
    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        // Capture the change for the audio thread. We try_lock to keep
        // GUI events non-blocking; if the audio thread is mid-drain we
        // skip this notification — the next move re-fires it.
        // ParamID is already u32 and ParamValue is already f64 — no
        // cast needed; rename for clarity in the queue tuple below.
        if let Some(mut q) = self.pending.try_lock() {
            // De-dupe consecutive edits on the same param so a fast
            // knob spin doesn't queue a thousand intermediate values.
            if let Some(last) = q.last_mut() {
                if last.0 == id {
                    last.1 = value;
                    return kResultOk;
                }
            }
            q.push((id, value));
        }
        if let Some(mut log) = self.log.try_lock() {
            if let Some(last) = log.last_mut() {
                if last.0 == id {
                    last.1 = value;
                    return kResultOk;
                }
            }
            // Bounded: if the app is not draining, the oldest moves go
            // rather than the memory.
            if log.len() >= 256 {
                log.remove(0);
            }
            log.push((id, value));
        }
        kResultOk
    }
    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }
    unsafe fn restartComponent(&self, _flags: i32) -> tresult {
        kResultOk
    }
}

impl Vst3PluginInstance {
    pub fn load(descriptor: PluginDescriptor) -> Result<Self, String> {
        Self::load_inner(descriptor, None)
    }

    /// Load the plug-in but reuse the supplied parameter queue instead
    /// of creating a fresh one. Used by the floating-editor path to
    /// route the editor's `IComponentHandler::performEdit` events into
    /// the same queue the chain instance drains, so GUI knob movements
    /// reach the audio chain.
    pub fn load_with_shared_pending(
        descriptor: PluginDescriptor,
        shared: crate::types::SharedParamQueue,
    ) -> Result<Self, String> {
        Self::load_inner(descriptor, Some(shared))
    }

    fn load_inner(
        descriptor: PluginDescriptor,
        shared_pending: Option<crate::types::SharedParamQueue>,
    ) -> Result<Self, String> {
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
        let library = unsafe { crate::load_plugin_library(&binary) }
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

        // Build the cross-thread parameter queue + a HardwaveComponentHandler
        // that pushes plugin-GUI knob movements into it. We then hand
        // the handler's COM pointer to the controller so the plugin
        // notifies us on every performEdit. The wrapper is held in
        // Vst3Inner so its lifetime tracks the plugin instance.
        let pending_params: crate::types::SharedParamQueue =
            shared_pending.unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
        let gui_edit_log: crate::types::SharedParamQueue = Arc::new(Mutex::new(Vec::new()));
        let handler_wrapper = vst3::ComWrapper::new(HardwaveComponentHandler {
            pending: Arc::clone(&pending_params),
            log: Arc::clone(&gui_edit_log),
        });
        if let Some(ctrl) = &controller {
            if let Some(handler_ptr) = handler_wrapper.to_com_ptr::<IComponentHandler>() {
                unsafe { ctrl.setComponentHandler(handler_ptr.as_ptr()) };
            }
        }

        let in_changes = vst3::ComWrapper::new(HostParameterChanges::new());
        let events = vst3::ComWrapper::new(HostEventList {
            events: Mutex::new(Vec::with_capacity(MAX_EVENTS)),
        });
        let inner = Arc::new(Vst3Inner {
            descriptor,
            library: Some(library),
            factory: Some(factory),
            component: Some(component),
            processor,
            controller,
            plug_view: None,
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
            pending_params,
            gui_edit_log,
            component_handler: Some(handler_wrapper),
            transport: TransportInfo::default(),
            host_param_changes: Vec::with_capacity(MAX_HOST_CHANGES),
            midi_map: std::collections::HashMap::new(),
            in_changes_ptr: in_changes.to_com_ptr::<IParameterChanges>(),
            in_changes,
            events_ptr: events.to_com_ptr::<IEventList>(),
            events,
            input_scratch: Vec::new(),
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
        // Detach + release the plug view first so the plugin can
        // clean up its editor state before its controller is dropped.
        if let Some(view) = self.plug_view.take() {
            unsafe { view.removed() };
        }
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
        self.processor.take();
        self.controller.take();
        self.component.take();
        self.factory.take();
        self.library.take();
    }
}

/// One audio class a VST3 module declares, as its factory reports it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vst3ClassSummary {
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub sub_categories: Vec<String>,
}

/// List the audio classes in a VST3 module by asking its factory. This
/// loads the module and runs its code, so only the describe child calls
/// it; the DAW reaches it through the scanner's describer. Bundles that
/// ship a moduleinfo.json never need it. Many (every nih-plug build among
/// them) do not, and were listed under their file name with vendor
/// "Unknown".
pub fn read_vst3_classes(bundle_path: &Path) -> Result<Vec<Vst3ClassSummary>, String> {
    use vst3::Steinberg::{IPluginFactory2, IPluginFactory2Trait, PClassInfo2, PFactoryInfo};
    let binary = resolve_vst3_binary(bundle_path)
        .ok_or_else(|| format!("no VST3 binary in {}", bundle_path.display()))?;
    let library = unsafe { crate::load_plugin_library(&binary) }
        .map_err(|e| format!("dlopen {}: {e}", binary.display()))?;
    let get_factory: libloading::Symbol<GetPluginFactoryFn> =
        unsafe { library.get(b"GetPluginFactory\0") }
            .map_err(|_| format!("{}: GetPluginFactory missing", binary.display()))?;
    let factory = unsafe { ComPtr::<IPluginFactory>::from_raw(get_factory()) }
        .ok_or_else(|| format!("{}: null factory", binary.display()))?;

    let mut factory_info: PFactoryInfo = unsafe { std::mem::zeroed() };
    let factory_vendor = if unsafe { factory.getFactoryInfo(&mut factory_info) } == kResultOk {
        class_info_name_to_string(&factory_info.vendor)
    } else {
        String::new()
    };
    let factory2 = factory.cast::<IPluginFactory2>();
    let mut classes = Vec::new();
    for i in 0..unsafe { factory.countClasses() }.max(0) {
        let summary = match factory2.as_ref() {
            Some(f2) => {
                let mut info: PClassInfo2 = unsafe { std::mem::zeroed() };
                if unsafe { f2.getClassInfo2(i, &mut info) } != kResultOk {
                    continue;
                }
                if class_info_name_to_string(&info.category) != "Audio Module Class" {
                    continue;
                }
                Vst3ClassSummary {
                    name: class_info_name_to_string(&info.name),
                    vendor: class_info_name_to_string(&info.vendor),
                    version: class_info_name_to_string(&info.version),
                    sub_categories: class_info_name_to_string(&info.subCategories)
                        .split('|')
                        .filter(|c| !c.is_empty())
                        .map(str::to_string)
                        .collect(),
                }
            }
            None => {
                let mut info: PClassInfo = unsafe { std::mem::zeroed() };
                if unsafe { factory.getClassInfo(i, &mut info) } != kResultOk {
                    continue;
                }
                if class_info_name_to_string(&info.category) != "Audio Module Class" {
                    continue;
                }
                Vst3ClassSummary {
                    name: class_info_name_to_string(&info.name),
                    vendor: String::new(),
                    version: String::new(),
                    sub_categories: Vec::new(),
                }
            }
        };
        let vendor = if summary.vendor.is_empty() {
            factory_vendor.clone()
        } else {
            summary.vendor.clone()
        };
        classes.push(Vst3ClassSummary { vendor, ..summary });
    }
    // The factory first, then the module it came from. The child exits
    // straight after, so the module is left loaded rather than unloaded
    // under a plug-in that may not expect it.
    drop(factory2);
    drop(factory);
    std::mem::forget(library);
    Ok(classes)
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
    // The plug-in's own count, held to something a plug-in could have;
    // the vector grows as parameters are actually read.
    let count = unsafe { ctrl.getParameterCount() }.clamp(0, 100_000);
    let mut out = Vec::with_capacity(count.min(4096) as usize);
    for i in 0..count {
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
        inner.midi_map = read_midi_mapping(inner.controller.as_ref());
        let block = inner.max_block.max(1) as usize;
        inner.input_scratch = (0..4).map(|_| Vec::with_capacity(block)).collect();
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

        // Drain knob/slider changes that the plugin's GUI emitted via
        // its IComponentHandler since the last block. Forwarding via
        // setParamNormalized keeps the controller and the audio
        // pipeline aligned without breaking through to the IParameterChanges
        // wire format (which would require building a per-block
        // parameter-changes stream and is the next milestone).
        // The processor hears parameters only through this block's
        // parameter changes: edits made in the plug-in's own window (its
        // controller already has them) and values the host set.
        inner.in_changes.clear();
        {
            let mut q = inner.pending_params.lock();
            for &(id, value) in q.iter() {
                inner.in_changes.add(id as ParamID, 0, value);
            }
            q.clear();
        }
        for &(id, value) in inner.host_param_changes.iter() {
            inner.in_changes.add(id as ParamID, 0, value);
        }
        inner.host_param_changes.clear();

        // Prepare output buffers.
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }

        // Build channel pointer arrays for input and output.
        // Copies of the inputs (a plug-in may write to its input buffers),
        // in buffers kept between blocks, and the channel pointers on the
        // stack: none of this allocates once the scratch has grown.
        let n_in = inputs.len().min(MAX_CHANNELS);
        let n_out = outputs.len().min(MAX_CHANNELS);
        if inner.input_scratch.len() < n_in {
            inner.input_scratch.resize_with(n_in, Vec::new);
        }
        for (copy, src) in inner.input_scratch.iter_mut().zip(inputs.iter()).take(n_in) {
            copy.clear();
            copy.extend_from_slice(&src[..num_samples.min(src.len())]);
            copy.resize(num_samples, 0.0);
        }
        let mut input_channel_ptrs = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
        for (ptr, copy) in input_channel_ptrs
            .iter_mut()
            .zip(inner.input_scratch.iter_mut())
            .take(n_in)
        {
            *ptr = copy.as_mut_ptr();
        }
        let mut output_channel_ptrs = [std::ptr::null_mut::<f32>(); MAX_CHANNELS];
        for (ptr, out) in output_channel_ptrs
            .iter_mut()
            .zip(outputs.iter_mut())
            .take(n_out)
        {
            *ptr = out.as_mut_ptr();
        }

        let mut in_bus = vst3::Steinberg::Vst::AudioBusBuffers {
            numChannels: n_in as i32,
            silenceFlags: 0,
            __field0: vst3::Steinberg::Vst::AudioBusBuffers__type0 {
                channelBuffers32: input_channel_ptrs.as_mut_ptr(),
            },
        };
        let mut out_bus = vst3::Steinberg::Vst::AudioBusBuffers {
            numChannels: n_out as i32,
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

        let mut context = process_context(&inner.transport, inner.sample_rate);
        data.processContext = &mut context;

        // Notes go in as events; CC, pitch bend and channel pressure as
        // the parameters the plug-in mapped them to.
        {
            let mut list = inner.events.events.lock();
            list.clear();
            encode_midi_events_for_vst3(midi_in, &mut list, &inner.midi_map, &inner.in_changes);
            if !list.is_empty() {
                if let Some(p) = &inner.events_ptr {
                    data.inputEvents = p.as_ptr();
                }
            }
        }
        if inner.in_changes.count() > 0 {
            if let Some(p) = &inner.in_changes_ptr {
                data.inputParameterChanges = p.as_ptr();
            }
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
        let value = value.clamp(0.0, 1.0);
        if let Some(ctrl) = &inner.controller {
            unsafe { ctrl.setParamNormalized(id, value) };
        }
        // And the audio processor, in the next block. A full queue keeps
        // the newest value of a parameter already in it.
        if let Some(slot) = inner.host_param_changes.iter_mut().find(|(p, _)| *p == id) {
            slot.1 = value;
        } else if inner.host_param_changes.len() < MAX_HOST_CHANGES {
            inner.host_param_changes.push((id, value));
        }
    }

    fn set_transport(&mut self, transport: TransportInfo) {
        if let Ok(inner) = self.inner_mut() {
            inner.transport = transport;
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

    fn open_editor(&mut self, parent: raw_window_handle::RawWindowHandle) -> bool {
        let Ok(inner) = self.inner_mut() else {
            return false;
        };
        let Some(controller) = &inner.controller else {
            return false;
        };
        // Close any existing editor first.
        if let Some(view) = inner.plug_view.take() {
            unsafe { view.removed() };
        }
        let view_name = ViewType::kEditor;
        let view_raw = unsafe { controller.createView(view_name) };
        let Some(view) = (unsafe { ComPtr::<IPlugView>::from_raw(view_raw) }) else {
            return false;
        };
        // Pick the platform type + native pointer from the handle.
        use raw_window_handle::RawWindowHandle as Rwh;
        let (handle_ptr, platform_type): (*mut c_void, FIDString) = match parent {
            #[cfg(target_os = "windows")]
            Rwh::Win32(h) => (h.hwnd.get() as *mut c_void, kPlatformTypeHWND),
            #[cfg(target_os = "macos")]
            Rwh::AppKit(h) => (h.ns_view.as_ptr(), kPlatformTypeNSView),
            #[cfg(target_os = "linux")]
            Rwh::Xlib(h) => (h.window as *mut c_void, kPlatformTypeX11EmbedWindowID),
            #[cfg(target_os = "linux")]
            Rwh::Xcb(h) => (
                h.window.get() as usize as *mut c_void,
                kPlatformTypeX11EmbedWindowID,
            ),
            _ => return false,
        };
        // Some plugins need isPlatformTypeSupported first.
        let supported = unsafe { view.isPlatformTypeSupported(platform_type) };
        if supported != kResultOk {
            return false;
        }
        let attach_res = unsafe { view.attached(handle_ptr, platform_type) };
        if attach_res != kResultOk {
            return false;
        }
        inner.plug_view = Some(view);
        true
    }

    fn editor_size(&self) -> Option<(u32, u32)> {
        let inner: &Vst3Inner = &self.inner;
        let view = inner.plug_view.as_ref()?;
        let mut rect: vst3::Steinberg::ViewRect = unsafe { std::mem::zeroed() };
        if unsafe { view.getSize(&mut rect) } != kResultOk {
            return None;
        }
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        (w > 0 && h > 0).then_some((w as u32, h as u32))
    }

    fn close_editor(&mut self) {
        let Ok(inner) = self.inner_mut() else {
            return;
        };
        if let Some(view) = inner.plug_view.take() {
            unsafe { view.removed() };
        }
    }

    fn has_editor(&self) -> bool {
        self.inner.descriptor.has_editor
    }

    fn pending_params(&self) -> Option<crate::types::SharedParamQueue> {
        Some(Arc::clone(&self.inner.pending_params))
    }

    fn gui_edit_log(&self) -> Option<crate::types::SharedParamQueue> {
        Some(Arc::clone(&self.inner.gui_edit_log))
    }

    /// The presets the plug-in ships with.
    ///
    /// A VST3 publishes them as a program list on its edit controller.
    /// The host never asked, which is why the preset browser could show
    /// only what the user had saved. The first list is the one a plug-in
    /// with presets uses; the rest are per-unit lists, which only
    /// multi-timbral instruments have.
    fn factory_presets(&self) -> Vec<String> {
        let Some(ctrl) = &self.inner.controller else {
            return Vec::new();
        };
        let Some(units) = ctrl.cast::<IUnitInfo>() else {
            return Vec::new();
        };
        unsafe {
            if units.getProgramListCount() < 1 {
                return Vec::new();
            }
            let mut info: ProgramListInfo = std::mem::zeroed();
            if units.getProgramListInfo(0, &mut info) != kResultOk {
                return Vec::new();
            }
            let programs = info.programCount.clamp(0, 10_000);
            let mut names = Vec::with_capacity(programs.min(1024) as usize);
            for index in 0..programs {
                let mut name: [vst3::Steinberg::char16; 128] = [0; 128];
                if units.getProgramName(info.id, index, &mut name) == kResultOk {
                    names.push(wchar_string_to_rust(&name));
                } else {
                    names.push(format!("Preset {}", index + 1));
                }
            }
            names
        }
    }

    /// Switch to one of them.
    ///
    /// A program list is driven by the parameter the plug-in marks as
    /// its program change, which is the list's own id used as a
    /// parameter id, normalised over the number of programs.
    fn load_factory_preset(&mut self, index: usize) -> Result<(), String> {
        let Some(ctrl) = &self.inner.controller else {
            return Err("this plug-in has no edit controller".into());
        };
        let Some(units) = ctrl.cast::<IUnitInfo>() else {
            return Err("this plug-in publishes no presets of its own".into());
        };
        unsafe {
            let mut info: ProgramListInfo = std::mem::zeroed();
            if units.getProgramListCount() < 1
                || units.getProgramListInfo(0, &mut info) != kResultOk
            {
                return Err("this plug-in publishes no presets of its own".into());
            }
            let count = info.programCount.max(1) as f64;
            if index as f64 >= count {
                return Err("there is no preset at that place".into());
            }
            let normalised = if count > 1.0 {
                index as f64 / (count - 1.0)
            } else {
                0.0
            };
            ctrl.setParamNormalized(info.id as ParamID, normalised);
            // The audio side reads program changes through the same
            // queue a GUI edit uses, so the change is heard on the next
            // block rather than at the next reload.
            let mut q = self.inner.pending_params.lock();
            q.push((info.id as u32, normalised));
        }
        Ok(())
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

/// Most events and parameter queues one block carries; preallocated so
/// processing never allocates.
const MAX_EVENTS: usize = 512;
const MAX_CHANNELS: usize = 8;
const MAX_HOST_CHANGES: usize = 256;
const MAX_QUEUES: usize = 128;
const MAX_POINTS: usize = 64;

/// The song position as VST3 wants it.
fn process_context(t: &TransportInfo, sample_rate: f64) -> ProcessContext {
    // SAFETY: a plain C struct; zero is a valid value for every field.
    let mut c: ProcessContext = unsafe { std::mem::zeroed() };
    let (num, den) = (t.time_sig.0.max(1), t.time_sig.1.max(1));
    // Positions are in quarter notes; a bar holds num * 4 / den of them.
    let bar_quarters = num as f64 * 4.0 / den as f64;
    c.state = (StatesAndFlags_::kTempoValid
        | StatesAndFlags_::kTimeSigValid
        | StatesAndFlags_::kProjectTimeMusicValid
        | StatesAndFlags_::kBarPositionValid
        | if t.playing {
            StatesAndFlags_::kPlaying
        } else {
            0
        }) as u32;
    c.sampleRate = sample_rate;
    c.projectTimeSamples = t.position_samples as i64;
    c.continousTimeSamples = t.position_samples as i64;
    c.projectTimeMusic = t.position_beats;
    c.barPositionMusic = (t.position_beats / bar_quarters).floor() * bar_quarters;
    c.tempo = t.tempo;
    c.timeSigNumerator = num as i32;
    c.timeSigDenominator = den as i32;
    c
}

/// Which parameter each MIDI controller drives, per channel, as the
/// plug-in's controller maps them (CC 0..127, channel pressure 128, pitch
/// bend 129). Read once, when the plug-in is activated.
fn read_midi_mapping(
    controller: Option<&ComPtr<IEditController>>,
) -> std::collections::HashMap<(u8, u16), ParamID> {
    let mut map = std::collections::HashMap::new();
    let Some(mapping) = controller.and_then(|c| c.cast::<IMidiMapping>()) else {
        return map;
    };
    for channel in 0..16u8 {
        for ctrl in 0..=(ControllerNumbers_::kPitchBend as u16) {
            let mut id: ParamID = 0;
            let res = unsafe {
                mapping.getMidiControllerAssignment(0, channel as i16, ctrl as i16, &mut id)
            };
            if res == kResultOk {
                map.insert((channel, ctrl), id);
            }
        }
    }
    map
}

/// The host's MIDI for one block: notes and poly pressure as events, the
/// rest as the parameters the plug-in mapped them to.
fn encode_midi_events_for_vst3(
    midi: &[hardwave_midi::MidiEvent],
    out: &mut Vec<Event>,
    map: &std::collections::HashMap<(u8, u16), ParamID>,
    changes: &HostParameterChanges,
) {
    let mut push = |event: Event| {
        if out.len() < MAX_EVENTS {
            out.push(event);
        }
    };
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
                push(event);
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
                push(event);
            }
            hardwave_midi::MidiEvent::Aftertouch {
                timing,
                channel,
                note,
                pressure,
            } => {
                let mut event: Event = unsafe { std::mem::zeroed() };
                event.sampleOffset = timing as i32;
                event.r#type = EventTypes_::kPolyPressureEvent as u16;
                event.__field0.polyPressure = PolyPressureEvent {
                    channel: channel as i16,
                    pitch: note as i16,
                    pressure,
                    noteId: -1,
                };
                push(event);
            }
            hardwave_midi::MidiEvent::ControlChange {
                timing,
                channel,
                cc,
                value,
            } => {
                if let Some(&id) = map.get(&(channel, cc as u16)) {
                    changes.add(id, timing as i32, value.clamp(0.0, 1.0) as f64);
                }
            }
            hardwave_midi::MidiEvent::PitchBend {
                timing,
                channel,
                value,
            } => {
                if let Some(&id) = map.get(&(channel, ControllerNumbers_::kPitchBend as u16)) {
                    // -1..1 around the centre, to 0..1 around 0.5.
                    changes.add(
                        id,
                        timing as i32,
                        ((value.clamp(-1.0, 1.0) + 1.0) * 0.5) as f64,
                    );
                }
            }
            hardwave_midi::MidiEvent::ChannelPressure {
                timing,
                channel,
                pressure,
            } => {
                if let Some(&id) = map.get(&(channel, ControllerNumbers_::kAfterTouch as u16)) {
                    changes.add(id, timing as i32, pressure.clamp(0.0, 1.0) as f64);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HostParameterChanges — the IParameterChanges a block hands the processor.
// A fixed set of queues made up front and reused, so the audio thread never
// allocates; each queue keeps its points in time order, as VST3 requires.
// ---------------------------------------------------------------------------

struct ParamQueueData {
    id: ParamID,
    points: Vec<(i32, f64)>,
}

struct HostParamQueue {
    data: Mutex<ParamQueueData>,
}

impl vst3::Class for HostParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for HostParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.data.lock().id
    }

    unsafe fn getPointCount(&self) -> i32 {
        self.data.lock().points.len() as i32
    }

    unsafe fn getPoint(
        &self,
        index: i32,
        sample_offset: *mut i32,
        value: *mut ParamValue,
    ) -> tresult {
        let d = self.data.lock();
        let Some(&(offset, v)) = d.points.get(index.max(0) as usize).filter(|_| index >= 0) else {
            return vst3::Steinberg::kInvalidArgument;
        };
        if !sample_offset.is_null() {
            *sample_offset = offset;
        }
        if !value.is_null() {
            *value = v;
        }
        kResultOk
    }

    unsafe fn addPoint(&self, sample_offset: i32, value: ParamValue, index: *mut i32) -> tresult {
        let at = self.data.lock().add(sample_offset, value);
        match at {
            Some(i) => {
                if !index.is_null() {
                    *index = i as i32;
                }
                kResultOk
            }
            None => vst3::Steinberg::kResultFalse,
        }
    }
}

impl ParamQueueData {
    /// Add a point in time order; a point at the same offset replaces it.
    fn add(&mut self, offset: i32, value: f64) -> Option<usize> {
        let at = self.points.partition_point(|&(o, _)| o < offset);
        if let Some(p) = self.points.get_mut(at).filter(|p| p.0 == offset) {
            p.1 = value;
            return Some(at);
        }
        if self.points.len() >= MAX_POINTS {
            return None;
        }
        self.points.insert(at, (offset, value));
        Some(at)
    }
}

struct HostParameterChanges {
    queues: Vec<vst3::ComWrapper<HostParamQueue>>,
    ptrs: Vec<Option<ComPtr<IParamValueQueue>>>,
    used: Mutex<usize>,
}

impl HostParameterChanges {
    fn new() -> Self {
        let queues: Vec<_> = (0..MAX_QUEUES)
            .map(|_| {
                vst3::ComWrapper::new(HostParamQueue {
                    data: Mutex::new(ParamQueueData {
                        id: 0,
                        points: Vec::with_capacity(MAX_POINTS),
                    }),
                })
            })
            .collect();
        let ptrs = queues
            .iter()
            .map(|q| q.to_com_ptr::<IParamValueQueue>())
            .collect();
        Self {
            queues,
            ptrs,
            used: Mutex::new(0),
        }
    }

    fn clear(&self) {
        *self.used.lock() = 0;
    }

    fn count(&self) -> usize {
        *self.used.lock()
    }

    /// The queue for a parameter: the one already in use, or the next free.
    fn queue_for(&self, id: ParamID) -> Option<usize> {
        let mut used = self.used.lock();
        if let Some(i) = (0..*used).find(|&i| self.queues[i].data.lock().id == id) {
            return Some(i);
        }
        if *used >= self.queues.len() {
            return None;
        }
        let i = *used;
        {
            let mut d = self.queues[i].data.lock();
            d.id = id;
            d.points.clear();
        }
        *used += 1;
        Some(i)
    }

    /// Add a value for a parameter at a sample offset in this block.
    fn add(&self, id: ParamID, offset: i32, value: f64) {
        if let Some(i) = self.queue_for(id) {
            self.queues[i]
                .data
                .lock()
                .add(offset.max(0), value.clamp(0.0, 1.0));
        }
    }
}

impl vst3::Class for HostParameterChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for HostParameterChanges {
    unsafe fn getParameterCount(&self) -> i32 {
        self.count() as i32
    }

    unsafe fn getParameterData(&self, index: i32) -> *mut IParamValueQueue {
        if index < 0 || index as usize >= self.count() {
            return std::ptr::null_mut();
        }
        self.ptrs[index as usize]
            .as_ref()
            .map(|p| p.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut i32,
    ) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        let Some(i) = self.queue_for(*id) else {
            return std::ptr::null_mut();
        };
        if !index.is_null() {
            *index = i as i32;
        }
        self.ptrs[i]
            .as_ref()
            .map(|p| p.as_ptr())
            .unwrap_or(std::ptr::null_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transport_at(beats: f64, playing: bool) -> TransportInfo {
        TransportInfo {
            playing,
            tempo: 150.0,
            position_beats: beats,
            time_sig: (4, 4),
            sample_rate: 48_000.0,
            position_samples: (beats * 48_000.0 * 60.0 / 150.0) as u64,
        }
    }

    /// The context says where the song is, at what tempo, in which bar.
    #[test]
    fn the_process_context_carries_tempo_position_and_bar() {
        let c = process_context(&transport_at(9.5, true), 48_000.0);
        assert_eq!(c.tempo, 150.0);
        assert_eq!(c.projectTimeMusic, 9.5);
        assert_eq!(
            c.barPositionMusic, 8.0,
            "beat 9.5 is in the bar that starts at 8"
        );
        assert_eq!((c.timeSigNumerator, c.timeSigDenominator), (4, 4));
        assert_eq!(c.projectTimeSamples, 182_400);
        let s = c.state;
        for flag in [
            StatesAndFlags_::kPlaying,
            StatesAndFlags_::kTempoValid,
            StatesAndFlags_::kTimeSigValid,
            StatesAndFlags_::kProjectTimeMusicValid,
            StatesAndFlags_::kBarPositionValid,
        ] {
            assert!(s & flag as u32 != 0, "flag {flag} missing");
        }
        let stopped = process_context(&transport_at(0.0, false), 48_000.0);
        assert_eq!(stopped.state & StatesAndFlags_::kPlaying as u32, 0);
        // 6/8: a bar is three quarter notes.
        let mut t = transport_at(7.0, true);
        t.time_sig = (6, 8);
        assert_eq!(process_context(&t, 48_000.0).barPositionMusic, 6.0);
    }

    /// Points stay in time order per parameter, and a block starts empty.
    #[test]
    fn parameter_changes_keep_points_in_order_and_reset_per_block() {
        let ch = HostParameterChanges::new();
        ch.add(7, 64, 0.5);
        ch.add(7, 0, 0.1);
        ch.add(3, 10, 0.9);
        ch.add(7, 64, 0.6);
        assert_eq!(ch.count(), 2);
        unsafe {
            let q = &ch.queues[0];
            assert_eq!(q.getParameterId(), 7);
            assert_eq!(q.getPointCount(), 2);
            let (mut o, mut v) = (0, 0.0);
            q.getPoint(0, &mut o, &mut v);
            assert_eq!((o, v), (0, 0.1));
            q.getPoint(1, &mut o, &mut v);
            assert_eq!((o, v), (64, 0.6), "the later value at the same offset wins");
            assert!(!ch.getParameterData(1).is_null());
            assert!(ch.getParameterData(2).is_null());
        }
        ch.clear();
        assert_eq!(ch.count(), 0);
        ch.add(9, 0, 1.0);
        unsafe {
            assert_eq!(
                ch.queues[0].getPointCount(),
                1,
                "a reused queue starts empty"
            )
        };
    }

    /// Notes become events; CC and pitch bend become the parameters the
    /// plug-in mapped them to, with pitch bend centred on 0.5.
    #[test]
    fn midi_reaches_a_vst3_as_events_and_mapped_parameters() {
        use hardwave_midi::MidiEvent;
        let mut map = std::collections::HashMap::new();
        map.insert((0u8, 1u16), 100 as ParamID);
        map.insert((0u8, ControllerNumbers_::kPitchBend as u16), 200 as ParamID);
        let changes = HostParameterChanges::new();
        let mut events = Vec::new();
        encode_midi_events_for_vst3(
            &[
                MidiEvent::NoteOn {
                    timing: 3,
                    channel: 0,
                    note: 60,
                    velocity: 0.8,
                },
                MidiEvent::ControlChange {
                    timing: 10,
                    channel: 0,
                    cc: 1,
                    value: 0.25,
                },
                MidiEvent::ControlChange {
                    timing: 11,
                    channel: 0,
                    cc: 74,
                    value: 0.5,
                },
                MidiEvent::PitchBend {
                    timing: 20,
                    channel: 0,
                    value: 0.0,
                },
            ],
            &mut events,
            &map,
            &changes,
        );
        assert_eq!(events.len(), 1, "only the note is an event");
        assert_eq!(
            changes.count(),
            2,
            "CC 74 is not mapped, so it goes nowhere"
        );
        unsafe {
            let (mut o, mut v) = (0, 0.0);
            changes.queues[0].getPoint(0, &mut o, &mut v);
            assert_eq!((changes.queues[0].getParameterId(), o, v), (100, 10, 0.25));
            changes.queues[1].getPoint(0, &mut o, &mut v);
            assert_eq!((changes.queues[1].getParameterId(), o, v), (200, 20, 0.5));
        }
    }

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
