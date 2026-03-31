use serde::{Deserialize, Serialize};

use hardwave_midi::PPQ;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TempoRamp {
    Instant,
    Linear,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoEntry {
    pub tick: u64,
    pub bpm: f64,
    pub time_sig_num: u32,
    pub time_sig_den: u32,
    pub ramp: TempoRamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoMap {
    pub entries: Vec<TempoEntry>,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            entries: vec![TempoEntry {
                tick: 0,
                bpm: 140.0,
                time_sig_num: 4,
                time_sig_den: 4,
                ramp: TempoRamp::Instant,
            }],
        }
    }
}

impl TempoMap {
    /// Convert a tick position to an absolute sample position.
    pub fn tick_to_samples(&self, tick: u64, sample_rate: f64) -> u64 {
        let mut samples = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_bpm = self.entries[0].bpm;

        for entry in &self.entries[1..] {
            if entry.tick >= tick {
                break;
            }
            let dt = entry.tick - prev_tick;
            let secs = ticks_to_secs(dt, prev_bpm);
            samples += secs * sample_rate;
            prev_tick = entry.tick;
            prev_bpm = entry.bpm;
        }

        let remaining = tick - prev_tick;
        samples += ticks_to_secs(remaining, prev_bpm) * sample_rate;

        samples.round() as u64
    }

    /// Convert an absolute sample position to ticks.
    pub fn samples_to_tick(&self, target_samples: u64, sample_rate: f64) -> u64 {
        let mut samples_accum = 0.0_f64;
        let mut prev_tick = 0_u64;
        let mut prev_bpm = self.entries[0].bpm;

        for entry in &self.entries[1..] {
            let dt = entry.tick - prev_tick;
            let secs = ticks_to_secs(dt, prev_bpm);
            let seg_samples = secs * sample_rate;

            if samples_accum + seg_samples > target_samples as f64 {
                break;
            }
            samples_accum += seg_samples;
            prev_tick = entry.tick;
            prev_bpm = entry.bpm;
        }

        let remaining_samples = target_samples as f64 - samples_accum;
        let remaining_secs = remaining_samples / sample_rate;
        let remaining_ticks = secs_to_ticks(remaining_secs, prev_bpm);

        prev_tick + remaining_ticks
    }

    /// Get the BPM at a given tick.
    pub fn bpm_at(&self, tick: u64) -> f64 {
        let mut bpm = self.entries[0].bpm;
        for entry in &self.entries {
            if entry.tick > tick { break; }
            bpm = entry.bpm;
        }
        bpm
    }

    /// Convert tick to (bar, beat) tuple (1-indexed).
    pub fn tick_to_bar_beat(&self, tick: u64) -> (u32, f64) {
        let entry = self.entries.iter().rev().find(|e| e.tick <= tick)
            .unwrap_or(&self.entries[0]);

        let ticks_per_beat = PPQ;
        let ticks_per_bar = ticks_per_beat * entry.time_sig_num as u64;

        let relative_tick = tick - entry.tick;
        let bar = (relative_tick / ticks_per_bar) as u32 + 1;
        let beat = (relative_tick % ticks_per_bar) as f64 / ticks_per_beat as f64 + 1.0;

        (bar, beat)
    }
}

fn ticks_to_secs(ticks: u64, bpm: f64) -> f64 {
    let beats = ticks as f64 / PPQ as f64;
    beats * 60.0 / bpm
}

fn secs_to_ticks(secs: f64, bpm: f64) -> u64 {
    let beats = secs * bpm / 60.0;
    (beats * PPQ as f64).round() as u64
}
