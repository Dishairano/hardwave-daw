//! Native beat-repeat / stutter — captures a slice of incoming audio and
//! loops it for a few repeats with optional per-repeat decay, producing
//! the glitch/stutter effect from the Gross-Beat family.
//!
//! The slice is set in milliseconds, or with Sync in note values at the
//! song's tempo, lined up to the bar through the host transport
//! (`set_transport`).

use crate::format;
use hardwave_midi::MidiEvent;
use hardwave_plugin_host::types::{
    HostedPlugin, ParameterInfo, PluginCategory, PluginDescriptor, PluginFormat,
};
use raw_window_handle::RawWindowHandle;
use std::path::PathBuf;

const PARAM_MIX: u32 = 0;
const PARAM_SLICE: u32 = 1; // 30..500 ms
const PARAM_REPEATS: u32 = 2; // 1..8
const PARAM_DECAY: u32 = 3; // per-repeat gain 0.5..1.0
const PARAM_SYNC: u32 = 4; // off, or a note value
const PARAM_COUNT: u32 = 5;

/// Slice lengths in beats, for the tempo-synced side. A bar at four
/// four is four beats, which is where the long ones come from.
const SYNC_DIVISIONS: [(f32, &str); 7] = [
    (0.0, "Off"),
    (2.0, "1/2"),
    (1.0, "1/4"),
    (0.5, "1/8"),
    (0.25, "1/16"),
    (1.0 / 3.0, "1/8T"),
    (0.125, "1/32"),
];

const SLICE_MIN_MS: f32 = 30.0;
const SLICE_MAX_MS: f32 = 500.0;
const MAX_REPEATS: u32 = 8;

pub struct NativeStutter {
    descriptor: PluginDescriptor,
    sample_rate: f32,
    active: bool,
    // Hold buffer for the captured slice (stereo, interleaved L,R).
    hold_l: Vec<f32>,
    hold_r: Vec<f32>,
    slice_len: usize,
    pos_in_slice: usize,
    slice_in_group: u32,
    // Params
    mix: f32,
    slice_ms: f32,
    repeats: u32,
    decay: f32,
    /// Which entry of `SYNC_DIVISIONS` the slice follows. Zero is the
    /// millisecond slice, which is what this plug-in had before the
    /// host told it where the song was.
    sync: usize,
    /// Where the song is, as the host last said.
    transport: hardwave_plugin_host::types::TransportInfo,
}

impl NativeStutter {
    pub const ID: &'static str = "hardwave.native.stutter";

    pub fn descriptor() -> PluginDescriptor {
        PluginDescriptor {
            id: Self::ID.into(),
            name: "Hardwave Stutter".into(),
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
        let slice_ms = 125.0;
        let slice_len = ((slice_ms / 1000.0) * sr) as usize;
        Self {
            descriptor: Self::descriptor(),
            sample_rate: sr,
            active: false,
            hold_l: vec![0.0; slice_len.max(1)],
            hold_r: vec![0.0; slice_len.max(1)],
            slice_len: slice_len.max(1),
            pos_in_slice: 0,
            slice_in_group: 0,
            mix: 0.5,
            slice_ms,
            repeats: 4,
            decay: 0.85,
            sync: 0,
            transport: hardwave_plugin_host::types::TransportInfo::default(),
        }
    }

    /// The real value behind a 0..1 knob: milliseconds for the slice,
    /// the per-repeat gain for the decay, the mix itself otherwise.
    /// Setting a parameter and showing it both go through here, so the
    /// label is always what the sound is doing.
    fn from_normalised(id: u32, value: f64) -> f32 {
        let v = value.clamp(0.0, 1.0) as f32;
        match id {
            PARAM_SLICE => SLICE_MIN_MS + v * (SLICE_MAX_MS - SLICE_MIN_MS),
            PARAM_DECAY => 0.5 + v * 0.5,
            _ => v,
        }
    }

    /// How many slices a group plays for a 0..1 setting: 1 at 0, up to
    /// `MAX_REPEATS` at 1, one step per 1/(MAX_REPEATS - 1).
    fn repeats_from_normalised(value: f64) -> u32 {
        let v = value.clamp(0.0, 1.0) as f32;
        1 + (v * (MAX_REPEATS - 1) as f32).round() as u32
    }

    /// Which entry of `SYNC_DIVISIONS` a 0..1 setting picks: entry i sits
    /// at i/(len - 1).
    fn sync_from_normalised(value: f64) -> usize {
        let v = value.clamp(0.0, 1.0) as f32;
        let last = SYNC_DIVISIONS.len() - 1;
        (v * last as f32).round() as usize
    }

    /// How long a slice is, in samples.
    ///
    /// Synced, it is a note value at the song's tempo, so the stutter
    /// lands on the beat rather than drifting past it. Off, it is the
    /// millisecond setting.
    fn slice_samples(&self) -> usize {
        let beats = SYNC_DIVISIONS[self.sync.min(SYNC_DIVISIONS.len() - 1)].0;
        if beats > 0.0 {
            (beats as f64 * self.transport.samples_per_beat()).max(1.0) as usize
        } else {
            (((self.slice_ms / 1000.0) * self.sample_rate) as usize).max(1)
        }
    }

    fn recompute_slice(&mut self) {
        let len = self.slice_samples();
        self.slice_len = len;
        self.hold_l.resize(len, 0.0);
        self.hold_r.resize(len, 0.0);
        if self.pos_in_slice >= len {
            self.pos_in_slice = 0;
        }
    }
}

impl Default for NativeStutter {
    fn default() -> Self {
        Self::new()
    }
}

impl HostedPlugin for NativeStutter {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn activate(&mut self, sr: f64, _max: u32) -> Result<(), String> {
        self.sample_rate = (sr as f32).max(1.0);
        self.recompute_slice();
        self.pos_in_slice = 0;
        self.slice_in_group = 0;
        self.active = true;
        Ok(())
    }
    fn deactivate(&mut self) {
        self.active = false;
    }

