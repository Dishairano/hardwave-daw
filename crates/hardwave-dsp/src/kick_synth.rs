//! KickSynth — Hardwave's native hardstyle / harder-styles kick.
//!
//! Implements the four-layer architecture documented in the KickForge
//! product vision: **Transient · Punch · Bass · Tail**. Each layer
//! has its own oscillator + envelope and renders into the same
//! mono-summed output buffer, then we duplicate that to L/R for
//! stereo.
//!
//! Phase 1 (this commit): each layer is a single sine oscillator
//! with a frequency-sweep envelope. The transient is a high → mid
//! sweep (~3kHz → 150Hz over 5ms). Punch is the fundamental thump
//! (~150Hz → 60Hz over 30ms). Bass holds at 50Hz for the body decay.
//! Tail is a slow sub-bass at 35Hz.
//!
//! Phase 2 will add: layered Punch (two oscillators), saturation per
//! layer, FM modulation, FX chain (EQ + comp + drive + filter).
//! That matches the Serum 2-style FX chain in the vision memo.

use std::f32::consts::TAU;

const SAMPLE_RATE_DEFAULT: f32 = 48_000.0;

/// Per-layer ADSR-ish envelope. We model it as start_value → end_value
/// over `length_secs`, with an optional tail decay after that. Phase
/// stays linear; the harshness curve comes from each layer's own
/// frequency sweep, not the amplitude shape.
#[derive(Debug, Clone, Copy)]
pub struct LayerEnvelope {
    /// Total length of the audible portion, in seconds. Once we cross
    /// this we hit the linear release.
    pub length_secs: f32,
    /// How long the linear release tail runs after `length_secs`. The
    /// envelope reaches 0 at `length_secs + release_secs`.
    pub release_secs: f32,
    /// Peak gain — the layer's amplitude at the start of the envelope.
    pub peak_gain: f32,
}

impl LayerEnvelope {
    /// Sample the envelope at `t` seconds since the note-on.
    pub fn at(&self, t: f32) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        if t < self.length_secs {
            // Hold-style — full peak gain across the audible window.
            // The character comes from the frequency sweep below.
            return self.peak_gain;
        }
        let r_t = t - self.length_secs;
        if r_t < self.release_secs {
            let k = 1.0 - (r_t / self.release_secs);
            return self.peak_gain * k.max(0.0);
        }
        0.0
    }
}

/// Linear pitch sweep: starts at `start_hz`, ends at `end_hz`, over
/// `sweep_secs`. After the sweep we hold at `end_hz`.
#[derive(Debug, Clone, Copy)]
pub struct FrequencySweep {
    pub start_hz: f32,
    pub end_hz: f32,
    pub sweep_secs: f32,
}

impl FrequencySweep {
    pub fn at(&self, t: f32) -> f32 {
        if t <= 0.0 {
            return self.start_hz;
        }
        if t >= self.sweep_secs || self.sweep_secs <= 0.0 {
            return self.end_hz;
        }
        let k = t / self.sweep_secs;
        self.start_hz + (self.end_hz - self.start_hz) * k
    }
}

/// Per-layer oscillator waveform. Sine is the cleanest, saw / square
/// add hard upper harmonics for raw / uptempo character, triangle is
/// in between. Phase 1 of layer-waveform support: each layer picks
/// independently. Phase 2 will add per-layer drive / saturation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LayerWaveform {
    #[default]
    Sine,
    Saw,
    Square,
    Triangle,
}

