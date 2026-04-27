use spin::Lazy;
use x86_64::structures::idt::InterruptDescriptorTable;

use crate::arch::x86_64::gdt;
use crate::arch::x86_64::interrupts;
use crate::arch::x86_64::pic::InterruptIndex;

static IDT: Lazy<InterruptDescriptorTable> = Lazy::new(|| {
    let mut idt = InterruptDescriptorTable::new();
    idt.breakpoint
        .set_handler_fn(interrupts::breakpoint_handler);
    idt.double_fault
        .set_handler_fn(interrupts::double_fault_handler);
    // SAFETY: The IST entry is populated in the TSS during GDT initialization.
    unsafe {
        idt.double_fault
            .set_handler_fn(interrupts::double_fault_handler)
            .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
    }
    idt.page_fault
        .set_handler_fn(interrupts::page_fault_handler);
    idt.general_protection_fault
        .set_handler_fn(interrupts::general_protection_fault_handler);
    idt.divide_error
        .set_handler_fn(interrupts::divide_error_handler);
    idt[InterruptIndex::Timer.as_u8()].set_handler_fn(interrupts::timer_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8()].set_handler_fn(interrupts::keyboard_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 1].set_handler_fn(interrupts::irq2_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 2].set_handler_fn(interrupts::irq3_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 3].set_handler_fn(interrupts::irq4_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 4].set_handler_fn(interrupts::irq5_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 5].set_handler_fn(interrupts::irq6_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 6].set_handler_fn(interrupts::irq7_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 7].set_handler_fn(interrupts::irq8_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 8].set_handler_fn(interrupts::irq9_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 9].set_handler_fn(interrupts::irq10_interrupt_handler);
    idt[InterruptIndex::Keyboard.as_u8() + 10].set_handler_fn(interrupts::irq11_interrupt_handler);
    idt[InterruptIndex::Mouse.as_u8()].set_handler_fn(interrupts::mouse_interrupt_handler);
    idt[InterruptIndex::Mouse.as_u8() + 1].set_handler_fn(interrupts::irq13_interrupt_handler);
    idt[InterruptIndex::Mouse.as_u8() + 2].set_handler_fn(interrupts::irq14_interrupt_handler);
    idt[InterruptIndex::Mouse.as_u8() + 3].set_handler_fn(interrupts::irq15_interrupt_handler);
    idt
});

/// Load the interrupt descriptor table.
pub fn init() {
    IDT.load();
}
