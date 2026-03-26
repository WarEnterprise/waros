use crate::auth::session;
use crate::exec::fd_table::{DescriptorTarget, FileHandle};
use crate::exec::PROCESS_TABLE;
use crate::fs;

use super::{
    copy_from_user_ptr_checked, copy_to_user_ptr_checked, read_user_string_checked,
    write_struct_to_user_checked, FileStat, EBADF, EINVAL, ENOENT, ENOSYS, EPERM,
    MAX_USER_STRING_LEN,
};

pub fn sys_read(fd: u32, buffer: *mut u8, len: usize) -> i64 {
    // The current minimal ABI only supports plain WarFS file descriptors. Each
    // successful `open(path, 0, 0)` gets an independent per-FD read offset. `read`
    // consumes from the current offset, advances it by the bytes actually copied,
    // and returns 0 at EOF.
    let (path, start_offset) = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return EPERM;
        };
        let Some(descriptor) = process.fd_table.get_mut(fd) else {
            return EBADF;
        };
        match &mut descriptor.target {
            DescriptorTarget::File(handle) => (handle.path.clone(), handle.offset),
            _ => return ENOSYS,
        }
    };

    if len == 0 {
        return 0;
    }

    let Ok((_, data)) = fs::read_current(&path) else {
        return ENOENT;
    };
    let start = start_offset.min(data.len());
    let end = start.saturating_add(len).min(data.len());
    if start >= end {
        return 0;
    }
    let bytes = &data[start..end];
    let copied = match copy_to_user_ptr_checked(buffer, bytes) {
        Ok(copied) => copied,
        Err(error) => return error,
    };

    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    let Some(descriptor) = process.fd_table.get_mut(fd) else {
        return EBADF;
    };
    match &mut descriptor.target {
        DescriptorTarget::File(handle) => {
            handle.offset = start.saturating_add(copied);
            copied as i64
        }
        _ => ENOSYS,
    }
}

pub fn sys_write(fd: u32, buffer: *const u8, len: usize) -> i64 {
    let descriptor_target = {
        let process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get(pid)) else {
            return EPERM;
        };
        let Some(descriptor) = process.fd_table.get(fd) else {
            return EBADF;
        };
        descriptor.target.clone()
    };

    if len == 0 {
        return 0;
    }

    let bytes = match copy_from_user_ptr_checked(buffer, len) {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };

    match descriptor_target {
        DescriptorTarget::Stdout | DescriptorTarget::Stderr => {
            if let Ok(text) = core::str::from_utf8(bytes.as_slice()) {
                // Mirror user stdout/stderr to serial so headless QEMU smoke tests can
                // assert one real userspace-visible message without interactive input.
                crate::log_print!("{}", text);
            } else {
                for &byte in &bytes {
                    crate::kprint!("{}", char::from(byte));
                    crate::serial_print!("{}", char::from(byte));
                }
            }
            len as i64
        }
        _ => ENOSYS,
    }
}

pub fn sys_open(path: *const u8, flags: u32, mode: u32) -> i64 {
    if flags != 0 || mode != 0 {
        return ENOSYS;
    }

    let path = match read_user_string_checked(path, MAX_USER_STRING_LEN) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let resolved = session::resolve_path(&path);
    if fs::read_current(&resolved).is_err() {
        return ENOENT;
    }

    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    process
        .fd_table
        .insert(DescriptorTarget::File(FileHandle {
            path: resolved,
            offset: 0,
        })) as i64
}

pub fn sys_close(fd: u32) -> i64 {
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    if process.fd_table.close(fd) { 0 } else { EBADF }
}

pub fn sys_stat(path: *const u8, stat_out: *mut u8) -> i64 {
    let path = match read_user_string_checked(path, MAX_USER_STRING_LEN) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let resolved = session::resolve_path(&path);
    let Ok(entry) = fs::stat_current(&resolved) else {
        return ENOENT;
    };
    let stat = FileStat {
        size: entry.data.len() as u64,
        created_at: entry.created_at,
        modified_at: entry.modified_at,
        owner_uid: entry.owner_uid,
        readonly: u8::from(entry.readonly),
        _reserved: [0; 5],
    };
    match write_struct_to_user_checked(stat_out.cast::<FileStat>(), &stat) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

pub fn sys_seek(_fd: u32, _offset: i64, _whence: u32) -> i64 {
    ENOSYS
}

pub fn sys_getcwd(buffer: *mut u8, len: usize) -> i64 {
    if len == 0 {
        return EINVAL;
    }

    let cwd = session::resolve_path(".");
    let bytes = cwd.as_bytes();
    let to_copy = bytes.len().min(len.saturating_sub(1));
    let copied = match copy_to_user_ptr_checked(buffer, &bytes[..to_copy]) {
        Ok(copied) => copied,
        Err(error) => return error,
    };
    if let Err(error) = copy_to_user_ptr_checked(buffer.wrapping_add(copied), &[0]) {
        return error;
    }
    copied as i64
}

pub fn sys_chdir(path: *const u8) -> i64 {
    let path = match read_user_string_checked(path, MAX_USER_STRING_LEN) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let resolved = session::resolve_path(&path);
    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    process.cwd = resolved;
    0
}
