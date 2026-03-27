use alloc::string::String;
use alloc::vec::Vec;

pub mod ai;
pub mod crypto;
pub mod file;
pub mod io;
pub mod memory;
pub mod network;
pub mod process;
pub mod quantum;
pub mod signal;
pub mod time;

use crate::exec::PROCESS_TABLE;

pub const EPERM: i64 = -1;
pub const ENOENT: i64 = -2;
pub const ENOEXEC: i64 = -8;
pub const EBADF: i64 = -9;
pub const ECHILD: i64 = -10;
pub const ENOMEM: i64 = -12;
pub const EFAULT: i64 = -14;
pub const EINVAL: i64 = -22;
pub const ENOSYS: i64 = -38;
pub const MAX_USER_STRING_LEN: usize = 256;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WarExecStat {
    pub size: u64,
    pub file_type: u8,
    pub readonly: u8,
    pub _reserved: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct WarExecDirEntry {
    pub file_type: u8,
    pub name_len: u8,
    pub _reserved: [u8; 6],
    pub name: [u8; WAREXEC_DIRENT_NAME_CAPACITY],
}

pub const WAREXEC_FILE_TYPE_REGULAR: u8 = 1;
pub const WAREXEC_FILE_TYPE_DIRECTORY: u8 = 2;
pub const WAREXEC_DIRENT_NAME_CAPACITY: usize = 32;
pub const WAREXEC_OPEN_CREATE: u32 = 0x0000_0001;
pub const WAREXEC_OPEN_WRITE: u32 = 0x0000_0002;
pub const WAREXEC_OPEN_CREATE_WRITE: u32 = WAREXEC_OPEN_CREATE | WAREXEC_OPEN_WRITE;
pub const WAREXEC_OPEN_DIRECTORY: u32 = 0x0001_0000;

#[derive(Clone, Copy)]
pub enum WarExecPathKind {
    FileLike,
    Directory,
}

#[must_use]
pub fn current_pid() -> Option<u32> {
    crate::exec::current_pid()
}

pub fn validate_user_range(address: u64, len: usize) -> Result<(), i64> {
    if len == 0 {
        return Ok(());
    }
    let Some(pid) = current_pid() else {
        return Err(EPERM);
    };
    let process_table = PROCESS_TABLE.lock();
    let Some(process) = process_table.get(pid) else {
        return Err(EPERM);
    };
    if process.address_space.contains_user_range(address, len) {
        Ok(())
    } else {
        Err(EFAULT)
    }
}

pub fn validate_user_pointer<T>(pointer: *const T) -> Result<(), i64> {
    validate_user_range(pointer as u64, core::mem::size_of::<T>())
}

pub fn copy_from_user_ptr_checked(source: *const u8, len: usize) -> Result<Vec<u8>, i64> {
    if len == 0 {
        return Ok(Vec::new());
    }
    validate_user_range(source as u64, len)?;
    let mut bytes = Vec::with_capacity(len);
    bytes.resize(len, 0);
    // SAFETY: The user range was validated against the current process address space above.
    unsafe {
        core::ptr::copy_nonoverlapping(source, bytes.as_mut_ptr(), len);
    }
    Ok(bytes)
}

pub fn copy_to_user_ptr_checked(destination: *mut u8, bytes: &[u8]) -> Result<usize, i64> {
    if bytes.is_empty() {
        return Ok(0);
    }
    validate_user_range(destination as u64, bytes.len())?;
    // SAFETY: The destination range was validated against the current process address space above.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
    }
    Ok(bytes.len())
}

pub fn write_struct_to_user_checked<T: Copy>(destination: *mut T, value: &T) -> Result<(), i64> {
    validate_user_range(destination as u64, core::mem::size_of::<T>())?;
    // SAFETY: The destination range was validated against the current process address space above.
    unsafe {
        destination.write(*value);
    }
    Ok(())
}

pub fn read_user_string_checked(pointer: *const u8, max_len: usize) -> Result<String, i64> {
    if pointer.is_null() || max_len == 0 {
        return Err(EFAULT);
    }

    let Some(pid) = current_pid() else {
        return Err(EPERM);
    };
    let process_table = PROCESS_TABLE.lock();
    let Some(process) = process_table.get(pid) else {
        return Err(EPERM);
    };
    let address_space = process.address_space.clone();
    drop(process_table);

    let mut bytes = Vec::new();
    for index in 0..max_len {
        let Some(address) = (pointer as u64).checked_add(index as u64) else {
            return Err(EFAULT);
        };
        if !address_space.contains_user_range(address, 1) {
            return Err(EFAULT);
        }
        // SAFETY: Each byte address is validated before it is dereferenced.
        let byte = unsafe { pointer.add(index).read() };
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|_| EINVAL);
        }
        bytes.push(byte);
    }

    Err(EFAULT)
}

pub fn read_warexec_path_checked(
    pointer: *const u8,
    path_kind: WarExecPathKind,
) -> Result<String, i64> {
    let path = read_user_string_checked(pointer, MAX_USER_STRING_LEN)?;
    crate::fs::canonicalize_warexec_path(&path, matches!(path_kind, WarExecPathKind::Directory))
        .map_err(|_| EINVAL)
}

pub fn read_user_pointer_array_checked(
    pointer: *const *const u8,
    max_entries: usize,
) -> Result<Vec<*const u8>, i64> {
    if pointer.is_null() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for index in 0..max_entries {
        let entry_ptr = pointer.wrapping_add(index);
        validate_user_pointer(entry_ptr)?;
        // SAFETY: The pointer-sized slot was validated before it is read.
        let value = unsafe { entry_ptr.read() };
        if value.is_null() {
            break;
        }
        entries.push(value);
    }
    Ok(entries)
}

pub unsafe fn copy_to_user_ptr(destination: *mut u8, bytes: &[u8]) -> usize {
    if destination.is_null() {
        return 0;
    }
    // SAFETY: The caller validates the user pointer contract for the current syscall.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    bytes.len()
}

pub unsafe fn write_struct_to_user<T: Copy>(destination: *mut T, value: &T) -> bool {
    if destination.is_null() {
        return false;
    }
    // SAFETY: The caller validates the user pointer contract for the current syscall.
    unsafe { destination.write(*value) };
    true
}

pub unsafe fn read_user_string(pointer: *const u8, max_len: usize) -> Option<String> {
    if pointer.is_null() || max_len == 0 {
        return None;
    }

    let mut bytes = alloc::vec::Vec::new();
    for index in 0..max_len {
        // SAFETY: The syscall boundary owns validation for the user pointer.
        let byte = unsafe { pointer.add(index).read() };
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8(bytes).ok()
}
