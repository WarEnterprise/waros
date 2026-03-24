use crate::auth::session;
use crate::exec::fd_table::DescriptorTarget;
use crate::exec::PROCESS_TABLE;
use crate::fs;

use super::{copy_to_user_ptr, read_user_string, write_struct_to_user, FileStat, ENOSYS};

pub fn sys_read(fd: u32, buffer: *mut u8, len: usize) -> i64 {
    if buffer.is_null() {
        return -1;
    }

    let path = {
        let process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get(pid)) else {
            return -1;
        };
        let Some(descriptor) = process.fd_table.get(fd) else {
            return -1;
        };
        match &descriptor.target {
            DescriptorTarget::File(path) => path.clone(),
            _ => return ENOSYS,
        }
    };

    let Ok((_, data)) = fs::read_current(&path) else {
        return -1;
    };
    let bytes = &data[..data.len().min(len)];
    // SAFETY: The destination buffer belongs to the userspace caller.
    unsafe { copy_to_user_ptr(buffer, bytes) as i64 }
}

pub fn sys_write(fd: u32, buffer: *const u8, len: usize) -> i64 {
    if buffer.is_null() {
        return -1;
    }

    // SAFETY: The caller is responsible for passing a valid userspace pointer.
    let bytes = unsafe { core::slice::from_raw_parts(buffer, len) };
    match fd {
        1 | 2 => {
            if let Ok(text) = core::str::from_utf8(bytes) {
                crate::kprint!("{}", text);
            } else {
                for &byte in bytes {
                    crate::kprint!("{}", char::from(byte));
                }
            }
            len as i64
        }
        _ => ENOSYS,
    }
}

pub fn sys_open(path: *const u8, _flags: u32, _mode: u32) -> i64 {
    let Some(path) = (unsafe { read_user_string(path, 256) }) else {
        return -1;
    };
    let resolved = session::resolve_path(&path);
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return -1;
    };
    process
        .fd_table
        .insert(DescriptorTarget::File(resolved)) as i64
}

pub fn sys_close(fd: u32) -> i64 {
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return -1;
    };
    if process.fd_table.close(fd) { 0 } else { -1 }
}

pub fn sys_stat(path: *const u8, stat_out: *mut u8) -> i64 {
    let Some(path) = (unsafe { read_user_string(path, 256) }) else {
        return -1;
    };
    let Ok(entry) = fs::stat_current(&path) else {
        return -1;
    };
    let stat = FileStat {
        size: entry.data.len() as u64,
        created_at: entry.created_at,
        modified_at: entry.modified_at,
        owner_uid: entry.owner_uid,
        readonly: u8::from(entry.readonly),
        _reserved: [0; 5],
    };
    // SAFETY: The destination pointer belongs to the syscall caller.
    if unsafe { write_struct_to_user(stat_out.cast::<FileStat>(), &stat) } {
        0
    } else {
        -1
    }
}

pub fn sys_seek(_fd: u32, _offset: i64, _whence: u32) -> i64 {
    ENOSYS
}

pub fn sys_getcwd(buffer: *mut u8, len: usize) -> i64 {
    let cwd = session::resolve_path(".");
    let bytes = cwd.as_bytes();
    let to_copy = bytes.len().min(len.saturating_sub(1));
    // SAFETY: The destination buffer belongs to the syscall caller.
    let copied = unsafe { copy_to_user_ptr(buffer, &bytes[..to_copy]) };
    if copied == 0 {
        return -1;
    }
    // SAFETY: `buffer` is valid for `copied + 1` bytes by syscall contract.
    unsafe {
        buffer.add(copied).write(0);
    }
    copied as i64
}

pub fn sys_chdir(path: *const u8) -> i64 {
    let Some(path) = (unsafe { read_user_string(path, 256) }) else {
        return -1;
    };
    let resolved = session::resolve_path(&path);
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return -1;
    };
    process.cwd = resolved;
    0
}
