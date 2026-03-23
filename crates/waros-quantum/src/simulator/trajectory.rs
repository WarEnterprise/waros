use std::collections::HashMap;

use rand::Rng;

use crate::circuit::{Circuit, Instruction};
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::gate;
use crate::noise::{NoiseChannel, NoiseModel};
use crate::result::QuantumResult;

use super::statevector;

const TRAJECTORY_EPSILON: f64 = 1e-15;

pub(super) fn run_shot_mode(
    circuit: &Circuit,
    shots: u32,
    rng: &mut rand::rngs::StdRng,
    parallel: bool,
    noise_model: Option<&NoiseModel>,
) -> WarosResult<QuantumResult> {
    let num_qubits = circuit.num_qubits();
    let mut counts: HashMap<String, u32> = HashMap::new();

    for _ in 0..shots {
        let bitstring = run_single_shot(circuit, num_qubits, rng, parallel, noise_model)?;
        *counts.entry(bitstring).or_insert(0) += 1;
    }

    Ok(QuantumResult::new(
        circuit.num_classical_bits(),
        shots,
        counts,
    ))
}

fn run_single_shot(
    circuit: &Circuit,
    num_qubits: usize,
    rng: &mut impl Rng,
    parallel: bool,
    noise_model: Option<&NoiseModel>,
) -> WarosResult<String> {
    let mut state = statevector::zero_state(num_qubits)?;
    let mut classical_bits = vec![0u8; circuit.num_classical_bits()];

    for instruction in circuit.instructions() {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                statevector::apply_gate(&mut state, num_qubits, targets, gate, parallel);
                if let Some(model) = noise_model {
                    let channels = if gate.num_qubits == 1 {
                        &model.single_qubit_noise
                    } else {
                        &model.two_qubit_noise
                    };
                    for &qubit in targets {
                        apply_channels_to_qubit(
                            &mut state, num_qubits, qubit, channels, rng, parallel,
                        )?;
                    }
                }
            }
            Instruction::ConditionalGate {
                classical_bits: register_bits,
                value,
                gate,
                targets,
            } => {
                if read_classical_register(register_bits, &classical_bits) == *value {
                    statevector::apply_gate(&mut state, num_qubits, targets, gate, parallel);
                    if let Some(model) = noise_model {
                        let channels = if gate.num_qubits == 1 {
                            &model.single_qubit_noise
                        } else {
                            &model.two_qubit_noise
                        };
                        for &qubit in targets {
                            apply_channels_to_qubit(
                                &mut state, num_qubits, qubit, channels, rng, parallel,
                            )?;
                        }
                    }
                }
            }
            Instruction::Measure {
                qubit,
                classical_bit,
            } => {
                let measured = statevector::measure_qubit(&mut state, *qubit, rng)?;
                classical_bits[*classical_bit] =
                    apply_measurement_noise(measured, noise_model, rng);
            }
            Instruction::Barrier { qubits } => {
                if let Some(model) = noise_model {
                    for &qubit in qubits {
                        apply_channels_to_qubit(
                            &mut state,
                            num_qubits,
                            qubit,
                            &model.idle_noise,
                            rng,
                            parallel,
                        )?;
                    }
                }
            }
        }
    }

    Ok(classical_bits
        .iter()
        .map(|bit| char::from(b'0' + *bit))
        .collect())
}

fn read_classical_register(register: &[usize], classical_bits: &[u8]) -> usize {
    register
        .iter()
        .enumerate()
        .fold(0usize, |value, (offset, classical_bit)| {
            value | (usize::from(classical_bits[*classical_bit] == 1) << offset)
        })
}

