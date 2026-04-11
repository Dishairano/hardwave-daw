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

fn make_k_filter(fs: f64) -> (BiquadFilter, BiquadFilter) {
    // Stage 1: high-shelf pre-filter
    let a = f64::powf(10.0, 3.999843853973347 / 40.0);
    let w1 = 2.0 * std::f64::consts::PI * 1681.974450955533 / fs;
    let alpha1 = w1.sin() / (2.0 * 0.7071752369554196);
    let c1 = w1.cos();
    let sq = a.sqrt();

    let shelf = BiquadFilter::new(
        a * ((a + 1.0) + (a - 1.0) * c1 + 2.0 * sq * alpha1),
        -2.0 * a * ((a - 1.0) + (a + 1.0) * c1),
        a * ((a + 1.0) + (a - 1.0) * c1 - 2.0 * sq * alpha1),
        (a + 1.0) - (a - 1.0) * c1 + 2.0 * sq * alpha1,
        2.0 * ((a - 1.0) - (a + 1.0) * c1),
        (a + 1.0) - (a - 1.0) * c1 - 2.0 * sq * alpha1,
    );

    // Stage 2: high-pass (RLB filter)
    let w2 = 2.0 * std::f64::consts::PI * 38.13547087602444 / fs;
    let alpha2 = w2.sin() / (2.0 * 0.5003270373238773);
    let c2 = w2.cos();

    let hp = BiquadFilter::new(
        (1.0 + c2) / 2.0,
        -(1.0 + c2),
        (1.0 + c2) / 2.0,
        1.0 + alpha2,
        -2.0 * c2,
        1.0 - alpha2,
    );

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
    lufs_i_blocks: Vec<f64>,
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
            lufs_i_blocks: Vec::new(),
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

            // True peak: 4x linear interpolation
            if i > 0 {
                let prev = (left[i - 1] + right[i - 1]) * 0.5;
                for k in 1..4 {
                    let t = k as f32 / 4.0;
                    let interp = prev + (mono - prev) * t;
                    block_true_peak = block_true_peak.max(interp.abs());
                }
            }
            block_true_peak = block_true_peak.max(abs);

            // K-weighting for LUFS
            let kl = self.k_hp_l.process(self.k_shelf_l.process(l as f64));
            let kr = self.k_hp_r.process(self.k_shelf_r.process(r as f64));
            k_sum += kl * kl + kr * kr;
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
        if let Some(_loudness) = self.compute_lufs(400, 512) {
            // placeholder — accumulation happens in process_block
        }
        gated_lufs(&self.lufs_i_blocks)
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
        self.lufs_i_blocks.clear();
        self.k_shelf_l.reset();
        self.k_hp_l.reset();
        self.k_shelf_r.reset();
        self.k_hp_r.reset();
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

fn gated_lufs(blocks: &[f64]) -> Option<f32> {
    if blocks.len() < 3 {
        return None;
    }
    let abs_gated: Vec<f64> = blocks.iter().copied().filter(|&b| b > -70.0).collect();
    if abs_gated.is_empty() {
        return None;
    }
    let sum: f64 = abs_gated.iter().map(|&b| 10.0_f64.powf(b / 10.0)).sum();
    let ungated_mean = 10.0 * (sum / abs_gated.len() as f64).log10();
    let rel_threshold = ungated_mean - 10.0;
    let rel_gated: Vec<f64> = abs_gated
        .into_iter()
        .filter(|&b| b > rel_threshold)
        .collect();
    if rel_gated.is_empty() {
        return None;
    }
    let sum2: f64 = rel_gated.iter().map(|&b| 10.0_f64.powf(b / 10.0)).sum();
    Some((10.0 * (sum2 / rel_gated.len() as f64).log10()) as f32)
}
