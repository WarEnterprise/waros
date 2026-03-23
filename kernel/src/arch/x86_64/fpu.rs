use core::arch::asm;

/// Enable x87 FPU and SSE so shell-driven floating-point code can execute safely.
pub fn init() {
    unsafe {
        // SAFETY: This runs once during early boot before multitasking exists. It only updates
        // the architectural control registers needed to enable x87/SSE instructions and resets
        // the floating-point unit to a known state.
        asm!("fninit", options(nostack, preserves_flags));

        let mut cr0: u64;
        asm!("mov {}, cr0", out(reg) cr0, options(nostack, preserves_flags));
        cr0 &= !(1 << 2); // Clear CR0.EM to enable the FPU.
        cr0 &= !(1 << 3); // Clear CR0.TS so x87/SSE instructions do not trap.
        cr0 |= 1 << 1; // Set CR0.MP for WAIT/FWAIT behavior.
        asm!("mov cr0, {}", in(reg) cr0, options(nostack, preserves_flags));

        let mut cr4: u64;
        asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
        cr4 |= 1 << 9; // CR4.OSFXSR: enable FXSAVE/FXRSTOR and SSE state handling.
        cr4 |= 1 << 10; // CR4.OSXMMEXCPT: enable unmasked SIMD floating-point exceptions.
        asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
    }
}
