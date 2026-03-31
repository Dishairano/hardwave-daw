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
    pub sample_rate: u32,
    pub buffer_size: u32,
}

impl AudioDeviceManager {
    pub fn new() -> Self {
        let host = cpal::default_host();
        Self {
            host,
            output_stream: None,
            running: Arc::new(AtomicBool::new(false)),
            sample_rate: 48000,
            buffer_size: 512,
        }
    }

    /// List available output devices.
    pub fn list_output_devices(&self) -> Vec<DeviceInfo> {
        let default_name = self.host.default_output_device()
            .and_then(|d| d.name().ok())
            .unwrap_or_default();

        self.host.output_devices()
            .map(|devices| {
                devices.filter_map(|d| {
                    let name = d.name().ok()?;
                    let configs: Vec<SupportedStreamConfigRange> =
                        d.supported_output_configs().ok()?.collect();
                    let sample_rates: Vec<u32> = configs.iter()
                        .flat_map(|c| {
                            let min = c.min_sample_rate().0;
                            let max = c.max_sample_rate().0;
                            [44100, 48000, 88200, 96000, 192000].into_iter()
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
                }).collect()
            })
            .unwrap_or_default()
    }

    /// Start the output audio stream with the given callback.
    pub fn start<C: AudioCallback>(&mut self, mut callback: C) -> Result<(), AudioIoError> {
        let device = self.host.default_output_device()
            .ok_or(AudioIoError::NoOutputDevice)?;

        let config = StreamConfig {
            channels: 2,
            sample_rate: SampleRate(self.sample_rate),
            buffer_size: cpal::BufferSize::Fixed(self.buffer_size),
        };

        log::info!(
            "Starting audio: device={}, sr={}, buf={}",
            device.name().unwrap_or_default(),
            self.sample_rate,
            self.buffer_size,
        );

        self.running.store(true, Ordering::Relaxed);
        let running = Arc::clone(&self.running);

        let stream = device.build_output_stream(
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
            },
            None,
        ).map_err(|e| AudioIoError::Stream(e.to_string()))?;

        stream.play().map_err(|e| AudioIoError::Stream(e.to_string()))?;
        self.output_stream = Some(stream);

        Ok(())
    }

    /// Stop the audio stream.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.output_stream = None;
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
