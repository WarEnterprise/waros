use crate::exec::elf::parse_elf;
use crate::exec::process::{ProcessImageKind, ProcessState};
use crate::exec::{current_pid, loader, mark_exit, PROCESS_TABLE};
use crate::auth::UserRole;
use crate::auth::USER_DB;
use crate::fs;

use super::{
    read_user_pointer_array_checked, read_user_string_checked, read_warexec_path_checked,
    write_struct_to_user_checked, ECHILD, ENOENT, ENOEXEC, ENOMEM, ENOSYS, EPERM,
    MAX_USER_STRING_LEN, WarExecPathKind,
};

const WAIT_STATUS_EXIT_SHIFT: u32 = 8;

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
    // explicitly until address-space cloning and broader process-creation semantics are real.
    ENOSYS
}

pub fn sys_execve(path: *const u8, argv: *const *const u8, _envp: *const *const u8) -> i64 {
    let path_str = match read_warexec_path_checked(path, WarExecPathKind::FileLike) {
        Ok(path) => path,
        Err(error) => return error,
    };

    // Collect argv strings for the current narrow WarExec entry ABI. `envp` is
    // intentionally ignored for now; the replacement image receives an empty environment.
    let mut args: alloc::vec::Vec<alloc::string::String> = alloc::vec::Vec::new();
    let arg_ptrs = match read_user_pointer_array_checked(argv, 64) {
        Ok(arg_ptrs) => arg_ptrs,
        Err(error) => return error,
    };
    for arg_ptr in arg_ptrs {
        match read_user_string_checked(arg_ptr, MAX_USER_STRING_LEN) {
            Ok(arg) => args.push(arg),
            Err(error) => return error,
        }
    }
    let arg_refs: alloc::vec::Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let Some(pid) = current_pid() else { return EPERM; };
    let (uid, old_page_table_phys, old_address_space, old_capabilities) = {
        let process_table = PROCESS_TABLE.lock();
        let Some(proc) = process_table.get(pid) else { return EPERM; };
        (
            proc.uid,
            proc.page_table_phys,
            proc.address_space.clone(),
            proc.effective_capabilities,
        )
    };

    let role = role_for_uid(uid);
    let elf_data = match fs::FILESYSTEM.lock().read_as(&path_str, uid, role) {
        Ok(data) => data.to_vec(),
        Err(_) => return ENOENT,
    };
    let elf = match parse_elf(&elf_data) {
        Ok(e) => e,
        Err(_) => return ENOEXEC,
    };

    let new_cr3 = match loader::create_user_page_table_pub() {
        Ok(cr3) => cr3,
        Err(_) => return ENOMEM,
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
        Err(_) => return ENOEXEC,
    };

    // Commit the replacement image in place. WarExec intentionally keeps this narrow:
    // one process identity, one replacement ELF, same minimal argc/argv ABI, empty env.
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = process_table.get_mut(pid) else {
        return EPERM;
    };
    process.context = context;
    process.page_table_phys = new_cr3;
    process.address_space = address_space;
    process.memory_pages = memory_pages;
    process.name = path_str.rsplit('/').next().unwrap_or(path_str.as_str()).into();
    process.image_path = path_str;
    process.image_kind = ProcessImageKind::Elf;
    process.effective_capabilities =
        crate::security::capabilities::exec_capabilities(old_capabilities, uid);
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

pub fn sys_wait4(pid: i32, status_ptr: *mut i32, options: u32) -> i64 {
    if options != 0 {
        return ENOSYS;
    }
    if pid == 0 || pid < -1 {
        return ENOSYS;
    }

    let parent_pid = match current_pid() {
        Some(p) => p,
        None => return EPERM,
    };

    // WarExec currently exposes one narrow lifecycle observation path only:
    // `wait4(pid, status_ptr, 0)` where `pid` is either `-1` (any exited child)
    // or a direct child PID. The call only observes already-zombied children,
    // writes one exit-only status word, and immediately reaps the matching child.
    let found = PROCESS_TABLE.lock().find_zombie_child(parent_pid, pid);

    let (child_pid, exit_code) = match found {
        Some(r) => r,
        None => return ECHILD,
    };

    // Write exit status to user pointer.
    if !status_ptr.is_null() {
        let status = encode_wait_exit_status(exit_code);
        if let Err(error) = write_struct_to_user_checked(status_ptr, &status) {
            return error;
        }
    }

    PROCESS_TABLE.lock().remove(child_pid);
    i64::from(child_pid)
}

fn encode_wait_exit_status(exit_code: i32) -> i32 {
    (exit_code & 0xFF) << WAIT_STATUS_EXIT_SHIFT
}

fn role_for_uid(uid: u16) -> UserRole {
    USER_DB
        .lock()
        .find_by_uid(uid)
        .map(|user| user.role)
        .unwrap_or(UserRole::Admin)
}
