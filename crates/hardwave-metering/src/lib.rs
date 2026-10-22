//! Hardwave Metering — peak, RMS, true peak, LUFS (BS.1770), stereo analysis.
//! Ported from the Hardwave Analyser TypeScript engine.

use serde::Serialize;

// ---------------------------------------------------------------------------
// Biquad filter (used for K-weighting)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BiquadFilter {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl BiquadFilter {
    fn new(b0: f64, b1: f64, b2: f64, a0: f64, a1: f64, a2: f64) -> Self {
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

// ---------------------------------------------------------------------------
// ITU-R BS.1770 K-weighting filter
// ---------------------------------------------------------------------------

/// The K-weighting of BS.1770-4: a high shelf, then the RLB high-pass.
/// Designed by the bilinear transform from the standard's analog
/// parameters (as libebur128 does), which gives the published 48 kHz
/// coefficients exactly and the same curve at any other rate. It used the
/// audio-EQ-cookbook shelf, whose curve is not the standard's: a 1 kHz sine
/// read a quarter LU low.
fn make_k_filter(fs: f64) -> (BiquadFilter, BiquadFilter) {
    // Stage 1: high-shelf pre-filter
    let k = (std::f64::consts::PI * 1681.974450955533 / fs).tan();
    let q = 0.7071752369554196;
    let vh = 10f64.powf(3.999843853973347 / 20.0);
    let vb = vh.powf(0.4996667741545416);
    let a0 = 1.0 + k / q + k * k;
    let shelf = BiquadFilter::new(
        vh + vb * k / q + k * k,
        2.0 * (k * k - vh),
        vh - vb * k / q + k * k,
        a0,
        2.0 * (k * k - 1.0),
        1.0 - k / q + k * k,
    );

    // Stage 2: high-pass (RLB filter)
    let k = (std::f64::consts::PI * 38.13547087602444 / fs).tan();
    let q = 0.5003270373238773;
    let a0 = 1.0 + k / q + k * k;
    let hp = BiquadFilter::new(1.0, -2.0, 1.0, a0, 2.0 * (k * k - 1.0), 1.0 - k / q + k * k);

    (shelf, hp)
}

// ---------------------------------------------------------------------------
// Channel meter
// ---------------------------------------------------------------------------

/// Per-channel metering state.
#[derive(Clone)]
pub struct ChannelMeter {
    peak: f32,
    peak_hold: f32,
    /// Samples remaining before peak_hold starts falling.
    peak_hold_timer: u64,
    true_peak: f32,
    rms_sum: f64,
    rms_count: u64,
    rms_smooth: f32,
    clip: bool,
    sample_rate: f64,

    // K-weighting for LUFS
    k_shelf_l: BiquadFilter,
    k_hp_l: BiquadFilter,
    k_shelf_r: BiquadFilter,
    k_hp_r: BiquadFilter,
    k_sr: f64,

    // LUFS history (ring buffer of K-weighted mean-square per block)
    lufs_z: Vec<f64>,
    lufs_pos: usize,
    lufs_fill: usize,
    /// Gating blocks for integrated loudness. A list of them was declared
    /// and never filled, so integrated LUFS was always empty.
    gated: GatedLoudness,
    tp_l: TruePeak,
    tp_r: TruePeak,
}

const LUFS_HIST: usize = 300;
const PEAK_DECAY_DB: f32 = 0.0625; // ~9 dB/s at 144 fps
/// How long peak_hold stays latched after each new peak, in seconds.
const PEAK_HOLD_SEC: f64 = 1.5;
/// How fast peak_hold falls once the hold timer expires, in dB/second.
const PEAK_HOLD_FALL_DB_PER_SEC: f32 = 20.0;

impl ChannelMeter {
    pub fn new(sample_rate: f64) -> Self {
        let (shelf_l, hp_l) = make_k_filter(sample_rate);
        let (shelf_r, hp_r) = make_k_filter(sample_rate);

        Self {
            peak: -100.0,
            peak_hold: -100.0,
            peak_hold_timer: 0,
            true_peak: -100.0,
            rms_sum: 0.0,
            rms_count: 0,
            rms_smooth: 0.0,
            clip: false,
            sample_rate,
            k_shelf_l: shelf_l,
            k_hp_l: hp_l,
            k_shelf_r: shelf_r,
            k_hp_r: hp_r,
            k_sr: sample_rate,
            lufs_z: vec![0.0; LUFS_HIST],
            lufs_pos: 0,
            lufs_fill: 0,
            gated: GatedLoudness::new(sample_rate),
            tp_l: TruePeak::new(),
            tp_r: TruePeak::new(),
        }
    }

    /// Process a block of stereo samples and update meters.
    pub fn process_block(&mut self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len());
        if n == 0 {
            return;
        }

        let mut block_peak = 0.0_f32;
        let mut block_true_peak = 0.0_f32;
        let mut sum_sq = 0.0_f64;
        let mut k_sum = 0.0_f64;

        for i in 0..n {
            let l = left[i];
            let r = right[i];
            let mono = (l + r) * 0.5;

            let abs = mono.abs();
            block_peak = block_peak.max(abs);
            sum_sq += (mono as f64) * (mono as f64);

            // True peak per channel, 4x oversampled as BS.1770-4 asks. It
            // interpolated the mono sum in straight lines, which can never
            // read above the samples it joins, so it showed the sample peak
            // and missed every over between samples.
            block_true_peak = block_true_peak
                .max(self.tp_l.push(l))
                .max(self.tp_r.push(r))
                .max(l.abs())
                .max(r.abs());

            // K-weighting for LUFS
            let kl = self.k_hp_l.process(self.k_shelf_l.process(l as f64));
            let kr = self.k_hp_r.process(self.k_shelf_r.process(r as f64));
            k_sum += kl * kl + kr * kr;
            self.gated.push(kl * kl + kr * kr);
        }

        // Peak (with decay)
        let peak_db = to_db(block_peak);
        self.peak = (self.peak - PEAK_DECAY_DB).max(peak_db);
        self.true_peak = self.true_peak.max(to_db(block_true_peak));

        // Peak hold: latch on new peak, then hold for PEAK_HOLD_SEC, then fall at PEAK_HOLD_FALL_DB_PER_SEC.
        if peak_db >= self.peak_hold {
            self.peak_hold = peak_db;
            self.peak_hold_timer = (PEAK_HOLD_SEC * self.sample_rate) as u64;
        } else if self.peak_hold_timer >= n as u64 {
            self.peak_hold_timer -= n as u64;
        } else {
            self.peak_hold_timer = 0;
            let fall = PEAK_HOLD_FALL_DB_PER_SEC * (n as f32 / self.sample_rate as f32);
            self.peak_hold = (self.peak_hold - fall).max(-100.0);
        }

        // RMS
        self.rms_sum += sum_sq;
        self.rms_count += n as u64;
        let rms_lin = (sum_sq / n as f64).sqrt() as f32;
        self.rms_smooth = self.rms_smooth * 0.85 + rms_lin * 0.15;

        // Clip detection
        if block_peak > 0.999 {
            self.clip = true;
        }

        // LUFS accumulation
        self.lufs_z[self.lufs_pos] = k_sum / n as f64;
        self.lufs_pos = (self.lufs_pos + 1) % LUFS_HIST;
        if self.lufs_fill < LUFS_HIST {
            self.lufs_fill += 1;
        }
    }

    pub fn peak_db(&self) -> f32 {
        self.peak
    }
    pub fn peak_hold_db(&self) -> f32 {
        self.peak_hold
    }
    pub fn true_peak_db(&self) -> f32 {
        self.true_peak
    }
    pub fn rms_db(&self) -> f32 {
        to_db(self.rms_smooth)
    }
    pub fn clipped(&self) -> bool {
        self.clip
    }

    /// Momentary LUFS (400ms window).
    pub fn lufs_m(&self, block_size: usize) -> Option<f32> {
        self.compute_lufs(400, block_size)
    }

    /// Short-term LUFS (3s window).
    pub fn lufs_s(&self, block_size: usize) -> Option<f32> {
        self.compute_lufs(3000, block_size)
    }

    /// Integrated LUFS (BS.1770-4 gated).
    pub fn lufs_i(&self) -> Option<f32> {
        self.gated.integrated()
    }

    fn compute_lufs(&self, duration_ms: u32, block_size: usize) -> Option<f32> {
        if self.k_sr == 0.0 || self.lufs_fill == 0 {
            return None;
        }
        let blocks_needed =
            ((duration_ms as f64 * self.k_sr) / (1000.0 * block_size as f64)).round() as usize;
        if blocks_needed < 1 || self.lufs_fill < blocks_needed {
            return None;
        }

        let mut sum = 0.0_f64;
        for i in 0..blocks_needed {
            let pos = (self.lufs_pos + LUFS_HIST - 1 - i) % LUFS_HIST;
            sum += self.lufs_z[pos];
        }
        let mean = sum / blocks_needed as f64;
        Some((-0.691 + 10.0 * (mean + 1e-30).log10()) as f32)
    }

    pub fn reset(&mut self) {
        self.peak = -100.0;
        self.peak_hold = -100.0;
        self.peak_hold_timer = 0;
        self.true_peak = -100.0;
        self.rms_sum = 0.0;
        self.rms_count = 0;
        self.rms_smooth = 0.0;
        self.clip = false;
        self.lufs_z.fill(0.0);
        self.lufs_pos = 0;
        self.lufs_fill = 0;
        self.gated.reset();
        self.tp_l.reset();
        self.tp_r.reset();
        self.k_shelf_l.reset();
        self.k_hp_l.reset();
        self.k_shelf_r.reset();
        self.k_hp_r.reset();
    }
}

