//! How much memory this process is actually using.
//!
//! The toolbar's MEM meter read `performance.memory.usedJSHeapSize`, the
//! WebView's JavaScript heap. Almost nothing that costs memory in a DAW lives
//! there: the sample pool, the audio graph, every loaded plug-in and every
//! project buffer are in this process, on the Rust side. A project holding
//! several gigabytes of samples showed a few tens of megabytes, so the meter
//! was reassuring at exactly the moment it should not have been.

/// Resident memory in bytes, or None where it cannot be read.
pub fn resident_bytes() -> Option<u64> {
    platform::resident_bytes()
}

/// The machine's physical memory in bytes, so a reading can be shown as a
/// share of what is actually available rather than a bare number.
pub fn total_bytes() -> Option<u64> {
    platform::total_bytes()
}

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    pub fn resident_bytes() -> Option<u64> {
        // The working set is what Task Manager calls the memory a process is
        // using, which is the number a producer would compare against.
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: the pointer is to a live, correctly sized struct whose `cb`
        // tells the API its size, and the pseudo-handle from
        // GetCurrentProcess needs no closing.
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        };
        if ok == 0 {
            return None;
        }
        Some(counters.WorkingSetSize as u64)
    }

    pub fn total_bytes() -> Option<u64> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        // SAFETY: as above — a live struct that carries its own size.
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        if ok == 0 {
            return None;
        }
        Some(status.ullTotalPhys)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub fn resident_bytes() -> Option<u64> {
        // statm's second field is the resident set in pages.
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * page_size())
    }

    pub fn total_bytes() -> Option<u64> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        for line in meminfo.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
                return Some(kb * 1024);
            }
        }
        None
    }

    fn page_size() -> u64 {
        // 4 KiB everywhere this runs. Reading it properly needs libc, which
        // this crate does not depend on, and a wrong page size on an exotic
        // kernel would only mis-scale a meter on a development machine.
        4096
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub fn resident_bytes() -> Option<u64> {
        // macOS needs task_info, which would mean another dependency for a
        // platform that has no release build yet. The meter says "unknown"
        // rather than showing a number that means something else.
        None
    }

    pub fn total_bytes() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// True on the platforms this module can read. A function rather than a
    /// const so the assertions below are not constant expressions, which
    /// clippy reads as a mistake.
    fn readable() -> bool {
        cfg!(any(windows, target_os = "linux"))
    }

    #[test]
    fn the_machine_reports_its_memory() {
        match total_bytes() {
            // Anything under 256 MB is not a machine anyone runs a DAW on, so
            // it would mean the number is not what it claims to be.
            Some(bytes) => assert!(bytes > 256 * 1024 * 1024, "{bytes} bytes total"),
            None => assert!(!readable(), "this platform should report its memory"),
        }
    }

    #[test]
    fn the_reading_is_a_share_of_the_machine_not_more_than_it() {
        if let (Some(used), Some(total)) = (resident_bytes(), total_bytes()) {
            assert!(used < total, "{used} used of {total} total");
        }
    }

    #[test]
    fn this_process_is_using_some_memory() {
        // A running process has a resident set. Anything below a megabyte
        // means the number is not what it claims to be.
        match resident_bytes() {
            Some(bytes) => assert!(bytes > 1024 * 1024, "{bytes} bytes is not plausible"),
            // Only acceptable on a platform this has no reader for.
            None => assert!(!readable(), "this platform should read its own memory"),
        }
    }
}
