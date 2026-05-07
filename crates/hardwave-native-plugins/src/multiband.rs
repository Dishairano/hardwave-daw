//! Native 3-band multiband compressor — wraps
//! `hardwave_dsp::multiband::MultibandCompressor3`. Mirrors Fruity
//! Multiband Compressor / Maximus' "3 band" mode.

use hardwave_dsp::multiband::{BandCompressor, MultibandCompressor3};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_LO_MID: u32 = 0;
const PARAM_MID_HI: u32 = 1;
const PARAM_LOW_THRESH: u32 = 2;
const PARAM_LOW_RATIO: u32 = 3;
const PARAM_MID_THRESH: u32 = 4;
const PARAM_MID_RATIO: u32 = 5;
const PARAM_HI_THRESH: u32 = 6;
const PARAM_HI_RATIO: u32 = 7;
const PARAM_OUT_GAIN: u32 = 8;
const PARAM_COUNT: u32 = 9;

pub struct NativeMultiband {
    descriptor: PluginDescriptor,
    mb: MultibandCompressor3,
    sample_rate: f32,
    lo_mid_hz: f32,
    mid_hi_hz: f32,
    bands: [BandCompressor; 3],
    output_gain_db: f32,
    active: bool,
}

impl NativeMultiband {
    pub const ID: &'static str = "hardwave.native.multiband";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Multiband".into(),
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
        let lo_mid = 200.0_f32;
        let mid_hi = 2_000.0_f32;
        let bands = [
            BandCompressor { threshold_db: -18.0, ratio: 4.0, ..Default::default() },
            BandCompressor { threshold_db: -14.0, ratio: 3.0, ..Default::default() },
            BandCompressor { threshold_db: -16.0, ratio: 2.5, ..Default::default() },
        ];
        let mut mb = MultibandCompressor3::new(48_000.0, lo_mid, mid_hi);
        for (i, b) in bands.iter().enumerate() {
            mb.set_band_params(i, b.clone());
        }
        Self {
            descriptor: Self::descriptor(),
            mb,
            sample_rate: 48_000.0,
            lo_mid_hz: lo_mid,
            mid_hi_hz: mid_hi,
            bands,
            output_gain_db: 0.0,
            active: false,
        }
    }

    fn rebuild(&mut self) {
        self.mb = MultibandCompressor3::new(self.sample_rate, self.lo_mid_hz, self.mid_hi_hz);
        for (i, b) in self.bands.iter().enumerate() {
            self.mb.set_band_params(i, *b);
        }
        self.mb.set_output_gain_db(self.output_gain_db);
    }
}

impl Default for NativeMultiband {
    fn default() -> Self { Self::new() }
}

/// Map a normalised 0..=1 value to a frequency in `[lo, hi]` log-spaced.
fn freq_from_norm(v: f64, lo: f32, hi: f32) -> f32 {
    let v = v.clamp(0.0, 1.0) as f32;
    let l = lo.log10();
    let h = hi.log10();
    10.0_f32.powf(l + (h - l) * v)
}

fn freq_to_norm(hz: f32, lo: f32, hi: f32) -> f64 {
    let hz = hz.clamp(lo, hi);
    let l = lo.log10();
    let h = hi.log10();
    ((hz.log10() - l) / (h - l)).clamp(0.0, 1.0) as f64
}

/// Threshold normalised: -60..=0 dB linear.
fn thresh_from_norm(v: f64) -> f32 { (-60.0 + v.clamp(0.0, 1.0) * 60.0) as f32 }
fn thresh_to_norm(db: f32) -> f64 { ((db + 60.0) / 60.0).clamp(0.0, 1.0) as f64 }

/// Ratio normalised: 1..=20 log.
fn ratio_from_norm(v: f64) -> f32 {
    let v = v.clamp(0.0, 1.0) as f32;
    20.0_f32.powf(v) // 1..=20
}
fn ratio_to_norm(r: f32) -> f64 {
    let r = r.clamp(1.0, 20.0);
    (r.log10() / 20.0_f32.log10()).clamp(0.0, 1.0) as f64
}

