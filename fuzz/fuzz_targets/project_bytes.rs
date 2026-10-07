#![no_main]
//! A song from anywhere: disk, Workspace, a room partner.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(mut project) = hardwave_project::Project::from_bytes(data) {
        // Whatever opened must also survive being made safe again and
        // written back out.
        let _ = project.make_safe();
        let _ = project.to_bytes();
    }
});
