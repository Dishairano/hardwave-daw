//! Native reverb plug-in — wraps `hardwave_dsp::reverb::AlgorithmicReverb`
//! in HostedPlugin. Mirrors Fruity Reverb 2's main controls.

use crate::format;
use crate::tail;
use hardwave_dsp::reverb::AlgorithmicReverb;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat, Tail,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_ROOM_SIZE: u32 = 0;
const PARAM_DECAY: u32 = 1;
const PARAM_DAMPING: u32 = 2;
const PARAM_PRE_DELAY: u32 = 3;
const PARAM_MIX: u32 = 4;
const PARAM_COUNT: u32 = 5;

pub struct NativeReverb {
    descriptor: PluginDescriptor,
    rev: AlgorithmicReverb,
    sample_rate: f32,
    room_size: f32,
    decay_secs: f32,
    damping: f32,
    pre_delay_ms: f32,
    mix: f32,
    active: bool,
}

impl NativeReverb {
    pub const ID: &'static str = "hardwave.native.reverb";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Reverb".into(),
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
        Self {
            descriptor: Self::descriptor(),
            rev: AlgorithmicReverb::new(48_000.0),
            sample_rate: 48_000.0,
            room_size: 0.6,
            decay_secs: 2.0,
            damping: 0.5,
            pre_delay_ms: 20.0,
            mix: 0.3,
            active: false,
        }
    }

    /// Decay a 0..1 value sets: 0.1 s to 10 s, linear. Shared by the
    /// setter and the label.
    fn decay_secs_from_normalised(v: f64) -> f32 {
        (0.1 + v.clamp(0.0, 1.0) * 9.9) as f32
    }

    /// Pre-delay a 0..1 value sets: 0 to 200 ms, linear.
    fn pre_delay_ms_from_normalised(v: f64) -> f32 {
        (v.clamp(0.0, 1.0) * 200.0) as f32
    }

    fn refresh(&mut self) {
        self.rev.set_room_size(self.room_size);
        self.rev.set_decay_time(self.decay_secs);
        self.rev.set_damping(self.damping);
        self.rev.set_pre_delay_ms(self.pre_delay_ms);
        self.rev.set_mix(self.mix);
    }
}

