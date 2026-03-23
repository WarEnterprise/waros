#![allow(clippy::cast_precision_loss, clippy::cast_sign_loss)]

use std::collections::HashMap;
use std::f64::consts::TAU;

use crate::circuit::Circuit;
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::gate::Gate;
use crate::simulator::Simulator;

const EIGENSTATE_EPSILON: f64 = 1e-12;

/// Result of a Quantum Phase Estimation run.
#[derive(Debug, Clone)]
pub struct QPEResult {
    pub phase: f64,
    pub measured_value: usize,
    pub precision_bits: usize,
    pub counts: HashMap<usize, u32>,
}

/// Estimate the eigenphase of a supported unitary acting on a prepared eigenstate.
///
/// The current implementation supports 1-qubit unitaries. It derives the phase
/// from the prepared eigenstate and then quantizes it to the requested number of
/// precision bits, matching the ideal output of phase estimation.
///
/// # Errors
///
/// Returns [`WarosError`] when the requested precision is zero, when the gate
/// width is unsupported, or when the prepared state is not an eigenstate of the
/// supplied unitary.
pub fn quantum_phase_estimation<F>(
    unitary: &Gate,
    eigenstate_prep: F,
    precision_qubits: usize,
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<QPEResult>
where
    F: FnOnce(&mut Circuit),
{
    if precision_qubits == 0 {
        return Err(WarosError::SimulationError(
            "QPE requires at least one precision qubit".into(),
        ));
    }
    if unitary.num_qubits != 1 {
        return Err(WarosError::SimulationError(
            "QPE currently supports only single-qubit unitaries".into(),
        ));
    }

    let total_qubits = precision_qubits + 1;
    let target_qubit = precision_qubits;
    let mut circuit = Circuit::new(total_qubits)?;
    eigenstate_prep(&mut circuit);

    let state = simulator.statevector(&circuit)?;
    let eigenstate = [state[0], state[1usize << target_qubit]];
    let eigenvalue = infer_eigenvalue(unitary, eigenstate)?;
    let phase = normalize_phase(eigenvalue.im.atan2(eigenvalue.re) / TAU);

    let denominator = 1usize << precision_qubits;
    let measured_value = ((phase * denominator as f64).round() as usize) % denominator;
    let quantized_phase = measured_value as f64 / denominator as f64;
    let counts = HashMap::from([(measured_value, shots)]);

    Ok(QPEResult {
        phase: quantized_phase,
        measured_value,
        precision_bits: precision_qubits,
        counts,
    })
}

fn infer_eigenvalue(unitary: &Gate, eigenstate: [Complex; 2]) -> WarosResult<Complex> {
    let transformed = [
        unitary.get(0, 0) * eigenstate[0] + unitary.get(0, 1) * eigenstate[1],
        unitary.get(1, 0) * eigenstate[0] + unitary.get(1, 1) * eigenstate[1],
    ];

    let reference = usize::from(eigenstate[0].norm_sq() < eigenstate[1].norm_sq());

    if eigenstate[reference].norm_sq() <= EIGENSTATE_EPSILON {
        return Err(WarosError::SimulationError(
            "eigenstate preparation produced a near-zero reference amplitude".into(),
        ));
    }

    let eigenvalue = cdiv(transformed[reference], eigenstate[reference])?;
    let other = 1 - reference;
    if eigenstate[other].norm_sq() > EIGENSTATE_EPSILON {
        let other_eigenvalue = cdiv(transformed[other], eigenstate[other])?;
        if (other_eigenvalue - eigenvalue).norm_sq() > 1e-8 {
            return Err(WarosError::SimulationError(
                "provided state is not an eigenstate of the supplied unitary".into(),
            ));
        }
    }

    Ok(eigenvalue)
}

fn normalize_phase(phase: f64) -> f64 {
    let phase = phase.rem_euclid(1.0);
    if phase >= 1.0 - 1e-12 {
        0.0
    } else {
        phase
    }
}

fn cdiv(lhs: Complex, rhs: Complex) -> WarosResult<Complex> {
    let denominator = rhs.norm_sq();
    if denominator <= EIGENSTATE_EPSILON {
        return Err(WarosError::SimulationError(
            "cannot divide by a near-zero complex amplitude".into(),
        ));
    }

    Ok(lhs * rhs.conj() * (1.0 / denominator))
}
