use crate::arch::x86_64::gdt;

use super::{syscall, PROCESS_TABLE};

pub fn activate_process(pid: Option<u32>) {
    let Some(pid) = pid else {
        let stack_top = gdt::kernel_stack_top();
        syscall::set_kernel_stack_top(stack_top.as_u64());
        return;
    };

    let process_table = PROCESS_TABLE.lock();
    if let Some(process) = process_table.get(pid) {
        gdt::set_kernel_stack_top(x86_64::VirtAddr::new(process.kernel_stack_top));
        syscall::set_kernel_stack_top(process.kernel_stack_top);
    }
}
