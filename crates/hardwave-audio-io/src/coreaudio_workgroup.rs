//! macOS CoreAudio workgroup integration.
//!
//! macOS real-time audio threads perform best when they join the workgroup
//! advertised by the audio device (`kAudioDevicePropertyIOThreadOSWorkgroup`).
//! Joining the workgroup gives the kernel scheduler the context it needs to
//! co-schedule the audio thread with related workloads (plugin DSP, UI) so
//! the whole pipeline meets its deadline.
//!
//! This module is linked only on macOS and activated only when
//! `AudioDeviceManager` is asked for CoreAudio workgroup pinning.

#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::mem;
use std::os::raw::{c_int, c_uint};

// -- CoreAudio FFI ----------------------------------------------------------

const K_AUDIO_OBJECT_SYSTEM_OBJECT: u32 = 1;
const K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE: u32 = fourcc(b"dOut");
const K_AUDIO_DEVICE_PROPERTY_IO_THREAD_OS_WORKGROUP: u32 = fourcc(b"oswg");
const K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: u32 = fourcc(b"glob");
const K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: u32 = 0;

#[repr(C)]
struct AudioObjectPropertyAddress {
    selector: u32,
    scope: u32,
    element: u32,
}

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectGetPropertyData(
        in_object_id: u32,
        in_address: *const AudioObjectPropertyAddress,
        in_qualifier_size: u32,
        in_qualifier_data: *const c_void,
        io_data_size: *mut u32,
        out_data: *mut c_void,
    ) -> i32;
}

// -- libdispatch / os_workgroup FFI -----------------------------------------

#[repr(C)]
pub struct OsWorkgroup(*mut c_void);

// Apple's `os_workgroup_join_token_s` layout is an opaque buffer; the
// SDK header documents it as eight uint64_t reserved slots. Matching that
// exactly so os_workgroup_leave can write through the pointer.
#[repr(C)]
#[derive(Default)]
pub struct OsWorkgroupJoinToken {
    _reserved: [u64; 8],
}

#[link(name = "System", kind = "dylib")]
extern "C" {
    fn os_workgroup_join(wg: *mut c_void, token: *mut OsWorkgroupJoinToken) -> c_int;
    fn os_workgroup_leave(wg: *mut c_void, token: *mut OsWorkgroupJoinToken);
    fn os_release(obj: *mut c_void);
}

// -- Public API -------------------------------------------------------------

/// Handle returned after successfully joining a CoreAudio workgroup. Drop
/// leaves the workgroup and releases the underlying object.
pub struct WorkgroupMembership {
    workgroup: *mut c_void,
    token: OsWorkgroupJoinToken,
}

// The workgroup pointer is opaque; it's only used from the audio thread
// that created this handle. We mark it Send so the AudioDeviceManager can
// hold it, but the join/leave calls happen on the audio thread only.
unsafe impl Send for WorkgroupMembership {}

impl Drop for WorkgroupMembership {
    fn drop(&mut self) {
        unsafe {
            if !self.workgroup.is_null() {
                os_workgroup_leave(self.workgroup, &mut self.token);
                os_release(self.workgroup);
            }
        }
    }
}

/// Fetch the current default-output device's IO-thread workgroup and join it.
/// Returns `None` if macOS didn't advertise a workgroup for the device (which
/// happens on older hardware or virtual devices). Safe to call: any failure
/// degrades to "no workgroup, carry on in shared scheduling".
pub fn join_default_output_workgroup() -> Option<WorkgroupMembership> {
    unsafe {
        // 1. Find the default output device.
        let addr = AudioObjectPropertyAddress {
            selector: K_AUDIO_HARDWARE_PROPERTY_DEFAULT_OUTPUT_DEVICE,
            scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
            element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
        };
        let mut device_id: u32 = 0;
        let mut size = mem::size_of::<u32>() as u32;
        let status = AudioObjectGetPropertyData(
            K_AUDIO_OBJECT_SYSTEM_OBJECT,
            &addr,
            0,
            std::ptr::null(),
            &mut size,
            &mut device_id as *mut u32 as *mut c_void,
        );
        if status != 0 || device_id == 0 {
            log::info!("CoreAudio: no default output device (status={status})");
            return None;
        }

        // 2. Ask the device for its IO-thread workgroup.
        let wg_addr = AudioObjectPropertyAddress {
            selector: K_AUDIO_DEVICE_PROPERTY_IO_THREAD_OS_WORKGROUP,
            scope: K_AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
            element: K_AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
        };
        let mut workgroup: *mut c_void = std::ptr::null_mut();
        let mut wg_size = mem::size_of::<*mut c_void>() as u32;
        let status = AudioObjectGetPropertyData(
            device_id,
            &wg_addr,
            0,
            std::ptr::null(),
            &mut wg_size,
            &mut workgroup as *mut *mut c_void as *mut c_void,
        );
        if status != 0 || workgroup.is_null() {
            log::info!("CoreAudio: device {device_id} has no IO workgroup (status={status})");
            return None;
        }

        // 3. Join it from the current thread. Must be called on the audio
        //    callback thread for the scheduling hints to apply.
        let mut token = OsWorkgroupJoinToken::default();
        let rc: c_uint = os_workgroup_join(workgroup, &mut token) as c_uint;
        if rc != 0 {
            log::warn!("CoreAudio: os_workgroup_join failed ({rc})");
            os_release(workgroup);
            return None;
        }

        log::info!("CoreAudio: joined IO workgroup for device {device_id}");
        Some(WorkgroupMembership { workgroup, token })
    }
}
