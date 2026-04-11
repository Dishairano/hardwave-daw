//! Hardwave Audio I/O — cpal device management and real-time audio stream.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Host, SampleRate, StreamConfig, SupportedStreamConfigRange};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;

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
// Audio device manager
// ---------------------------------------------------------------------------

pub struct AudioDeviceManager {
    host: Host,
    output_stream: Option<cpal::Stream>,
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
}

impl AudioDeviceManager {
    pub fn new() -> Self {
        let host = cpal::default_host();
        Self {
            host,
            output_stream: None,
            running: Arc::new(AtomicBool::new(false)),
            stream_error: Arc::new(AtomicBool::new(false)),
            sample_rate: 48000,
            buffer_size: 512,
            selected_device: None,
            active_device_name: None,
        }
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
    pub fn start<C: AudioCallback>(&mut self, mut callback: C) -> Result<(), AudioIoError> {
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

        let config = StreamConfig {
            channels: 2,
            sample_rate: SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(self.buffer_size),
        };

        let resolved_name = device.name().unwrap_or_default();
        log::info!(
            "Starting audio: device={}, sr={}, buf={}",
            resolved_name,
            self.sample_rate,
            self.buffer_size,
        );
        self.active_device_name = Some(resolved_name);

        self.running.store(true, Ordering::Relaxed);
        self.stream_error.store(false, Ordering::Relaxed);
        let running = Arc::clone(&self.running);
        let stream_error = Arc::clone(&self.stream_error);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    if !running.load(Ordering::Relaxed) {
                        data.fill(0.0);
                        return;
                    }
                    let num_frames = data.len() / 2;
                    callback.process(data, num_frames, 2);
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
        self.active_device_name = None;
        log::info!("Audio stream stopped");
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
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
