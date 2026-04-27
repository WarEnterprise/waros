use core::sync::atomic::{AtomicU64, Ordering};

use sha3::{Digest, Sha3_256};

static SALT_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Number of SHA3-256 stretching iterations for password hashing.
/// Higher = slower brute-force attacks. 10000 is a pragmatic minimum for a
/// no_std kernel without hardware AES; comparable to PBKDF2-HMAC-SHA256 at
/// similar iteration count.
const KDF_ITERATIONS: u32 = 10_000;

#[must_use]
pub fn hash_password(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    // Initial hash: H0 = SHA3-256(salt || password)
    let mut hasher = Sha3_256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    let mut state = hasher.finalize();

    // Key stretching: Hi = SHA3-256(Hi-1 || salt || i)
    // This makes brute-force proportionally more expensive.
    for i in 1..KDF_ITERATIONS {
        let mut hasher = Sha3_256::new();
        hasher.update(&state);
        hasher.update(salt);
        hasher.update(&i.to_le_bytes());
        state = hasher.finalize();
    }

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&state);
    hash
}

#[must_use]
pub fn generate_salt(uid: u16) -> [u8; 16] {
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let counter = SALT_COUNTER.fetch_add(1, Ordering::Relaxed);

    let mut hasher = Sha3_256::new();
    hasher.update(&ticks.to_le_bytes());
    hasher.update(&counter.to_le_bytes());
    hasher.update(&uid.to_le_bytes());
    hasher.update(&(ticks ^ counter).to_le_bytes());

    // Mix in hardware entropy from RDRAND if available
    let mut rdrand_buf = [0u8; 16];
    crate::security::crypt::entropy::random_bytes(&mut rdrand_buf);
    hasher.update(&rdrand_buf);

    let digest = hasher.finalize();

    let mut salt = [0u8; 16];
    salt.copy_from_slice(&digest[..16]);
    salt
}

#[must_use]
pub fn constant_time_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    let mut diff = 0u8;
    for index in 0..left.len() {
        diff |= left[index] ^ right[index];
    }
    diff == 0
}