fn apply_measurement_noise(
    outcome: u8,
    noise_model: Option<&NoiseModel>,
    rng: &mut impl Rng,
) -> u8 {
    let Some(model) = noise_model else {
        return outcome;
    };
    let mut noisy_outcome = outcome;
    for channel in &model.measurement_noise {
        match channel {
            NoiseChannel::ReadoutError { probability } | NoiseChannel::BitFlip { probability } => {
                if rng.gen::<f64>() < probability.clamp(0.0, 1.0) {
                    noisy_outcome ^= 1;
                }
            }
            NoiseChannel::Depolarizing { probability } => {
                if rng.gen::<f64>() < probability.clamp(0.0, 1.0) {
                    noisy_outcome = u8::from(rng.gen::<f64>() >= 0.5);
                }
            }
            NoiseChannel::AmplitudeDamping { .. }
            | NoiseChannel::PhaseDamping { .. }
            | NoiseChannel::PhaseFlip { .. } => {}
        }
    }
    noisy_outcome
}

fn apply_channels_to_qubit(
    state: &mut [Complex],
    num_qubits: usize,
    qubit: usize,
    channels: &[NoiseChannel],
    rng: &mut impl Rng,
    parallel: bool,
) -> WarosResult<()> {
    for channel in channels {
        match channel {
            NoiseChannel::Depolarizing { probability } => {
                let probability = probability.clamp(0.0, 1.0);
                if rng.gen::<f64>() < probability {
                    match rng.gen_range(0..3) {
                        0 => statevector::apply_1q(state, num_qubits, qubit, &gate::x(), parallel),
                        1 => statevector::apply_1q(state, num_qubits, qubit, &gate::y(), parallel),
                        _ => statevector::apply_1q(state, num_qubits, qubit, &gate::z(), parallel),
                    }
                }
            }
            NoiseChannel::AmplitudeDamping { gamma } => {
                apply_amplitude_damping(state, qubit, *gamma, rng)?;
            }
            NoiseChannel::PhaseDamping { lambda } => {
                let probability = (lambda.clamp(0.0, 1.0) * 0.5).clamp(0.0, 0.5);
                if rng.gen::<f64>() < probability {
                    statevector::apply_1q(state, num_qubits, qubit, &gate::z(), parallel);
                }
            }
            NoiseChannel::BitFlip { probability } => {
                if rng.gen::<f64>() < probability.clamp(0.0, 1.0) {
                    statevector::apply_1q(state, num_qubits, qubit, &gate::x(), parallel);
                }
            }
            NoiseChannel::PhaseFlip { probability } => {
                if rng.gen::<f64>() < probability.clamp(0.0, 1.0) {
                    statevector::apply_1q(state, num_qubits, qubit, &gate::z(), parallel);
                }
            }
            NoiseChannel::ReadoutError { .. } => {}
        }
    }
    Ok(())
}

fn apply_amplitude_damping(
    state: &mut [Complex],
    qubit: usize,
    gamma: f64,
    rng: &mut impl Rng,
) -> WarosResult<()> {
    let gamma = gamma.clamp(0.0, 1.0);
    if gamma <= TRAJECTORY_EPSILON {
        return Ok(());
    }

    let probability_one = statevector::probability_one(state, qubit).clamp(0.0, 1.0);
    let jump_probability = (gamma * probability_one).clamp(0.0, 1.0);
    let mask = 1usize << qubit;

    if rng.gen::<f64>() < jump_probability {
        if probability_one <= TRAJECTORY_EPSILON {
            return Ok(());
        }
        let inv_norm = 1.0 / probability_one.sqrt();
        for base in 0..state.len() {
            if base & mask != 0 {
                continue;
            }
            let excited_amplitude = state[base | mask];
            state[base] = excited_amplitude * inv_norm;
            state[base | mask] = Complex::ZERO;
        }
        return Ok(());
    }

    let no_jump_probability = 1.0 - jump_probability;
    if no_jump_probability <= TRAJECTORY_EPSILON {
        return Err(WarosError::NumericalInstability(
            "amplitude damping normalization collapsed",
        ));
    }

    let inv_norm = 1.0 / no_jump_probability.sqrt();
    let damping = (1.0 - gamma).sqrt();
    for base in 0..state.len() {
        if base & mask != 0 {
            continue;
        }
        state[base] = state[base] * inv_norm;
        state[base | mask] = state[base | mask] * (damping * inv_norm);
    }
    Ok(())
}
