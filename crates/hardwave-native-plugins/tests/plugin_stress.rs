//! Stress tests — instantiate 50+ real `HostedPlugin` instances
//! (NativeEq + NativeCompressor) and run process() on each to prove
//! the host can actually sustain the roadmap's 50+ plugin claim
//! rather than just declaring the contract.

use hardwave_native_plugins::{NativeCompressor, NativeEq};
use hardwave_plugin_host::types::HostedPlugin;

const SAMPLE_RATE: f64 = 48_000.0;
const BLOCK_SIZE: usize = 512;

fn sine_block(freq: f32, sr: f32, phase: &mut f32) -> Vec<f32> {
    let step = 2.0 * std::f32::consts::PI * freq / sr;
    (0..BLOCK_SIZE)
        .map(|_| {
            let s = phase.sin();
            *phase += step;
            if *phase > std::f32::consts::TAU {
                *phase -= std::f32::consts::TAU;
            }
            s
        })
        .collect()
}

#[test]
fn fifty_native_plugins_process_blocks_without_panic() {
    // 25 EQs + 25 Compressors = 50 plugin instances.
    let mut eqs: Vec<NativeEq> = (0..25).map(|_| NativeEq::new()).collect();
    let mut comps: Vec<NativeCompressor> = (0..25).map(|_| NativeCompressor::new()).collect();

    for eq in eqs.iter_mut() {
        eq.activate(SAMPLE_RATE, BLOCK_SIZE as u32).unwrap();
        // Enable the low-shelf band at +3 dB so it's actually doing
        // work rather than a bypass pass-through.
        eq.set_parameter_value(0, 1.0);
        eq.set_parameter_value(2, 3.0);
    }
    for c in comps.iter_mut() {
        c.activate(SAMPLE_RATE, BLOCK_SIZE as u32).unwrap();
        c.set_parameter_value(0, -12.0); // threshold
        c.set_parameter_value(1, 3.0); // ratio 3:1
    }

    let mut phase = 0.0_f32;
    let mut midi_out = Vec::new();
    let mut any_nan = false;
    let mut any_inf = false;
    for _block in 0..200 {
        let input = sine_block(440.0, SAMPLE_RATE as f32, &mut phase);
        for eq in eqs.iter_mut() {
            let mut outputs = vec![Vec::new(), Vec::new()];
            eq.process(&[&input, &input], &mut outputs, &[], &mut midi_out, BLOCK_SIZE);
            for ch in outputs {
                for s in ch {
                    if s.is_nan() {
                        any_nan = true;
                    }
                    if s.is_infinite() {
                        any_inf = true;
                    }
                }
            }
        }
        for c in comps.iter_mut() {
            let mut outputs = vec![Vec::new(), Vec::new()];
            c.process(&[&input, &input], &mut outputs, &[], &mut midi_out, BLOCK_SIZE);
            for ch in outputs {
                for s in ch {
                    if s.is_nan() {
                        any_nan = true;
                    }
                    if s.is_infinite() {
                        any_inf = true;
                    }
                }
            }
        }
    }
    assert!(!any_nan, "50 plugins produced NaN samples");
    assert!(!any_inf, "50 plugins produced infinite samples");
}

#[test]
fn fifty_plugin_chain_processes_sequentially_without_nan() {
    // 50 EQs processing in series — each one's output feeds the next.
    let mut eqs: Vec<NativeEq> = (0..50).map(|_| NativeEq::new()).collect();
    for eq in eqs.iter_mut() {
        eq.activate(SAMPLE_RATE, BLOCK_SIZE as u32).unwrap();
    }
    let mut phase = 0.0_f32;
    let mut midi_out = Vec::new();
    for _block in 0..50 {
        let mut signal_l = sine_block(880.0, SAMPLE_RATE as f32, &mut phase);
        let mut signal_r = signal_l.clone();
        for eq in eqs.iter_mut() {
            let mut outputs = vec![Vec::new(), Vec::new()];
            eq.process(
                &[&signal_l, &signal_r],
                &mut outputs,
                &[],
                &mut midi_out,
                BLOCK_SIZE,
            );
            signal_l = outputs[0].clone();
            signal_r = outputs[1].clone();
        }
        for s in signal_l.iter().chain(signal_r.iter()) {
            assert!(s.is_finite(), "chain produced non-finite sample");
        }
    }
}
