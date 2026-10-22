//! Flush denormal numbers to zero on the audio thread.
//!
//! A reverb or filter tail decays towards zero and eventually produces
//! denormals: numbers so small the CPU handles them in a slow path, tens to
//! hundreds of times slower than normal arithmetic. The audio does not sound
//! different, the CPU load simply climbs at the end of every tail, and on a
//! busy project that is a dropout with no visible cause.
//!
//! Every DAW turns them off on its audio thread. This does the same, once per
//! callback: the cost is a couple of instructions, and doing it per callback
//! rather than once means it still holds if the driver hands us a different
//! thread, which WASAPI and CoreAudio both may do.

/// Bit 15 of MXCSR: results that would be denormal are written as zero.
#[cfg(target_arch = "x86_64")]
const MXCSR_FLUSH_TO_ZERO: u32 = 1 << 15;
/// Bit 6 of MXCSR: denormal inputs are read as zero.
#[cfg(target_arch = "x86_64")]
const MXCSR_DENORMALS_ARE_ZERO: u32 = 1 << 6;

/// This thread's SSE control register.
///
/// # Safety
/// Reads a register that always exists on x86_64.
#[cfg(target_arch = "x86_64")]
#[inline]
pub unsafe fn read_mxcsr() -> u32 {
    let mut csr: u32 = 0;
    std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack, preserves_flags));
    csr
}

/// Turn on flush-to-zero and denormals-are-zero for this thread.
///
/// x86_64 has both in the SSE control register. aarch64 has the same thing in
/// FPCR bit 24. Anything else keeps the default behaviour, which is correct,
/// just slower on tails.
#[inline]
pub fn flush_denormals_to_zero() {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: reads and writes this thread's own SSE control register,
        // which is what this function is for. Every x86_64 CPU has SSE2, so
        // both bits exist. The intrinsics for these are deprecated, hence
        // the two instructions.
        unsafe {
            let mut csr = read_mxcsr();
            csr |= MXCSR_FLUSH_TO_ZERO | MXCSR_DENORMALS_ARE_ZERO;
            std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack, preserves_flags));
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // FPCR bit 24 is Flush-to-Zero. Read, set, write back, so nothing
        // else in the register is disturbed.
        // SAFETY: reads and writes this thread's own floating-point control
        // register.
        unsafe {
            let mut fpcr: u64;
            std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack));
            fpcr |= 1 << 24;
            std::arch::asm!("msr fpcr, {}", in(reg) fpcr, options(nomem, nostack));
        }
    }
}

/// Flush denormals to zero until this is dropped, then put the thread's
/// floating-point mode back as it was.
///
/// For work that runs on a thread the audio engine does not own, such as
/// an export on a blocking-task thread: the mode should not outlive the
/// render on a thread other work goes on to use.
pub struct DenormalsFlushed {
    #[cfg(target_arch = "x86_64")]
    saved: u32,
    #[cfg(target_arch = "aarch64")]
    saved: u64,
}

/// Turn denormal flushing on for as long as the returned value lives.
#[inline]
pub fn flush_denormals_while() -> DenormalsFlushed {
    #[cfg(target_arch = "x86_64")]
    let saved = {
        // SAFETY: reads this thread's own SSE control register.
        unsafe { read_mxcsr() }
    };
    #[cfg(target_arch = "aarch64")]
    let saved = {
        let fpcr: u64;
        // SAFETY: reads this thread's own floating-point control register.
        unsafe { std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack)) };
        fpcr
    };
    flush_denormals_to_zero();
    DenormalsFlushed {
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        saved,
    }
}

impl Drop for DenormalsFlushed {
    fn drop(&mut self) {
        #[cfg(target_arch = "x86_64")]
        // SAFETY: writes back the value this thread's register had.
        unsafe {
            std::arch::asm!("ldmxcsr [{}]", in(reg) &self.saved, options(nostack, preserves_flags));
        }
        #[cfg(target_arch = "aarch64")]
        // SAFETY: writes back the value this thread's register had.
        unsafe {
            std::arch::asm!("msr fpcr, {}", in(reg) self.saved, options(nomem, nostack));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn a_scoped_flush_puts_the_thread_back_as_it_was() {
        std::thread::spawn(|| {
            // SAFETY: reads this thread's SSE control register.
            let before = unsafe { read_mxcsr() };
            assert_eq!(
                before & MXCSR_FLUSH_TO_ZERO,
                0,
                "a new thread starts without it"
            );
            {
                let _on = flush_denormals_while();
                let during = unsafe { read_mxcsr() };
                assert_ne!(during & MXCSR_FLUSH_TO_ZERO, 0);
            }
            assert_eq!(unsafe { read_mxcsr() }, before);
        })
        .join()
        .unwrap();
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn a_denormal_becomes_zero_once_it_is_on() {
        flush_denormals_to_zero();
        // SAFETY: reads this thread's SSE control register.
        let csr = unsafe { read_mxcsr() };
        assert_ne!(csr & MXCSR_FLUSH_TO_ZERO, 0, "flush-to-zero is on");
        assert_ne!(
            csr & MXCSR_DENORMALS_ARE_ZERO,
            0,
            "denormals-are-zero is on"
        );

        // The classic tail: a value decaying past the smallest normal float.
        // std::hint::black_box keeps the optimiser from folding the loop.
        let mut x = std::hint::black_box(1.0e-38_f32);
        for _ in 0..40 {
            x = std::hint::black_box(x * 0.1);
        }
        assert_eq!(x, 0.0, "a decaying tail must reach zero, not denormals");
    }

    #[test]
    fn turning_it_on_twice_is_harmless() {
        flush_denormals_to_zero();
        flush_denormals_to_zero();
        // 1.0 still behaves: this is not a mode that breaks normal maths.
        let x = std::hint::black_box(1.0_f32) * std::hint::black_box(0.5_f32);
        assert_eq!(x, 0.5);
    }
}