// ---------------------------------------------------------------------------
// True peak (BS.1770-4 Annex 2) and gated integrated loudness
// ---------------------------------------------------------------------------

/// The 48-tap interpolation filter of BS.1770-4 Annex 2, as its four
/// phases: each turns the last 12 samples into one of the four samples at
/// four times the rate.
const TRUE_PEAK_PHASES: [[f32; 12]; 4] = [
    [
        0.001_708_984_4,
        0.010_986_328,
        -0.019_653_32,
        0.033_203_125,
        -0.059_448_242,
        0.137_329_1,
        0.972_167_97,
        -0.102_294_92,
        0.047_607_42,
        -0.026_611_328,
        0.014_892_578,
        -0.008_300_781,
    ],
    [
        -0.029_174_805,
        0.029_296_875,
        -0.051_757_812,
        0.089_111_33,
        -0.166_503_9,
        0.465_087_9,
        0.779_785_16,
        -0.200_317_38,
        0.101_562_5,
        -0.058_227_54,
        0.033_081_055,
        -0.018_920_898,
    ],
    [
        -0.018_920_898,
        0.033_081_055,
        -0.058_227_54,
        0.101_562_5,
        -0.200_317_38,
        0.779_785_16,
        0.465_087_9,
        -0.166_503_9,
        0.089_111_33,
        -0.051_757_812,
        0.029_296_875,
        -0.029_174_805,
    ],
    [
        -0.008_300_781,
        0.014_892_578,
        -0.026_611_328,
        0.047_607_42,
        -0.102_294_92,
        0.972_167_97,
        0.137_329_1,
        -0.059_448_242,
        0.033_203_125,
        -0.019_653_32,
        0.010_986_328,
        0.001_708_984_4,
    ],
];

