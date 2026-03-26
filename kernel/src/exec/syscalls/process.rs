use alloc::string::ToString;
use alloc::vec;

use crate::exec::{current_pid, loader, mark_exit, PROCESS_TABLE, SCHEDULER};
use crate::exec::process::{CpuContext, Priority, ProcessImageKind, ProcessState};
use crate::exec::address_space::AddressSpace;
use crate::exec::fd_table::FileDescriptorTable;
use crate::exec::scheduler::DEFAULT_TIME_SLICE;
use crate::exec::elf::parse_elf;
use crate::auth::session;
use crate::auth::UserRole;
use crate::auth::USER_DB;
use crate::fs;

use super::{read_user_string, ENOSYS};

pub fn sys_getpid() -> i64 {
    current_pid().map_or(-1, i64::from)
}

pub fn sys_getppid() -> i64 {
    let process_table = PROCESS_TABLE.lock();
    current_pid()
        .and_then(|pid| process_table.get(pid).map(|process| process.parent_pid))
        .map_or(-1, i64::from)
}

pub fn sys_getuid() -> i64 {
    let process_table = PROCESS_TABLE.lock();
    current_pid()
        .and_then(|pid| process_table.get(pid).map(|process| process.uid))
        .map_or(-1, i64::from)
}

pub fn sys_fork() -> i64 {
    // Clone the current process into a new process entry.
    let parent = {
        let process_table = PROCESS_TABLE.lock();
        let Some(pid) = current_pid() else { return -1; };
        process_table.get(pid).cloned()
    };
    let Some(parent) = parent else { return -1; };

    // Create a new page table for the child.
    let new_cr3 = match loader::create_user_page_table_pub() {
        Ok(cr3) => cr3,
        Err(_) => return -12, // ENOMEM
    };

    let child_kernel_stack = vec![0u8; 16 * 1024];
    let child_kernel_stack_top = child_kernel_stack.as_ptr() as u64 + child_kernel_stack.len() as u64;

    let mut child_context = parent.context.clone();
    child_context.rax = 0; // fork() returns 0 in child
    child_context.cr3 = new_cr3;

    let child = crate::exec::process::Process {
        pid: 0,
        parent_pid: parent.pid,
        name: parent.name.clone(),
        uid: parent.uid,
        state: ProcessState::Ready,
        context: child_context,
        exit_code: None,
        page_table_phys: new_cr3,
        address_space: AddressSpace::new(new_cr3),
        kernel_stack: child_kernel_stack,
        kernel_stack_top: child_kernel_stack_top,
        fd_table: parent.fd_table.clone(),
        cwd: parent.cwd.clone(),
        env: parent.env.clone(),
        quantum_registers: alloc::vec::Vec::new(),
        crypto_keys: alloc::vec::Vec::new(),
        priority: parent.priority,
        cpu_ticks: 0,
        time_slice: DEFAULT_TIME_SLICE,
        created_at: crate::arch::x86_64::interrupts::tick_count(),
        blocked_on: None,
        syscall_count: 0,
        page_fault_count: 0,
        memory_pages: 0,
        task_id: None,
        image_kind: ProcessImageKind::Elf,
        image_path: parent.image_path.clone(),
    };

    let child_pid = match PROCESS_TABLE.lock().create_process(child) {
        Ok(pid) => pid,
        Err(_) => return -12,
    };
    SCHEDULER.lock().enqueue(child_pid, parent.priority);
    i64::from(child_pid)
}

pub fn sys_execve(path: *const u8, argv: *const *const u8, _envp: *const *const u8) -> i64 {
    let Some(path_str) = (unsafe { read_user_string(path, 256) }) else {
        return -22; // EINVAL
    };

    // Collect argv strings.
    let mut args: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    if !argv.is_null() {
        for i in 0..64usize {
            // SAFETY: argv is a null-terminated array of pointers provided by userspace.
            let arg_ptr = unsafe { argv.add(i).read() };
            if arg_ptr.is_null() {
                break;
            }
            if let Some(arg) = unsafe { read_user_string(arg_ptr, 256) } {
                args.push(arg);
            }
        }
    }
    let arg_refs: alloc::vec::Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let Some(pid) = current_pid() else { return -1; };
    let (uid, _parent_pid, priority, _cwd) = {
        let process_table = PROCESS_TABLE.lock();
        let Some(proc) = process_table.get(pid) else { return -1; };
        (proc.uid, proc.parent_pid, proc.priority, proc.cwd.clone())
    };

    // Replace the current process image: teardown old memory, load new ELF.
    let _ = loader::teardown_process(pid);

    let role = role_for_uid(uid);
    let elf_data = match fs::FILESYSTEM.lock().read_as(&path_str, uid, role) {
        Ok(data) => data.to_vec(),
        Err(_) => return -2, // ENOENT
    };
    let elf = match parse_elf(&elf_data) {
        Ok(e) => e,
        Err(_) => return -8, // ENOEXEC
    };

    let new_cr3 = match loader::create_user_page_table_pub() {
        Ok(cr3) => cr3,
        Err(_) => return -12,
    };

    let saved_cr3 = loader::kernel_cr3();
    // SAFETY: new_cr3 preserves the non-user kernel/runtime mappings required after the CR3 switch.
    unsafe {
        use x86_64::registers::control::{Cr3, Cr3Flags};
        use x86_64::structures::paging::PhysFrame;
        use x86_64::PhysAddr;
        Cr3::write(PhysFrame::containing_address(PhysAddr::new(new_cr3)), Cr3Flags::empty());
    }

    let map_result = loader::map_process_image_pub(&path_str, new_cr3, &elf, &elf_data, &arg_refs, &[]);

    unsafe {
        use x86_64::registers::control::{Cr3, Cr3Flags};
        use x86_64::structures::paging::PhysFrame;
        use x86_64::PhysAddr;
        Cr3::write(PhysFrame::containing_address(PhysAddr::new(saved_cr3)), Cr3Flags::empty());
    }

    let (address_space, context, _) = match map_result {
        Ok(r) => r,
        Err(_) => return -8,
    };

    // Update the process in place with the new image.
    let mut process_table = PROCESS_TABLE.lock();
    if let Some(process) = process_table.get_mut(pid) {
        process.context = context;
        process.page_table_phys = new_cr3;
        process.address_space = address_space;
        process.image_path = path_str;
        process.image_kind = ProcessImageKind::Elf;
    }

    0
}

pub fn sys_exit(code: i32) -> i64 {
    if let Some(pid) = current_pid() {
        mark_exit(pid, code);
        crate::exec::syscall::request_kernel_return(code);
        0
    } else {
        -1
    }
}

pub fn sys_wait4(pid: i32, status_ptr: *mut i32, _options: u32) -> i64 {
    let parent_pid = match current_pid() {
        Some(p) => p,
        None => return -1,
    };

    let found = PROCESS_TABLE.lock().find_zombie_child(parent_pid, pid);

    let (child_pid, exit_code) = match found {
        Some(r) => r,
        None => return -10, // ECHILD
    };

    // Write exit status to user pointer.
    if !status_ptr.is_null() {
        let status = (exit_code & 0xFF) << 8;
        // SAFETY: status_ptr is a userspace pointer provided by the caller.
        unsafe { status_ptr.write(status); }
    }

    PROCESS_TABLE.lock().remove(child_pid);
    i64::from(child_pid)
}

fn role_for_uid(uid: u16) -> UserRole {
    USER_DB
        .lock()
        .find_by_uid(uid)
        .map(|user| user.role)
        .unwrap_or(UserRole::Admin)
}
