use crate::arch::x86_64::{interrupts, pit};

use super::{write_struct_to_user, Timespec};

pub fn sys_clock_gettime(_clock_id: u32, out: *mut u8) -> i64 {
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
    let start = interrupts::tick_count();
    let deadline = start.saturating_add((total_ms.saturating_mul(100) / 1_000).max(1));
    while interrupts::tick_count() < deadline {
        let _ = crate::net::poll();
        x86_64::instructions::hlt();
    }
    0
}