impl LayerWaveform {
    /// Sample the waveform at `phase` in radians (0..2π).
    #[inline]
    pub fn sample(self, phase: f32) -> f32 {
        match self {
            LayerWaveform::Sine => phase.sin(),
            LayerWaveform::Saw => {
                // 0..2π → -1..1, descending. Bandlimited not needed at
                // kick frequencies since the human ear quietly tolerates
                // aliasing here.
                let t = phase / std::f32::consts::TAU;
                2.0 * (t - (t + 0.5).floor())
            }
            LayerWaveform::Square => {
                if phase < std::f32::consts::PI {
                    1.0
                } else {
                    -1.0
                }
            }
            LayerWaveform::Triangle => {
                let t = phase / std::f32::consts::TAU;
                let a = (t - 0.5).abs() * 4.0 - 1.0;
                a.clamp(-1.0, 1.0)
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub envelope: LayerEnvelope,
    pub sweep: FrequencySweep,
    /// Oscillator waveform for this layer. Defaults to Sine for
    /// backward compatibility with the old preset definitions.
    pub waveform: LayerWaveform,
}

impl Layer {
    /// Helper constructor for the existing preset literal style. Lets
    /// us extend the struct without rewriting every literal.
    pub const fn sine(envelope: LayerEnvelope, sweep: FrequencySweep) -> Self {
        Self {
            envelope,
            sweep,
            waveform: LayerWaveform::Sine,
        }
    }
}

/// One in-flight KickSynth voice. Tracks the elapsed time since
/// note-on so each layer can sample its own envelope + sweep
/// independently. Mono internally; the host duplicates to stereo.
#[derive(Debug, Clone)]
pub struct KickVoice {
    pub age_samples: u64,
    pub velocity: f32,
    pub phase_per_layer: [f32; 4],
}

impl KickVoice {
    pub fn new(velocity: f32) -> Self {
        Self {
            age_samples: 0,
            velocity: velocity.clamp(0.0, 1.0),
            phase_per_layer: [0.0; 4],
        }
    }
}

/// The full KickSynth instrument. Holds the four hard-coded layer
/// definitions for now; future commits expose them as user-editable
/// params and add presets ("Frenchcore A", "Rawphoric Long", etc.).
pub struct KickSynth {
    pub sample_rate: f32,
    pub layers: [Layer; 4],
    pub voice: Option<KickVoice>,
    /// Post-mix drive (0..=1). 0 = clean tanh @ unity gain. Higher
    /// values pre-multiply the summed signal before tanh, so the
    /// soft-clipper bites harder and brings up upper harmonics —
    /// the classic raw / uptempo "distorted kick" sound.
    pub drive: f32,
}

impl Default for KickSynth {
    fn default() -> Self {
        Self::new(SAMPLE_RATE_DEFAULT)
    }
}

impl KickSynth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            layers: hardstyle_default_layers(),
            voice: None,
            drive: 0.0,
        }
    }

    /// Set the post-mix drive amount (0..=1). Clamped on entry.
    pub fn set_drive(&mut self, d: f32) {
        self.drive = d.clamp(0.0, 1.0);
    }

    /// Trigger a new voice. Replaces any in-flight voice (mono mode);
    /// hardstyle kicks rarely overlap so this matches the genre.
    pub fn note_on(&mut self, _pitch: u8, velocity: f32) {
        self.voice = Some(KickVoice::new(velocity));
    }

    /// Replace one layer's params wholesale. Cheap struct copy; safe
    /// to call from a graph rebuild on the UI thread before audio
    /// resumes processing this instance.
    pub fn set_layer(&mut self, idx: usize, layer: Layer) {
        if idx < self.layers.len() {
            self.layers[idx] = layer;
        }
    }

    /// Note-off currently a no-op — the envelope's natural release
    /// drives decay. Kicks aren't gated like sustained synths.
    pub fn note_off(&mut self) {}

