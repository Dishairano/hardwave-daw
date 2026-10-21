//! Where a thread is, as a list of module-and-offset frames, for the log.
//!
//! The main-thread watchdog (backend_stats) could only say that the main thread had stopped
//! answering and which command, if any, was running. The overnight soak on the test VM
//! (19-20 Oct 2026) logged freezes of 2 to 14 s at RDP reconnects and at a sign-in in another
//! session, with no command running, and nothing said what the thread was doing. A stack at
//! that moment does: a plug-in's DLL, WebView2, a nested message loop in user32, a wait in ntdll.
//!
//! Only return addresses are taken, as module plus offset: no memory, no file contents, nothing
//! from the project. Our own frames are offsets into the stripped executable of that version.

/// Snapshots per run of the app: enough to see a pattern, not enough to fill the log.
pub const PER_RUN: u32 = 10;

/// One frame: the module the address is in, and the offset from the module's start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub module: Option<String>,
    pub address: usize,
    pub offset: usize,
}

/// The frames as one log line, innermost first. A run of the same frame (a loop that calls
/// itself) is written once with a count.
pub fn describe(frames: &[Frame]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < frames.len() {
        let f = &frames[i];
        let mut n = 1;
        while i + n < frames.len() && frames[i + n] == *f {
            n += 1;
        }
        let one = match &f.module {
            Some(m) => format!("{m}+0x{:x}", f.offset),
            None => format!("0x{:x}", f.address),
        };
        parts.push(if n > 1 { format!("{one} (x{n})") } else { one });
        i += n;
    }
    if parts.is_empty() {
        "(no frames)".to_string()
    } else {
        parts.join(" < ")
    }
}

#[cfg(windows)]
pub use windows_stack::{capture, main_thread, others, remember_main_thread};

/// Elsewhere there is no stack to take; the watchdog says so in its log line.
#[cfg(not(windows))]
pub fn main_thread() -> Option<isize> {
    None
}

#[cfg(not(windows))]
pub fn remember_main_thread() {}

#[cfg(not(windows))]
pub fn capture(_thread: isize) -> Vec<Frame> {
    Vec::new()
}

#[cfg(not(windows))]
pub fn others(_exe: &str) -> Vec<(String, Vec<Frame>)> {
    Vec::new()
}

