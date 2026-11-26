//! Native convolution reverb — wraps `ConvolutionReverb` plus the
//! procedural IR library (LargeHall / SmallRoom / Plate / Spring).
//! Mirrors Fruity Convolver / Logic Space Designer at the basic-
//! features level: pick a space, set mix, pre-delay, tail tone.

use crate::format;
use crate::tail;
use hardwave_dsp::convolution::{ConvolutionReverb, PreparedIr};
use hardwave_dsp::ir_library::{synthesize_ir, IrPreset};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat, Tail,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_PRESET: u32 = 0;
const PARAM_PRE_DELAY: u32 = 1;
const PARAM_LOW_CPU: u32 = 2;
const PARAM_LOW_CUT: u32 = 3;
const PARAM_HIGH_CUT: u32 = 4;
const PARAM_WIDTH: u32 = 5;
const PARAM_MIX: u32 = 6;
const PARAM_COUNT: u32 = 7;

fn preset_from_norm(v: f32) -> IrPreset {
    let i = ((v.clamp(0.0, 1.0) * 4.0).floor() as i32).clamp(0, 3);
    match i {
        0 => IrPreset::LargeHall,
        1 => IrPreset::SmallRoom,
        2 => IrPreset::Plate,
        _ => IrPreset::Spring,
    }
}

fn preset_index(p: IrPreset) -> usize {
    match p {
        IrPreset::LargeHall => 0,
        IrPreset::SmallRoom => 1,
        IrPreset::Plate => 2,
        IrPreset::Spring => 3,
    }
}

fn preset_to_norm(p: IrPreset) -> f64 {
    (preset_index(p) as f64 + 0.5) / 4.0
}

/// Space names by `preset_index`, which is also the order
/// `preset_from_norm` picks them in, so choice i sits at i / 3.
const PRESET_NAMES: [&str; 4] = ["Hall", "Room", "Plate", "Spring"];

/// Pre-delay in milliseconds (0..200) from the 0..1 knob. The setter and
/// the text both use these so what is shown is what is set.
fn pre_delay_ms_from_norm(v: f64) -> f32 {
    (v * 200.0) as f32
}

/// How much of the impulse tail is kept (10..100 %): the knob's floor is
/// the shortest tail `set_low_cpu_mode` accepts.
fn low_cpu_from_norm(v: f64) -> f32 {
    v.clamp(0.1, 1.0) as f32
}

/// Tail low cut in hertz, 20 Hz..1 kHz on a log scale.
fn low_cut_hz_from_norm(v: f64) -> f32 {
    let lo = 20.0_f32.log10();
    let hi = 1_000.0_f32.log10();
    10.0_f32.powf(lo + (hi - lo) * v as f32)
}

/// Tail high cut in hertz, 1 kHz..20 kHz on a log scale.
fn high_cut_hz_from_norm(v: f64) -> f32 {
    let lo = 1_000.0_f32.log10();
    let hi = 20_000.0_f32.log10();
    10.0_f32.powf(lo + (hi - lo) * v as f32)
}

/// Stereo width (0..2, 1 is the impulse as recorded) from the 0..1 knob.
fn width_from_norm(v: f64) -> f32 {
    (v * 2.0) as f32
}

pub struct NativeConvReverb {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    rev: ConvolutionReverb,
    /// Every space's impulse, worked out when the reverb starts or its
    /// rate changes, so picking another space while playing is a copy.
    /// It used to synthesize and transform the impulse on the audio
    /// thread: allocations and hundreds of FFTs in one block.
    bank: Vec<PreparedIr>,
    preset: IrPreset,
    pre_delay_ms: f32,
    low_cpu: f32,
    low_cut_hz: f32,
    high_cut_hz: f32,
    width: f32,
    mix: f32,
    active: bool,
}

impl NativeConvReverb {
    pub const ID: &'static str = "hardwave.native.conv_reverb";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Conv Reverb".into(),
            vendor: "Hardwave".into(),
            version: "1.0.0".into(),
            format: PluginFormat::Clap,
            path: PathBuf::from("<native>"),
            category: PluginCategory::Effect,
            num_inputs: 2,
            num_outputs: 2,
            has_midi_input: false,
            has_editor: false,
        }
    }

    pub fn new() -> Self {
        let sr = 48_000.0_f32;
        let mut rev = ConvolutionReverb::new(sr);
        let bank = prepare_bank(&mut rev, sr);
        rev.load_prepared(&bank[preset_index(IrPreset::Plate)]);
        rev.set_mix(0.3);
        rev.set_pre_delay_ms(15.0);
        rev.set_stereo_width(1.0);
        rev.set_tail_eq(120.0, 12_000.0);
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            rev,
            bank,
            preset: IrPreset::Plate,
            pre_delay_ms: 15.0,
            low_cpu: 1.0,
            low_cut_hz: 120.0,
            high_cut_hz: 12_000.0,
            width: 1.0,
            mix: 0.3,
            active: false,
        }
    }

    fn rebuild_ir(&mut self) {
        self.rev
            .load_prepared(&self.bank[preset_index(self.preset)]);
        self.rev.set_low_cpu_mode(self.low_cpu);
    }

    fn refresh(&mut self) {
        self.rev.set_mix(self.mix);
        self.rev.set_pre_delay_ms(self.pre_delay_ms);
        self.rev.set_stereo_width(self.width);
        self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
    }
}

