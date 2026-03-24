use sha3::{Digest, Sha3_256};

use super::{copy_to_user_ptr, ENOSYS};

pub fn sys_kem_keygen(_public_key: *mut u8, _secret_key: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_kem_encapsulate(_public_key: *const u8, _ciphertext: *mut u8, _shared_secret: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_kem_decapsulate(
    _secret_key: *const u8,
    _ciphertext: *const u8,
    _shared_secret: *mut u8,
) -> i64 {
    ENOSYS
}

pub fn sys_sign(_secret_key: *const u8, _message: *const u8, _len: usize, _signature: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_verify(_public_key: *const u8, _message: *const u8, _len: usize, _signature: *const u8) -> i64 {
    ENOSYS
}

pub fn sys_sha3_256(buffer: *const u8, len: usize, output: *mut u8) -> i64 {
    if buffer.is_null() || output.is_null() {
        return -1;
    }
    // SAFETY: The caller is responsible for passing a valid user buffer.
    let bytes = unsafe { core::slice::from_raw_parts(buffer, len) };
    let digest = Sha3_256::digest(bytes);
    // SAFETY: The destination buffer is owned by the caller of the syscall.
    unsafe { copy_to_user_ptr(output, &digest) as i64 }
}

pub fn sys_random_bytes(output: *mut u8, len: usize) -> i64 {
    if output.is_null() {
        return -1;
    }

    let mut state = crate::arch::x86_64::interrupts::tick_count()
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(0x5741_524F_53);
    let mut bytes = alloc::vec![0u8; len];
    for byte in &mut bytes {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = state as u8;
    }

    // SAFETY: The destination buffer is owned by the caller of the syscall.
    unsafe { copy_to_user_ptr(output, &bytes) as i64 }
}