    fn set_transport(&mut self, transport: hardwave_plugin_host::types::TransportInfo) {
        let tempo_changed = (transport.tempo - self.transport.tempo).abs() > 0.001;
        let rate_changed = (transport.sample_rate - self.transport.sample_rate).abs() > 0.001;
        self.transport = transport;
        if self.sync > 0 && (tempo_changed || rate_changed) {
            self.recompute_slice();
        }
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
        let mix = self.mix;
        // Synced: the group starts where the bar does, so the pattern
        // is in the same place every time round instead of wherever
        // the last block happened to leave it.
        if self.sync > 0 && self.transport.playing && self.slice_len > 0 {
            let group = (self.slice_len * self.repeats.max(1) as usize) as f64;
            let into_song = self.transport.position_beats * self.transport.samples_per_beat();
            let into_group = into_song.rem_euclid(group);
            let slice = self.slice_len as f64;
            self.slice_in_group = ((into_group / slice) as u32).min(self.repeats.saturating_sub(1));
            self.pos_in_slice = (into_group % slice) as usize;
            if self.pos_in_slice >= self.hold_l.len() {
                self.pos_in_slice = 0;
            }
        }
        for i in 0..num_samples {
            let in_l = inputs[0].get(i).copied().unwrap_or(0.0);
            let in_r = inputs[1].get(i).copied().unwrap_or(0.0);

            let (wet_l, wet_r) = if self.slice_in_group == 0 {
                // First slice of the group: pass through AND record it.
                self.hold_l[self.pos_in_slice] = in_l;
                self.hold_r[self.pos_in_slice] = in_r;
                (in_l, in_r)
            } else {
                // Subsequent slices: replay the captured slice, decayed.
                let g = self.decay.powi(self.slice_in_group as i32);
                (
                    self.hold_l[self.pos_in_slice] * g,
                    self.hold_r[self.pos_in_slice] * g,
                )
            };

            outputs[0][i] = in_l * (1.0 - mix) + wet_l * mix;
            outputs[1][i] = in_r * (1.0 - mix) + wet_r * mix;

            self.pos_in_slice += 1;
            if self.pos_in_slice >= self.slice_len {
                self.pos_in_slice = 0;
                self.slice_in_group += 1;
                if self.slice_in_group >= self.repeats {
                    self.slice_in_group = 0;
                }
            }
        }
    }

    fn get_parameter_count(&self) -> u32 {
        PARAM_COUNT
    }

    fn get_parameter_info(&self, index: u32) -> Option<ParameterInfo> {
        let (name, default, unit) = match index {
            PARAM_MIX => ("Mix", 0.5, ""),
            PARAM_SLICE => (
                "Slice",
                ((125.0 - SLICE_MIN_MS) / (SLICE_MAX_MS - SLICE_MIN_MS)) as f64,
                "ms",
            ),
            PARAM_REPEATS => ("Repeats", 3.0 / (MAX_REPEATS - 1) as f64, ""),
            PARAM_DECAY => ("Decay", (0.85 - 0.5) / 0.5, ""),
            PARAM_SYNC => ("Sync", 0.0, "off / 1-2 to 1-32"),
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
            PARAM_MIX => self.mix as f64,
            PARAM_SLICE => ((self.slice_ms - SLICE_MIN_MS) / (SLICE_MAX_MS - SLICE_MIN_MS))
                .clamp(0.0, 1.0) as f64,
            PARAM_REPEATS => ((self.repeats - 1) as f32 / (MAX_REPEATS - 1) as f32) as f64,
            PARAM_DECAY => ((self.decay - 0.5) / 0.5).clamp(0.0, 1.0) as f64,
            PARAM_SYNC => self.sync as f64 / (SYNC_DIVISIONS.len() - 1) as f64,
            _ => 0.0,
        }
    }

