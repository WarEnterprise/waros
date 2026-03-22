use sha3::{
    digest::{ExtendableOutput, Update, XofReader},
    Digest, Sha3_256, Sha3_512, Shake128, Shake256,
};

/// Compute SHA3-256 over `data`.
#[must_use]
pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    Update::update(&mut hasher, data);
    hasher.finalize().into()
}

/// Compute SHA3-512 over `data`.
#[must_use]
pub fn sha3_512(data: &[u8]) -> [u8; 64] {
    let mut hasher = Sha3_512::new();
    Update::update(&mut hasher, data);
    hasher.finalize().into()
}

/// Compute SHAKE128 over `data` and return `output_len` bytes.
#[must_use]
pub fn shake128(data: &[u8], output_len: usize) -> Vec<u8> {
    let mut hasher = Shake128::default();
    hasher.update(data);
    let mut reader = hasher.finalize_xof();
    let mut output = vec![0u8; output_len];
    reader.read(&mut output);
    output
}

/// Compute SHAKE256 over `data` and return `output_len` bytes.
#[must_use]
pub fn shake256(data: &[u8], output_len: usize) -> Vec<u8> {
    let mut hasher = Shake256::default();
    hasher.update(data);
    let mut reader = hasher.finalize_xof();
    let mut output = vec![0u8; output_len];
    reader.read(&mut output);
    output
}
