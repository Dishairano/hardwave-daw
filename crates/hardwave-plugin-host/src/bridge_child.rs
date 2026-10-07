//! The child end of the plug-in bridge: one plug-in, answering frames.
//!
//! Shared by the sandbox child (the DAW re-run with a switch) and the
//! separate helper binary (the 32-bit host on Windows), so both speak
//! the protocol the same way.
//!
//! The frames go out on a private copy of the process's standard
//! output, and standard output itself is pointed at nothing before the
//! plug-in is loaded. A plug-in that prints, which many do, then prints
//! into nothing instead of into the middle of a frame.

use std::io::{BufReader, BufWriter, Read, Write};

use crate::bridge_protocol::*;
use crate::types::HostedPlugin;

/// Take standard output for frames, leaving the plug-in a stdout that
/// goes nowhere. Call before the plug-in is loaded.
pub fn private_stdout() -> std::io::Result<std::fs::File> {
    imp::private_stdout()
}

#[cfg(unix)]
mod imp {
    use std::os::fd::FromRawFd;

    pub fn private_stdout() -> std::io::Result<std::fs::File> {
        // Safety: plain descriptor calls on this process's own stdout;
        // every result is checked before it is used.
        unsafe {
            let ours = libc::dup(1);
            if ours < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY);
            if null >= 0 {
                libc::dup2(null, 1);
                libc::close(null);
            }
            Ok(std::fs::File::from_raw_fd(ours))
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub fn private_stdout() -> std::io::Result<std::fs::File> {
        // Safety: handle calls on this process's own standard output;
        // every result is checked before it is used.
        unsafe {
            let current = GetStdHandle(STD_OUTPUT_HANDLE);
            if current == 0 || current == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut ours: HANDLE = 0;
            let process = GetCurrentProcess();
            if DuplicateHandle(
                process,
                current,
                process,
                &mut ours,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            ) == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            let nul: Vec<u16> = "NUL\0".encode_utf16().collect();
            let null = CreateFileW(
                nul.as_ptr(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            );
            if null != INVALID_HANDLE_VALUE {
                SetStdHandle(STD_OUTPUT_HANDLE, null);
            }
            Ok(std::fs::File::from_raw_handle(ours as _))
        }
    }
}

/// Answer frames for one plug-in until the host says stop or goes away.
pub fn serve(plugin: &mut dyn HostedPlugin, input: impl Read, output: impl Write) -> i32 {
    let mut reader = BufReader::new(input);
    let mut writer = BufWriter::new(output);
    let mut left = vec![0.0f32; 4096];
    let mut right = vec![0.0f32; 4096];
    let mut outputs = vec![Vec::new(), Vec::new()];

    // The host going away, or the stream ending, ends this process too:
    // there is nothing here without it.
    while let Ok(Some(frame)) = Frame::read(&mut reader) {
        match frame.kind {
            REQ_SHUTDOWN => break,
            REQ_ACTIVATE => {
                if let Some((sample_rate, max_block)) = frame.read_activate() {
                    let block = if max_block == 0 {
                        4096
                    } else {
                        max_block.min(65_536)
                    };
                    if left.len() < block as usize {
                        left.resize(block as usize, 0.0);
                        right.resize(block as usize, 0.0);
                    }
                    let _ = plugin.activate(sample_rate, block);
                }
                if Frame::new(RES_OK, Vec::new()).write(&mut writer).is_err() {
                    break;
                }
            }
            REQ_SET_PARAM => {
                if let Some((id, value)) = frame.read_set_param() {
                    plugin.set_parameter_value(id, value);
                }
            }
            REQ_SET_STATE => {
                let _ = plugin.set_state(&frame.payload);
            }
            REQ_GET_STATE => {
                if Frame::new(RES_STATE, plugin.get_state())
                    .write(&mut writer)
                    .is_err()
                {
                    break;
                }
            }
            REQ_AUDIO => {
                let frames = (frame.payload.len() / 8).min(65_536);
                if left.len() < frames {
                    left.resize(frames, 0.0);
                    right.resize(frames, 0.0);
                }
                frame.read_audio(&mut left[..frames], &mut right[..frames]);
                let inputs: [&[f32]; 2] = [&left[..frames], &right[..frames]];
                plugin.process(&inputs, &mut outputs, &[], &mut Vec::new(), frames);
                let out_l = outputs.first().map(|c| c.as_slice()).unwrap_or(&[]);
                let out_r = outputs.get(1).map(|c| c.as_slice()).unwrap_or(out_l);
                let mut reply = Frame::audio(out_l, out_r);
                reply.kind = RES_AUDIO;
                if reply.write(&mut writer).is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
    0
}
