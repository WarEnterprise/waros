use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::instructions::interrupts;

use crate::serial_println;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;
const IRQ_TIMER: u8 = 0;
const IRQ_KEYBOARD: u8 = 1;
const IRQ_CASCADE: u8 = 2;
const IRQ_MOUSE: u8 = 12;

/// Interrupt vector indices after PIC remapping.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Mouse = PIC_2_OFFSET + 4,
}

impl InterruptIndex {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

/// Initialize the legacy 8259 PIC pair and remap IRQs to vectors 32-47.
pub unsafe fn init() {
    // SAFETY: Only called once during early boot before interrupts are enabled.
    unsafe {
        interrupts::without_interrupts(|| {
            let mut pics = PICS.lock();
            pics.initialize();
            let masks_before = pics.read_masks();
            let primary_mask = masks_before[0]
                & !(1 << IRQ_TIMER)
                & !(1 << IRQ_KEYBOARD)
                & !(1 << IRQ_CASCADE);
            let secondary_mask = masks_before[1] & !(1 << (IRQ_MOUSE - 8));
            pics.write_masks(primary_mask, secondary_mask);
            serial_println!(
                "[PIC] masks before primary=0x{:02X} secondary=0x{:02X} after primary=0x{:02X} secondary=0x{:02X}",
                masks_before[0],
                masks_before[1],
                primary_mask,
                secondary_mask
            );
        });
    }
}

/// Notify the PIC that the interrupt has been fully handled.
pub fn end_of_interrupt(index: InterruptIndex) {
    interrupts::without_interrupts(|| {
        // SAFETY: `index` is one of the remapped hardware IRQ vectors owned by the PIC.
        unsafe {
            PICS.lock().notify_end_of_interrupt(index.as_u8());
        }
    });
}

#[must_use]
pub fn read_masks() -> [u8; 2] {
    interrupts::without_interrupts(|| {
        // SAFETY: Reading PIC masks is safe under the global PIC mutex and does not alter routing.
        unsafe { PICS.lock().read_masks() }
    })
}

#[must_use]
pub const fn vector_for_irq_line(line: u8) -> u8 {
    PIC_1_OFFSET + line
}

pub fn end_of_interrupt_line(line: u8) {
    interrupts::without_interrupts(|| {
        // SAFETY: `line` is a legacy IRQ line delivered through the remapped PIC pair.
        unsafe {
            PICS.lock()
                .notify_end_of_interrupt(vector_for_irq_line(line));
        }
    });
}
