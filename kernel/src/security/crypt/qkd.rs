use alloc::vec::Vec;

use super::entropy;

/// BB84 basis: rectilinear (Z) or diagonal (X).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Basis {
    Z, // |0⟩, |1⟩
    X, // |+⟩, |−⟩
}

/// Result of a BB84 QKD simulation round.
pub struct Bb84Result {
    pub qubits_sent: usize,
    pub matching_bases: usize,
    pub error_rate: f64,
    pub eve_detected: bool,
    pub final_key_bits: usize,
    pub key: Vec<u8>,
}

/// Run a BB84 quantum key distribution simulation.
///
/// Uses real random numbers from the entropy pool to simulate:
/// 1. Alice prepares qubits in random bases with random values
/// 2. Bob measures in random bases
/// 3. They compare bases, keep matching bits
/// 4. Estimate error rate from a subset
/// 5. Privacy amplification to produce final key
pub fn simulate_bb84(n_qubits: usize) -> Bb84Result {
    let n = n_qubits.max(16).min(4096);

    // Alice: random bits and random bases
    let mut alice_bits = Vec::with_capacity(n);
    let mut alice_bases = Vec::with_capacity(n);
    for _ in 0..n {
        let r = entropy::random_u64();
        alice_bits.push((r & 1) as u8);
        alice_bases.push(if (r >> 1) & 1 == 0 { Basis::Z } else { Basis::X });
    }

    // Bob: random measurement bases
    let mut bob_bases = Vec::with_capacity(n);
    for _ in 0..n {
        let r = entropy::random_u64();
        bob_bases.push(if r & 1 == 0 { Basis::Z } else { Basis::X });
    }

    // Bob's measurement results:
    // If same basis → gets Alice's bit
    // If different basis → random result (50/50)
    let mut bob_bits = Vec::with_capacity(n);
    for i in 0..n {
        if alice_bases[i] == bob_bases[i] {
            bob_bits.push(alice_bits[i]);
        } else {
            let r = entropy::random_u64();
            bob_bits.push((r & 1) as u8);
        }
    }

    // Sifting: keep only bits where bases match
    let mut sifted_alice = Vec::new();
    let mut sifted_bob = Vec::new();
    for i in 0..n {
        if alice_bases[i] == bob_bases[i] {
            sifted_alice.push(alice_bits[i]);
            sifted_bob.push(bob_bits[i]);
        }
    }

    let matching_bases = sifted_alice.len();

    // Error estimation: use first 25% of sifted bits as check bits
    let check_count = (matching_bases / 4).max(1);
    let mut errors = 0usize;
    for i in 0..check_count.min(sifted_alice.len()) {
        if sifted_alice[i] != sifted_bob[i] {
            errors += 1;
        }
    }
    let error_rate = if check_count > 0 {
        errors as f64 / check_count as f64
    } else {
        0.0
    };

    // Eve detection: error > 11% indicates eavesdropping
    let eve_detected = error_rate > 0.11;

    // Privacy amplification: remaining bits after removing check bits
    let remaining_bits = if matching_bases > check_count {
        matching_bases - check_count
    } else {
        0
    };

    // Final key: compress to ~half the remaining bits for security margin
    let final_bits = remaining_bits / 2;
    let key_bytes = (final_bits + 7) / 8;

    let mut key = Vec::with_capacity(key_bytes);
    let mut bit_index = check_count; // start after check bits
    let mut current_byte = 0u8;
    let mut bit_in_byte = 0;

    for _ in 0..final_bits {
        if bit_index < sifted_alice.len() {
            current_byte |= sifted_alice[bit_index] << bit_in_byte;
            bit_index += 1;
        }
        bit_in_byte += 1;
        if bit_in_byte == 8 {
            key.push(current_byte);
            current_byte = 0;
            bit_in_byte = 0;
        }
    }
    if bit_in_byte > 0 {
        key.push(current_byte);
    }

    Bb84Result {
        qubits_sent: n,
        matching_bases,
        error_rate,
        eve_detected,
        final_key_bits: final_bits,
        key,
    }
}

/// Count QKD keys stored in /etc/quantum_keys/
pub fn stored_key_count() -> usize {
    let fs = crate::fs::FILESYSTEM.lock();
    fs.list()
        .iter()
        .filter(|e| e.name.starts_with("/etc/quantum_keys/"))
        .count()
}
