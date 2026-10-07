#![no_main]
//! A packet on the OSC port.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hardwave_midi::osc::OscMessage::from_bytes(data);
});
