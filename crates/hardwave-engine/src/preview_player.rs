//! Auditioning a sample through the engine rather than through the WebView.
//!
//! Clicking a file in the browser used to create an HTML `Audio` element and
//! play it inside the webview. That plays through whatever output the browser
//! considers default, which is not the device the DAW is using: on ASIO or
//! WASAPI-exclusive, where the device is held by the engine, the preview is
//! either silent or comes out of the wrong speakers. It also ignored the
//! project's sample rate and the preview volume the engine already knows.
//!
//! Previewing here means the audition comes out of the same device, at the
//! same rate, at a level the DAW controls.

use crate::audio_pool::{AudioBuffer, AudioPool};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// What the UI thread asks for, read by the audio thread each block.
///
/// Deliberately not a mutex around the buffer: the audio thread must never
/// wait on the UI thread, so the request is a few atomics plus a pool id the
/// audio thread resolves itself.
#[derive(Clone)]
pub struct PreviewRequest {
    /// Set when a new preview should start. The audio thread takes it.
    start: Arc<AtomicBool>,
    /// Set to stop whatever is playing.
    stop: Arc<AtomicBool>,
    /// Linear gain in f32 bits.
    volume: Arc<AtomicU32>,
    /// Pool id of the source to audition. Guarded by a lock only the UI
    /// thread takes for writing; the audio thread clones the string once when
    /// a start is pending, not per block.
    source_id: Arc<parking_lot::Mutex<String>>,
}

impl PreviewRequest {
    pub fn new() -> Self {
        Self {
            start: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(0.7f32.to_bits())),
            source_id: Arc::new(parking_lot::Mutex::new(String::new())),
        }
    }

    /// Ask for a sample to start playing.
    /// A second call replaces whatever is playing: the audio thread builds a
    /// new voice from this request, so two clicks never layer.
    pub fn play(&self, source_id: &str) {
        *self.source_id.lock() = source_id.to_string();
        self.stop.store(false, Ordering::Relaxed);
        self.start.store(true, Ordering::Release);
    }

    pub fn stop_playing(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn set_volume(&self, linear: f32) {
        self.volume
            .store(linear.clamp(0.0, 4.0).to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume.load(Ordering::Relaxed)).clamp(0.0, 4.0)
    }
}

impl Default for PreviewRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// Plays one sample at a time, on the audio thread.
pub struct PreviewPlayer {
    request: PreviewRequest,
    playing: Option<Playing>,
}

struct Playing {
    buffer: Arc<AudioBuffer>,
    /// Position in source frames, as a float because the source rate and the
    /// device rate need not match.
    position: f64,
    step: f64,
}

impl PreviewPlayer {
    pub fn new(request: PreviewRequest) -> Self {
        Self {
            request,
            playing: None,
        }
    }

