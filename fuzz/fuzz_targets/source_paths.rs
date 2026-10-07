#![no_main]
//! Sample paths a project names.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(path) = std::str::from_utf8(data) {
        let dir = std::path::Path::new("/songs/Song");
        let _ = hardwave_engine::source_paths::check(path, Some(dir), &["nas".to_string()]);
        let _ = hardwave_engine::source_paths::check_video(path, None, &[]);
        let _ = hardwave_engine::source_paths::network_server(path);
    }
});
