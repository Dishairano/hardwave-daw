//! Reading what a CLAP library contains, in a process of its own.
//!
//! A CLAP declares its plug-ins from code: the library has to be loaded
//! and run to list them. Doing that in the DAW meant every .clap in a
//! plug-in folder ran inside the DAW at every launch, before the crash
//! probe or the sandbox had any say. Here it runs in a child: this same
//! binary with a switch, which loads the library, prints the list as
//! JSON and exits. A library that crashes or hangs costs that child.

use hardwave_plugin_host::clap_ffi::ReadDescriptor;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

/// How long a library may take to say what it contains.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(10);
/// The most a description may be; a real one is a few kilobytes.
const MOST_OUTPUT: u64 = 1024 * 1024;

/// What a CLAP library contains, read by a child process, or nothing
/// when the child crashed, hung or said something unreadable.
pub fn describe_out_of_process(library: &Path) -> Option<Vec<ReadDescriptor>> {
    use std::process::{Command, Stdio};
    let exe = std::env::current_exe().ok()?;
    let mut child = Command::new(&exe)
        .arg("--describe-clap")
        .arg(library)
        .current_dir(exe.parent().unwrap_or(Path::new(".")))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut out = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut out).take(MOST_OUTPUT).read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + DESCRIBE_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                log::warn!(
                    "{} could not be read; it is listed by name only",
                    library.display()
                );
                return None;
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                log::warn!("{} took too long to say what it is", library.display());
                return None;
            }
        }
    }
    let bytes = reader.join().ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The child: load one library, print what it contains, exit.
pub fn run_describe_child(library: &str) -> i32 {
    // What the library prints while it loads goes nowhere; the answer
    // goes on a private copy of stdout.
    let mut out = match hardwave_plugin_host::bridge_child::private_stdout() {
        Ok(out) => out,
        Err(_) => return 4,
    };
    let list = hardwave_plugin_host::clap_ffi::read_clap_descriptors(Path::new(library))
        .unwrap_or_default();
    match serde_json::to_vec(&list) {
        Ok(json) if out.write_all(&json).is_ok() => 0,
        _ => 3,
    }
}