    /// True while a sample is being auditioned, for the UI's stop button.
    pub fn is_playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Mix the preview into an interleaved stereo block.
    ///
    /// Called with the device's rate so a 44.1 kHz sample auditioned on a
    /// 48 kHz device plays at the right pitch instead of slightly sharp.
    pub fn render(
        &mut self,
        output: &mut [f32],
        num_frames: usize,
        pool: &AudioPool,
        device_rate: f64,
    ) {
        if self.request.stop.swap(false, Ordering::Acquire) {
            self.playing = None;
        }
        if self.request.start.swap(false, Ordering::Acquire) {
            let id = self.request.source_id.lock().clone();
            self.playing = pool.get(&id).map(|buffer| {
                let source_rate = if buffer.sample_rate > 0 {
                    buffer.sample_rate as f64
                } else {
                    device_rate
                };
                Playing {
                    buffer,
                    position: 0.0,
                    step: if device_rate > 0.0 {
                        source_rate / device_rate
                    } else {
                        1.0
                    },
                }
            });
        }

        let Some(state) = self.playing.as_mut() else {
            return;
        };
        let volume = self.request.volume();
        let frames = state.buffer.num_frames;
        if frames == 0 || state.buffer.channels.is_empty() {
            self.playing = None;
            return;
        }

        for frame in 0..num_frames {
            let idx = state.position as usize;
            if idx >= frames {
                // Ran off the end: stop rather than looping, because an
                // audition that loops forever is a stuck sound the user has
                // to hunt for.
                self.playing = None;
                return;
            }
            let left = state.buffer.channels[0][idx];
            let right = state
                .buffer
                .channels
                .get(1)
                .map(|c| c[idx])
                // Mono sources are auditioned in the middle, not just on the
                // left, which is what taking channel 0 alone would do.
                .unwrap_or(left);

            let i = frame * 2;
            if i + 1 >= output.len() {
                break;
            }
            output[i] += left * volume;
            output[i + 1] += right * volume;
            state.position += state.step;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_with(id: &str, frames: usize, rate: u32, channels: usize) -> AudioPool {
        let pool = AudioPool::new();
        let data: Vec<Vec<f32>> = (0..channels).map(|_| vec![0.5f32; frames]).collect();
        pool.insert(
            id.to_string(),
            AudioBuffer {
                channels: data,
                sample_rate: rate,
                num_frames: frames,
            },
        );
        pool
    }

    fn peak(out: &[f32]) -> f32 {
        out.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn a_requested_sample_is_audible_on_the_engines_own_output() {
        let pool = pool_with("s", 4_800, 48_000, 2);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());

        let mut out = vec![0.0f32; 512 * 2];
        player.render(&mut out, 512, &pool, 48_000.0);
        assert_eq!(peak(&out), 0.0, "nothing plays until it is asked for");

        req.play("s");
        let mut out = vec![0.0f32; 512 * 2];
        player.render(&mut out, 512, &pool, 48_000.0);
        assert!(peak(&out) > 0.0, "the audition must reach the output");
        assert!(player.is_playing());
    }

    #[test]
    fn stopping_is_immediate() {
        let pool = pool_with("s", 4_800, 48_000, 2);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.play("s");
        let mut out = vec![0.0f32; 128 * 2];
        player.render(&mut out, 128, &pool, 48_000.0);
        assert!(player.is_playing());

        req.stop_playing();
        let mut out = vec![0.0f32; 128 * 2];
        player.render(&mut out, 128, &pool, 48_000.0);
        assert_eq!(peak(&out), 0.0, "stop must silence it in the next block");
        assert!(!player.is_playing());
    }

    #[test]
    fn a_second_click_replaces_the_first_instead_of_layering() {
        let pool = pool_with("a", 4_800, 48_000, 1);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());

        req.play("a");
        let mut first = vec![0.0f32; 64 * 2];
        player.render(&mut first, 64, &pool, 48_000.0);

        // Same sample again: one voice, so the level must not double.
        req.play("a");
        let mut second = vec![0.0f32; 64 * 2];
        player.render(&mut second, 64, &pool, 48_000.0);
        assert!(
            (peak(&second) - peak(&first)).abs() < 1e-6,
            "two clicks stacked: {} then {}",
            peak(&first),
            peak(&second)
        );
    }

    #[test]
    fn it_stops_at_the_end_rather_than_looping() {
        // Shorter than one block, so the end is reached inside this render.
        let pool = pool_with("s", 100, 48_000, 1);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.play("s");

        let mut out = vec![0.0f32; 512 * 2];
        player.render(&mut out, 512, &pool, 48_000.0);

        assert!(!player.is_playing(), "a finished audition must stop itself");
        // Audio at the start, silence after the source ran out.
        assert!(out[0].abs() > 0.0);
        assert_eq!(out[400], 0.0, "nothing after the end of the sample");
    }

    #[test]
    fn a_sample_at_another_rate_plays_at_the_right_pitch() {
        // A 44.1 kHz sample on a 48 kHz device must be read slower than one
        // frame per frame, or it sounds sharp.
        let pool = pool_with("s", 48_000, 44_100, 1);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.play("s");
        let mut out = vec![0.0f32; 480 * 2];
        player.render(&mut out, 480, &pool, 48_000.0);

        // 480 device frames at 44100/48000 should consume about 441 source
        // frames.
        let consumed = player.playing.as_ref().unwrap().position;
        assert!(
            (consumed - 441.0).abs() < 1.0,
            "read {consumed} source frames for 480 device frames"
        );
    }

    #[test]
    fn a_mono_sample_is_auditioned_in_the_middle() {
        let pool = pool_with("s", 4_800, 48_000, 1);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.play("s");
        let mut out = vec![0.0f32; 64 * 2];
        player.render(&mut out, 64, &pool, 48_000.0);

        assert!(out[0].abs() > 0.0);
        assert_eq!(out[0], out[1], "mono must come out of both speakers");
    }

    #[test]
    fn asking_for_a_sample_that_is_not_loaded_does_nothing_bad() {
        let pool = AudioPool::new();
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.play("missing");
        let mut out = vec![0.0f32; 64 * 2];
        player.render(&mut out, 64, &pool, 48_000.0);
        assert_eq!(peak(&out), 0.0);
        assert!(!player.is_playing());
    }

    #[test]
    fn the_volume_the_daw_shows_is_the_volume_it_plays_at() {
        let pool = pool_with("s", 4_800, 48_000, 1);
        let req = PreviewRequest::new();
        let mut player = PreviewPlayer::new(req.clone());
        req.set_volume(0.25);
        req.play("s");
        let mut out = vec![0.0f32; 64 * 2];
        player.render(&mut out, 64, &pool, 48_000.0);
        // Source is 0.5 everywhere, so a quarter gain is 0.125.
        assert!((peak(&out) - 0.125).abs() < 1e-6, "got {}", peak(&out));
    }
}