#[cfg(windows)]
mod windows_stack {
    use super::Frame;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, HANDLE, HMODULE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind, CONTEXT, CONTEXT_FULL_AMD64,
        UNW_FLAG_NHANDLER,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, GetCurrentThreadId, GetThreadDescription, GetThreadPriority,
        OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_PRIORITY_NORMAL,
        THREAD_QUERY_INFORMATION, THREAD_SUSPEND_RESUME,
    };

    /// Frames taken at most; deeper than this is the message loop's own plumbing.
    const MAX_FRAMES: usize = 48;

    static MAIN: OnceLock<isize> = OnceLock::new();
    static MAIN_ID: OnceLock<u32> = OnceLock::new();

    /// `GetThreadContext` wants the context on a 16-byte boundary, which the binding's own type
    /// does not ask for.
    #[repr(C, align(16))]
    struct AlignedContext(CONTEXT);

    /// Keep a handle to the calling thread as the main thread. Call on the main thread, once.
    pub fn remember_main_thread() {
        // SAFETY: plain calls; the handle is kept for the life of the process.
        let handle = unsafe {
            OpenThread(
                THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                0,
                GetCurrentThreadId(),
            )
        };
        if !handle.is_null() {
            let _ = MAIN.set(handle as isize);
            // SAFETY: a plain call.
            let _ = MAIN_ID.set(unsafe { GetCurrentThreadId() });
        }
    }

    /// The other threads of this process that are running code of `exe`, with their name and
    /// frames: who holds what a stuck main thread waits for. Threads above normal priority (the
    /// audio threads) are never stopped, so a snapshot cannot make the sound drop out.
    pub fn others(exe: &str) -> Vec<(String, Vec<Frame>)> {
        // SAFETY: plain calls; the snapshot handle is closed below.
        let (me, pid, snapshot) = unsafe {
            (
                GetCurrentThreadId(),
                GetCurrentProcessId(),
                CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0),
            )
        };
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let main = MAIN_ID.get().copied();
        let mut ids = Vec::new();
        // SAFETY: a zeroed entry with its size set, as Thread32First asks.
        let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        // SAFETY: the snapshot and the entry above.
        let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
        while more {
            let id = entry.th32ThreadID;
            if entry.th32OwnerProcessID == pid && id != me && Some(id) != main {
                ids.push(id);
            }
            // SAFETY: as above.
            more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot opened above, closed once.
        unsafe { CloseHandle(snapshot) };

        let mut found = Vec::new();
        for id in ids {
            // SAFETY: a plain call; the handle is closed below.
            let handle = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
                    0,
                    id,
                )
            };
            if handle.is_null() {
                continue;
            }
            // SAFETY: the handle just opened.
            let priority = unsafe { GetThreadPriority(handle) };
            if priority <= THREAD_PRIORITY_NORMAL {
                let name = thread_name(handle).unwrap_or_else(|| format!("thread {id}"));
                let frames = capture(handle as isize);
                if frames.iter().any(|f| f.module.as_deref() == Some(exe)) {
                    found.push((name, frames));
                }
            }
            // SAFETY: the handle opened above, closed once.
            unsafe { CloseHandle(handle) };
        }
        found
    }

    /// The name a thread was given (Rust's thread names arrive here), if any.
    fn thread_name(thread: HANDLE) -> Option<String> {
        let mut text: *mut u16 = std::ptr::null_mut();
        // SAFETY: the out pointer is ours; the string it receives is freed with LocalFree.
        let result = unsafe { GetThreadDescription(thread, &mut text) };
        if result < 0 || text.is_null() {
            return None;
        }
        // SAFETY: a 0-terminated string from GetThreadDescription.
        let name = unsafe {
            let len = (0..).take_while(|&i| *text.add(i) != 0).count();
            String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
        };
        // SAFETY: allocated by GetThreadDescription, freed once.
        unsafe { LocalFree(text as _) };
        (!name.is_empty()).then_some(name)
    }

    pub fn main_thread() -> Option<isize> {
        MAIN.get().copied()
    }

    /// The frames of `thread`, innermost first. The thread is stopped only while its registers
    /// and return addresses are read; nothing is allocated meanwhile, because the stopped thread
    /// may hold the heap's lock.
    pub fn capture(thread: isize) -> Vec<Frame> {
        let thread = thread as HANDLE;
        let mut addresses = [0u64; MAX_FRAMES];
        let mut count = 0usize;
        // SAFETY: a zeroed CONTEXT is valid input once ContextFlags is set; the thread handle has
        // suspend and get-context rights; every address read below is on the stopped thread's
        // own stack, which stays mapped while the thread exists.
        unsafe {
            let mut context: AlignedContext = std::mem::zeroed();
            context.0.ContextFlags = CONTEXT_FULL_AMD64;
            if SuspendThread(thread) == u32::MAX {
                return Vec::new();
            }
            if GetThreadContext(thread, &mut context.0) != 0 {
                let ctx = &mut context.0;
                while count < MAX_FRAMES && ctx.Rip != 0 {
                    addresses[count] = ctx.Rip;
                    count += 1;
                    let mut image_base = 0u64;
                    let entry =
                        RtlLookupFunctionEntry(ctx.Rip, &mut image_base, std::ptr::null_mut());
                    if entry.is_null() {
                        // A leaf function: its return address is on top of the stack.
                        if ctx.Rsp == 0 {
                            break;
                        }
                        ctx.Rip = *(ctx.Rsp as *const u64);
                        ctx.Rsp += 8;
                    } else {
                        let mut handler_data = std::ptr::null_mut();
                        let mut establisher = 0u64;
                        RtlVirtualUnwind(
                            UNW_FLAG_NHANDLER,
                            image_base,
                            ctx.Rip,
                            entry,
                            ctx,
                            &mut handler_data,
                            &mut establisher,
                            std::ptr::null_mut(),
                        );
                    }
                }
            }
            ResumeThread(thread);
        }
        addresses[..count]
            .iter()
            .map(|&a| frame_at(a as usize))
            .collect()
    }

    fn frame_at(address: usize) -> Frame {
        let mut module: HMODULE = std::ptr::null_mut();
        // SAFETY: FROM_ADDRESS takes any address; UNCHANGED_REFCOUNT takes no reference.
        let found = unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                address as *const u16,
                &mut module,
            )
        } != 0;
        if !found || module.is_null() {
            return Frame {
                module: None,
                address,
                offset: 0,
            };
        }
        let mut path = [0u16; 260];
        // SAFETY: a buffer of the length passed.
        let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
        let path = String::from_utf16_lossy(&path[..len as usize]);
        let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
        Frame {
            module: Some(name),
            address,
            offset: address.wrapping_sub(module as usize),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{describe, Frame};

    fn frame(module: &str, offset: usize) -> Frame {
        Frame {
            module: Some(module.to_string()),
            address: 0x1000 + offset,
            offset,
        }
    }

    #[test]
    fn frames_are_module_plus_offset_innermost_first() {
        let line = describe(&[frame("ntdll.dll", 0x9d4c4), frame("user32.dll", 0x1a2b)]);
        assert_eq!(line, "ntdll.dll+0x9d4c4 < user32.dll+0x1a2b");
    }

    #[test]
    fn a_frame_outside_any_module_is_its_address() {
        let line = describe(&[Frame {
            module: None,
            address: 0xdead0,
            offset: 0,
        }]);
        assert_eq!(line, "0xdead0");
    }

    #[test]
    fn a_repeated_frame_is_written_once_with_a_count() {
        let a = frame("plugin.dll", 0x10);
        let line = describe(&[a.clone(), a.clone(), a, frame("user32.dll", 0x20)]);
        assert_eq!(line, "plugin.dll+0x10 (x3) < user32.dll+0x20");
    }

    #[test]
    fn no_frames_says_so() {
        assert_eq!(describe(&[]), "(no frames)");
    }

    /// The walk itself, on a thread parked in a function of this test binary.
    #[cfg(windows)]
    #[test]
    fn a_parked_thread_is_found_in_this_binary() {
        use std::os::windows::io::AsRawHandle;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        #[inline(never)]
        fn spin_here(stop: &AtomicBool) {
            while !stop.load(Ordering::Relaxed) {
                std::hint::spin_loop();
            }
        }

        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || spin_here(&stop))
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        let frames = super::capture(worker.as_raw_handle() as isize);
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();

        let me = std::env::current_exe().unwrap();
        let me = me.file_name().unwrap().to_string_lossy().to_string();
        assert!(frames.len() > 2, "frames: {}", describe(&frames));
        assert_eq!(
            frames[0].module.as_deref(),
            Some(me.as_str()),
            "frames: {}",
            describe(&frames)
        );
    }
}