/// Every space's impulse at `sample_rate`, in `preset_index` order, each
/// scaled by `level_matched`.
fn prepare_bank(rev: &mut ConvolutionReverb, sample_rate: f32) -> Vec<PreparedIr> {
    [
        IrPreset::LargeHall,
        IrPreset::SmallRoom,
        IrPreset::Plate,
        IrPreset::Spring,
    ]
    .into_iter()
    .map(|p| {
        let (mut l, mut r) = synthesize_ir(p, sample_rate);
        level_matched(&mut l, &mut r);
        rev.prepare_ir(&l, &r)
    })
    .collect()
}

/// Scale an impulse so its energy per channel is 1: steady noise then
/// comes out of the reverb as loud as it went in, whatever the space.
/// The library scales its impulses to a peak of almost 1, which with
/// seconds of tail behind it is thousands of times the energy: at the
/// default 30 % a snare came out 21 dB louder than it went in.
fn level_matched(left: &mut [f32], right: &mut [f32]) {
    let energy: f64 = left
        .iter()
        .chain(right.iter())
        .map(|&v| v as f64 * v as f64)
        .sum::<f64>()
        / 2.0;
    if energy > 1e-12 {
        let gain = (1.0 / energy.sqrt()) as f32;
        for v in left.iter_mut().chain(right.iter_mut()) {
            *v *= gain;
        }
    }
}

