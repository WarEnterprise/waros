use crate::arch::x86_64::port;

const PIT_BASE_FREQUENCY_HZ: u32 = 1_193_182;
const PIT_COMMAND_PORT: u16 = 0x43;
const PIT_CHANNEL0_PORT: u16 = 0x40;

pub const PIT_FREQUENCY_HZ: u32 = 100;

/// Program the legacy PIT channel 0 to generate the scheduler tick rate.
pub fn init() {
    let divisor = (PIT_BASE_FREQUENCY_HZ / PIT_FREQUENCY_HZ)
        .clamp(1, u32::from(u16::MAX)) as u16;

    port::outb(PIT_COMMAND_PORT, 0x36);
    port::outb(PIT_CHANNEL0_PORT, (divisor & 0x00FF) as u8);
    port::outb(PIT_CHANNEL0_PORT, (divisor >> 8) as u8);
}
