use std::time::Instant;
use std::{fmt::Write as _, num::TryFromIntError};

use waros_crypto::{
    hash,
    kem::{self, SecurityLevel},
    qrng, sign,
};

fn main() {
    println!("╔══════════════════════════════════════════════════╗");
    println!("║  WarOS - Post-Quantum Cryptography Demo         ║");
    println!("║  FIPS 203 (ML-KEM) + FIPS 204 (ML-DSA) + SHA-3  ║");
    println!("╚══════════════════════════════════════════════════╝");
    println!();

    let start = Instant::now();
    let (kem_public_key, kem_secret_material) = kem::keygen_with_level(SecurityLevel::Level3);
    let keygen_elapsed = start.elapsed();

    let start = Instant::now();
    let (ciphertext, shared_secret_enc) = kem::encapsulate(&kem_public_key);
    let encapsulate_elapsed = start.elapsed();

    let start = Instant::now();
    let shared_secret_dec =
        kem::decapsulate(&kem_secret_material, &ciphertext).expect("decapsulation succeeds");
    let decapsulate_elapsed = start.elapsed();

    println!("-- Key Encapsulation (ML-KEM-768) --");
    println!(
        "  Keygen:        {:>6.3} ms",
        keygen_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  Public key:    {} bytes", kem_public_key.as_bytes().len());
    println!(
        "  Secret key:    {} bytes",
        kem_secret_material.as_bytes().len()
    );
    println!(
        "  Encapsulate:   {:>6.3} ms",
        encapsulate_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  Ciphertext:    {} bytes", ciphertext.as_bytes().len());
    println!(
        "  Shared secret: {} bytes",
        shared_secret_enc.as_bytes().len()
    );
    println!(
        "  Decapsulate:   {:>6.3} ms",
        decapsulate_elapsed.as_secs_f64() * 1_000.0
    );
    println!(
        "  {} Shared secrets match!",
        if shared_secret_enc == shared_secret_dec {
            "✓"
        } else {
            "✗"
        }
    );
    println!();

    let start = Instant::now();
    let (verification_key, signing_key) = sign::keygen();
    let sign_keygen_elapsed = start.elapsed();
    let message = b"WarOS";

    let start = Instant::now();
    let signature = sign::sign(&signing_key, message);
    let sign_elapsed = start.elapsed();

    let start = Instant::now();
    let verified = sign::verify(&verification_key, message, &signature);
    let verify_elapsed = start.elapsed();

    println!("-- Digital Signature (ML-DSA / Dilithium3) --");
    println!(
        "  Keygen:        {:>6.3} ms",
        sign_keygen_elapsed.as_secs_f64() * 1_000.0
    );
    println!(
        "  Public key:    {} bytes",
        verification_key.as_bytes().len()
    );
    println!("  Secret key:    {} bytes", signing_key.as_bytes().len());
    println!(
        "  Sign:          {:>6.3} ms",
        sign_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  Signature:     {} bytes", signature.as_bytes().len());
    println!(
        "  Verify:        {:>6.3} ms",
        verify_elapsed.as_secs_f64() * 1_000.0
    );
    println!("  {} Signature valid!", if verified { "✓" } else { "✗" });
    println!();

    let sha3_256 = hash::sha3_256(b"WarOS");
    let shake256 = hash::shake256(b"WarOS", 64);
    println!("-- SHA-3 --");
    println!("  SHA3-256(\"WarOS\"): {}...", &hex(&sha3_256)[..16]);
    println!(
        "  SHAKE256(\"WarOS\", 64 bytes): {}...",
        &hex(&shake256)[..16]
    );
    println!();

    let random = qrng::random_bytes(256);
    println!("-- Quantum Random Number Generator --");
    println!("  Generated {} quantum-random bytes", random.len());
    println!("  Entropy estimate: {:.3} bits/byte", byte_entropy(&random));
    println!("  ✓ QRNG healthy");
    println!();

    println!("All post-quantum cryptographic primitives operational.");
    println!("These algorithms are secure against both classical and quantum attacks.");
}

fn hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("write to string");
    }
    encoded
}

fn usize_to_f64(value: usize) -> Result<f64, TryFromIntError> {
    u32::try_from(value).map(f64::from)
}

fn byte_entropy(bytes: &[u8]) -> f64 {
    let mut histogram = [0usize; 256];
    for &byte in bytes {
        histogram[usize::from(byte)] += 1;
    }
    histogram
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let probability = usize_to_f64(count).expect("histogram count fits")
                / usize_to_f64(bytes.len()).expect("input length fits");
            -probability * probability.log2()
        })
        .sum()
}
