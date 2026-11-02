//! Makes the drum one-shots that ship with the DAW (src-tauri/drums).
//!
//! A new project opens with Clap, Hat and Snare channels next to the
//! kick synth, and these are the sounds in them, plus a few
//! alternatives in the same folder. They are synthesised here rather
//! than recorded, so they are ours, small, and can be remade or retuned
//! from this file:
//!
//!     hw-daw-cargo run -p hardwave-dsp --example make_drum_kit -- src-tauri/drums
//!
//! Add `--report` to print each sound's envelope and spectrum instead
//! of writing files. The noise comes from a fixed seed, so running it
//! again writes the same sounds.

use hardwave_dsp::biquad::{Biquad, BiquadKind};
use rustfft::{num_complex::Complex32, FftPlanner};
use std::f32::consts::PI;
use std::path::Path;

const SR: f32 = 44_100.0;
/// Every sound peaks here: loud enough to sit next to the kick synth,
/// with room left for the sampler's own gain.
const PEAK_DBFS: f32 = -1.0;

/// One sound in the kit: its file name and how it is made.
struct Sound {
    file: &'static str,
    make: fn() -> Vec<f32>,
}

const KIT: [Sound; 8] = [
    Sound {
        file: "Clap 1.wav",
        make: clap_tight,
    },
    Sound {
        file: "Clap 2.wav",
        make: clap_wide,
    },
    Sound {
        file: "Hat 1.wav",
        make: hat_closed,
    },
    Sound {
        file: "Hat 2.wav",
        make: hat_tick,
    },
    Sound {
        file: "Open hat 1.wav",
        make: hat_open,
    },
    Sound {
        file: "Snare 1.wav",
        make: snare_body,
    },
    Sound {
        file: "Snare 2.wav",
        make: snare_tight,
    },
    Sound {
        file: "Rim 1.wav",
        make: rim,
    },
];

/// A small fixed-seed noise source (xorshift32), so the kit is the
/// same every time it is made.
struct Noise(u32);

