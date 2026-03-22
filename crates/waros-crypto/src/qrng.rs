use waros_quantum::{Circuit, Simulator};

/// Generate `n` quantum-random bytes via simulated |+> measurements.
#[must_use]
pub fn random_bytes(n: usize) -> Vec<u8> {
    let bits = random_bits(n.saturating_mul(8));
    bits.chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0u8, |byte, (index, bit)| byte | (u8::from(*bit) << index))
        })
        .collect()
}

/// Generate a quantum-random `u64`.
#[must_use]
pub fn random_u64() -> u64 {
    let bytes = random_bytes(8);
    let mut output = [0u8; 8];
    output.copy_from_slice(&bytes);
    u64::from_le_bytes(output)
}

/// Generate `n` quantum-random bits.
#[must_use]
pub fn random_bits(n: usize) -> Vec<bool> {
    let mut output = Vec::with_capacity(n);
    let simulator = Simulator::new();

    while output.len() < n {
        let batch = (n - output.len()).min(8);
        let Some(bits) = sample_batch(&simulator, batch) else {
            break;
        };
        output.extend(bits.into_iter().take(batch));
    }

    output
}

/// Generate a 32-byte quantum-random seed.
#[must_use]
pub fn random_seed() -> [u8; 32] {
    let bytes = random_bytes(32);
    let mut output = [0u8; 32];
    output.copy_from_slice(&bytes);
    output
}

fn sample_batch(simulator: &Simulator, width: usize) -> Option<Vec<bool>> {
    let mut circuit = Circuit::new(width).ok()?;
    for qubit in 0..width {
        circuit.h(qubit).ok()?;
    }
    circuit.measure_all().ok()?;
    let result = simulator.run(&circuit, 1).ok()?;
    let bitstring = result.most_probable().0;
    Some(bitstring.bytes().map(|bit| bit == b'1').collect())
}
