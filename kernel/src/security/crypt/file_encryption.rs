use alloc::vec::Vec;

use sha3::{Digest, Sha3_256};

use super::entropy;

/// Derive a 256-bit key from password + salt using iterated SHA-3.
fn derive_key(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut state = [0u8; 32];
    let mut hasher = Sha3_256::new();
    hasher.update(password);
    hasher.update(salt);
    let result = hasher.finalize();
    state.copy_from_slice(&result);

    for _ in 1..rounds {
        let mut hasher = Sha3_256::new();
        hasher.update(&state);
        hasher.update(password);
        let result = hasher.finalize();
        state.copy_from_slice(&result);
    }

    state
}

/// XOR-based stream cipher using SHA-3 in counter mode (AES-GCM requires more setup;
/// this is a pure-Rust fallback that works in no_std without hardware AES).
fn ctr_crypt(key: &[u8; 32], nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len());
    let mut counter = 0u64;
    let mut offset = 0;

    while offset < data.len() {
        let mut hasher = Sha3_256::new();
        hasher.update(key);
        hasher.update(nonce);
        hasher.update(&counter.to_le_bytes());
        let block = hasher.finalize();

        let chunk_len = (data.len() - offset).min(32);
        for i in 0..chunk_len {
            output.push(data[offset + i] ^ block[i]);
        }

        offset += chunk_len;
        counter += 1;
    }

    output
}

/// Compute a MAC tag over ciphertext for authentication.
fn compute_tag(key: &[u8; 32], nonce: &[u8; 12], ciphertext: &[u8]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(b"warcrypt-tag");
    hasher.update(key);
    hasher.update(nonce);
    hasher.update(ciphertext);
    hasher.update(&(ciphertext.len() as u64).to_le_bytes());
    let result = hasher.finalize();
    let mut tag = [0u8; 32];
    tag.copy_from_slice(&result);
    tag
}

const KDF_ROUNDS: u32 = 10_000;

/// Encrypt data with a password.
/// Returns: [16-byte salt][12-byte nonce][32-byte tag][ciphertext]
pub fn encrypt(data: &[u8], password: &str) -> Vec<u8> {
    let mut salt = [0u8; 16];
    entropy::random_bytes(&mut salt);

    let mut nonce = [0u8; 12];
    entropy::random_bytes(&mut nonce);

    let key = derive_key(password.as_bytes(), &salt, KDF_ROUNDS);
    let ciphertext = ctr_crypt(&key, &nonce, data);
    let tag = compute_tag(&key, &nonce, &ciphertext);

    let mut output = Vec::with_capacity(16 + 12 + 32 + ciphertext.len());
    output.extend_from_slice(&salt);
    output.extend_from_slice(&nonce);
    output.extend_from_slice(&tag);
    output.extend_from_slice(&ciphertext);
    output
}

/// Decrypt data with a password.
/// Expects format: [16-byte salt][12-byte nonce][32-byte tag][ciphertext]
pub fn decrypt(encrypted: &[u8], password: &str) -> Result<Vec<u8>, &'static str> {
    if encrypted.len() < 16 + 12 + 32 {
        return Err("data too short");
    }

    let salt = &encrypted[..16];
    let nonce: [u8; 12] = encrypted[16..28].try_into().map_err(|_| "bad nonce")?;
    let stored_tag = &encrypted[28..60];
    let ciphertext = &encrypted[60..];

    let key = derive_key(password.as_bytes(), salt, KDF_ROUNDS);

    // Verify tag before decrypting
    let expected_tag = compute_tag(&key, &nonce, ciphertext);
    if stored_tag != expected_tag.as_slice() {
        return Err("authentication failed: wrong password or corrupted data");
    }

    Ok(ctr_crypt(&key, &nonce, ciphertext))
}

/// Count encrypted files in the filesystem.
pub fn encrypted_file_count() -> usize {
    let fs = crate::fs::FILESYSTEM.lock();
    fs.list()
        .iter()
        .filter(|e| e.name.ends_with(".enc"))
        .count()
}