/// One channel's true-peak reader: the samples between the samples, at four
/// times the rate, through the BS.1770-4 filter.
#[derive(Clone)]
pub struct TruePeak {
    history: [f32; 12],
    pos: usize,
}

impl Default for TruePeak {
    fn default() -> Self {
        Self::new()
    }
}

impl TruePeak {
    pub fn new() -> Self {
        Self {
            history: [0.0; 12],
            pos: 0,
        }
    }

    /// Take one sample; returns the largest magnitude among the four
    /// oversampled points it brings. The filter runs about six samples late.
    pub fn push(&mut self, sample: f32) -> f32 {
        self.history[self.pos] = sample;
        self.pos = (self.pos + 1) % 12;
        let mut peak = 0.0_f32;
        for phase in &TRUE_PEAK_PHASES {
            let mut acc = 0.0_f32;
            for (k, c) in phase.iter().enumerate() {
                acc += c * self.history[(self.pos + 11 - k) % 12];
            }
            peak = peak.max(acc.abs());
        }
        peak
    }

    pub fn reset(&mut self) {
        self.history = [0.0; 12];
        self.pos = 0;
    }
}

/// Lowest loudness a gating block can have (the absolute gate) and the
/// histogram's step: 0.1 LU bins up to +10 LUFS.
const GATE_FLOOR: f64 = -70.0;
const GATE_STEP: f64 = 0.1;
const GATE_BINS: usize = 800;

/// Integrated loudness as BS.1770-4 gates it: 400 ms blocks every 100 ms,
/// blocks under -70 LUFS left out, then blocks more than 10 LU under the
/// mean of the rest left out. The blocks are kept as a histogram of their
/// energy, so an hour costs the same memory as a second and nothing is
/// allocated while metering.
#[derive(Clone)]
struct GatedLoudness {
    /// Samples in 100 ms.
    hop: usize,
    /// K-weighted energy of the last four 100 ms parts.
    parts: [f64; 4],
    filled: usize,
    next: usize,
    current: f64,
    current_len: usize,
    /// Per 0.1 LU bin: how many blocks, and their summed mean square.
    counts: Vec<u32>,
    energy: Vec<f64>,
}

impl GatedLoudness {
    fn new(sample_rate: f64) -> Self {
        Self {
            hop: ((sample_rate * 0.1).round() as usize).max(1),
            parts: [0.0; 4],
            filled: 0,
            next: 0,
            current: 0.0,
            current_len: 0,
            counts: vec![0; GATE_BINS],
            energy: vec![0.0; GATE_BINS],
        }
    }

