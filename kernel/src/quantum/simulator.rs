use alloc::vec;
use alloc::vec::Vec;

use libm::sqrt;

use crate::quantum::gates::{Gate1Q, Gate2Q};
use crate::quantum::state::{cadd, cmul, cscale, norm_sq, QuantumState, EPSILON};

/// Tiny xorshift PRNG for kernel-side measurement sampling.
pub struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    /// Construct the generator from a non-zero seed.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    /// Return the next pseudo-random 64-bit value.
    #[must_use]
    pub fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }

    /// Return a floating-point sample in the range `[0, 1)`.
    #[must_use]
    pub fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// Apply a single-qubit gate to the state vector.
pub fn apply_1q(
    state: &mut QuantumState,
    target: usize,
    gate: &Gate1Q,
) -> Result<(), &'static str> {
    validate_qubit(state, target)?;

    let mask = 1usize << target;
    for index in 0..state.dimension() {
        if index & mask != 0 {
            continue;
        }

        let pair_index = index | mask;
        let amplitude_zero = state.amplitudes[index];
        let amplitude_one = state.amplitudes[pair_index];

        state.amplitudes[index] = cadd(
            cmul(gate.matrix[0][0], amplitude_zero),
            cmul(gate.matrix[0][1], amplitude_one),
        );
        state.amplitudes[pair_index] = cadd(
            cmul(gate.matrix[1][0], amplitude_zero),
            cmul(gate.matrix[1][1], amplitude_one),
        );
    }

    Ok(())
}

/// Apply a two-qubit gate using little-endian basis ordering.
pub fn apply_2q(
    state: &mut QuantumState,
    q0: usize,
    q1: usize,
    gate: &Gate2Q,
) -> Result<(), &'static str> {
    validate_two_qubits(state, q0, q1)?;

    let mask0 = 1usize << q0;
    let mask1 = 1usize << q1;

    for index in 0..state.dimension() {
        if index & mask0 != 0 || index & mask1 != 0 {
            continue;
        }

        let indices = [index, index | mask1, index | mask0, index | mask0 | mask1];
        let amplitudes = [
            state.amplitudes[indices[0]],
            state.amplitudes[indices[1]],
            state.amplitudes[indices[2]],
            state.amplitudes[indices[3]],
        ];

        for row in 0..4 {
            let mut value = (0.0, 0.0);
            for column in 0..4 {
                value = cadd(value, cmul(gate.matrix[row][column], amplitudes[column]));
            }
            state.amplitudes[indices[row]] = value;
        }
    }

    Ok(())
}

/// Apply a Toffoli gate directly by swapping target amplitudes when both controls are `1`.
pub fn apply_toffoli(
    state: &mut QuantumState,
    control0: usize,
    control1: usize,
    target: usize,
) -> Result<(), &'static str> {
    validate_qubit(state, control0)?;
    validate_qubit(state, control1)?;
    validate_qubit(state, target)?;

    if control0 == control1 || control0 == target || control1 == target {
        return Err("Toffoli requires three distinct qubits");
    }

    let control0_mask = 1usize << control0;
    let control1_mask = 1usize << control1;
    let target_mask = 1usize << target;

    for index in 0..state.dimension() {
        let controls_high = index & control0_mask != 0 && index & control1_mask != 0;
        let target_low = index & target_mask == 0;

        if controls_high && target_low {
            let partner = index | target_mask;
            state.amplitudes.swap(index, partner);
        }
    }

    Ok(())
}

/// Measure all qubits without collapsing the register.
pub fn measure_all(
    state: &QuantumState,
    shots: usize,
    rng: &mut Xorshift64,
) -> Vec<(usize, usize)> {
    let qubits: Vec<usize> = (0..state.num_qubits).collect();
    measure_selected(state, &qubits, shots, rng).unwrap_or_default()
}

/// Measure a subset of qubits without collapsing the register.
pub fn measure_selected(
    state: &QuantumState,
    qubits: &[usize],
    shots: usize,
    rng: &mut Xorshift64,
) -> Result<Vec<(usize, usize)>, &'static str> {
    validate_qubit_set(state, qubits)?;

    let probabilities = aggregate_probabilities(state, qubits);
    Ok(sample_counts(&probabilities, shots, rng))
}