    fn set_parameter_value(&mut self, id: u32, value: f64) {
        let real = Self::from_normalised(id, value);
        match id {
            PARAM_MIX => self.mix = real,
            PARAM_SLICE => {
                self.slice_ms = real;
                self.recompute_slice();
            }
            PARAM_REPEATS => self.repeats = Self::repeats_from_normalised(value),
            PARAM_DECAY => self.decay = real,
            PARAM_SYNC => {
                self.sync = Self::sync_from_normalised(value);
                self.recompute_slice();
            }
            _ => {}
        }
    }

    fn parameter_text(&self, id: u32, value: f64) -> Option<String> {
        let real = Self::from_normalised(id, value) as f64;
        Some(match id {
            PARAM_MIX => format::pct(real),
            PARAM_SLICE => format::ms(real),
            PARAM_REPEATS => Self::repeats_from_normalised(value).to_string(),
            // Each repeat's level against the one before it.
            PARAM_DECAY => format::pct(real),
            PARAM_SYNC => SYNC_DIVISIONS[Self::sync_from_normalised(value)]
                .1
                .to_string(),
            _ => return None,
        })
    }

    fn parameter_options(&self, id: u32) -> Option<Vec<String>> {
        match id {
            PARAM_REPEATS => Some((1..=MAX_REPEATS).map(|n| n.to_string()).collect()),
            PARAM_SYNC => Some(
                SYNC_DIVISIONS
                    .iter()
                    .map(|(_, label)| label.to_string())
                    .collect(),
            ),
            _ => None,
        }
    }

    fn get_state(&self) -> Vec<u8> {
        format!(
            "{{\"mix\":{},\"slice\":{},\"rep\":{},\"dec\":{}}}",
            self.mix, self.slice_ms, self.repeats, self.decay
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
        if let Some(v) = read("mix") {
            self.mix = v.clamp(0.0, 1.0);
        }
        if let Some(v) = read("slice") {
            self.slice_ms = v.clamp(SLICE_MIN_MS, SLICE_MAX_MS);
        }
        if let Some(v) = read("rep") {
            self.repeats = (v as u32).clamp(1, MAX_REPEATS);
        }
        if let Some(v) = read("dec") {
            self.decay = v.clamp(0.5, 1.0);
        }
        self.recompute_slice();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &mut NativeStutter, l: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let r = l.to_vec();
        let inputs: [&[f32]; 2] = [l, &r];
        let mut outputs = vec![vec![0.0; l.len()], vec![0.0; l.len()]];
        s.process(&inputs, &mut outputs, &[], &mut Vec::new(), l.len());
        (outputs[0].clone(), outputs[1].clone())
    }

    #[test]
    fn silence_in_silence_out() {
        let mut s = NativeStutter::new();
        s.activate(48_000.0, 512).unwrap();
        let (o, _) = run(&mut s, &vec![0.0; 4096]);
        assert!(o.iter().all(|v| v.abs() < 1e-6));
    }

    #[test]
    fn first_slice_passes_through_fully_wet() {
        let mut s = NativeStutter::new();
        s.mix = 1.0;
        s.slice_ms = 1.0; // tiny slice for the test
        s.repeats = 2;
        s.activate(48_000.0, 512).unwrap();
        // slice_len at 48k, 1ms = 48 samples. First slice = pass-through.
        let input: Vec<f32> = (0..s.slice_len).map(|i| (i as f32 * 0.01).sin()).collect();
        let (o, _) = run(&mut s, &input);
        for (a, b) in o.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6, "first slice must pass through");
        }
    }

    #[test]
    fn second_slice_repeats_first_with_decay() {
        let mut s = NativeStutter::new();
        s.mix = 1.0;
        s.repeats = 2;
        s.decay = 0.5;
        s.activate(48_000.0, 512).unwrap();
        let len = s.slice_len;
        // Feed slice 1 = constant 1.0, slice 2 = constant 0.0.
        let mut input = vec![1.0; len];
        input.extend(vec![0.0; len]);
        let (o, _) = run(&mut s, &input);
        // Slice 1 passes 1.0; slice 2 replays slice 1 (=1.0) * decay 0.5.
        assert!((o[0] - 1.0).abs() < 1e-6);
        assert!((o[len] - 0.5).abs() < 1e-6, "second slice = first * decay");
    }

    #[test]
    fn inactive_passes_through() {
        let mut s = NativeStutter::new();
        let (o, _) = run(&mut s, &[0.1, -0.2, 0.3]);
        assert_eq!(o, vec![0.1, -0.2, 0.3]);
    }