impl Noise {
    fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn filter(kind: BiquadKind, hz: f32, q: f32, gain_db: f32) -> Biquad {
    let mut f = Biquad::default();
    f.set(kind, SR, hz, q, gain_db);
    f
}

fn samples(secs: f32) -> usize {
    (secs * SR) as usize
}

/// Seconds since the start, for sample `i`.
fn time(i: usize) -> f32 {
    i as f32 / SR
}

/// A hit that rises in `attack` seconds and falls away with time
/// constant `tau`, starting `at` seconds in.
fn hit(t: f32, at: f32, attack: f32, tau: f32) -> f32 {
    let t = t - at;
    if t < 0.0 {
        return 0.0;
    }
    let rise = if attack > 0.0 {
        (t / attack).min(1.0)
    } else {
        1.0
    };
    rise * (-t / tau).exp()
}

/// A square wave built from its odd harmonics below 18 kHz, so the
/// metal in the hats has no aliasing whistle in it.
fn soft_square(phase: f32, hz: f32) -> f32 {
    let mut sum = 0.0;
    let mut k = 1.0;
    while hz * k < 18_000.0 {
        sum += (phase * k).sin() / k;
        k += 2.0;
    }
    sum * 4.0 / PI
}

/// The clap: a few noise bursts a few milliseconds apart (several hands
/// that do not quite land together), then a short tail, through a
/// band around 1.2 kHz with a brighter layer on top for the snap.
fn clap(bursts: &[(f32, f32)], tail_tau: f32, secs: f32, centre_hz: f32, seed: u32) -> Vec<f32> {
    let mut noise = Noise::new(seed);
    let mut body = filter(BiquadKind::BandPass, centre_hz, 1.1, 0.0);
    let mut snap = filter(BiquadKind::BandPass, 4_200.0, 0.9, 0.0);
    let mut low_cut = filter(BiquadKind::HighPass, 380.0, 0.7, 0.0);
    let mut air = filter(BiquadKind::LowPass, 11_000.0, 0.7, 0.0);
    let last = bursts.last().map(|b| b.0).unwrap_or(0.0);
    (0..samples(secs))
        .map(|i| {
            let t = time(i);
            let bursts: f32 = bursts
                .iter()
                .map(|&(at, level)| level * hit(t, at, 0.0003, 0.0032))
                .sum();
            let tail = 0.55 * hit(t, last, 0.001, tail_tau);
            let n = noise.next() * (bursts + tail);
            let x = body.process_mono(n) * 1.6 + snap.process_mono(n) * 0.7;
            // A little drive packs the bursts together, as a hard
            // clap in the genre sounds; the drive's fizz on top goes.
            air.process_mono((low_cut.process_mono(x) * 1.8).tanh())
        })
        .collect()
}

fn clap_tight() -> Vec<f32> {
    clap(
        &[(0.0, 0.75), (0.0085, 0.85), (0.0175, 0.8), (0.0265, 1.0)],
        0.042,
        0.30,
        1_250.0,
        0x00C1_A901,
    )
}

fn clap_wide() -> Vec<f32> {
    clap(
        &[
            (0.0, 0.6),
            (0.011, 0.8),
            (0.0215, 0.7),
            (0.0335, 0.9),
            (0.044, 1.0),
        ],
        0.075,
        0.45,
        980.0,
        0x00C1_A902,
    )
}

/// The metal in a hat: six detuned square waves at the ratios of the
/// classic analogue hat, with noise, high-passed so only the shimmer
/// is left.
fn hat(tau: f32, secs: f32, metal: f32, high_pass_hz: f32, seed: u32) -> Vec<f32> {
    const RATIOS: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
    let mut noise = Noise::new(seed);
    let mut band = filter(BiquadKind::BandPass, 10_000.0, 0.8, 0.0);
    let mut hp1 = filter(BiquadKind::HighPass, high_pass_hz, 0.7, 0.0);
    let mut hp2 = filter(BiquadKind::HighPass, high_pass_hz, 0.7, 0.0);
    let mut phases = [0.0f32; 6];
    (0..samples(secs))
        .map(|i| {
            let t = time(i);
            let mut m = 0.0;
            for (phase, ratio) in phases.iter_mut().zip(RATIOS) {
                let hz = ratio * 1.7;
                m += soft_square(*phase, hz);
                *phase = (*phase + 2.0 * PI * hz / SR) % (2.0 * PI);
            }
            let source = metal * m / 6.0 + (1.0 - metal) * noise.next();
            let env = hit(t, 0.0, 0.0002, tau);
            let x = band.process_mono(source) * 0.6 + source * 0.4;
            hp2.process_mono(hp1.process_mono(x)) * env * 2.0
        })
        .collect()
}

fn hat_closed() -> Vec<f32> {
    hat(0.017, 0.12, 0.55, 7_000.0, 0x0000_4A71)
}

fn hat_tick() -> Vec<f32> {
    hat(0.010, 0.08, 0.3, 9_000.0, 0x0000_4A72)
}

fn hat_open() -> Vec<f32> {
    hat(0.11, 0.60, 0.55, 6_500.0, 0x0000_4A73)
}

/// The snare: a drum body (two modes that drop in pitch as they are
/// struck), a burst of bright noise for the wires, and a click on top.
fn snare(
    body_hz: f32,
    body_tau: f32,
    noise_tau: f32,
    wires_hz: f32,
    secs: f32,
    seed: u32,
) -> Vec<f32> {
    let mut noise = Noise::new(seed);
    let mut wires = filter(BiquadKind::HighPass, wires_hz, 0.7, 0.0);
    let mut sizzle = filter(BiquadKind::Peak, 5_200.0, 0.9, 4.0);
    let mut hiss = filter(BiquadKind::LowPass, 10_000.0, 0.7, 0.0);
    let mut click_hp = filter(BiquadKind::HighPass, 4_000.0, 0.7, 0.0);
    let (mut p1, mut p2) = (0.0f32, 0.0f32);
    (0..samples(secs))
        .map(|i| {
            let t = time(i);
            let drop = (-t / 0.008).exp();
            let f1 = body_hz * (1.0 + 0.6 * drop);
            let f2 = body_hz * 1.74 * (1.0 + 0.4 * drop);
            p1 += 2.0 * PI * f1 / SR;
            p2 += 2.0 * PI * f2 / SR;
            let body = p1.sin() * hit(t, 0.0, 0.0005, body_tau)
                + 0.5 * p2.sin() * hit(t, 0.0, 0.0005, body_tau * 0.7);
            let n = noise.next();
            let wire = hiss.process_mono(sizzle.process_mono(wires.process_mono(n)))
                * hit(t, 0.0005, 0.001, noise_tau);
            let click = click_hp.process_mono(n) * hit(t, 0.0, 0.0, 0.0015);
            ((body * 0.8 + wire * 1.3 + click * 0.5) * 1.5).tanh()
        })
        .collect()
}

fn snare_body() -> Vec<f32> {
    snare(190.0, 0.055, 0.062, 1_800.0, 0.40, 0x054A_2E01)
}

fn snare_tight() -> Vec<f32> {
    snare(230.0, 0.038, 0.048, 2_500.0, 0.30, 0x054A_2E02)
}

/// The rim: two short, high, wooden modes and a click.
fn rim() -> Vec<f32> {
    let mut noise = Noise::new(0x0000_B111);
    let mut click_band = filter(BiquadKind::BandPass, 3_000.0, 1.5, 0.0);
    (0..samples(0.12))
        .map(|i| {
            let t = time(i);
            let tone = (2.0 * PI * 1_650.0 * t).sin() * hit(t, 0.0, 0.0002, 0.010)
                + 0.6 * (2.0 * PI * 480.0 * t).sin() * hit(t, 0.0, 0.0002, 0.014);
            let click = click_band.process_mono(noise.next()) * hit(t, 0.0, 0.0, 0.002);
            (tone + click * 1.5).tanh()
        })
        .collect()
}

/// The last steps every sound goes through: no DC, no silence at the
/// end, a short fade so the last sample is zero, and the peak at
/// `PEAK_DBFS`.
fn finish(mut x: Vec<f32>) -> Vec<f32> {
    let mut dc = filter(BiquadKind::HighPass, 20.0, 0.7, 0.0);
    for s in x.iter_mut() {
        *s = dc.process_mono(*s);
    }
    let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let floor = peak * 10f32.powf(-72.0 / 20.0);
    let end = x.iter().rposition(|s| s.abs() > floor).map_or(0, |p| p + 1);
    x.truncate(end);
    let fade = samples(0.005).min(x.len());
    let n = x.len();
    for (k, s) in x[n - fade..].iter_mut().enumerate() {
        *s *= 1.0 - (k + 1) as f32 / fade as f32;
    }
    let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let gain = 10f32.powf(PEAK_DBFS / 20.0) / peak.max(1e-9);
    for s in x.iter_mut() {
        *s *= gain;
    }
    x
}

fn make(sound: &Sound) -> Vec<f32> {
    finish((sound.make)())
}

fn write(path: &Path, x: &[f32]) -> Result<(), hound::Error> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR as u32,
        // 16-bit: every WAV reader takes it, and with the peak at -1 dBFS
        // a one-shot has all the range it needs.
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for s in x {
        w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)?;
    }
    w.finalize()
}

