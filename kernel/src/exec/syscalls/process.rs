use crate::exec::elf::parse_elf;
use crate::exec::process::{ProcessImageKind, ProcessState};
use crate::exec::{current_pid, loader, mark_exit, PROCESS_TABLE};
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
    // WarOS does not currently expose a fork ABI. Keep the number reserved but fail
    // explicitly until address-space cloning, descriptor offsets, and wait semantics are real.
    ENOSYS
}

pub fn sys_execve(path: *const u8, argv: *const *const u8, _envp: *const *const u8) -> i64 {
    let Some(path_str) = (unsafe { read_user_string(path, 256) }) else {
        return -22; // EINVAL
    };

    // Collect argv strings for the current narrow WarExec entry ABI. `envp` is
    // intentionally ignored for now; the replacement image receives an empty environment.
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
    let (uid, old_page_table_phys, old_address_space) = {
        let process_table = PROCESS_TABLE.lock();
        let Some(proc) = process_table.get(pid) else { return -1; };
        (proc.uid, proc.page_table_phys, proc.address_space.clone())
    };

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

    let saved_cr3 = {
        use x86_64::registers::control::Cr3;
        let (frame, _) = Cr3::read();
        frame.start_address().as_u64()
    };
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

    let (address_space, context, memory_pages) = match map_result {
        Ok(r) => r,
        Err(_) => return -8,
    };

    // Commit the replacement image in place. WarExec intentionally keeps this narrow:
    // one process identity, one replacement ELF, same minimal argc/argv ABI, empty env.
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = process_table.get_mut(pid) else {
        return -1;
    };
    process.context = context;
    process.page_table_phys = new_cr3;
    process.address_space = address_space;
    process.memory_pages = memory_pages;
    process.name = path_str.rsplit('/').next().unwrap_or(path_str.as_str()).into();
    process.image_path = path_str;
    process.image_kind = ProcessImageKind::Elf;
    process.env.clear();
    process.exit_code = None;
    process.state = ProcessState::Running;
    drop(process_table);

    if let Err(error) = loader::teardown_process_image(old_page_table_phys, &old_address_space) {
        crate::serial_println!(
            "[INFO] WarExec exec: previous image cleanup incomplete ({:?})",
            error
        );
    }

    crate::exec::syscall::request_exec_transition();
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