    #[test]
    fn choices_match_what_the_setting_picks() {
        let s = NativeStutter::new();
        let slice = s.get_parameter_info(PARAM_SLICE).unwrap().default_value;
        assert_eq!(
            s.parameter_text(PARAM_SLICE, slice).as_deref(),
            Some("125 ms")
        );
        let decay = s.get_parameter_info(PARAM_DECAY).unwrap().default_value;
        assert_eq!(
            s.parameter_text(PARAM_DECAY, decay).as_deref(),
            Some("85 %")
        );
        let sync = s.get_parameter_info(PARAM_SYNC).unwrap().default_value;
        assert_eq!(s.parameter_text(PARAM_SYNC, sync).as_deref(), Some("Off"));
        assert_eq!(
            s.parameter_options(PARAM_SYNC).unwrap(),
            vec!["Off", "1/2", "1/4", "1/8", "1/16", "1/8T", "1/32"]
        );
        for id in [PARAM_REPEATS, PARAM_SYNC] {
            let options = s.parameter_options(id).unwrap();
            let last = (options.len() - 1) as f64;
            for (i, label) in options.iter().enumerate() {
                let v = i as f64 / last;
                assert_eq!(s.parameter_text(id, v).as_ref(), Some(label));
                // And the setting really lands on that choice.
                let mut probe = NativeStutter::new();
                probe.set_parameter_value(id, v);
                assert!((probe.get_parameter_value(id) - v).abs() < 1e-6);
            }
        }
        for id in 0..PARAM_COUNT {
            for step in 0..=10 {
                assert!(s.parameter_text(id, step as f64 / 10.0).is_some());
            }
        }
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use hardwave_plugin_host::types::TransportInfo;

    fn transport(tempo: f64, position_beats: f64) -> TransportInfo {
        TransportInfo {
            playing: true,
            tempo,
            position_beats,
            time_sig: (4, 4),
            sample_rate: 48_000.0,
            position_samples: 0,
        }
    }

    #[test]
    fn a_synced_slice_is_a_note_value_at_the_songs_tempo() {
        let mut st = NativeStutter::new();
        st.activate(48_000.0, 512).expect("activate");
        st.set_transport(transport(120.0, 0.0));
        // 1/4 at 120 bpm is half a second.
        st.set_parameter_value(PARAM_SYNC, 2.0 / (SYNC_DIVISIONS.len() - 1) as f64);
        assert_eq!(st.slice_len, 24_000);

        // The same setting at 140 follows the tempo.
        st.set_transport(transport(140.0, 0.0));
        let expected = (48_000.0 * 60.0 / 140.0) as usize;
        assert!(
            (st.slice_len as i64 - expected as i64).abs() <= 1,
            "a quarter at 140 is about {expected} samples, got {}",
            st.slice_len
        );
    }

    #[test]
    fn with_sync_off_the_slice_is_the_millisecond_setting() {
        let mut st = NativeStutter::new();
        st.activate(48_000.0, 512).expect("activate");
        st.set_transport(transport(120.0, 0.0));
        st.set_parameter_value(PARAM_SYNC, 0.0);
        st.set_parameter_value(PARAM_SLICE, 0.0); // 30 ms
        assert_eq!(st.slice_len, (0.030 * 48_000.0) as usize);
        // A tempo change leaves a millisecond slice alone.
        st.set_transport(transport(174.0, 0.0));
        assert_eq!(st.slice_len, (0.030 * 48_000.0) as usize);
    }

    #[test]
    fn a_synced_group_starts_where_the_bar_does() {
        let mut st = NativeStutter::new();
        st.activate(48_000.0, 512).expect("activate");
        st.set_parameter_value(PARAM_SYNC, 2.0 / (SYNC_DIVISIONS.len() - 1) as f64);
        st.set_parameter_value(PARAM_REPEATS, 1.0 / (MAX_REPEATS - 1) as f64); // 2 repeats
        st.set_transport(transport(120.0, 0.0));

        let frames = 256;
        let input: Vec<f32> = (0..frames).map(|n| (n as f32 * 0.01).sin()).collect();
        let inputs: Vec<&[f32]> = vec![&input, &input];
        let mut outputs = vec![Vec::new(), Vec::new()];

        // Three beats in: with a quarter-note slice and two repeats the
        // group is two beats long, so beat three is the start of a
        // group again, not wherever the last block ended.
        st.set_transport(transport(120.0, 3.0));
        st.process(&inputs, &mut outputs, &[], &mut Vec::new(), frames);
        assert_eq!(st.slice_in_group, 1, "beat three is the second slice");

        st.set_transport(transport(120.0, 4.0));
        st.process(&inputs, &mut outputs, &[], &mut Vec::new(), frames);
        assert_eq!(st.slice_in_group, 0, "beat four starts a group");
    }
}
