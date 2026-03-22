use std::mem::size_of;

use rand::Rng;
use rayon::prelude::*;

use crate::circuit::{Circuit, Instruction};
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::gate::Gate;

const NORMALIZATION_EPSILON: f64 = 1e-10;
const MEASUREMENT_EPSILON: f64 = 1e-15;
pub(super) const PARALLEL_QUBIT_THRESHOLD: usize = 16;

#[derive(Debug, Clone)]
pub(super) struct StateVectorSoA {
    real: Vec<f64>,
    imag: Vec<f64>,
}

impl StateVectorSoA {
    pub(super) fn probabilities(&self) -> Vec<f64> {
        self.real
            .iter()
            .zip(self.imag.iter())
            .map(|(real, imag)| real * real + imag * imag)
            .collect()
    }

    pub(super) fn into_aos(self) -> Vec<Complex> {
        self.real
            .into_iter()
            .zip(self.imag)
            .map(|(re, im)| Complex::new(re, im))
            .collect()
    }
}

pub(super) fn zero_state(num_qubits: usize) -> WarosResult<Vec<Complex>> {
    let dim = 1usize << num_qubits;
    let required_bytes = dim * size_of::<Complex>();
    let mut state = Vec::new();
    state
        .try_reserve_exact(dim)
        .map_err(|_| WarosError::InsufficientMemory(num_qubits, required_bytes))?;
    state.resize(dim, Complex::ZERO);
    state[0] = Complex::ONE;
    Ok(state)
}

pub(super) fn zero_state_soa(num_qubits: usize) -> WarosResult<StateVectorSoA> {
    let dim = 1usize << num_qubits;
    let required_bytes = dim * (size_of::<f64>() * 2);

    let mut real = Vec::new();
    real.try_reserve_exact(dim)
        .map_err(|_| WarosError::InsufficientMemory(num_qubits, required_bytes))?;
    real.resize(dim, 0.0);
    real[0] = 1.0;

    let mut imag = Vec::new();
    imag.try_reserve_exact(dim)
        .map_err(|_| WarosError::InsufficientMemory(num_qubits, required_bytes))?;
    imag.resize(dim, 0.0);

    Ok(StateVectorSoA { real, imag })
}

pub(super) fn apply_gate_sequence(
    state: &mut [Complex],
    num_qubits: usize,
    instructions: &[Instruction],
    parallel: bool,
) {
    for instruction in instructions {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                apply_gate(state, num_qubits, targets, gate, parallel);
            }
            Instruction::ConditionalGate { .. }
            | Instruction::Measure { .. }
            | Instruction::Barrier { .. } => {}
        }
    }
}

pub(super) fn apply_gate_sequence_soa(
    state: &mut StateVectorSoA,
    num_qubits: usize,
    instructions: &[Instruction],
    parallel: bool,
) {
    for instruction in instructions {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                apply_gate_soa(state, num_qubits, targets, gate, parallel);
            }
            Instruction::ConditionalGate { .. }
            | Instruction::Measure { .. }
            | Instruction::Barrier { .. } => {}
        }
    }
}

pub(super) fn apply_gate(
    state: &mut [Complex],
    num_qubits: usize,
    targets: &[usize],
    gate: &Gate,
    parallel: bool,
) {
    match gate.num_qubits {
        1 => apply_1q(state, num_qubits, targets[0], gate, parallel),
        2 => apply_2q(state, num_qubits, targets[0], targets[1], gate, parallel),
        _ => unreachable!("unsupported gate width"),
    }
}

pub(super) fn apply_gate_soa(
    state: &mut StateVectorSoA,
    num_qubits: usize,
    targets: &[usize],
    gate: &Gate,
    parallel: bool,
) {
    match gate.num_qubits {
        1 => apply_1q_soa(state, num_qubits, targets[0], gate, parallel),
        2 => apply_2q_soa(state, num_qubits, targets[0], targets[1], gate, parallel),
        _ => unreachable!("unsupported gate width"),
    }
}

pub(super) fn apply_1q(
    state: &mut [Complex],
    num_qubits: usize,
    target: usize,
    gate: &Gate,
    parallel: bool,
) {
    debug_assert!(target < num_qubits, "single-qubit target out of range");

    let mask = 1usize << target;
    let block_size = mask << 1;

    if parallel {
        state
            .par_chunks_exact_mut(block_size)
            .for_each(|block| apply_1q_block(block, mask, gate));
    } else {
        for block in state.chunks_exact_mut(block_size) {
            apply_1q_block(block, mask, gate);
        }
    }

    debug_assert_normalized(state);
}

