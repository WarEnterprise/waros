use alloc::string::String;

use sha3::{Digest, Sha3_256};

#[derive(Debug, Clone)]
pub struct IntegrityEntry {
    pub path: String,
    pub sha3_hash: [u8; 32],
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct IntegrityViolation {
    pub path: String,
    pub expected: String,
    pub actual: String,
}

pub fn compute_hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&result);
    hash
}

pub fn hash_to_hex(hash: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for byte in hash {
        s.push_str(&alloc::format!("{:02x}", byte));
    }
    s
}

pub fn verify_entry(entry: &IntegrityEntry) -> Result<(), IntegrityViolation> {
    let fs = crate::fs::FILESYSTEM.lock();
    match fs.read(&entry.path) {
        Ok(data) => {
            let actual_hash = compute_hash(data);
            if actual_hash == entry.sha3_hash {
                Ok(())
            } else {
                Err(IntegrityViolation {
                    path: entry.path.clone(),
                    expected: hash_to_hex(&entry.sha3_hash),
                    actual: hash_to_hex(&actual_hash),
                })
            }
        }
        Err(_) => Err(IntegrityViolation {
            path: entry.path.clone(),
            expected: hash_to_hex(&entry.sha3_hash),
            actual: String::from("<file missing>"),
        }),
    }
}