/// Level in dB over 1 ms windows, the envelope a listener hears.
fn envelope_db(x: &[f32]) -> Vec<f32> {
    x.chunks(samples(0.001).max(1))
        .map(|w| {
            let rms = (w.iter().map(|s| s * s).sum::<f32>() / w.len() as f32).sqrt();
            20.0 * rms.max(1e-9).log10()
        })
        .collect()
}

/// Milliseconds until the 1 ms level has fallen `db` below its loudest
/// moment and stays there.
fn decay_ms(x: &[f32], db: f32) -> usize {
    let env = envelope_db(x);
    let top = env.iter().cloned().fold(f32::MIN, f32::max);
    env.iter().rposition(|&e| e > top - db).map_or(0, |p| p + 1)
}

/// Share of the energy in each band, from one FFT over the whole sound.
fn band_shares(x: &[f32], bands: &[(f32, f32)]) -> Vec<f32> {
    let n = x.len().next_power_of_two().max(1024);
    let mut buf: Vec<Complex32> = x.iter().map(|&s| Complex32::new(s, 0.0)).collect();
    buf.resize(n, Complex32::new(0.0, 0.0));
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    let power: Vec<(f32, f32)> = buf[..n / 2]
        .iter()
        .enumerate()
        .map(|(k, c)| (k as f32 * SR / n as f32, c.norm_sqr()))
        .collect();
    let total: f32 = power.iter().map(|p| p.1).sum::<f32>().max(1e-12);
    bands
        .iter()
        .map(|&(lo, hi)| {
            power
                .iter()
                .filter(|p| p.0 >= lo && p.0 < hi)
                .map(|p| p.1)
                .sum::<f32>()
                / total
        })
        .collect()
}

fn centroid_hz(x: &[f32]) -> f32 {
    let n = x.len().next_power_of_two().max(1024);
    let mut buf: Vec<Complex32> = x.iter().map(|&s| Complex32::new(s, 0.0)).collect();
    buf.resize(n, Complex32::new(0.0, 0.0));
    FftPlanner::new().plan_fft_forward(n).process(&mut buf);
    let (mut num, mut den) = (0.0f32, 0.0f32);
    for (k, c) in buf[..n / 2].iter().enumerate() {
        let p = c.norm_sqr();
        num += p * k as f32 * SR / n as f32;
        den += p;
    }
    num / den.max(1e-12)
}

/// The separate hands in a clap: 1 ms envelope peaks in the first
/// 40 ms that stand out from the dips around them.
fn hits_in_first_40ms(x: &[f32]) -> usize {
    let env = envelope_db(x);
    let head = &env[..env.len().min(40)];
    (1..head.len().saturating_sub(1))
        .filter(|&i| head[i] > head[i - 1] && head[i] >= head[i + 1])
        .filter(|&i| {
            let before = head[i.saturating_sub(5)..i]
                .iter()
                .cloned()
                .fold(f32::MAX, f32::min);
            head[i] - before > 3.0
        })
        .count()
}

