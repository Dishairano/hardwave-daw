#![no_main]
//! An FL Studio project someone sent.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = hardwave_project::flp_parser::parse(data);
});