pub(super) fn apply_1q_soa(
    state: &mut StateVectorSoA,
    num_qubits: usize,
    target: usize,
    gate: &Gate,
    parallel: bool,
) {
    debug_assert!(target < num_qubits, "single-qubit target out of range");

    let mask = 1usize << target;
    let block_size = mask << 1;

    if parallel {
        state
            .real
            .par_chunks_exact_mut(block_size)
            .zip(state.imag.par_chunks_exact_mut(block_size))
            .for_each(|(real_block, imag_block)| {
                apply_1q_block_soa(real_block, imag_block, mask, gate);
            });
    } else {
        for (real_block, imag_block) in state
            .real
            .chunks_exact_mut(block_size)
            .zip(state.imag.chunks_exact_mut(block_size))
        {
            apply_1q_block_soa(real_block, imag_block, mask, gate);
        }
    }

    debug_assert_normalized_soa(state);
}

pub(super) fn apply_2q(
    state: &mut [Complex],
    num_qubits: usize,
    q0: usize,
    q1: usize,
    gate: &Gate,
    parallel: bool,
) {
    debug_assert!(q0 < num_qubits, "two-qubit target out of range");
    debug_assert!(q1 < num_qubits, "two-qubit target out of range");
    debug_assert_ne!(q0, q1, "two-qubit gate requires distinct qubits");

    let block_size = 1usize << (q0.max(q1) + 1);
    if parallel {
        state
            .par_chunks_exact_mut(block_size)
            .for_each(|block| apply_2q_block(block, q0, q1, gate));
    } else {
        for block in state.chunks_exact_mut(block_size) {
            apply_2q_block(block, q0, q1, gate);
        }
    }

    debug_assert_normalized(state);
}

pub(super) fn apply_2q_soa(
    state: &mut StateVectorSoA,
    num_qubits: usize,
    q0: usize,
    q1: usize,
    gate: &Gate,
    parallel: bool,
) {
    debug_assert!(q0 < num_qubits, "two-qubit target out of range");
    debug_assert!(q1 < num_qubits, "two-qubit target out of range");
    debug_assert_ne!(q0, q1, "two-qubit gate requires distinct qubits");

    let block_size = 1usize << (q0.max(q1) + 1);
    if parallel {
        state
            .real
            .par_chunks_exact_mut(block_size)
            .zip(state.imag.par_chunks_exact_mut(block_size))
            .for_each(|(real_block, imag_block)| {
                apply_2q_block_soa(real_block, imag_block, q0, q1, gate);
            });
    } else {
        for (real_block, imag_block) in state
            .real
            .chunks_exact_mut(block_size)
            .zip(state.imag.chunks_exact_mut(block_size))
        {
            apply_2q_block_soa(real_block, imag_block, q0, q1, gate);
        }
    }

    debug_assert_normalized_soa(state);
}

fn apply_1q_block(block: &mut [Complex], mask: usize, gate: &Gate) {
    for base in 0..mask {
        let pair_index = base | mask;
        let amplitude_zero = block[base];
        let amplitude_one = block[pair_index];
        block[base] = gate.get(0, 0) * amplitude_zero + gate.get(0, 1) * amplitude_one;
        block[pair_index] = gate.get(1, 0) * amplitude_zero + gate.get(1, 1) * amplitude_one;
    }
}

fn apply_1q_block_soa(real_block: &mut [f64], imag_block: &mut [f64], mask: usize, gate: &Gate) {
    let m00 = gate.get(0, 0);
    let m01 = gate.get(0, 1);
    let m10 = gate.get(1, 0);
    let m11 = gate.get(1, 1);

    for base in 0..mask {
        let pair_index = base | mask;
        let amplitude_zero = (real_block[base], imag_block[base]);
        let amplitude_one = (real_block[pair_index], imag_block[pair_index]);
        let row_zero = add_complex_pairs(
            mul_complex_pair(m00, amplitude_zero),
            mul_complex_pair(m01, amplitude_one),
        );
        let row_one = add_complex_pairs(
            mul_complex_pair(m10, amplitude_zero),
            mul_complex_pair(m11, amplitude_one),
        );
        real_block[base] = row_zero.0;
        imag_block[base] = row_zero.1;
        real_block[pair_index] = row_one.0;
        imag_block[pair_index] = row_one.1;
    }
}

fn apply_2q_block(block: &mut [Complex], q0: usize, q1: usize, gate: &Gate) {
    let mask0 = 1usize << q0;
    let mask1 = 1usize << q1;
    for base in 0..block.len() {
        if base & mask0 != 0 || base & mask1 != 0 {
            continue;
        }

        let indices = [base, base | mask1, base | mask0, base | mask0 | mask1];
        let amplitudes = [
            block[indices[0]],
            block[indices[1]],
            block[indices[2]],
            block[indices[3]],
        ];

        for row in 0..4 {
            block[indices[row]] = gate.get(row, 0) * amplitudes[0]
                + gate.get(row, 1) * amplitudes[1]
                + gate.get(row, 2) * amplitudes[2]
                + gate.get(row, 3) * amplitudes[3];
        }
    }
}

