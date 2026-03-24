use alloc::string::String;

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

pub const ENOSYS: i64 = -38;
pub const EINVAL: i64 = -22;
pub const EPERM: i64 = -1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FileStat {
    pub size: u64,
    pub created_at: u64,
    pub modified_at: u64,
    pub owner_uid: u16,
    pub readonly: u8,
    pub _reserved: [u8; 5],
}

#[must_use]
pub fn current_pid() -> Option<u32> {
    crate::exec::current_pid()
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
