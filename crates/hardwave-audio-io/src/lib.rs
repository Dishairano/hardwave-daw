//! Hardwave Audio I/O — cpal device management and real-time audio stream.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Host, SampleRate, StreamConfig, SupportedStreamConfigRange};
use parking_lot::Mutex as PlMutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use thiserror::Error;

pub use rtrb;

#[cfg(target_os = "windows")]
mod wasapi_exclusive;

#[cfg(target_os = "macos")]
mod coreaudio_workgroup;

#[derive(Error, Debug)]
pub enum AudioIoError {
    #[error("No output device available")]
    NoOutputDevice,
    #[error("No input device available")]
    NoInputDevice,
    #[error("Device error: {0}")]
    Device(String),
    #[error("Stream error: {0}")]
    Stream(String),
}

// ---------------------------------------------------------------------------
// Device info
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub sample_rates: Vec<u32>,
    pub max_channels: u16,
}

// ---------------------------------------------------------------------------
// Audio callback
// ---------------------------------------------------------------------------

/// Trait implemented by the engine to receive audio callbacks.
pub trait AudioCallback: Send + 'static {
    /// Called on the real-time audio thread with output buffer to fill.
    /// `output` is interleaved stereo (L, R, L, R, ...).
    fn process(&mut self, output: &mut [f32], num_frames: usize, num_channels: u16);
}

// ---------------------------------------------------------------------------
// Input peak tracker
// ---------------------------------------------------------------------------

/// Lock-free peak meter shared between the cpal input callback and the UI
/// thread. Each callback writes the block's max(|sample|) into the L/R slots
/// using `fetch_max`; the UI reads and resets them on each poll. The stored
/// values are f32 linear peaks, bit-punned through AtomicU32.
#[derive(Default)]
pub struct InputPeakTracker {
    l_bits: AtomicU32,
    r_bits: AtomicU32,
}

impl InputPeakTracker {
    fn new() -> Self {
        Self::default()
    }

    /// Merge a block peak into the latched value, keeping the max. Called
    /// from the audio input callback.
    fn record(&self, left: f32, right: f32) {
        fn max_f32(slot: &AtomicU32, candidate: f32) {
            let candidate = candidate.abs();
            let mut current = slot.load(Ordering::Relaxed);
            loop {
                let current_f = f32::from_bits(current);
                if candidate <= current_f {
                    return;
                }
                match slot.compare_exchange_weak(
                    current,
                    candidate.to_bits(),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return,
                    Err(seen) => current = seen,
                }
            }
        }
        max_f32(&self.l_bits, left);
        max_f32(&self.r_bits, right);
    }

    /// Read and reset both channel peaks. Returns linear peaks (0..1+).
    pub fn take(&self) -> (f32, f32) {
        let l = f32::from_bits(self.l_bits.swap(0, Ordering::Relaxed));
        let r = f32::from_bits(self.r_bits.swap(0, Ordering::Relaxed));
        (l, r)
    }
}

// ---------------------------------------------------------------------------
// Audio device manager
// ---------------------------------------------------------------------------

pub struct AudioDeviceManager {
    host: Host,
    output_stream: Option<cpal::Stream>,
    #[cfg(target_os = "windows")]
    wasapi_stream: Option<wasapi_exclusive::WasapiExclusiveStream>,
    running: Arc<AtomicBool>,
    /// Set by the cpal error callback when the device fails at runtime
    /// (e.g. USB interface unplugged). Polled by the engine to trigger
    /// a restart on the system default device.
    stream_error: Arc<AtomicBool>,
    pub sample_rate: u32,
    pub buffer_size: u32,
    /// Name of the selected output device (None = system default).
    pub selected_device: Option<String>,
    /// Name of the device currently running (set after successful start()).
    active_device_name: Option<String>,
    /// Name of the selected input device (None = system default). Recorded here
    /// so the engine can open an input stream with the user's preference; no
    /// stream is created until the recording pipeline actually asks for one.
    pub selected_input_device: Option<String>,
    /// How many channels the engine should open on the input stream: 1 = mono
    /// (sums), 2 = stereo. Devices with more channels are downmixed.
    pub input_channels: u16,
    /// Request WASAPI exclusive mode (Windows only). When true and the active
    /// host is WASAPI, the next stream start tries to acquire the endpoint
    /// exclusively for lower latency and bit-perfect output. No effect on
    /// other platforms or host backends.
    pub wasapi_exclusive: bool,
    /// True once an exclusive-mode WASAPI stream is live (for UI reporting).
    active_exclusive: bool,
    /// Active input stream (pre-record monitoring). Independent from the
    /// output stream; started on demand by the recording/monitor UI.
    input_stream: Option<cpal::Stream>,
    /// Peak meter that the input callback writes into. Cloned into the
    /// callback so the UI can poll it lock-free.
    input_peaks: Arc<InputPeakTracker>,
    /// Optional lock-free ring producer the input callback writes interleaved
    /// stereo samples into. When set, the engine's InputNode drains from the
    /// matching consumer on the audio thread so armed tracks hear live input.
    input_monitor_producer: Arc<PlMutex<Option<rtrb::Producer<f32>>>>,
    /// Sample rate the input stream is actually running at (may differ from
    /// `sample_rate` when the input device rejects the output's rate).
    input_active_sample_rate: u32,
    /// Buffer size the input stream is running at.
    input_active_buffer_size: u32,
}

