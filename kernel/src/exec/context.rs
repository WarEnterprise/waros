use crate::arch::x86_64::gdt;

use super::{syscall, PROCESS_TABLE};

/// Activate a process's kernel stack. Called from `set_current_pid()` which
/// may run in IRQ context (via timer_tick → select_next). Uses `try_lock()`
/// to prevent deadlock when foreground code holds PROCESS_TABLE.
pub fn activate_process(pid: Option<u32>) {
    let Some(pid) = pid else {
        let stack_top = gdt::kernel_stack_top();
        syscall::set_kernel_stack_top(stack_top.as_u64());
        return;
    };

    if let Some(process_table) = PROCESS_TABLE.try_lock() {
        if let Some(process) = process_table.get(pid) {
            gdt::set_kernel_stack_top(x86_64::VirtAddr::new(process.kernel_stack_top));
            syscall::set_kernel_stack_top(process.kernel_stack_top);
        }
    }
    // If try_lock fails, the kernel stack stays at whatever it was.
    // The foreground code that holds the lock will set it correctly
    // when it releases the lock and the next timer tick succeeds.
}
