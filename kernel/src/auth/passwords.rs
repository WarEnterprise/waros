use core::sync::atomic::{AtomicU64, Ordering};

use sha3::{Digest, Sha3_256};

static SALT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[must_use]
pub fn hash_password(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(salt);
    hasher.update(password.as_bytes());
    let digest = hasher.finalize();

    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    hash
}

#[must_use]
pub fn generate_salt(uid: u16) -> [u8; 16] {
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let counter = SALT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut seed = [0u8; 24];
    seed[..8].copy_from_slice(&ticks.to_le_bytes());
    seed[8..16].copy_from_slice(&counter.to_le_bytes());
    seed[16..18].copy_from_slice(&uid.to_le_bytes());
    seed[18..24].copy_from_slice(&(ticks ^ counter).to_be_bytes()[..6]);

    let mut hasher = Sha3_256::new();
    hasher.update(seed);
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