const BANDS: [(f32, f32); 6] = [
    (0.0, 150.0),
    (150.0, 500.0),
    (500.0, 2_000.0),
    (2_000.0, 6_000.0),
    (6_000.0, 12_000.0),
    (12_000.0, 22_050.0),
];

fn report(name: &str, x: &[f32]) {
    let shares = band_shares(x, &BANDS);
    let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    println!(
        "{name:<16} {:>4} ms  peak {:>5.2} dBFS  -20 dB at {:>3} ms  -40 dB at {:>3} ms  centroid {:>5.0} Hz  hits {}",
        x.len() * 1000 / SR as usize,
        20.0 * peak.log10(),
        decay_ms(x, 20.0),
        decay_ms(x, 40.0),
        centroid_hz(x),
        hits_in_first_40ms(x),
    );
    println!(
        "{:<16} energy  <150 {:>4.1}%  150-500 {:>4.1}%  0.5-2k {:>4.1}%  2-6k {:>4.1}%  6-12k {:>4.1}%  >12k {:>4.1}%",
        "",
        shares[0] * 100.0,
        shares[1] * 100.0,
        shares[2] * 100.0,
        shares[3] * 100.0,
        shares[4] * 100.0,
        shares[5] * 100.0,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let only_report = args.iter().any(|a| a == "--report");
    let out = args.iter().find(|a| !a.starts_with("--"));
    for sound in &KIT {
        let x = make(sound);
        report(sound.file, &x);
        if only_report {
            continue;
        }
        let Some(dir) = out else {
            eprintln!("usage: make_drum_kit <folder> | --report");
            std::process::exit(2);
        };
        std::fs::create_dir_all(dir).expect("make the folder");
        write(&Path::new(dir).join(sound.file), &x).expect("write the sound");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(file: &str) -> Vec<f32> {
        make(KIT.iter().find(|s| s.file == file).expect(file))
    }

    fn share(x: &[f32], lo: f32, hi: f32) -> f32 {
        band_shares(x, &[(lo, hi)])[0]
    }

    #[test]
    fn every_sound_peaks_at_minus_one_db_and_starts_on_its_hit() {
        for s in &KIT {
            let x = make(s);
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                (20.0 * peak.log10() - PEAK_DBFS).abs() < 0.05,
                "{} peak",
                s.file
            );
            let head = x[..samples(0.005)]
                .iter()
                .fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                head > peak * 0.25,
                "{} has its hit in the first 5 ms",
                s.file
            );
            assert_eq!(*x.last().unwrap(), 0.0, "{} ends on silence", s.file);
        }
    }

    #[test]
    fn the_kit_is_the_same_every_time_it_is_made() {
        for s in &KIT {
            assert_eq!(make(s), make(s), "{}", s.file);
        }
    }

    #[test]
    fn the_whole_kit_stays_well_under_a_megabyte() {
        let bytes: usize = KIT.iter().map(|s| 44 + make(s).len() * 2).sum();
        assert!(bytes < 600_000, "{bytes} bytes");
    }

    #[test]
    fn the_clap_is_tight_layered_and_in_the_mids() {
        let x = sound("Clap 1.wav");
        assert!(hits_in_first_40ms(&x) >= 3, "you hear the separate hands");
        assert!(
            decay_ms(&x, 40.0) < 250,
            "a short tail: {} ms",
            decay_ms(&x, 40.0)
        );
        assert!(
            share(&x, 500.0, 6_000.0) > 0.6,
            "the clap lives in the mids"
        );
        assert!(
            share(&x, 0.0, 300.0) < 0.03,
            "and leaves the low end to the kick"
        );
    }

    #[test]
    fn the_closed_hats_are_crisp_and_short() {
        for file in ["Hat 1.wav", "Hat 2.wav"] {
            let x = sound(file);
            assert!(share(&x, 6_000.0, 22_050.0) > 0.7, "{file} is bright");
            assert!(
                decay_ms(&x, 40.0) < 110,
                "{file} is closed: {} ms",
                decay_ms(&x, 40.0)
            );
        }
        let open = sound("Open hat 1.wav");
        assert!(decay_ms(&open, 40.0) > 300, "the open hat rings");
    }

    #[test]
    fn the_snare_has_a_body_and_wires() {
        for file in ["Snare 1.wav", "Snare 2.wav"] {
            let x = sound(file);
            assert!(share(&x, 150.0, 500.0) > 0.12, "{file} has a body");
            assert!(
                share(&x, 2_000.0, 22_050.0) > 0.25,
                "{file} has the noise of the wires"
            );
            assert!(
                share(&x, 0.0, 120.0) < 0.05,
                "{file} stays out of the kick's way"
            );
        }
    }
}
