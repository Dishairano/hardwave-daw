//! Native limiter plugin — brick-wall peak limiter with lookahead,
//! ceiling, drive, and program-dependent release. Wraps the
//! envelope-follower + gain-reduction primitives from
//! `hardwave_dsp::dynamics` in the `HostedPlugin` trait.
//!
//! Param map mirrors Fruity Limiter's "limit" mode at default:
//! Threshold (catches the signal), Ceiling (true peak), Release
//! (recovery time), Drive (input gain → harder bite for harderstyles).

use hardwave_dsp::dynamics::{
    compressor_gain_reduction_db, db_to_linear, linear_to_db, DetectMode, EnvelopeFollower,
};
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_THRESHOLD: u32 = 0;
const PARAM_CEILING: u32 = 1;
const PARAM_RELEASE: u32 = 2;
const PARAM_DRIVE: u32 = 3;
const PARAM_COUNT: u32 = 4;

const LOOKAHEAD_SAMPLES: usize = 64;

/// Effective ratio used to derive gain-reduction from the existing
/// compressor curve. A "limiter" is a compressor with very high
/// ratio; 100:1 gives a brick-wall feel without exotic math.
const LIMITER_RATIO: f32 = 100.0;
const LIMITER_KNEE_DB: f32 = 0.5;

pub struct NativeLimiter {
    descriptor: PluginDescriptor,
    env_l: EnvelopeFollower,
    env_r: EnvelopeFollower,
    threshold_db: f32,
    ceiling_db: f32,
    release_ms: f32,
    drive_db: f32,
    sample_rate: f32,
    active: bool,
    /// Per-channel lookahead delay so the limiter has 64 samples of
    /// foresight before pulling down on a peak. Without this, sharp
    /// transients clip even with the gain reduction tracking them.
    delay_l: Vec<f32>,
    delay_r: Vec<f32>,
    delay_idx: usize,
}

impl NativeLimiter {
    pub const ID: &'static str = "hardwave.native.limiter";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Limiter".into(),
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
            env_l: EnvelopeFollower::default(),
            env_r: EnvelopeFollower::default(),
            threshold_db: -3.0,
            ceiling_db: -0.3,
            release_ms: 80.0,
            drive_db: 0.0,
            sample_rate: 48_000.0,
            active: false,
            delay_l: vec![0.0; LOOKAHEAD_SAMPLES],
            delay_r: vec![0.0; LOOKAHEAD_SAMPLES],
            delay_idx: 0,
        }
    }

    fn update_envelope(&mut self) {
        // Attack of a limiter is essentially the lookahead duration —
        // we advertise 0.1 ms attack so the envelope tracks transients
        // tightly. Release is user-controlled.
        self.env_l.set_times(0.1, self.release_ms, self.sample_rate);
        self.env_r.set_times(0.1, self.release_ms, self.sample_rate);
        self.env_l.set_mode(DetectMode::Peak);
        self.env_r.set_mode(DetectMode::Peak);
    }

    /// Convert a normalised 0..=1 automation value to the parameter's
    /// real range. Symmetric counterpart of `to_normalised`.
    fn from_normalised(id: u32, v: f64) -> f64 {
        let v = v.clamp(0.0, 1.0);
        match id {
            PARAM_THRESHOLD => -60.0 + v * 60.0,
            PARAM_CEILING => -20.0 + v * 20.0,
            PARAM_RELEASE => 1.0 + v * 999.0,
            PARAM_DRIVE => v * 24.0,
            _ => v,
        }
    }

    fn to_normalised(id: u32, v: f64) -> f64 {
        match id {
            PARAM_THRESHOLD => ((v + 60.0) / 60.0).clamp(0.0, 1.0),
            PARAM_CEILING => ((v + 20.0) / 20.0).clamp(0.0, 1.0),
            PARAM_RELEASE => ((v - 1.0) / 999.0).clamp(0.0, 1.0),
            PARAM_DRIVE => (v / 24.0).clamp(0.0, 1.0),
            _ => v.clamp(0.0, 1.0),
        }
    }
}

impl Default for NativeLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeLimiter {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sample_rate: f64, _max_block_size: u32) -> Result<(), String> {
        self.sample_rate = sample_rate.max(1.0) as f32;
        self.update_envelope();
        self.delay_l.fill(0.0);
        self.delay_r.fill(0.0);
        self.delay_idx = 0;
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

