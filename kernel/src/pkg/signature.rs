use alloc::string::String;
use alloc::vec::Vec;

use sha3::{Digest, Sha3_256};

/// War Enterprise embedded root public key (placeholder — replace with real ML-DSA pk bytes).
pub const WAR_ENTERPRISE_PK: [u8; 64] = [
    0x57, 0x61, 0x72, 0x45, 0x6E, 0x74, 0x65, 0x72, 0x70, 0x72, 0x69, 0x73, 0x65, 0x2D, 0x50, 0x4B,
    0x2D, 0x76, 0x31, 0x2E, 0x30, 0x2D, 0x57, 0x61, 0x72, 0x4F, 0x53, 0x2D, 0x50, 0x6F, 0x73, 0x74,
    0x51, 0x75, 0x61, 0x6E, 0x74, 0x75, 0x6D, 0x2D, 0x50, 0x4B, 0x67, 0x2D, 0x53, 0x69, 0x67, 0x6E,
    0x69, 0x6E, 0x67, 0x2D, 0x53, 0x79, 0x73, 0x74, 0x65, 0x6D, 0x2D, 0x76, 0x31, 0x2E, 0x30, 0x00,
];

const DOMAIN_PREFIX: &[u8] = b"WarPkg-v1\x00";

/// Bootstrap package signature format used until the kernel links a full ML-DSA verifier.
/// The signature is the hex-encoded SHA3-256 digest of the package payload.
#[must_use]
pub fn sign_bootstrap(payload: &[u8]) -> String {
    hex_encode(&Sha3_256::digest(payload))
}

#[must_use]
pub fn verify_bootstrap(payload: &[u8], signature: &str) -> bool {
    sign_bootstrap(payload) == signature
}

/// Sign a package payload with the bootstrap scheme (SHA3-256 + domain prefix).
/// When ML-DSA is available, this should be replaced with a real ML-DSA signature.
#[must_use]
pub fn sign_pkg(payload: &[u8]) -> String {
    let mut data = alloc::vec::Vec::with_capacity(DOMAIN_PREFIX.len() + payload.len());
    data.extend_from_slice(DOMAIN_PREFIX);
    data.extend_from_slice(payload);
    hex_encode(&Sha3_256::digest(&data))
}

/// Verify a package signature. Accepts both the legacy bootstrap format and
/// the new domain-prefixed format.
#[must_use]
pub fn verify_pkg(payload: &[u8], signature: &str) -> bool {
    // Try new domain-prefixed format.
    if sign_pkg(payload) == signature {
        return true;
    }
    // Fall back to legacy SHA3 of raw payload.
    verify_bootstrap(payload, signature)
}

#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0F));
    }
    output
}

#[must_use]
pub fn sha256_hex(payload: &[u8]) -> String {
    use sha2::Digest;

    hex_encode(&sha2::Sha256::digest(payload))
}

#[must_use]
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }

    let mut output = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = decode_digit(bytes[index])?;
        let low = decode_digit(bytes[index + 1])?;
        output.push((high << 4) | low);
        index += 2;
    }
    Some(output)
}

fn hex_digit(value: u8) -> char {
    match value & 0x0F {
        0..=9 => char::from(b'0' + (value & 0x0F)),
        10..=15 => char::from(b'a' + ((value & 0x0F) - 10)),
        _ => '0',
    }
}

fn decode_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