impl Default for NativeConvReverb {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeConvReverb {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.rev = ConvolutionReverb::new(self.sample_rate);
        self.bank = prepare_bank(&mut self.rev, self.sample_rate);
        self.rebuild_ir();
        self.refresh();
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
        _midi_in: &[MidiEvent],
        _midi_out: &mut Vec<MidiEvent>,
        num_samples: usize,
    ) {
        for out in outputs.iter_mut() {
            out.clear();
            out.resize(num_samples, 0.0);
        }
        if !self.active || inputs.len() < 2 || outputs.len() < 2 {
            for ch in 0..outputs.len().min(inputs.len()) {
                let n = inputs[ch].len().min(num_samples);
                outputs[ch][..n].copy_from_slice(&inputs[ch][..n]);
            }
            return;
        }
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            let (l, r) = self.rev.process(in_l, in_r);
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_PRESET => ("Preset", preset_to_norm(IrPreset::Plate), ""),
            PARAM_PRE_DELAY => ("Pre-Delay", (15.0_f64 / 200.0).clamp(0.0, 1.0), "ms"),
            // How much of the impulse is kept (10..100 %): shorter costs
            // less CPU. Called "Low-CPU" before, which read like a switch.
            PARAM_LOW_CPU => ("Tail Length", 1.0, "%"),
            PARAM_LOW_CUT => (
                "Low Cut",
                ((120.0_f64.log10() - 20.0_f64.log10()) / (1_000.0_f64.log10() - 20.0_f64.log10()))
                    .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_HIGH_CUT => (
                "High Cut",
                ((12_000.0_f64.log10() - 1_000.0_f64.log10())
                    / (20_000.0_f64.log10() - 1_000.0_f64.log10()))
                .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_WIDTH => ("Width", 0.5, ""),
            PARAM_MIX => ("Mix", 0.3, "%"),
            _ => return None,
        };
        Some(ParameterInfo {
            id: index,
            name: name.into(),
            default_value: default,
            min: 0.0,
            max: 1.0,
            unit: unit.into(),
            automatable: true,
        })
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_PRESET => preset_to_norm(self.preset),
            PARAM_PRE_DELAY => (self.pre_delay_ms / 200.0).clamp(0.0, 1.0) as f64,
            PARAM_LOW_CPU => self.low_cpu.clamp(0.0, 1.0) as f64,
            PARAM_LOW_CUT => {
                let lo = 20.0_f32.log10();
                let hi = 1_000.0_f32.log10();
                ((self.low_cut_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_HIGH_CUT => {
                let lo = 1_000.0_f32.log10();
                let hi = 20_000.0_f32.log10();
                ((self.high_cut_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            PARAM_WIDTH => (self.width / 2.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_PRESET => {
                self.preset = preset_from_norm(v as f32);
                self.rebuild_ir();
                self.refresh();
            }
            PARAM_PRE_DELAY => {
                self.pre_delay_ms = pre_delay_ms_from_norm(v);
                self.rev.set_pre_delay_ms(self.pre_delay_ms);
            }
            PARAM_LOW_CPU => {
                self.low_cpu = low_cpu_from_norm(v);
                self.rev.set_low_cpu_mode(self.low_cpu);
            }
            PARAM_LOW_CUT => {
                self.low_cut_hz = low_cut_hz_from_norm(v);
                self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
            }
            PARAM_HIGH_CUT => {
                self.high_cut_hz = high_cut_hz_from_norm(v);
                self.rev.set_tail_eq(self.low_cut_hz, self.high_cut_hz);
            }
            PARAM_WIDTH => {
                self.width = width_from_norm(v);
                self.rev.set_stereo_width(self.width);
            }
            PARAM_MIX => {
                self.mix = v as f32;
                self.rev.set_mix(self.mix);
            }
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        // Same clamp as the setter, so the text names what the value sets.
        let v = value.clamp(0.0, 1.0);
        let text = match id {
            PARAM_PRESET => PRESET_NAMES[preset_index(preset_from_norm(v as f32))].to_string(),
            PARAM_PRE_DELAY => format::ms(pre_delay_ms_from_norm(v) as f64),
            PARAM_LOW_CPU => format::pct(low_cpu_from_norm(v) as f64),
            PARAM_LOW_CUT => format::hz(low_cut_hz_from_norm(v) as f64),
            PARAM_HIGH_CUT => format::hz(high_cut_hz_from_norm(v) as f64),
            PARAM_WIDTH => format::pct(width_from_norm(v) as f64),
            PARAM_MIX => format::pct(v),
            _ => return None,
        };
        Some(text)
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_PRESET => Some(PRESET_NAMES.iter().map(|s| s.to_string()).collect()),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"preset\":{},\"pre\":{},\"cpu\":{},\"lc\":{},\"hc\":{},\"w\":{},\"mix\":{}}}",
            preset_to_norm(self.preset),
            self.pre_delay_ms,
            self.low_cpu,
            self.low_cut_hz,
            self.high_cut_hz,
            self.width,
            self.mix
        )
        .into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find([',', '}']).unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("preset") {
            self.preset = preset_from_norm(v);
        }
        if let Some(v) = read("pre") {
            self.pre_delay_ms = v.clamp(0.0, 200.0);
        }
        if let Some(v) = read("cpu") {
            self.low_cpu = v.clamp(0.1, 1.0);
        }
        if let Some(v) = read("lc") {
            self.low_cut_hz = v.clamp(20.0, 1_000.0);
        }
        if let Some(v) = read("hc") {
            self.high_cut_hz = v.clamp(1_000.0, 20_000.0);
        }
        if let Some(v) = read("w") {
            self.width = v.clamp(0.0, 2.0);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        self.rebuild_ir();
        self.refresh();
        Ok(())
    }

    /// The pre-delay, the impulse as far as it is kept, a block of the
    /// partitioned engine, and the tail filters ringing out.
    fn tail(&self) -> Tail {
        let sr = self.sample_rate as f64;
        let ir =
            hardwave_dsp::convolution::MAX_IR_SAMPLES as f64 * self.low_cpu.clamp(0.1, 1.0) as f64;
        let secs = self.pre_delay_ms.clamp(0.0, 1_000.0) as f64 / 1000.0
            + (ir + 512.0) / sr
            + tail::filter_ring_secs(self.low_cut_hz as f64, 0.707, 1);
        tail::from_secs(secs, self.sample_rate)
    }

    fn latency_samples(&self) -> u32 {
        0
    }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool {
        false
    }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_as_text_and_choices_match_the_setter() {
        let mut p = NativeConvReverb::new();
        let shown = |p: &NativeConvReverb, id: u32| p.parameter_text(id, p.get_parameter_value(id));
        assert_eq!(shown(&p, PARAM_PRESET).as_deref(), Some("Plate"));
        assert_eq!(shown(&p, PARAM_PRE_DELAY).as_deref(), Some("15.0 ms"));
        assert_eq!(shown(&p, PARAM_LOW_CUT).as_deref(), Some("120 Hz"));
        assert_eq!(shown(&p, PARAM_HIGH_CUT).as_deref(), Some("12.00 kHz"));
        assert_eq!(shown(&p, PARAM_WIDTH).as_deref(), Some("100 %"));
        assert_eq!(
            p.parameter_text(PARAM_LOW_CPU, 0.0).as_deref(),
            Some("10 %")
        );

        let options = p.parameter_options(PARAM_PRESET).unwrap();
        let last = (options.len() - 1) as f64;
        for (i, label) in options.iter().enumerate() {
            let v = i as f64 / last;
            assert_eq!(p.parameter_text(PARAM_PRESET, v).as_ref(), Some(label));
            p.set_parameter_value(PARAM_PRESET, v);
            assert_eq!(shown(&p, PARAM_PRESET).as_ref(), Some(label));
        }
        for id in 0..PARAM_COUNT {
            assert!(
                p.parameter_text(id, 0.5).is_some(),
                "parameter {id} has no text"
            );
        }
    }
}

#[cfg(test)]
mod level_tests {
    use super::*;

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&v| v as f64 * v as f64).sum()
    }

    /// Half a second of steady noise, then three seconds of silence.
    fn noise_burst() -> Vec<f32> {
        let mut seed: u32 = 0x2545_f491;
        let mut x: Vec<f32> = (0..24_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
            })
            .collect();
        x.resize(24_000 + 3 * 48_000, 0.0);
        x
    }

    fn run(p: &mut NativeConvReverb, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        for block in input.chunks(512) {
            p.process(
                &[block, block],
                &mut outputs,
                &[],
                &mut midi_out,
                block.len(),
            );
            out.extend_from_slice(&outputs[0]);
        }
        out
    }

    /// At its default settings the reverb is about as loud as the dry
    /// signal, and so is every space at 100 % wet. The impulses were
    /// scaled to a peak of almost 1, which with seconds of tail behind
    /// it is a huge amount of energy: a snare went from -4.5 to +17 dBFS.
    #[test]
    fn every_space_is_about_as_loud_as_the_dry_signal() {
        let dry = noise_burst();
        let e_dry = energy(&dry);

        let mut p = NativeConvReverb::new();
        p.activate(48_000.0, 512).unwrap();
        let out = run(&mut p, &dry);
        let db = 10.0 * (energy(&out) / e_dry).log10();
        assert!(
            db.abs() < 3.0,
            "default settings: {db:.2} dB from the dry signal"
        );

        // A snare-like hit: noise dying away over 50 ms. Its peak stays
        // the dry hit's peak, give or take 3 dB.
        let hit: Vec<f32> = dry
            .iter()
            .enumerate()
            .map(|(n, &v)| v * (-(n as f32) / 2_400.0).exp())
            .collect();
        let peak_dry = hit.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let mut p = NativeConvReverb::new();
        p.activate(48_000.0, 512).unwrap();
        let out = run(&mut p, &hit);
        let peak = out.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
        let peak_db = 20.0 * (peak / peak_dry).log10();
        assert!(
            peak_db.abs() < 3.0,
            "a hit at default settings peaks {peak_db:.2} dB from the dry hit"
        );

        for (i, name) in PRESET_NAMES.iter().enumerate() {
            let mut p = NativeConvReverb::new();
            p.activate(48_000.0, 512).unwrap();
            p.set_parameter_value(PARAM_PRESET, i as f64 / 3.0);
            p.set_parameter_value(PARAM_MIX, 1.0);
            let wet = run(&mut p, &dry);
            let db = 10.0 * (energy(&wet) / e_dry).log10();
            assert!(
                db.abs() < 4.0,
                "{name} at 100 %: {db:.2} dB from the dry signal"
            );
        }
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    /// Switching space while playing is a copy of what was worked out at
    /// the start: the same sound as loading that space from scratch.
    #[test]
    fn a_space_picked_while_playing_sounds_as_loaded_fresh() {
        let mut switched = NativeConvReverb::new();
        switched.set_parameter_value(PARAM_PRESET, preset_to_norm(IrPreset::SmallRoom));
        let mut fresh = NativeConvReverb::new();
        let (mut l, mut r) = synthesize_ir(IrPreset::SmallRoom, 48_000.0);
        level_matched(&mut l, &mut r);
        fresh.rev.load_ir_stereo(&l, &r);
        fresh.rev.set_low_cpu_mode(fresh.low_cpu);
        let (mut a, mut b) = (0.0_f32, 0.0_f32);
        for n in 0..4_000 {
            let x = if n == 0 { 1.0 } else { 0.0 };
            let (l1, _) = switched.rev.process(x, x);
            let (l2, _) = fresh.rev.process(x, x);
            a += l1.abs();
            b += (l1 - l2).abs();
        }
        assert!(a > 0.01, "the room rings");
        assert!(b < 1e-4, "same as loaded fresh: {b}");
    }
}
