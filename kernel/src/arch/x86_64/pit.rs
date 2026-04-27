use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::x86_64::port;

const PIT_BASE_FREQUENCY_HZ: u32 = 1_193_182;
const PIT_COMMAND_PORT: u16 = 0x43;
const PIT_CHANNEL0_PORT: u16 = 0x40;
const PIT_LATCH_COUNT: u8 = 0x00;

pub const PIT_FREQUENCY_HZ: u32 = 100;

static PIT_INITIALIZED: AtomicBool = AtomicBool::new(false);
static PIT_DIVISOR: AtomicU32 = AtomicU32::new(0);

/// Program the legacy PIT channel 0 to generate the scheduler tick rate.
pub fn init() {
    let divisor = (PIT_BASE_FREQUENCY_HZ / PIT_FREQUENCY_HZ).clamp(1, u32::from(u16::MAX)) as u16;
    PIT_DIVISOR.store(u32::from(divisor), Ordering::Relaxed);
    PIT_INITIALIZED.store(true, Ordering::Relaxed);

    port::outb(PIT_COMMAND_PORT, 0x36);
    port::outb(PIT_CHANNEL0_PORT, (divisor & 0x00FF) as u8);
    port::outb(PIT_CHANNEL0_PORT, (divisor >> 8) as u8);
}

/// Return the elapsed milliseconds since the PIT was initialized.
#[must_use]
pub fn elapsed_millis(ticks: u64) -> u64 {
    if !PIT_INITIALIZED.load(Ordering::Relaxed) {
        return 0;
    }

    let divisor = PIT_DIVISOR.load(Ordering::Relaxed);
    if divisor == 0 {
        return 0;
    }

    let ticks_ms = ticks.saturating_mul(1_000) / u64::from(PIT_FREQUENCY_HZ);
    let current_count = u64::from(read_current_count());
    let elapsed_counts = u64::from(divisor).saturating_sub(current_count.min(u64::from(divisor)));
    let sub_tick_ms = elapsed_counts.saturating_mul(1_000) / u64::from(PIT_BASE_FREQUENCY_HZ);
    ticks_ms.saturating_add(sub_tick_ms)
}

/// Poll the PIT counter until the global tick count advances or the counter wraps
/// `max_wraps` times. This keeps late boot progress from depending on a timer IRQ
/// that may not arrive on every machine.
#[must_use]
pub fn wait_for_tick_advance(start_ticks: u64, max_wraps: u8) -> bool {
    if crate::arch::x86_64::interrupts::tick_count() != start_ticks {
        return true;
    }

    let mut previous = read_current_count();
    let mut wraps = 0u8;
    loop {
        if crate::arch::x86_64::interrupts::tick_count() != start_ticks {
            return true;
        }

        let current = read_current_count();
        if current > previous {
            wraps = wraps.saturating_add(1);
            if wraps >= max_wraps {
                crate::arch::x86_64::interrupts::note_observed_tick_floor(
                    start_ticks.saturating_add(u64::from(wraps)),
                );
                return false;
            }
        }
        previous = current;
        core::hint::spin_loop();
    }
}

fn read_current_count() -> u16 {
    port::outb(PIT_COMMAND_PORT, PIT_LATCH_COUNT);
    let low = port::inb(PIT_CHANNEL0_PORT);
    let high = port::inb(PIT_CHANNEL0_PORT);
    u16::from_le_bytes([low, high])
}
