//! Native sub-bass enhancer — splits sub band (≤80 Hz) and applies
//! gentle saturation + boost. Mirrors Waves MaxxBass / FabFilter Pro-Q
//! sub-shelf treatment essentials. Hardstyle producers use this to
//! glue 808 sub-layers under a kick punch.

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use hardwave_dsp::synth_extras::FilterDrive;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_CROSSOVER: u32 = 0;
const PARAM_BOOST: u32 = 1;
const PARAM_DRIVE: u32 = 2;
const PARAM_MIX: u32 = 3;
const PARAM_COUNT: u32 = 4;

pub struct NativeSubBass {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    lp_l: Biquad,
    lp_r: Biquad,
    hp_l: Biquad,
    hp_r: Biquad,
    drive_l: FilterDrive,
    drive_r: FilterDrive,
    crossover_hz: f32,
    boost_db: f32,
    mix: f32,
    needs_recoef: bool,
    active: bool,
}

impl NativeSubBass {
    pub const ID: &'static str = "hardwave.native.sub_bass";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Sub Bass".into(),
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
            sample_rate: 48_000.0,
            lp_l: Biquad::default(),
            lp_r: Biquad::default(),
            hp_l: Biquad::default(),
            hp_r: Biquad::default(),
            drive_l: FilterDrive::new(),
            drive_r: FilterDrive::new(),
            crossover_hz: 80.0,
            boost_db: 4.0,
            mix: 0.5,
            needs_recoef: true,
            active: false,
        }
    }

    fn ensure_coefs(&mut self) {
        if !self.needs_recoef {
            return;
        }
        for f in [&mut self.lp_l, &mut self.lp_r] {
            f.set(
                BiquadKind::LowPass,
                self.sample_rate,
                self.crossover_hz,
                0.707,
                0.0,
            );
        }
        for f in [&mut self.hp_l, &mut self.hp_r] {
            f.set(
                BiquadKind::HighPass,
                self.sample_rate,
                self.crossover_hz,
                0.707,
                0.0,
            );
        }
        self.needs_recoef = false;
    }
}

impl Default for NativeSubBass {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeSubBass {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.lp_l.reset();
        self.lp_r.reset();
        self.hp_l.reset();
        self.hp_r.reset();
        self.needs_recoef = true;
        self.ensure_coefs();
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
        self.ensure_coefs();
        let boost_lin = 10.0_f32.powf(self.boost_db / 20.0);
        let mix = self.mix;
        let dry_mix = 1.0 - mix;
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);
            // Split sub vs above
            let sub_l = self.lp_l.process_mono(in_l);
            let sub_r = self.lp_r.process_mono(in_r);
            let high_l = self.hp_l.process_mono(in_l);
            let high_r = self.hp_r.process_mono(in_r);
            // Saturate + boost sub band
            let sat_l = self.drive_l.process(sub_l) * boost_lin;
            let sat_r = self.drive_r.process(sub_r) * boost_lin;
            // Recombine — only the sub band is processed; high band passes
            let processed_l = high_l + sat_l;
            let processed_r = high_r + sat_r;
            outputs[0][i] = in_l * dry_mix + processed_l * mix;
            outputs[1][i] = in_r * dry_mix + processed_r * mix;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_CROSSOVER => (
                "Crossover",
                ((80.0_f64.log10() - 30.0_f64.log10()) / (300.0_f64.log10() - 30.0_f64.log10()))
                    .clamp(0.0, 1.0),
                "Hz",
            ),
            PARAM_BOOST => ("Boost", (4.0_f64 + 12.0) / 24.0, "dB"),
            PARAM_DRIVE => ("Drive", 0.3, "%"),
            PARAM_MIX => ("Mix", 0.5, "%"),
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
            PARAM_CROSSOVER => {
                let lo = 30.0_f32.log10();
                let hi = 300.0_f32.log10();
                ((self.crossover_hz.log10() - lo) / (hi - lo)).clamp(0.0, 1.0) as f64
            }
            // -12..=+12 dB -> 0..=1
            PARAM_BOOST => ((self.boost_db + 12.0) / 24.0).clamp(0.0, 1.0) as f64,
            PARAM_DRIVE => self.drive_l.amount() as f64,
            PARAM_MIX => self.mix as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let v = value.clamp(0.0, 1.0);
        match id {
            PARAM_CROSSOVER => {
                let lo = 30.0_f32.log10();
                let hi = 300.0_f32.log10();
                self.crossover_hz = 10.0_f32.powf(lo + (hi - lo) * v as f32);
                self.needs_recoef = true;
            }
            PARAM_BOOST => self.boost_db = (v * 24.0 - 12.0) as f32,
            PARAM_DRIVE => {
                self.drive_l.set_amount(v as f32);
                self.drive_r.set_amount(v as f32);
            }
            PARAM_MIX => self.mix = v as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"cross\":{},\"boost\":{},\"drive\":{},\"mix\":{}}}",
            self.crossover_hz,
            self.boost_db,
            self.drive_l.amount(),
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
            let end = rest
                .find(|c: char| c == ',' || c == '}')
                .unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("cross") {
            self.crossover_hz = v.clamp(30.0, 300.0);
            self.needs_recoef = true;
        }
        if let Some(v) = read("boost") {
            self.boost_db = v.clamp(-12.0, 12.0);
        }
        if let Some(v) = read("drive") {
            let amt = v.clamp(0.0, 1.0);
            self.drive_l.set_amount(amt);
            self.drive_r.set_amount(amt);
        }
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        Ok(())
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