impl Default for NativeReverb {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeReverb {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.rev = AlgorithmicReverb::new(self.sample_rate);
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
        match index {
            PARAM_ROOM_SIZE => Some(ParameterInfo {
                id: PARAM_ROOM_SIZE,
                name: "Size".into(),
                default_value: 0.6,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: true,
            }),
            PARAM_DECAY => Some(ParameterInfo {
                id: PARAM_DECAY,
                name: "Decay".into(),
                default_value: ((2.0_f64 - 0.1) / 9.9).clamp(0.0, 1.0),
                min: 0.0,
                max: 1.0,
                unit: "s".into(),
                automatable: true,
            }),
            PARAM_DAMPING => Some(ParameterInfo {
                id: PARAM_DAMPING,
                name: "Damping".into(),
                default_value: 0.5,
                min: 0.0,
                max: 1.0,
                unit: "".into(),
                automatable: true,
            }),
            PARAM_PRE_DELAY => Some(ParameterInfo {
                id: PARAM_PRE_DELAY,
                name: "Pre-Delay".into(),
                default_value: (20.0_f64 / 200.0).clamp(0.0, 1.0),
                min: 0.0,
                max: 1.0,
                unit: "ms".into(),
                automatable: true,
            }),
            PARAM_MIX => Some(ParameterInfo {
                id: PARAM_MIX,
                name: "Mix".into(),
                default_value: 0.3,
                min: 0.0,
                max: 1.0,
                unit: "%".into(),
                automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_ROOM_SIZE => self.room_size as f64,
            PARAM_DECAY => ((self.decay_secs - 0.1) / 9.9).clamp(0.0, 1.0) as f64,
            PARAM_DAMPING => self.damping as f64,
            PARAM_PRE_DELAY => (self.pre_delay_ms / 200.0).clamp(0.0, 1.0) as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_ROOM_SIZE => self.room_size = v as f32,
            PARAM_DECAY => self.decay_secs = Self::decay_secs_from_normalised(v),
            PARAM_DAMPING => self.damping = v as f32,
            PARAM_PRE_DELAY => self.pre_delay_ms = Self::pre_delay_ms_from_normalised(v),
            PARAM_MIX => self.mix = v as f32,
            _ => {}
        }
        self.refresh();
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        let v = value.clamp(0.0, 1.0);
        Some(match id {
            PARAM_ROOM_SIZE | PARAM_DAMPING | PARAM_MIX => format::pct(v),
            PARAM_DECAY => format::secs(Self::decay_secs_from_normalised(v) as f64),
            PARAM_PRE_DELAY => format::ms(Self::pre_delay_ms_from_normalised(v) as f64),
            _ => return None,
        })
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"size\":{},\"decay\":{},\"damping\":{},\"pre\":{},\"mix\":{}}}",
            self.room_size, self.decay_secs, self.damping, self.pre_delay_ms, self.mix
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
        if let Some(v) = read("size") {
            self.room_size = v;
        }
        if let Some(v) = read("decay") {
            self.decay_secs = v;
        }
        if let Some(v) = read("damping") {
            self.damping = v;
        }
        if let Some(v) = read("pre") {
            self.pre_delay_ms = v;
        }
        if let Some(v) = read("mix") {
            self.mix = v;
        }
        self.refresh();
        Ok(())
    }

    /// The pre-delay, then the decay stretched from the 60 dB it is set by
    /// to well past the 150 dB the host listens down to (three times the
    /// decay is 180 dB, room for a resonance that built up louder than the
    /// input), plus the combs' and diffusers' own lengths.
    fn tail(&self) -> Tail {
        let secs = self.pre_delay_ms.clamp(0.0, 1_000.0) as f64 / 1000.0
            + self.decay_secs.clamp(0.05, 30.0) as f64 * 3.0
            + 0.3;
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
    fn parameter_text_reads_real_units() {
        let r = NativeReverb::new();
        let decay = r.get_parameter_info(PARAM_DECAY).unwrap().default_value;
        assert_eq!(
            r.parameter_text(PARAM_DECAY, decay).as_deref(),
            Some("2.00 s")
        );
        assert_eq!(
            r.parameter_text(PARAM_DECAY, 0.0).as_deref(),
            Some("100 ms")
        );
        let pre = r.get_parameter_info(PARAM_PRE_DELAY).unwrap().default_value;
        assert_eq!(
            r.parameter_text(PARAM_PRE_DELAY, pre).as_deref(),
            Some("20.0 ms")
        );
        assert_eq!(r.parameter_text(PARAM_MIX, 0.3).as_deref(), Some("30 %"));
        for id in 0..PARAM_COUNT {
            for v in [0.0, 0.5, 1.0] {
                assert!(r.parameter_text(id, v).is_some(), "no text for {id}");
            }
            assert!(r.parameter_options(id).is_none());
        }
    }

    /// Runs `input` (mono, on both sides) through the plug-in in blocks
    /// the way a track does, and returns the left output.
    fn run(r: &mut NativeReverb, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len());
        let mut outputs = vec![Vec::new(), Vec::new()];
        let mut midi_out = Vec::new();
        for block in input.chunks(512) {
            r.process(
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

    fn started() -> NativeReverb {
        let mut r = NativeReverb::new();
        r.activate(48_000.0, 512).unwrap();
        r
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&v| v as f64 * v as f64).sum()
    }

    /// Half a second of steady noise, then silence for `tail_secs`.
    fn noise_burst(tail_secs: f32) -> Vec<f32> {
        let mut seed: u32 = 0x2545_f491;
        let mut x: Vec<f32> = (0..24_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
            })
            .collect();
        x.resize(24_000 + (tail_secs * 48_000.0) as usize, 0.0);
        x
    }

    /// The time the tail takes to fall 60 dB, from the energy left in it
    /// (Schroeder's backward integration), timed from -5 to -35 dB.
    fn rt60(out: &[f32]) -> f32 {
        let mut left = vec![0.0_f64; out.len() + 1];
        for i in (0..out.len()).rev() {
            left[i] = left[i + 1] + out[i] as f64 * out[i] as f64;
        }
        let total = left[0];
        let when = |db: f64| {
            let level = total * 10f64.powf(db / 10.0);
            left.iter().position(|&e| e < level).unwrap_or(out.len()) as f32
        };
        2.0 * (when(-35.0) - when(-5.0)) / 48_000.0
    }

    /// An impulse rings out for the decay the knob is set to. The wet
    /// path went through a tail filter that was never set up, whose
    /// coefficients were all zero: 100 % mix gave silence and 30 % only
    /// turned the dry signal down.
    #[test]
    fn an_impulse_rings_out_for_the_decay_it_is_set_to() {
        for secs in [1.0_f64, 3.0] {
            let mut r = started();
            r.set_parameter_value(PARAM_MIX, 1.0);
            r.set_parameter_value(PARAM_DAMPING, 0.0);
            r.set_parameter_value(PARAM_PRE_DELAY, 0.0);
            r.set_parameter_value(PARAM_DECAY, (secs - 0.1) / 9.9);
            let mut impulse = vec![0.0_f32; (secs as usize + 2) * 48_000];
            impulse[0] = 1.0;
            let out = run(&mut r, &impulse);
            assert!(energy(&out) > 0.01, "the reverb rings: {}", energy(&out));
            let measured = rt60(&out) as f64;
            assert!(
                (measured - secs).abs() < secs * 0.2,
                "set to {secs} s, rang for {measured} s"
            );
        }
    }

    /// Mix 0 is the dry signal untouched, 100 % is the reverb alone, and
    /// anything between is a blend about as loud as the dry signal.
    #[test]
    fn mix_gives_dry_only_a_blend_or_wet_only_at_a_matched_level() {
        let dry = noise_burst(3.0);
        let e_dry = energy(&dry);

        let mut r = started();
        r.set_parameter_value(PARAM_MIX, 0.0);
        assert_eq!(run(&mut r, &dry), dry, "mix 0 is the dry signal");

        let mut r = started();
        r.set_parameter_value(PARAM_MIX, 1.0);
        let wet = run(&mut r, &dry);
        // The default 20 ms pre-delay: nothing comes out before it, so
        // none of the dry signal is in there.
        assert!(wet[..900].iter().all(|v| v.abs() < 1e-6), "no dry at 100 %");
        let wet_db = 10.0 * (energy(&wet) / e_dry).log10();
        assert!(
            wet_db.abs() < 3.0,
            "100 % is {wet_db:.2} dB from the dry signal"
        );
        let tail = &wet[24_000 + 4_800..24_000 + 9_600];
        assert!(
            energy(tail) > 1e-3,
            "the tail goes on after the sound stops"
        );

        let mut r = started();
        let blend = run(&mut r, &dry);
        let blend_db = 10.0 * (energy(&blend) / e_dry).log10();
        assert!(
            blend_db.abs() < 3.0,
            "30 % is {blend_db:.2} dB from the dry signal"
        );
        assert!(energy(&blend[24_000 + 4_800..]) > 1e-4, "30 % has a tail");
    }

    fn tail_samples(r: &NativeReverb) -> usize {
        match r.tail() {
            Tail::Samples(n) => n as usize,
            other => panic!("a reverb tail ends: {other:?}"),
        }
    }

    /// The tail it states grows with the decay and the pre-delay.
    #[test]
    fn the_stated_tail_follows_decay_and_pre_delay() {
        let r = started();
        let default = tail_samples(&r);
        // 20 ms pre-delay and 2 s decay to well past -150 dB.
        assert!(default > 5 * 48_000, "{default}");
        let mut r = started();
        r.set_parameter_value(PARAM_DECAY, 1.0);
        assert!(tail_samples(&r) > default);
        r.set_parameter_value(PARAM_DECAY, 0.0);
        assert!(tail_samples(&r) < default);
        let mut r = started();
        r.set_parameter_value(PARAM_PRE_DELAY, 1.0);
        assert!(tail_samples(&r) > default);
    }

    /// The stated tail is long enough: an impulse, then that much silence,
    /// and the next block is below -150 dBFS. The host stops calling the
    /// reverb after it, so a shorter one would cut the reverb off.
    #[test]
    fn the_stated_tail_is_long_enough() {
        for mix in [0.3, 1.0] {
            let mut r = started();
            r.set_parameter_value(PARAM_MIX, mix);
            let n = tail_samples(&r);
            let mut input = vec![0.0_f32; 1 + n + 512];
            input[0] = 1.0;
            let out = run(&mut r, &input);
            let peak = out.iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.01, "the reverb rang: {peak}");
            let after = out[1 + n..].iter().fold(0.0_f32, |m, v| m.max(v.abs()));
            assert!(after < 3.2e-8, "mix {mix}: {after} left after the tail");
        }
    }
}
