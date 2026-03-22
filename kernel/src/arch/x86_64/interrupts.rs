use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptStackFrame, PageFaultErrorCode};

use crate::arch::x86_64::pic::{self, InterruptIndex};
use crate::arch::x86_64::port;
use crate::drivers::keyboard;
use crate::serial_println;

static TICKS: AtomicU64 = AtomicU64::new(0);

/// Return the number of timer ticks since boot.
#[must_use]
pub fn tick_count() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

fn print_stack(label: &str, stack_frame: InterruptStackFrame) {
    serial_println!("[EXCEPTION] {label}");
    serial_println!("{:#?}", stack_frame);
}

/// Breakpoint exception handler.
pub extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    print_stack("BREAKPOINT", stack_frame);
}

/// Divide-by-zero exception handler.
pub extern "x86-interrupt" fn divide_error_handler(stack_frame: InterruptStackFrame) {
    print_stack("DIVIDE ERROR", stack_frame);
    crate::display::console::try_write_fmt(format_args!("\n[KERNEL PANIC] DIVIDE ERROR\n"));
    crate::arch::x86_64::hlt_loop();
}

/// General protection fault handler.
pub extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial_println!("[EXCEPTION] GENERAL PROTECTION FAULT ({error_code:#x})");
    serial_println!("{:#?}", stack_frame);
    crate::display::console::try_write_fmt(format_args!(
        "\n[KERNEL PANIC] GENERAL PROTECTION FAULT ({error_code:#x})\n"
    ));
    crate::arch::x86_64::hlt_loop();
}

/// Double fault handler.
pub extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    serial_println!("[EXCEPTION] DOUBLE FAULT ({error_code:#x})");
    serial_println!("{:#?}", stack_frame);
    crate::display::console::try_write_fmt(format_args!("\n[KERNEL PANIC] DOUBLE FAULT\n"));
    crate::arch::x86_64::hlt_loop();
}

/// Page fault handler.
pub extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed_address = Cr2::read();
    serial_println!("[EXCEPTION] PAGE FAULT");
    serial_println!("  accessed address: {:?}", accessed_address);
    serial_println!("  error code: {:?}", error_code);
    serial_println!("{:#?}", stack_frame);
    crate::display::console::try_write_fmt(format_args!("\n[KERNEL PANIC] PAGE FAULT\n"));
    crate::arch::x86_64::hlt_loop();
}

/// Timer interrupt handler (IRQ0 -> vector 32).
pub extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    TICKS.fetch_add(1, Ordering::Relaxed);
    pic::end_of_interrupt(InterruptIndex::Timer);
}

/// Keyboard interrupt handler (IRQ1 -> vector 33).
pub extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let scancode = port::inb(0x60);
    keyboard::handle_scancode(scancode);
    pic::end_of_interrupt(InterruptIndex::Keyboard);
}
