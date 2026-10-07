#![no_main]
//! An AIFF sample from a pack or a share.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hardwave_dsp::aiff_reader::decode_aiff(data);
});
