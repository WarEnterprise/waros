use crate::auth::session;
use crate::exec::fd_table::{
    DescriptorTarget, DirectoryEntryHandle, DirectoryHandle, FileHandle, FileHandleAccess,
};
use crate::exec::PROCESS_TABLE;
use crate::fs;
use crate::fs::FsError;

use super::{
    copy_from_user_ptr_checked, copy_to_user_ptr_checked, read_user_string_checked,
    read_warexec_path_checked, write_struct_to_user_checked, WarExecDirEntry, WarExecStat,
    WarExecPathKind, WAREXEC_DIRENT_NAME_CAPACITY, WAREXEC_FILE_TYPE_DIRECTORY,
    WAREXEC_FILE_TYPE_REGULAR, WAREXEC_OPEN_CREATE_WRITE, WAREXEC_OPEN_DIRECTORY, EBADF,
    EEXIST, EINVAL, ENAMETOOLONG, ENOENT, ENOSPC, ENOSYS, EPERM, MAX_USER_STRING_LEN,
};

fn map_fs_error(error: FsError) -> i64 {
    match error {
        FsError::FileNotFound => ENOENT,
        FsError::AlreadyExists => EEXIST,
        FsError::PermissionDenied | FsError::ReadOnly => EPERM,
        FsError::InvalidFilename => EINVAL,
        FsError::FilenameTooLong => ENAMETOOLONG,
        FsError::FileTooLarge => ENOSPC,
        FsError::FilesystemFull => ENOSPC,
    }
}

fn warexec_stat_from_entry(entry: &fs::FileEntry) -> WarExecStat {
    WarExecStat {
        size: entry.data.len() as u64,
        file_type: WAREXEC_FILE_TYPE_REGULAR,
        readonly: u8::from(entry.readonly),
        _reserved: [0; 6],
    }
}

fn directory_handle_from_path(path: &str) -> Result<DirectoryHandle, i64> {
    let (resolved, entries) = fs::list_entries_current(Some(path)).map_err(map_fs_error)?;
    let entries = entries
        .into_iter()
        .map(|entry| DirectoryEntryHandle {
            name: entry.name,
            is_dir: entry.is_dir,
        })
        .collect();
    Ok(DirectoryHandle {
        path: resolved,
        entries,
        cursor: 0,
    })
}

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
            DescriptorTarget::File(handle)
                if matches!(handle.access, FileHandleAccess::ReadOnly) =>
            {
                (handle.path.clone(), handle.offset)
            }
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
        DescriptorTarget::File(handle)
            if matches!(handle.access, FileHandleAccess::ReadOnly) =>
        {
            handle.offset = start.saturating_add(copied);
            copied as i64
        }
        _ => ENOSYS,
    }
}

pub fn sys_write(fd: u32, buffer: *const u8, len: usize) -> i64 {
    if len == 0 {
        return 0;
    }

    let bytes = match copy_from_user_ptr_checked(buffer, len) {
        Ok(bytes) => bytes,
        Err(error) => return error,
    };

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
        DescriptorTarget::File(handle)
            if matches!(handle.access, FileHandleAccess::CreateWrite) =>
        {
            let mut process_table = PROCESS_TABLE.lock();
            let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
                return EPERM;
            };
            let Some(descriptor) = process.fd_table.get_mut(fd) else {
                return EBADF;
            };
            let DescriptorTarget::File(handle) = &mut descriptor.target else {
                return ENOSYS;
            };
            if !matches!(handle.access, FileHandleAccess::CreateWrite) {
                return ENOSYS;
            }
            let Some(end_offset) = handle.offset.checked_add(bytes.len()) else {
                return EINVAL;
            };
            if end_offset > fs::MAX_FILE_SIZE {
                return EINVAL;
            }
            if handle.staged_data.len() < end_offset {
                handle.staged_data.resize(end_offset, 0);
            }
            handle.staged_data[handle.offset..end_offset].copy_from_slice(bytes.as_slice());
            handle.offset = end_offset;
            bytes.len() as i64
        }
        _ => ENOSYS,
    }
}