impl HostedPlugin for NativeMultiband {
    fn descriptor(&self) -> &PluginDescriptor { &self.descriptor }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = sr.max(1.0) as f32;
        self.rebuild();
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) { self.active = false; }

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
            let (l, r) = self.mb.process(in_l, in_r);
            outputs[0][i] = l;
            outputs[1][i] = r;
        }
    }

    fn get_parameter_count(&self) -> u32 { PARAM_COUNT }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_LO_MID => Some(ParameterInfo {
                id: PARAM_LO_MID, name: "Low/Mid".into(),
                default_value: freq_to_norm(200.0, 50.0, 1_000.0),
                min: 0.0, max: 1.0, unit: "Hz".into(), automatable: true,
            }),
            PARAM_MID_HI => Some(ParameterInfo {
                id: PARAM_MID_HI, name: "Mid/High".into(),
                default_value: freq_to_norm(2_000.0, 500.0, 12_000.0),
                min: 0.0, max: 1.0, unit: "Hz".into(), automatable: true,
            }),
            PARAM_LOW_THRESH => Some(ParameterInfo {
                id: PARAM_LOW_THRESH, name: "Low Thresh".into(),
                default_value: thresh_to_norm(-18.0),
                min: 0.0, max: 1.0, unit: "dB".into(), automatable: true,
            }),
            PARAM_LOW_RATIO => Some(ParameterInfo {
                id: PARAM_LOW_RATIO, name: "Low Ratio".into(),
                default_value: ratio_to_norm(4.0),
                min: 0.0, max: 1.0, unit: ":1".into(), automatable: true,
            }),
            PARAM_MID_THRESH => Some(ParameterInfo {
                id: PARAM_MID_THRESH, name: "Mid Thresh".into(),
                default_value: thresh_to_norm(-14.0),
                min: 0.0, max: 1.0, unit: "dB".into(), automatable: true,
            }),
            PARAM_MID_RATIO => Some(ParameterInfo {
                id: PARAM_MID_RATIO, name: "Mid Ratio".into(),
                default_value: ratio_to_norm(3.0),
                min: 0.0, max: 1.0, unit: ":1".into(), automatable: true,
            }),
            PARAM_HI_THRESH => Some(ParameterInfo {
                id: PARAM_HI_THRESH, name: "High Thresh".into(),
                default_value: thresh_to_norm(-16.0),
                min: 0.0, max: 1.0, unit: "dB".into(), automatable: true,
            }),
            PARAM_HI_RATIO => Some(ParameterInfo {
                id: PARAM_HI_RATIO, name: "High Ratio".into(),
                default_value: ratio_to_norm(2.5),
                min: 0.0, max: 1.0, unit: ":1".into(), automatable: true,
            }),
            PARAM_OUT_GAIN => Some(ParameterInfo {
                id: PARAM_OUT_GAIN, name: "Output".into(),
                default_value: 0.5,
                min: 0.0, max: 1.0, unit: "dB".into(), automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        match id {
            PARAM_LO_MID => freq_to_norm(self.lo_mid_hz, 50.0, 1_000.0),
            PARAM_MID_HI => freq_to_norm(self.mid_hi_hz, 500.0, 12_000.0),
            PARAM_LOW_THRESH => thresh_to_norm(self.bands[0].threshold_db),
            PARAM_LOW_RATIO => ratio_to_norm(self.bands[0].ratio),
            PARAM_MID_THRESH => thresh_to_norm(self.bands[1].threshold_db),
            PARAM_MID_RATIO => ratio_to_norm(self.bands[1].ratio),
            PARAM_HI_THRESH => thresh_to_norm(self.bands[2].threshold_db),
            PARAM_HI_RATIO => ratio_to_norm(self.bands[2].ratio),
            // -24..=24 dB mapped to 0..=1, 0.5 = unity
            PARAM_OUT_GAIN => ((self.output_gain_db + 24.0) / 48.0).clamp(0.0, 1.0) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let mut update_band: Option<usize> = None;
        match id {
            PARAM_LO_MID => {
                self.lo_mid_hz = freq_from_norm(value, 50.0, 1_000.0);
                self.mb.set_crossovers(self.lo_mid_hz, self.mid_hi_hz);
            }
            PARAM_MID_HI => {
                self.mid_hi_hz = freq_from_norm(value, 500.0, 12_000.0);
                self.mb.set_crossovers(self.lo_mid_hz, self.mid_hi_hz);
            }
            PARAM_LOW_THRESH => { self.bands[0].threshold_db = thresh_from_norm(value); update_band = Some(0); }
            PARAM_LOW_RATIO => { self.bands[0].ratio = ratio_from_norm(value); update_band = Some(0); }
            PARAM_MID_THRESH => { self.bands[1].threshold_db = thresh_from_norm(value); update_band = Some(1); }
            PARAM_MID_RATIO => { self.bands[1].ratio = ratio_from_norm(value); update_band = Some(1); }
            PARAM_HI_THRESH => { self.bands[2].threshold_db = thresh_from_norm(value); update_band = Some(2); }
            PARAM_HI_RATIO => { self.bands[2].ratio = ratio_from_norm(value); update_band = Some(2); }
            PARAM_OUT_GAIN => {
                self.output_gain_db = (value.clamp(0.0, 1.0) * 48.0 - 24.0) as f32;
                self.mb.set_output_gain_db(self.output_gain_db);
            }
            _ => {}
        }
        if let Some(i) = update_band {
            self.mb.set_band_params(i, self.bands[i]);
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"lm\":{},\"mh\":{},\"lt\":{},\"lr\":{},\"mt\":{},\"mr\":{},\"ht\":{},\"hr\":{},\"og\":{}}}",
            self.lo_mid_hz, self.mid_hi_hz,
            self.bands[0].threshold_db, self.bands[0].ratio,
            self.bands[1].threshold_db, self.bands[1].ratio,
            self.bands[2].threshold_db, self.bands[2].ratio,
            self.output_gain_db
        ).into_bytes()
    }

    fn set_state(&mut self, state: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(state).map_err(|e| e.to_string())?;
        let read = |key: &str| -> Option<f32> {
            let needle = format!("\"{key}\":");
            let i = s.find(&needle)?;
            let rest = &s[i + needle.len()..];
            let end = rest.find(|c: char| c == ',' || c == '}').unwrap_or(rest.len());
            rest[..end].trim().parse::<f32>().ok()
        };
        if let Some(v) = read("lm") { self.lo_mid_hz = v; }
        if let Some(v) = read("mh") { self.mid_hi_hz = v; }
        if let Some(v) = read("lt") { self.bands[0].threshold_db = v; }
        if let Some(v) = read("lr") { self.bands[0].ratio = v; }
        if let Some(v) = read("mt") { self.bands[1].threshold_db = v; }
        if let Some(v) = read("mr") { self.bands[1].ratio = v; }
        if let Some(v) = read("ht") { self.bands[2].threshold_db = v; }
        if let Some(v) = read("hr") { self.bands[2].ratio = v; }
        if let Some(v) = read("og") { self.output_gain_db = v; }
        self.rebuild();
        Ok(())
    }

    fn latency_samples(&self) -> u32 { 0 }
    fn open_editor(&mut self, _: RawWindowHandle) -> bool { false }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
