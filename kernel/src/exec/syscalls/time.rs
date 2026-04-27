use crate::arch::x86_64::{interrupts, pit};

use super::{write_struct_to_user, Timespec, EINVAL};

const CLOCK_MONOTONIC: u32 = 1;
const CLOCK_MONOTONIC_RAW: u32 = 4;
const CLOCK_MONOTONIC_COARSE: u32 = 6;
const CLOCK_BOOTTIME: u32 = 7;

pub fn sys_clock_gettime(clock_id: u32, out: *mut u8) -> i64 {
    if !matches!(
        clock_id,
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME
    ) {
        return EINVAL;
    }
    let millis = pit::elapsed_millis(interrupts::tick_count());
    let timespec = Timespec {
        tv_sec: (millis / 1_000) as i64,
        tv_nsec: ((millis % 1_000) * 1_000_000) as i64,
    };
    // SAFETY: The caller provides a valid userspace destination pointer.
    if unsafe { write_struct_to_user(out.cast::<Timespec>(), &timespec) } {
        0
    } else {
        -1
    }
}

pub fn sys_nanosleep(request: *const u8, _remaining: *mut u8) -> i64 {
    if request.is_null() {
        return -1;
    }
    // SAFETY: The caller provides a valid userspace pointer to `Timespec`.
    let requested = unsafe { request.cast::<Timespec>().read() };
    let total_ms = (requested.tv_sec.max(0) as u64)
        .saturating_mul(1_000)
        .saturating_add((requested.tv_nsec.max(0) as u64) / 1_000_000);
    let mut remaining = (total_ms.saturating_mul(100) / 1_000).max(1);
    while remaining > 0 {
        let _ = crate::net::poll();
        let start = interrupts::tick_count();
        if pit::wait_for_tick_advance(start, 1) {
            remaining =
                remaining.saturating_sub(interrupts::tick_count().saturating_sub(start).max(1));
        } else {
            remaining = remaining.saturating_sub(1);
        }
    }
    0
}
