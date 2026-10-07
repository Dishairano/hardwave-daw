#![no_main]
//! What a crashed or hostile plug-in process writes back to the DAW.
use hardwave_plugin_host::bridge_protocol::Frame;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut input = std::io::Cursor::new(data);
    while let Ok(Some(frame)) = Frame::read(&mut input) {
        let mut left = [0.0f32; 512];
        let mut right = [0.0f32; 512];
        let _ = frame.read_audio(&mut left, &mut right);
        let _ = frame.read_set_param();
        let _ = frame.read_activate();
    }
});