impl AudioDeviceManager {
    /// List audio host backends the current build can speak to. On Linux this
    /// will include ALSA and (if the `jack` feature was enabled) JACK; on
    /// Windows, WASAPI (+ ASIO if built with the ASIO SDK); on macOS, CoreAudio.
    pub fn list_hosts() -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|id| id.name().to_string())
            .collect()
    }

    /// Current host backend name.
    pub fn host_name(&self) -> String {
        self.host.id().name().to_string()
    }

    /// Switch to a different host backend by name (e.g. "JACK", "ALSA",
    /// "WASAPI", "CoreAudio"). Stops the stream if running; caller is
    /// responsible for restarting.
    pub fn set_host(&mut self, host_name: &str) -> Result<(), AudioIoError> {
        let id = cpal::available_hosts()
            .into_iter()
            .find(|id| id.name().eq_ignore_ascii_case(host_name))
            .ok_or_else(|| AudioIoError::Device(format!("Unknown host: {host_name}")))?;
        let host = cpal::host_from_id(id).map_err(|e| AudioIoError::Device(e.to_string()))?;
        self.stop();
        self.host = host;
        self.selected_device = None;
        Ok(())
    }

    pub fn new() -> Self {
        let host = cpal::default_host();
        Self {
            host,
            output_stream: None,
            #[cfg(target_os = "windows")]
            wasapi_stream: None,
            running: Arc::new(AtomicBool::new(false)),
            stream_error: Arc::new(AtomicBool::new(false)),
            sample_rate: 48000,
            buffer_size: 512,
            selected_device: None,
            active_device_name: None,
            selected_input_device: None,
            input_channels: 2,
            wasapi_exclusive: false,
            active_exclusive: false,
            input_stream: None,
            input_peaks: Arc::new(InputPeakTracker::new()),
            input_monitor_producer: Arc::new(PlMutex::new(None)),
            input_active_sample_rate: 0,
            input_active_buffer_size: 0,
        }
    }

    /// Whether the current stream is running in WASAPI exclusive mode.
    pub fn is_exclusive_active(&self) -> bool {
        self.active_exclusive
    }

    /// Whether WASAPI exclusive mode is applicable (Windows + WASAPI host).
    pub fn exclusive_available(&self) -> bool {
        cfg!(target_os = "windows") && self.host.id().name().eq_ignore_ascii_case("WASAPI")
    }

    /// Whether the stream reported an error since the last check.
    /// Resets the flag on read.
    pub fn take_stream_error(&self) -> bool {
        self.stream_error.swap(false, Ordering::Relaxed)
    }

    /// Non-destructive peek at the stream-error flag (for dev inspection).
    pub fn peek_stream_error(&self) -> bool {
        self.stream_error.load(Ordering::Relaxed)
    }

    /// Force the stream-error flag. Used by the dev panel to verify
    /// that the engine's device-recovery path runs end-to-end.
    pub fn inject_stream_error(&self) {
        self.stream_error.store(true, Ordering::Relaxed);
    }

    /// Name of the device currently in use, or None if not started.
    pub fn active_device_name(&self) -> Option<&str> {
        self.active_device_name.as_deref()
    }

    /// List available input devices.
    pub fn list_input_devices(&self) -> Vec<DeviceInfo> {
        let default_name = self
            .host
            .default_input_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        self.host
            .input_devices()
            .map(|devices| {
                devices
                    .filter_map(|d| {
                        let name = d.name().ok()?;
                        let configs: Vec<SupportedStreamConfigRange> =
                            d.supported_input_configs().ok()?.collect();
                        let sample_rates: Vec<u32> = configs
                            .iter()
                            .flat_map(|c| {
                                let min = c.min_sample_rate().0;
                                let max = c.max_sample_rate().0;
                                [44100, 48000, 88200, 96000, 192000]
                                    .into_iter()
                                    .filter(move |&sr| sr >= min && sr <= max)
                            })
                            .collect();
                        let max_channels = configs.iter().map(|c| c.channels()).max().unwrap_or(0);
                        Some(DeviceInfo {
                            is_default: name == default_name,
                            name,
                            sample_rates,
                            max_channels,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Resolve the output device — selected by name, or system default.
    fn resolve_output_device(&self) -> Result<cpal::Device, AudioIoError> {
        if let Some(ref name) = self.selected_device {
            if let Ok(devices) = self.host.output_devices() {
                for d in devices {
                    if d.name().ok().as_deref() == Some(name) {
                        return Ok(d);
                    }
                }
            }
            log::warn!(
                "Selected device '{}' not found, falling back to default",
                name
            );
        }
        self.host
            .default_output_device()
            .ok_or(AudioIoError::NoOutputDevice)
    }

    /// Cheap fingerprint of currently-visible output devices. Used by the UI
    /// thread to detect hot-plug events without re-sending the full list.
    pub fn output_device_fingerprint(&self) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        if let Ok(devices) = self.host.output_devices() {
            for d in devices {
                if let Ok(name) = d.name() {
                    for &byte in name.as_bytes() {
                        hash ^= byte as u64;
                        hash = hash.wrapping_mul(0x100000001b3);
                    }
                    hash ^= 0xff;
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
        }
        hash
    }

    /// List available output devices.
    pub fn list_output_devices(&self) -> Vec<DeviceInfo> {
        let default_name = self
            .host
            .default_output_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        self.host
            .output_devices()
            .map(|devices| {
                devices
                    .filter_map(|d| {
                        let name = d.name().ok()?;
                        let configs: Vec<SupportedStreamConfigRange> =
                            d.supported_output_configs().ok()?.collect();
                        let sample_rates: Vec<u32> = configs
                            .iter()
                            .flat_map(|c| {
                                let min = c.min_sample_rate().0;
                                let max = c.max_sample_rate().0;
                                [44100, 48000, 88200, 96000, 192000]
                                    .into_iter()
                                    .filter(move |&sr| sr >= min && sr <= max)
                            })
                            .collect();
                        let max_channels = configs.iter().map(|c| c.channels()).max().unwrap_or(0);
                        Some(DeviceInfo {
                            is_default: name == default_name,
                            name,
                            sample_rates,
                            max_channels,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Start the output audio stream with the given callback.
    pub fn start<C: AudioCallback>(&mut self, callback: C) -> Result<(), AudioIoError> {
        let device = self.resolve_output_device()?;

        // Negotiate sample rate: check if the requested rate is supported,
        // otherwise fall back to the device's preferred rate.
        let supported_rate = self.is_rate_supported(&device, self.sample_rate);
        if !supported_rate {
            if let Ok(default_config) = device.default_output_config() {
                let fallback = default_config.sample_rate().0;
                log::warn!(
                    "Requested sample rate {} not supported, falling back to {}",
                    self.sample_rate,
                    fallback
                );
                self.sample_rate = fallback;
            }
        }

        let resolved_name = device.name().unwrap_or_default();
        let exclusive_requested =
            self.wasapi_exclusive && self.host.id().name().eq_ignore_ascii_case("WASAPI");
        log::info!(
            "Starting audio: device={}, sr={}, buf={}, exclusive={}",
            resolved_name,
            self.sample_rate,
            self.buffer_size,
            exclusive_requested,
        );
        self.active_device_name = Some(resolved_name.clone());

        self.running.store(true, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);

        #[cfg(target_os = "windows")]
        {
            if exclusive_requested {
                // User explicitly asked for exclusive mode. If we can't get it
                // (format rejection, device locked by another app), surface a
                // hard error — silently downgrading to shared mode would
                // contradict the toggle state they just set.
                let stream = wasapi_exclusive::WasapiExclusiveStream::start(
                    self.selected_device.as_deref(),
                    self.sample_rate,
                    callback,
                    Arc::clone(&self.stream_error),
                )?;
                self.wasapi_stream = Some(stream);
                self.active_exclusive = true;
                return Ok(());
            }
            self.active_exclusive = false;
        }
        #[cfg(not(target_os = "windows"))]
        {
            if exclusive_requested {
                log::warn!(
                    "WASAPI exclusive mode requested but current platform is not Windows — ignoring"
                );
            }
            self.active_exclusive = false;
        }

        let config = StreamConfig {
            channels: 2,
            sample_rate: SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(self.buffer_size),
        };
        let running = Arc::clone(&self.running);
        let stream_error = Arc::clone(&self.stream_error);
        let mut cb = callback;

        // On macOS, the first invocation of the callback (which runs on the
        // CoreAudio IOProc thread) joins the device's IO workgroup so the
        // kernel co-schedules us with the rest of the audio pipeline. The
        // membership handle is held for the stream's lifetime and released
        // when the closure drops.
        #[cfg(target_os = "macos")]
        let mut workgroup: Option<coreaudio_workgroup::WorkgroupMembership> = None;
        #[cfg(target_os = "macos")]
        let mut workgroup_joined = false;

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    #[cfg(target_os = "macos")]
                    {
                        if !workgroup_joined {
                            workgroup = coreaudio_workgroup::join_default_output_workgroup();
                            workgroup_joined = true;
                        }
                    }

                    if !running.load(Ordering::Relaxed) {
                        data.fill(0.0);
                        return;
                    }
                    let num_frames = data.len() / 2;
                    cb.process(data, num_frames, 2);
                },
                move |err| {
                    log::error!("Audio stream error: {}", err);
                    // Signal the engine to restart on the default device.
                    stream_error.store(true, Ordering::Relaxed);
                },
                None,
            )
            .map_err(|e| AudioIoError::Stream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioIoError::Stream(e.to_string()))?;
        self.output_stream = Some(stream);

        Ok(())
    }

    /// Check if a sample rate is supported by a device.
    fn is_rate_supported(&self, device: &cpal::Device, rate: u32) -> bool {
        device
            .supported_output_configs()
            .ok()
            .map(|configs| {
                configs.into_iter().any(|c| {
                    rate >= c.min_sample_rate().0
                        && rate <= c.max_sample_rate().0
                        && c.channels() >= 2
                })
            })
            .unwrap_or(false)
    }

    /// Stop the audio stream.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.output_stream = None;
        #[cfg(target_os = "windows")]
        {
            if let Some(stream) = self.wasapi_stream.take() {
                stream.stop();
            }
        }
        self.active_device_name = None;
        self.active_exclusive = false;
        log::info!("Audio stream stopped");
    }

    pub fn is_running(&self) -> bool {
        if self.running.load(Ordering::Relaxed) {
            return true;
        }
        #[cfg(target_os = "windows")]
        {
            if self.wasapi_stream.is_some() {
                return true;
            }
        }
        false
    }

    /// Resolve the input device — selected by name, or system default.
    fn resolve_input_device(&self) -> Result<cpal::Device, AudioIoError> {
        if let Some(ref name) = self.selected_input_device {
            if let Ok(devices) = self.host.input_devices() {
                for d in devices {
                    if d.name().ok().as_deref() == Some(name) {
                        return Ok(d);
                    }
                }
            }
            log::warn!(
                "Selected input device '{}' not found, falling back to default",
                name
            );
        }
        self.host
            .default_input_device()
            .ok_or(AudioIoError::NoInputDevice)
    }

    /// Pick a sample rate the input device supports. Prefers the current
    /// output rate (`self.sample_rate`) so the monitor path stays aligned,
    /// then falls back to the device's default config.
    fn negotiate_input_rate(&self, device: &cpal::Device) -> u32 {
        let requested = self.sample_rate;
        if let Ok(configs) = device.supported_input_configs() {
            for c in configs {
                if requested >= c.min_sample_rate().0 && requested <= c.max_sample_rate().0 {
                    return requested;
                }
            }
        }
        device
            .default_input_config()
            .map(|c| c.sample_rate().0)
            .unwrap_or(requested)
    }

    /// Start the input stream with pre-record level monitoring. Safe to call
    /// again to restart with updated device/channels.
    pub fn start_input_stream(&mut self) -> Result<(), AudioIoError> {
        self.stop_input_stream();

        let device = self.resolve_input_device()?;
        let channels = self.input_channels.clamp(1, 2);
        let sample_rate = self.negotiate_input_rate(&device);

        // Prefer the output buffer size so the monitor latency tracks the
        // rest of the engine. cpal falls back to the device default if the
        // hint is rejected, which is fine for monitoring.
        let buffer_size = self.buffer_size;
        let config = StreamConfig {
            channels,
            sample_rate: SampleRate(sample_rate),
            buffer_size: cpal::BufferSize::Fixed(buffer_size),
        };

        log::info!(
            "Starting input stream: device={}, sr={}, buf={}, ch={}",
            device.name().unwrap_or_default(),
            sample_rate,
            buffer_size,
            channels,
        );

        let peaks = Arc::clone(&self.input_peaks);
        let monitor_producer = Arc::clone(&self.input_monitor_producer);
        let chans = channels;
        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _info: &cpal::InputCallbackInfo| {
                    if data.is_empty() {
                        return;
                    }
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;

                    // Also stream samples into the monitor ring buffer when
                    // it's attached. We push up to 2 frames of interleaved
                    // stereo per source frame; excess samples are silently
                    // dropped when the ring is full (consumer has gone away
                    // or isn't draining fast enough).
                    let mut producer_guard = monitor_producer.try_lock();
                    let mut producer = producer_guard.as_mut().and_then(|g| g.as_mut());

                    match chans {
                        1 => {
                            for &s in data {
                                let a = s.abs();
                                if a > max_l {
                                    max_l = a;
                                }
                                if let Some(ref mut p) = producer {
                                    let _ = p.push(s);
                                    let _ = p.push(s);
                                }
                            }
                            max_r = max_l;
                        }
                        _ => {
                            // Interleaved stereo (or more; we only look at
                            // the first two channels).
                            let step = chans as usize;
                            let mut i = 0;
                            while i + 1 < data.len() {
                                let l = data[i];
                                let r = data[i + 1];
                                let al = l.abs();
                                let ar = r.abs();
                                if al > max_l {
                                    max_l = al;
                                }
                                if ar > max_r {
                                    max_r = ar;
                                }
                                if let Some(ref mut p) = producer {
                                    let _ = p.push(l);
                                    let _ = p.push(r);
                                }
                                i += step;
                            }
                        }
                    }
                    peaks.record(max_l, max_r);
                },
                move |err| {
                    log::error!("Input stream error: {}", err);
                },
                None,
            )
            .map_err(|e| AudioIoError::Stream(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioIoError::Stream(e.to_string()))?;

        self.input_stream = Some(stream);
        self.input_active_sample_rate = sample_rate;
        self.input_active_buffer_size = buffer_size;
        Ok(())
    }

    /// Stop the input stream if running. Resets the peak tracker so the next
    /// open of the meter starts from silence.
    pub fn stop_input_stream(&mut self) {
        if self.input_stream.take().is_some() {
            log::info!("Input stream stopped");
        }
        let _ = self.input_peaks.take();
        self.input_active_sample_rate = 0;
        self.input_active_buffer_size = 0;
    }

    pub fn is_input_running(&self) -> bool {
        self.input_stream.is_some()
    }

    /// Attach (or detach, when `None`) the ring-buffer producer the input
    /// callback pushes interleaved stereo samples into. The engine owns the
    /// matching consumer and drains it on the audio thread.
    pub fn set_input_monitor_producer(&self, producer: Option<rtrb::Producer<f32>>) {
        *self.input_monitor_producer.lock() = producer;
    }

    /// Snapshot and reset the current input peak. Returns linear L/R peaks.
    pub fn take_input_peak(&self) -> (f32, f32) {
        self.input_peaks.take()
    }

    /// Sample rate the input stream is running at (0 when stopped).
    pub fn input_active_sample_rate(&self) -> u32 {
        self.input_active_sample_rate
    }

    /// Buffer size the input stream is running at (0 when stopped).
    pub fn input_active_buffer_size(&self) -> u32 {
        self.input_active_buffer_size
    }
}

impl Default for AudioDeviceManager {
    fn default() -> Self {
        Self::new()
    }
}

// Safety: cpal::Stream is a handle to an OS audio thread — safe to move between threads.
// The stream itself is only started/stopped from the engine thread.
unsafe impl Send for AudioDeviceManager {}