/// Projectively measure a single qubit and collapse the state.
pub fn measure_qubit_collapse(
    state: &mut QuantumState,
    qubit: usize,
    rng: &mut Xorshift64,
) -> Result<u8, &'static str> {
    validate_qubit(state, qubit)?;

    let mask = 1usize << qubit;
    let probability_one: f64 = state
        .amplitudes
        .iter()
        .enumerate()
        .filter(|(index, _)| index & mask != 0)
        .map(|(_, amplitude)| norm_sq(*amplitude))
        .sum();

    let probability_one = probability_one.clamp(0.0, 1.0);
    let draw = rng.next_f64();
    let outcome = if probability_one <= EPSILON {
        0u8
    } else if (1.0 - probability_one) <= EPSILON || draw < probability_one {
        1u8
    } else {
        0u8
    };

    let selected_probability = if outcome == 1 {
        probability_one
    } else {
        1.0 - probability_one
    };
    if selected_probability <= EPSILON {
        return Err("Measurement collapsed onto a numerically empty branch");
    }

    let normalization = 1.0 / sqrt(selected_probability);
    for (index, amplitude) in state.amplitudes.iter_mut().enumerate() {
        let matches = ((index & mask) != 0) == (outcome == 1);
        if matches {
            *amplitude = cscale(*amplitude, normalization);
        } else {
            *amplitude = (0.0, 0.0);
        }
    }

    Ok(outcome)
}

fn aggregate_probabilities(state: &QuantumState, qubits: &[usize]) -> Vec<f64> {
    let mut probabilities = vec![0.0; 1usize << qubits.len()];
    for (basis_index, amplitude) in state.amplitudes.iter().copied().enumerate() {
        let mut reduced_index = 0usize;
        for (reduced_bit, qubit) in qubits.iter().copied().enumerate() {
            if (basis_index >> qubit) & 1 == 1 {
                reduced_index |= 1usize << reduced_bit;
            }
        }
        probabilities[reduced_index] += norm_sq(amplitude);
    }
    probabilities
}

fn sample_counts(probabilities: &[f64], shots: usize, rng: &mut Xorshift64) -> Vec<(usize, usize)> {
    if shots == 0 || probabilities.is_empty() {
        return Vec::new();
    }

    let mut counts = vec![0usize; probabilities.len()];
    for _ in 0..shots {
        let draw = rng.next_f64();
        let mut cumulative = 0.0;
        let mut selected = probabilities.len().saturating_sub(1);

        for (index, probability) in probabilities.iter().copied().enumerate() {
            cumulative += probability;
            if draw < cumulative || index + 1 == probabilities.len() {
                selected = index;
                break;
            }
        }

        counts[selected] += 1;
    }

    let mut results: Vec<(usize, usize)> = counts
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .collect();
    results.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    results
}

fn validate_qubit(state: &QuantumState, qubit: usize) -> Result<(), &'static str> {
    if qubit >= state.num_qubits {
        Err("Qubit index out of range")
    } else {
        Ok(())
    }
}

fn validate_two_qubits(state: &QuantumState, q0: usize, q1: usize) -> Result<(), &'static str> {
    validate_qubit(state, q0)?;
    validate_qubit(state, q1)?;
    if q0 == q1 {
        Err("Two-qubit gate requires two distinct qubits")
    } else {
        Ok(())
    }
}

fn validate_qubit_set(state: &QuantumState, qubits: &[usize]) -> Result<(), &'static str> {
    let mut seen = 0u32;
    for qubit in qubits.iter().copied() {
        validate_qubit(state, qubit)?;
        if qubit >= u32::BITS as usize {
            return Err("Qubit set exceeds kernel measurement bitset range");
        }
        let mask = 1u32 << qubit;
        if seen & mask != 0 {
            return Err("Measurement qubits must be unique");
        }
        seen |= mask;
    }
    Ok(())
}