fn apply_2q_block_soa(
    real_block: &mut [f64],
    imag_block: &mut [f64],
    q0: usize,
    q1: usize,
    gate: &Gate,
) {
    let mask0 = 1usize << q0;
    let mask1 = 1usize << q1;
    for base in 0..real_block.len() {
        if base & mask0 != 0 || base & mask1 != 0 {
            continue;
        }

        let indices = [base, base | mask1, base | mask0, base | mask0 | mask1];
        let amplitudes = [
            (real_block[indices[0]], imag_block[indices[0]]),
            (real_block[indices[1]], imag_block[indices[1]]),
            (real_block[indices[2]], imag_block[indices[2]]),
            (real_block[indices[3]], imag_block[indices[3]]),
        ];

        for row in 0..4 {
            let result = add_complex_pairs(
                add_complex_pairs(
                    mul_complex_pair(gate.get(row, 0), amplitudes[0]),
                    mul_complex_pair(gate.get(row, 1), amplitudes[1]),
                ),
                add_complex_pairs(
                    mul_complex_pair(gate.get(row, 2), amplitudes[2]),
                    mul_complex_pair(gate.get(row, 3), amplitudes[3]),
                ),
            );
            real_block[indices[row]] = result.0;
            imag_block[indices[row]] = result.1;
        }
    }
}

pub(super) fn measure_qubit(
    state: &mut [Complex],
    qubit: usize,
    rng: &mut impl Rng,
) -> WarosResult<u8> {
    let probability_zero = probability_zero(state, qubit).clamp(0.0, 1.0);
    let outcome = u8::from(rng.gen::<f64>() >= probability_zero);

    let kept_probability = if outcome == 0 {
        probability_zero
    } else {
        1.0 - probability_zero
    };
    if kept_probability <= MEASUREMENT_EPSILON {
        return Err(WarosError::NumericalInstability(
            "collapsing a near-zero measurement branch",
        ));
    }

    let norm = kept_probability.sqrt();
    let mask = 1usize << qubit;
    for (index, amplitude) in state.iter_mut().enumerate() {
        let bit_is_one = (index & mask) != 0;
        if bit_is_one == (outcome == 1) {
            *amplitude = *amplitude * (1.0 / norm);
        } else {
            *amplitude = Complex::ZERO;
        }
    }

    debug_assert_normalized(state);
    Ok(outcome)
}

pub(super) fn probability_one(state: &[Complex], qubit: usize) -> f64 {
    let mask = 1usize << qubit;
    state
        .iter()
        .enumerate()
        .filter_map(|(index, amplitude)| ((index & mask) != 0).then_some(amplitude.norm_sq()))
        .sum()
}

fn probability_zero(state: &[Complex], qubit: usize) -> f64 {
    1.0 - probability_one(state, qubit)
}

pub(super) fn has_mid_circuit_measurement(circuit: &Circuit) -> bool {
    let mut saw_measurement = false;
    for instruction in circuit.instructions() {
        match instruction {
            Instruction::Measure { .. } => saw_measurement = true,
            Instruction::ConditionalGate { .. } => return true,
            Instruction::GateOp { .. } if saw_measurement => return true,
            Instruction::GateOp { .. } | Instruction::Barrier { .. } => {}
        }
    }
    false
}

pub(super) fn sample(probabilities: &[f64], rng: &mut impl Rng) -> usize {
    let draw: f64 = rng.gen();
    let mut cumulative = 0.0;
    for (index, probability) in probabilities.iter().enumerate() {
        cumulative += probability;
        if draw < cumulative {
            return index;
        }
    }
    probabilities.len() - 1
}

pub(super) fn basis_state_string(index: usize, num_qubits: usize) -> String {
    (0..num_qubits)
        .map(|qubit| if (index >> qubit) & 1 == 1 { '1' } else { '0' })
        .collect()
}

fn mul_complex_pair(lhs: Complex, rhs: (f64, f64)) -> (f64, f64) {
    (
        lhs.re * rhs.0 - lhs.im * rhs.1,
        lhs.re * rhs.1 + lhs.im * rhs.0,
    )
}

fn add_complex_pairs(lhs: (f64, f64), rhs: (f64, f64)) -> (f64, f64) {
    (lhs.0 + rhs.0, lhs.1 + rhs.1)
}

fn debug_assert_normalized(state: &[Complex]) {
    let norm: f64 = state.iter().map(|amplitude| amplitude.norm_sq()).sum();
    debug_assert!(
        (norm - 1.0).abs() <= NORMALIZATION_EPSILON,
        "state normalization drifted to {norm:.16}"
    );
}

fn debug_assert_normalized_soa(state: &StateVectorSoA) {
    let norm: f64 = state
        .real
        .iter()
        .zip(state.imag.iter())
        .map(|(real, imag)| real * real + imag * imag)
        .sum();
    debug_assert!(
        (norm - 1.0).abs() <= NORMALIZATION_EPSILON,
        "state normalization drifted to {norm:.16}"
    );
}