    /// Render `out_l` and `out_r` ADDITIVELY. Caller is responsible
    /// for clearing the buffers when starting a fresh block.
    pub fn render_into(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        let Some(voice) = self.voice.as_mut() else {
            return;
        };
        let n = out_l.len().min(out_r.len());
        let sr = self.sample_rate.max(1.0);
        let inv_sr = 1.0 / sr;

        for i in 0..n {
            let t = (voice.age_samples as f32) * inv_sr;
            let mut sample = 0.0;
            for (li, layer) in self.layers.iter().enumerate() {
                let env = layer.envelope.at(t);
                if env <= 0.0 {
                    continue;
                }
                let freq = layer.sweep.at(t);
                let mut phase = voice.phase_per_layer[li];
                let s = layer.waveform.sample(phase) * env;
                phase += TAU * freq * inv_sr;
                if phase >= TAU {
                    phase -= TAU;
                }
                voice.phase_per_layer[li] = phase;
                sample += s;
            }
            sample *= voice.velocity;
            // Pre-clip drive: at drive=0 we're at 1.0×, at drive=1
            // we're at ~5× → tanh's nonlinearity dominates and the
            // kick gets aggressive harmonics. Math: 1 + 4·drive.
            let drive_gain = 1.0 + 4.0 * self.drive;
            let driven = sample * drive_gain;
            let clipped = soft_clip(driven);
            out_l[i] += clipped;
            out_r[i] += clipped;
            voice.age_samples += 1;

            // When all layers have ended, retire the voice so the
            // next note-on starts fresh.
            let total_secs = (voice.age_samples as f32) * inv_sr;
            if self
                .layers
                .iter()
                .all(|l| total_secs > l.envelope.length_secs + l.envelope.release_secs)
            {
                self.voice = None;
                return;
            }
        }
    }
}

/// Tanh-shaped soft clipper — same flavour as a saturation knob.
/// Bounds output to ±1 without the harsh fold of a hard clip.
#[inline]
fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// Named preset bank. Each preset returns a full 4-layer setup the
/// user can drop onto a KickSynth track in one click. The defaults
/// below cover the core sub-genres of "harder styles" that Hardwave
/// targets — adding more is a matter of appending to this list and
/// the matching `apply_preset` arm.
pub const PRESET_NAMES: &[&str] = &[
    "Hardstyle Default",
    "Frenchcore Punch",
    "Rawphoric Long",
    "Uptempo Tight",
];

/// Apply a named preset by overwriting all four layers. Unknown
/// names fall back to the hardstyle default — easier than returning
/// an error at this callsite.
pub fn preset_layers(name: &str) -> [Layer; 4] {
    match name {
        "Frenchcore Punch" => [
            // Tight bright transient
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.004,
                    release_secs: 0.010,
                    peak_gain: 0.40,
                },
                FrequencySweep {
                    start_hz: 4500.0,
                    end_hz: 800.0,
                    sweep_secs: 0.004,
                },
            ),
            // Punchy mid
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.020,
                    release_secs: 0.025,
                    peak_gain: 0.65,
                },
                FrequencySweep {
                    start_hz: 280.0,
                    end_hz: 80.0,
                    sweep_secs: 0.020,
                },
            ),
            // Short body — frenchcore is fast so kicks don't ring
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.090,
                    release_secs: 0.080,
                    peak_gain: 0.40,
                },
                FrequencySweep {
                    start_hz: 70.0,
                    end_hz: 55.0,
                    sweep_secs: 0.090,
                },
            ),
            // Minimal tail
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.180,
                    release_secs: 0.140,
                    peak_gain: 0.20,
                },
                FrequencySweep {
                    start_hz: 38.0,
                    end_hz: 35.0,
                    sweep_secs: 0.180,
                },
            ),
        ],
        "Rawphoric Long" => [
            // Dirty sweep
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.008,
                    release_secs: 0.025,
                    peak_gain: 0.35,
                },
                FrequencySweep {
                    start_hz: 2400.0,
                    end_hz: 400.0,
                    sweep_secs: 0.008,
                },
            ),
            // Heavy punch with wider sweep
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.040,
                    release_secs: 0.080,
                    peak_gain: 0.60,
                },
                FrequencySweep {
                    start_hz: 180.0,
                    end_hz: 55.0,
                    sweep_secs: 0.040,
                },
            ),
            // Long body — the rawphoric "ringing" feel
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.300,
                    release_secs: 0.300,
                    peak_gain: 0.50,
                },
                FrequencySweep {
                    start_hz: 55.0,
                    end_hz: 45.0,
                    sweep_secs: 0.300,
                },
            ),
            // Big sub tail
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.700,
                    release_secs: 0.500,
                    peak_gain: 0.40,
                },
                FrequencySweep {
                    start_hz: 32.0,
                    end_hz: 28.0,
                    sweep_secs: 0.700,
                },
            ),
        ],
        "Uptempo Tight" => [
            // Snappy click
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.003,
                    release_secs: 0.008,
                    peak_gain: 0.50,
                },
                FrequencySweep {
                    start_hz: 5000.0,
                    end_hz: 900.0,
                    sweep_secs: 0.003,
                },
            ),
            // Hard punch
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.018,
                    release_secs: 0.022,
                    peak_gain: 0.70,
                },
                FrequencySweep {
                    start_hz: 320.0,
                    end_hz: 70.0,
                    sweep_secs: 0.018,
                },
            ),
            // Short body for fast tempo (180+ BPM)
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.070,
                    release_secs: 0.060,
                    peak_gain: 0.45,
                },
                FrequencySweep {
                    start_hz: 75.0,
                    end_hz: 60.0,
                    sweep_secs: 0.070,
                },
            ),
            // Cut tail short
            Layer::sine(
                LayerEnvelope {
                    length_secs: 0.120,
                    release_secs: 0.080,
                    peak_gain: 0.25,
                },
                FrequencySweep {
                    start_hz: 40.0,
                    end_hz: 36.0,
                    sweep_secs: 0.120,
                },
            ),
        ],
        _ => hardstyle_default_layers(),
    }
}