    /// One frame's K-weighted energy, both channels summed.
    fn push(&mut self, energy: f64) {
        self.current += energy;
        self.current_len += 1;
        if self.current_len < self.hop {
            return;
        }
        self.parts[self.next] = self.current;
        self.next = (self.next + 1) % 4;
        self.filled = (self.filled + 1).min(4);
        self.current = 0.0;
        self.current_len = 0;
        if self.filled < 4 {
            return;
        }
        let mean_square = self.parts.iter().sum::<f64>() / (4 * self.hop) as f64;
        let loudness = -0.691 + 10.0 * (mean_square + 1e-30).log10();
        if loudness <= GATE_FLOOR {
            return;
        }
        let bin = (((loudness - GATE_FLOOR) / GATE_STEP) as usize).min(GATE_BINS - 1);
        self.counts[bin] += 1;
        self.energy[bin] += mean_square;
    }

    fn integrated(&self) -> Option<f32> {
        let mean = |from: usize| {
            let n: u64 = self.counts[from..].iter().map(|&c| c as u64).sum();
            (n > 0).then(|| self.energy[from..].iter().sum::<f64>() / n as f64)
        };
        let loudness = |ms: f64| -0.691 + 10.0 * ms.log10();
        let ungated = loudness(mean(0)?);
        let threshold = ungated - 10.0;
        let first = (((threshold - GATE_FLOOR) / GATE_STEP).max(0.0) as usize).min(GATE_BINS - 1);
        Some(loudness(mean(first)?) as f32)
    }

    fn reset(&mut self) {
        self.parts = [0.0; 4];
        self.filled = 0;
        self.next = 0;
        self.current = 0.0;
        self.current_len = 0;
        self.counts.fill(0);
        self.energy.fill(0.0);
    }
}

// ---------------------------------------------------------------------------
// Meter snapshot (sent to UI)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct MeterSnapshot {
    pub peak_db: f32,
    pub peak_hold_db: f32,
    pub true_peak_db: f32,
    pub rms_db: f32,
    pub lufs_m: Option<f32>,
    pub lufs_s: Option<f32>,
    pub lufs_i: Option<f32>,
    pub clipped: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_db(linear: f32) -> f32 {
    (20.0 * (linear + 1e-10).log10()).clamp(-100.0, 6.0)
}

#[cfg(test)]
mod loudness_tests {
    use super::ChannelMeter;

    /// `secs` of a sine at `freq` and `db` dBFS on both sides, read through
    /// a meter in 512-frame blocks.
    fn meter_sine(meter: &mut ChannelMeter, sr: f64, freq: f64, db: f64, phase: f64, secs: f64) {
        let amp = 10f64.powf(db / 20.0);
        let n = (sr * secs) as usize;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / sr + phase).sin()) as f32
            })
            .collect();
        for block in x.chunks(512) {
            meter.process_block(block, block);
        }
    }

    #[test]
    fn integrated_loudness_of_a_known_sine() {
        // BS.1770's own reference: a 1 kHz sine at 0 dBFS in one channel
        // reads -3.01 LKFS, so in both channels its level in dBFS.
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            let mut m = ChannelMeter::new(sr);
            meter_sine(&mut m, sr, 1000.0, -6.0, 0.0, 10.0);
            let lufs = m.lufs_i().expect("ten seconds is enough to measure");
            assert!((lufs + 6.0).abs() < 0.05, "{sr} Hz: LUFS {lufs}");
        }
    }

    #[test]
    fn silence_after_the_music_does_not_pull_integrated_down() {
        let mut m = ChannelMeter::new(48_000.0);
        meter_sine(&mut m, 48_000.0, 1000.0, -20.0, 0.0, 10.0);
        let music = m.lufs_i().unwrap();
        meter_sine(&mut m, 48_000.0, 1000.0, -200.0, 0.0, 10.0);
        let with_silence = m.lufs_i().unwrap();
        // The few blocks that hold the end of the music and the start of the
        // silence count, as the standard says; the silence itself does not.
        assert!(
            (music - with_silence).abs() < 0.15,
            "{music} then {with_silence}"
        );
    }

    #[test]
    fn a_peak_between_the_samples_is_read() {
        // A quarter of the sample rate, 45 degrees off: every sample sits at
        // 0.707 of the wave's real height, 3 dB under it.
        let mut m = ChannelMeter::new(48_000.0);
        meter_sine(
            &mut m,
            48_000.0,
            12_000.0,
            0.0,
            std::f64::consts::FRAC_PI_4,
            1.0,
        );
        let tp = m.true_peak_db();
        assert!(
            tp > -0.6 && tp < 0.6,
            "true peak {tp}, the samples are at -3.0"
        );
    }

    #[test]
    fn nothing_measured_says_nothing_and_reset_forgets() {
        let mut m = ChannelMeter::new(48_000.0);
        assert_eq!(m.lufs_i(), None);
        meter_sine(&mut m, 48_000.0, 1000.0, -6.0, 0.0, 2.0);
        assert!(m.lufs_i().is_some());
        m.reset();
        assert_eq!(m.lufs_i(), None);
    }
}
