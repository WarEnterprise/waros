pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod pic;
pub mod port;

/// Halt the CPU forever, waking only for interrupts.
pub fn hlt_loop() -> ! {
    loop {
        x86_64::instructions::hlt();
    }
}