/// Hard-coded "default Hardstyle" layer definitions. These are tuned
/// to land on the right side of "punchy but not honky" and are good
/// enough to be the first audible kick out of a fresh KickSynth.
fn hardstyle_default_layers() -> [Layer; 4] {
    [
        // Transient — the click. Sweeps high to mid, very short.
        Layer::sine(
            LayerEnvelope {
                length_secs: 0.005,
                release_secs: 0.015,
                peak_gain: 0.30,
            },
            FrequencySweep {
                start_hz: 3000.0,
                end_hz: 600.0,
                sweep_secs: 0.005,
            },
        ),
        // Punch 1 — fundamental thump.
        Layer::sine(
            LayerEnvelope {
                length_secs: 0.025,
                release_secs: 0.040,
                peak_gain: 0.55,
            },
            FrequencySweep {
                start_hz: 220.0,
                end_hz: 65.0,
                sweep_secs: 0.025,
            },
        ),
        // Bass — sustained body of the kick.
        Layer::sine(
            LayerEnvelope {
                length_secs: 0.180,
                release_secs: 0.180,
                peak_gain: 0.45,
            },
            FrequencySweep {
                start_hz: 60.0,
                end_hz: 50.0,
                sweep_secs: 0.180,
            },
        ),
        // Tail — the long sub-bass rumble.
        Layer::sine(
            LayerEnvelope {
                length_secs: 0.450,
                release_secs: 0.350,
                peak_gain: 0.30,
            },
            FrequencySweep {
                start_hz: 35.0,
                end_hz: 32.0,
                sweep_secs: 0.450,
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_finishes_within_total_envelope() {
        let mut k = KickSynth::new(48_000.0);
        k.note_on(60, 1.0);
        // 1 second of audio is more than enough — total tail is ~800ms.
        let mut l = vec![0.0_f32; 48_000];
        let mut r = vec![0.0_f32; 48_000];
        k.render_into(&mut l, &mut r);
        assert!(k.voice.is_none(), "voice should retire by 1 second");
    }

    #[test]
    fn first_few_samples_are_loud() {
        let mut k = KickSynth::new(48_000.0);
        k.note_on(60, 1.0);
        let mut l = vec![0.0_f32; 64];
        let mut r = vec![0.0_f32; 64];
        k.render_into(&mut l, &mut r);
        let peak = l.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            peak > 0.10,
            "transient should be audible at the very start, got {peak}"
        );
    }
}
