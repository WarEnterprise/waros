use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};

use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptStackFrame, PageFaultErrorCode};

use crate::arch::x86_64::pic::{self, InterruptIndex};
use crate::arch::x86_64::port;
use crate::boot::trace;
use crate::drivers::keyboard;
use crate::memory;
use crate::serial_println;

static TICKS: AtomicU64 = AtomicU64::new(0);
static OBSERVED_TICKS: AtomicU64 = AtomicU64::new(0);
static NETWORK_IRQ_MASK: AtomicU16 = AtomicU16::new(0);
static NETWORK_IRQ_PENDING: AtomicU16 = AtomicU16::new(0);

/// Return the number of timer ticks since boot.
#[must_use]
pub fn tick_count() -> u64 {
    irq_tick_count().max(OBSERVED_TICKS.load(Ordering::Relaxed))
}

/// Return the raw number of ticks delivered by IRQ0.
#[must_use]
pub fn irq_tick_count() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Record monotonic timer progress observed through PIT polling when IRQ delivery lags.
pub fn note_observed_tick_floor(ticks: u64) {
    let mut current = OBSERVED_TICKS.load(Ordering::Relaxed);
    while current < ticks {
        match OBSERVED_TICKS.compare_exchange_weak(
            current,
            ticks,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
}

#[must_use]
pub fn register_network_irq(line: u8) -> bool {
    if !(3..16).contains(&line) || line == 12 {
        return false;
    }
    NETWORK_IRQ_MASK.fetch_or(1u16 << line, Ordering::Relaxed);
    true
}

#[must_use]
pub fn take_pending_network_irqs() -> u16 {
    NETWORK_IRQ_PENDING.swap(0, Ordering::AcqRel)
}

fn handle_pic_line(line: u8) {
    let bit = 1u16 << line;
    if NETWORK_IRQ_MASK.load(Ordering::Relaxed) & bit != 0 {
        NETWORK_IRQ_PENDING.fetch_or(bit, Ordering::Release);
    }
    pic::end_of_interrupt_line(line);
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
    panic!("Divide error exception");
}

/// General protection fault handler.
pub extern "x86-interrupt" fn general_protection_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) {
    serial_println!("[EXCEPTION] GENERAL PROTECTION FAULT ({error_code:#x})");
    serial_println!("{:#?}", stack_frame);
    panic!("General protection fault ({error_code:#x})");
}

/// Double fault handler.
pub extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: u64,
) -> ! {
    serial_println!("[EXCEPTION] DOUBLE FAULT ({error_code:#x})");
    serial_println!("{:#?}", stack_frame);
    panic!("Double fault ({error_code:#x})");
}

/// Page fault handler.
pub extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    let accessed_address_raw = Cr2::read_raw();
    trace::record_page_fault(accessed_address_raw);
    serial_println!("[EXCEPTION] PAGE FAULT");
    match Cr2::read() {
        Ok(accessed_address) => serial_println!("  accessed address: {:?}", accessed_address),
        Err(_) => {
            serial_println!("  accessed address: non-canonical 0x{accessed_address_raw:016X}")
        }
    }
    serial_println!("  error code: {:?}", error_code);
    if let Some(classification) = memory::classify_direct_map_address(accessed_address_raw) {
        serial_println!(
            "  direct-map physical: 0x{:016X}",
            classification.physical_address
        );
        if let Some(region) = classification.region {
            serial_println!(
                "  physical region: {} [0x{:016X}-0x{:016X})",
                memory::memory_region_kind_name(region.kind),
                region.start,
                region.end
            );
        }
    }
    serial_println!("{:#?}", stack_frame);
    panic!(
        "Page fault while accessing 0x{accessed_address_raw:016X} with error {:?}",
        error_code
    );
}

/// Timer interrupt handler (IRQ0 -> vector 32).
pub extern "x86-interrupt" fn timer_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let next_tick = TICKS.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    note_observed_tick_floor(next_tick);
    crate::exec::tick();
    pic::end_of_interrupt(InterruptIndex::Timer);
}

/// Keyboard interrupt handler (IRQ1 -> vector 33).
pub extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let status = port::inb(0x64);
    keyboard::handle_irq1(status);
    pic::end_of_interrupt(InterruptIndex::Keyboard);
}

/// Mouse interrupt handler (IRQ12 -> vector 44).
pub extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    let byte = port::inb(0x60);
    crate::gui::mouse::handle_byte(byte);
    pic::end_of_interrupt(InterruptIndex::Mouse);
}

pub extern "x86-interrupt" fn irq2_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(2);
}

pub extern "x86-interrupt" fn irq3_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(3);
}

pub extern "x86-interrupt" fn irq4_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(4);
}

pub extern "x86-interrupt" fn irq5_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(5);
}

pub extern "x86-interrupt" fn irq6_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(6);
}

pub extern "x86-interrupt" fn irq7_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(7);
}

pub extern "x86-interrupt" fn irq8_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(8);
}

pub extern "x86-interrupt" fn irq9_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(9);
}

pub extern "x86-interrupt" fn irq10_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(10);
}

pub extern "x86-interrupt" fn irq11_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(11);
}

pub extern "x86-interrupt" fn irq13_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(13);
}

pub extern "x86-interrupt" fn irq14_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(14);
}

pub extern "x86-interrupt" fn irq15_interrupt_handler(_stack_frame: InterruptStackFrame) {
    handle_pic_line(15);
}