pub fn sys_open(path: *const u8, flags: u32, mode: u32) -> i64 {
    if mode != 0 {
        return ENOSYS;
    }
    let open_directory = flags == WAREXEC_OPEN_DIRECTORY;
    let open_create_write = flags == WAREXEC_OPEN_CREATE_WRITE;
    if flags != 0 && !open_directory && !open_create_write {
        return ENOSYS;
    }

    let resolved = match read_warexec_path_checked(
        path,
        if open_directory {
            WarExecPathKind::Directory
        } else {
            WarExecPathKind::FileLike
        },
    ) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let descriptor_target = if open_directory {
        match directory_handle_from_path(&resolved) {
            Ok(handle) => DescriptorTarget::Directory(handle),
            Err(error) => return error,
        }
    } else if open_create_write {
        if let Err(error) = fs::validate_create_new_current(&resolved) {
            return map_fs_error(error);
        }
        DescriptorTarget::File(FileHandle {
            path: resolved,
            offset: 0,
            access: FileHandleAccess::CreateWrite,
            staged_data: alloc::vec::Vec::new(),
        })
    } else {
        if let Err(error) = fs::read_current(&resolved) {
            return map_fs_error(error);
        }
        DescriptorTarget::File(FileHandle {
            path: resolved,
            offset: 0,
            access: FileHandleAccess::ReadOnly,
            staged_data: alloc::vec::Vec::new(),
        })
    };

    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    process.fd_table.insert(descriptor_target) as i64
}

pub fn sys_close(fd: u32) -> i64 {
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

    if let DescriptorTarget::File(handle) = &descriptor_target {
        if matches!(handle.access, FileHandleAccess::CreateWrite) {
            if let Err(error) = fs::create_new_current(&handle.path, &handle.staged_data) {
                return map_fs_error(error);
            }
        }
    }

    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return EPERM;
    };
    if process.fd_table.close(fd) { 0 } else { EBADF }
}

pub fn sys_stat(path: *const u8, stat_out: *mut u8) -> i64 {
    let resolved = match read_warexec_path_checked(path, WarExecPathKind::FileLike) {
        Ok(path) => path,
        Err(error) => return error,
    };
    let entry = match fs::stat_current(&resolved) {
        Ok(entry) => entry,
        Err(error) => return map_fs_error(error),
    };
    let stat = warexec_stat_from_entry(&entry);
    match write_struct_to_user_checked(stat_out.cast::<WarExecStat>(), &stat) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

pub fn sys_fstat(fd: u32, stat_out: *mut u8) -> i64 {
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

    let stat = match descriptor_target {
        DescriptorTarget::File(handle)
            if matches!(handle.access, FileHandleAccess::CreateWrite) =>
        {
            WarExecStat {
                size: handle.staged_data.len() as u64,
                file_type: WAREXEC_FILE_TYPE_REGULAR,
                readonly: 0,
                _reserved: [0; 6],
            }
        }
        DescriptorTarget::File(handle) => {
            let entry = match fs::stat_current(&handle.path) {
                Ok(entry) => entry,
                Err(error) => return map_fs_error(error),
            };
            warexec_stat_from_entry(&entry)
        }
        _ => return ENOSYS,
    };
    match write_struct_to_user_checked(stat_out.cast::<WarExecStat>(), &stat) {
        Ok(()) => 0,
        Err(error) => error,
    }
}

pub fn sys_readdir(fd: u32, entry_out: *mut u8) -> i64 {
    let next_entry = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return EPERM;
        };
        let Some(descriptor) = process.fd_table.get_mut(fd) else {
            return EBADF;
        };
        match &mut descriptor.target {
            DescriptorTarget::Directory(handle) => {
                if handle.cursor >= handle.entries.len() {
                    return 0;
                }
                let entry = handle.entries[handle.cursor].clone();
                handle.cursor = handle.cursor.saturating_add(1);
                entry
            }
            _ => return ENOSYS,
        }
    };

    let name_bytes = next_entry.name.as_bytes();
    if name_bytes.len() > WAREXEC_DIRENT_NAME_CAPACITY {
        return EINVAL;
    }

    let mut name = [0u8; WAREXEC_DIRENT_NAME_CAPACITY];
    name[..name_bytes.len()].copy_from_slice(name_bytes);
    let entry = WarExecDirEntry {
        file_type: if next_entry.is_dir {
            WAREXEC_FILE_TYPE_DIRECTORY
        } else {
            WAREXEC_FILE_TYPE_REGULAR
        },
        name_len: name_bytes.len() as u8,
        _reserved: [0; 6],
        name,
    };
    match write_struct_to_user_checked(entry_out.cast::<WarExecDirEntry>(), &entry) {
        Ok(()) => 1,
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
