//! Windows WASAPI exclusive-mode output stream.
//!
//! cpal 0.15 exposes WASAPI only in shared mode, so exclusive-mode rendering
//! is implemented here directly on top of IAudioClient via the `wasapi` crate.
//! An event-driven render thread blocks on the WASAPI event handle, fills
//! the device buffer from the engine callback, and releases it.
//!
//! COM interfaces are initialized *inside* the render thread to stay within
//! a single apartment — the `wasapi` crate's types wrap `NonNull<c_void>`
//! which is not `Send`. The thread sends its initialization result back
//! through a oneshot channel so `start()` can report format errors
//! synchronously.

#![cfg(target_os = "windows")]

use crate::{AudioCallback, AudioIoError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use wasapi::{get_default_device, initialize_mta, Direction, SampleType, ShareMode, WaveFormat};

pub struct WasapiExclusiveStream {
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WasapiExclusiveStream {
    pub fn start<C: AudioCallback>(
        device_name: Option<&str>,
        sample_rate: u32,
        callback: C,
        error_flag: Arc<AtomicBool>,
    ) -> Result<Self, AudioIoError> {
        let running = Arc::new(AtomicBool::new(true));
        let running_thread = Arc::clone(&running);
        let device_name_owned = device_name.map(|s| s.to_string());

        let (init_tx, init_rx) = mpsc::channel::<Result<(), AudioIoError>>();

        let thread = thread::Builder::new()
            .name("hardwave-wasapi-excl".into())
            .spawn(move || {
                run_render_thread(
                    device_name_owned.as_deref(),
                    sample_rate,
                    callback,
                    running_thread,
                    error_flag,
                    init_tx,
                );
            })
            .map_err(|e| AudioIoError::Stream(format!("spawn render thread: {e}")))?;

        match init_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                running,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AudioIoError::Stream(
                    "WASAPI render thread exited before reporting init".into(),
                ))
            }
        }
    }

    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WasapiExclusiveStream {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run_render_thread<C: AudioCallback>(
    device_name: Option<&str>,
    sample_rate: u32,
    mut callback: C,
    running: Arc<AtomicBool>,
    error_flag: Arc<AtomicBool>,
    init_tx: mpsc::Sender<Result<(), AudioIoError>>,
) {
    if let Err(e) = initialize_mta().ok() {
        let _ = init_tx.send(Err(AudioIoError::Stream(format!(
            "COM MTA init failed: {e:?}"
        ))));
        return;
    }

    let device = match resolve_render_device(device_name) {
        Ok(d) => d,
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };

    let mut audio_client = match device.get_iaudioclient() {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(AudioIoError::Stream(format!(
                "get_iaudioclient: {e:?}"
            ))));
            return;
        }
    };

    let desired_format = WaveFormat::new(32, 32, &SampleType::Float, sample_rate as usize, 2, None);

    let (_def_period, min_period) = match audio_client.get_periods() {
        Ok(p) => p,
        Err(e) => {
            let _ = init_tx.send(Err(AudioIoError::Stream(format!("get_periods: {e:?}"))));
            return;
        }
    };

    if let Err(e) = audio_client.initialize_client(
        &desired_format,
        min_period,
        &Direction::Render,
        &ShareMode::Exclusive,
        false,
    ) {
        let _ = init_tx.send(Err(AudioIoError::Stream(format!(
            "initialize_client (exclusive): {e:?} — device may not support the requested format"
        ))));
        return;
    }

    let h_event = match audio_client.set_get_eventhandle() {
        Ok(h) => h,
        Err(e) => {
            let _ = init_tx.send(Err(AudioIoError::Stream(format!(
                "set_get_eventhandle: {e:?}"
            ))));
            return;
        }
    };

    let buffer_frame_count = match audio_client.get_bufferframecount() {
        Ok(n) => n,
        Err(e) => {
            let _ = init_tx.send(Err(AudioIoError::Stream(format!(
                "get_bufferframecount: {e:?}"
            ))));
            return;
        }
    };

    let render_client = match audio_client.get_audiorenderclient() {
        Ok(c) => c,
        Err(e) => {
            let _ = init_tx.send(Err(AudioIoError::Stream(format!(
                "get_audiorenderclient: {e:?}"
            ))));
            return;
        }
    };

    if let Err(e) = audio_client.start_stream() {
        let _ = init_tx.send(Err(AudioIoError::Stream(format!("start_stream: {e:?}"))));
        return;
    }

    // Init succeeded — signal the caller so start() returns Ok.
    let _ = init_tx.send(Ok(()));

    let bytes_per_frame: usize = 2 /* channels */ * 4 /* bytes/f32 */;
    let mut scratch = vec![0f32; buffer_frame_count as usize * 2];
    let mut bytes = vec![0u8; buffer_frame_count as usize * bytes_per_frame];

    while running.load(Ordering::Relaxed) {
        if h_event.wait_for_event(500).is_err() {
            continue;
        }

        let frames = match audio_client.get_available_space_in_frames() {
            Ok(f) => f,
            Err(e) => {
                log::error!("WASAPI available-space query failed: {e:?}");
                error_flag.store(true, Ordering::Relaxed);
                break;
            }
        };
        if frames == 0 {
            continue;
        }

        let samples = frames as usize * 2;
        if scratch.len() < samples {
            scratch.resize(samples, 0.0);
            bytes.resize(frames as usize * bytes_per_frame, 0);
        }
        let out = &mut scratch[..samples];
        out.fill(0.0);
        callback.process(out, frames as usize, 2);

        let bytes_out = &mut bytes[..frames as usize * bytes_per_frame];
        for (i, s) in out.iter().enumerate() {
            let b = s.to_le_bytes();
            let off = i * 4;
            bytes_out[off..off + 4].copy_from_slice(&b);
        }

        if let Err(e) = render_client.write_to_device(frames as usize, bytes_out, None) {
            log::error!("WASAPI write_to_device failed: {e:?}");
            error_flag.store(true, Ordering::Relaxed);
            break;
        }
    }

    let _ = audio_client.stop_stream();
}

fn resolve_render_device(name: Option<&str>) -> Result<wasapi::Device, AudioIoError> {
    if let Some(target) = name {
        let collection = wasapi::DeviceCollection::new(&Direction::Render)
            .map_err(|e| AudioIoError::Stream(format!("DeviceCollection: {e:?}")))?;
        let count = collection
            .get_nbr_devices()
            .map_err(|e| AudioIoError::Stream(format!("get_nbr_devices: {e:?}")))?;
        for i in 0..count {
            if let Ok(dev) = collection.get_device_at_index(i) {
                if let Ok(n) = dev.get_friendlyname() {
                    if n.eq_ignore_ascii_case(target) {
                        return Ok(dev);
                    }
                }
            }
        }
        log::warn!(
            "WASAPI: device '{}' not found for exclusive mode, using default",
            target
        );
    }
    get_default_device(&Direction::Render)
        .map_err(|e| AudioIoError::Stream(format!("default render device: {e:?}")))
}
