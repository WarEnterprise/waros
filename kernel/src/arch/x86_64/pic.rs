use pic8259::ChainedPics;
use spin::Mutex;

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

/// Interrupt vector indices after PIC remapping.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
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
        PICS.lock().initialize();
    }
}

/// Notify the PIC that the interrupt has been fully handled.
pub fn end_of_interrupt(index: InterruptIndex) {
    // SAFETY: `index` is one of the remapped hardware IRQ vectors owned by the PIC.
    unsafe {
        PICS.lock().notify_end_of_interrupt(index.as_u8());
    }
}