        let drive_lin = db_to_linear(self.drive_db);
        let ceiling_lin = db_to_linear(self.ceiling_db);

        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0) * drive_lin;
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0) * drive_lin;

            // Push driven sample into lookahead, read 64-sample-old
            // value out. Envelope follower runs on the INCOMING sample
            // so by the time we apply gain to the OUTGOING sample the
            // reduction is already in place for the upcoming peak.
            let out_l = self.delay_l[self.delay_idx];
            let out_r = self.delay_r[self.delay_idx];
            self.delay_l[self.delay_idx] = in_l;
            self.delay_r[self.delay_idx] = in_r;
            self.delay_idx = (self.delay_idx + 1) % LOOKAHEAD_SAMPLES;

            let env_l = self.env_l.process(in_l);
            let env_r = self.env_r.process(in_r);
            let env_db_l = linear_to_db(env_l);
            let env_db_r = linear_to_db(env_r);

            let red_l_db = compressor_gain_reduction_db(
                env_db_l,
                self.threshold_db,
                LIMITER_RATIO,
                LIMITER_KNEE_DB,
            );
            let red_r_db = compressor_gain_reduction_db(
                env_db_r,
                self.threshold_db,
                LIMITER_RATIO,
                LIMITER_KNEE_DB,
            );
            let g_l = db_to_linear(red_l_db);
            let g_r = db_to_linear(red_r_db);

            // Apply gain reduction, then hard-clamp to the ceiling so
            // even a noisy detector can't sneak through.
            let post_l = (out_l * g_l).clamp(-ceiling_lin, ceiling_lin);
            let post_r = (out_r * g_r).clamp(-ceiling_lin, ceiling_lin);
            outputs[0][i] = post_l;
            outputs[1][i] = post_r;
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        match index {
            PARAM_THRESHOLD => Some(ParameterInfo {
                id: PARAM_THRESHOLD,
                name: "Threshold".to_string(),
                default_value: Self::to_normalised(PARAM_THRESHOLD, -3.0),
                min: 0.0,
                max: 1.0,
                unit: "dB".to_string(),
                automatable: true,
            }),
            PARAM_CEILING => Some(ParameterInfo {
                id: PARAM_CEILING,
                name: "Ceiling".to_string(),
                default_value: Self::to_normalised(PARAM_CEILING, -0.3),
                min: 0.0,
                max: 1.0,
                unit: "dB".to_string(),
                automatable: true,
            }),
            PARAM_RELEASE => Some(ParameterInfo {
                id: PARAM_RELEASE,
                name: "Release".to_string(),
                default_value: Self::to_normalised(PARAM_RELEASE, 80.0),
                min: 0.0,
                max: 1.0,
                unit: "ms".to_string(),
                automatable: true,
            }),
            PARAM_DRIVE => Some(ParameterInfo {
                id: PARAM_DRIVE,
                name: "Drive".to_string(),
                default_value: Self::to_normalised(PARAM_DRIVE, 0.0),
                min: 0.0,
                max: 1.0,
                unit: "dB".to_string(),
                automatable: true,
            }),
            _ => None,
        }
    }

    fn get_parameter_value(&self, id: u32) -> f64 {
        let raw = match id {
            PARAM_THRESHOLD => self.threshold_db as f64,
            PARAM_CEILING => self.ceiling_db as f64,
            PARAM_RELEASE => self.release_ms as f64,
            PARAM_DRIVE => self.drive_db as f64,
            _ => 0.0,
        };
        Self::to_normalised(id, raw)
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let raw = Self::from_normalised(id, value);
        match id {
            PARAM_THRESHOLD => self.threshold_db = raw as f32,
            PARAM_CEILING => self.ceiling_db = raw as f32,
            PARAM_RELEASE => {
                self.release_ms = raw as f32;
                self.update_envelope();
            }
            PARAM_DRIVE => self.drive_db = raw as f32,
            _ => {}
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"threshold\":{},\"ceiling\":{},\"release\":{},\"drive\":{}}}",
            self.threshold_db, self.ceiling_db, self.release_ms, self.drive_db
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
        if let Some(v) = read("threshold") { self.threshold_db = v; }
        if let Some(v) = read("ceiling") { self.ceiling_db = v; }
        if let Some(v) = read("release") {
            self.release_ms = v;
            self.update_envelope();
        }
        if let Some(v) = read("drive") { self.drive_db = v; }
        Ok(())
    }

    fn latency_samples(&self) -> u32 {
        LOOKAHEAD_SAMPLES as u32
    }

    fn open_editor(&mut self, _parent_handle: RawWindowHandle) -> bool {
        // Editor is a Tauri React panel keyed off the plug-in id; the
        // host's native-window editor path is unused for natives.
        false
    }
    fn close_editor(&mut self) {}
    fn has_editor(&self) -> bool { false }
}
