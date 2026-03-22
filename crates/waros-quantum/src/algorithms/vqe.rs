#![allow(clippy::cast_precision_loss, clippy::missing_errors_doc)]

use crate::circuit::Circuit;
use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::result::QuantumResult;
use crate::simulator::Simulator;

/// Single-qubit Pauli operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pauli {
    I,
    X,
    Y,
    Z,
}

/// A single Pauli-string term inside a Hamiltonian.
#[derive(Debug, Clone)]
pub struct PauliTerm {
    pub coefficient: f64,
    pub paulis: Vec<(usize, Pauli)>,
}

/// A Hamiltonian expressed as a weighted sum of Pauli strings.
#[derive(Debug, Clone)]
pub struct Hamiltonian {
    pub terms: Vec<PauliTerm>,
    pub num_qubits: usize,
}

/// Supported ansatz families for the VQE optimizer.
#[derive(Debug, Clone, Copy)]
pub enum AnsatzType {
    RyLinear,
    HardwareEfficient { layers: usize },
    UCCSD,
}

/// Result of a VQE optimization run.
#[derive(Debug, Clone)]
pub struct VQEResult {
    pub energy: f64,
    pub optimal_params: Vec<f64>,
    pub energy_history: Vec<f64>,
    pub iterations: usize,
    pub converged: bool,
}

/// Run a small-scale VQE optimization loop.
pub fn vqe(
    hamiltonian: &Hamiltonian,
    ansatz: AnsatzType,
    initial_params: &[f64],
    max_iterations: usize,
    shots_per_eval: u32,
    simulator: &Simulator,
) -> WarosResult<VQEResult> {
    validate_param_count(ansatz, hamiltonian.num_qubits, initial_params.len())?;

    let mut params = initial_params.to_vec();
    let mut best_params = params.clone();
    let mut energy_history = Vec::new();
    let mut best_energy = f64::INFINITY;
    let learning_rate = 0.25;
    let epsilon = 0.05;
    let mut converged = false;

    for _ in 0..max_iterations {
        let energy =
            evaluate_expectation(hamiltonian, &ansatz, &params, shots_per_eval, simulator)?;
        energy_history.push(energy);
        if energy < best_energy {
            best_energy = energy;
            best_params.clone_from(&params);
        }

        let gradient = numerical_gradient(
            hamiltonian,
            &ansatz,
            &params,
            epsilon,
            shots_per_eval,
            simulator,
        )?;

        let mut improved = false;
        for scale in [
            learning_rate,
            learning_rate * 0.5,
            learning_rate * 0.25,
            learning_rate * 0.125,
        ] {
            let candidate: Vec<f64> = params
                .iter()
                .zip(gradient.iter())
                .map(|(parameter, derivative)| parameter - scale * derivative)
                .collect();
            let candidate_energy =
                evaluate_expectation(hamiltonian, &ansatz, &candidate, shots_per_eval, simulator)?;

            if candidate_energy + 1e-10 < energy {
                params = candidate;
                improved = true;
                break;
            }
        }

        if !improved {
            converged = true;
            break;
        }

        if energy_history.len() >= 5 {
            let recent = &energy_history[energy_history.len() - 5..];
            if variance(recent) < 1e-9 {
                converged = true;
                break;
            }
        }
    }

    let final_energy =
        evaluate_expectation(hamiltonian, &ansatz, &params, shots_per_eval, simulator)?;
    if final_energy < best_energy {
        best_energy = final_energy;
        best_params = params;
    }

    let iterations = energy_history.len();
    Ok(VQEResult {
        energy: best_energy,
        optimal_params: best_params,
        energy_history,
        iterations,
        converged,
    })
}

/// Evaluate `<psi(theta)|H|psi(theta)>`.
pub fn evaluate_expectation(
    hamiltonian: &Hamiltonian,
    ansatz: &AnsatzType,
    params: &[f64],
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<f64> {
    let mut circuit = Circuit::new(hamiltonian.num_qubits)?;
    apply_ansatz(&mut circuit, ansatz, params)?;

    if simulator.noise_model().is_ideal() {
        let state = simulator.statevector(&circuit)?;
        Ok(hamiltonian
            .terms
            .iter()
            .map(|term| term.coefficient * pauli_expectation_from_state(&state, term))
            .sum())
    } else {
        let mut total_energy = 0.0;
        for term in &hamiltonian.terms {
            if term.paulis.is_empty() {
                total_energy += term.coefficient;
                continue;
            }

            let mut measurement_circuit = circuit.clone();
            rotate_into_measurement_basis(&mut measurement_circuit, &term.paulis)?;
            measurement_circuit.measure_all()?;
            let result = simulator.run(&measurement_circuit, shots.max(1))?;
            total_energy += term.coefficient * pauli_expectation_from_counts(&result, &term.paulis);
        }
        Ok(total_energy)
    }
}

impl Hamiltonian {
    /// Two-qubit hydrogen Hamiltonian in the STO-3G basis.
    #[must_use]
    pub fn hydrogen_molecule() -> Self {
        Self {
            num_qubits: 2,
            terms: vec![
                PauliTerm {
                    coefficient: -1.0524,
                    paulis: vec![],
                },
                PauliTerm {
                    coefficient: 0.3979,
                    paulis: vec![(1, Pauli::Z)],
                },
                PauliTerm {
                    coefficient: -0.3979,
                    paulis: vec![(0, Pauli::Z)],
                },
                PauliTerm {
                    coefficient: -0.0112,
                    paulis: vec![(0, Pauli::Z), (1, Pauli::Z)],
                },
                PauliTerm {
                    coefficient: 0.1809,
                    paulis: vec![(0, Pauli::X), (1, Pauli::X)],
                },
            ],
        }
    }

    /// Transverse-field Ising chain with open boundary conditions.
    #[must_use]
    pub fn ising_chain(num_qubits: usize, coupling: f64, field: f64) -> Self {
        let mut terms = Vec::new();
        for qubit in 0..num_qubits.saturating_sub(1) {
            terms.push(PauliTerm {
                coefficient: -coupling,
                paulis: vec![(qubit, Pauli::Z), (qubit + 1, Pauli::Z)],
            });
        }
        for qubit in 0..num_qubits {
            terms.push(PauliTerm {
                coefficient: -field,
                paulis: vec![(qubit, Pauli::X)],
            });
        }

        Self { terms, num_qubits }
    }
}

fn apply_ansatz(circuit: &mut Circuit, ansatz: &AnsatzType, params: &[f64]) -> WarosResult<()> {
    match *ansatz {
        AnsatzType::RyLinear => {
            for (qubit, theta) in params.iter().copied().enumerate() {
                circuit.ry(qubit, theta)?;
            }
            for qubit in 0..params.len().saturating_sub(1) {
                circuit.cnot(qubit, qubit + 1)?;
            }
        }
        AnsatzType::HardwareEfficient { layers } => {
            let num_qubits = circuit.num_qubits();
            let mut cursor = 0usize;
            for _ in 0..layers {
                for qubit in 0..num_qubits {
                    circuit.ry(qubit, params[cursor])?;
                    circuit.rz(qubit, params[cursor + 1])?;
                    cursor += 2;
                }
                for qubit in 0..num_qubits.saturating_sub(1) {
                    circuit.cnot(qubit, qubit + 1)?;
                }
            }
        }
        AnsatzType::UCCSD => {
            if circuit.num_qubits() != 2 || params.len() != 2 {
                return Err(WarosError::SimulationError(
                    "demo UCCSD ansatz currently expects 2 qubits and 2 parameters".into(),
                ));
            }
            circuit.ry(0, params[0])?;
            circuit.ry(1, params[1])?;
            circuit.cnot(0, 1)?;
            circuit.rz(1, params[0] - params[1])?;
            circuit.cnot(0, 1)?;
        }
    }
    Ok(())
}

fn pauli_expectation_from_state(state: &[Complex], term: &PauliTerm) -> f64 {
    if term.paulis.is_empty() {
        return 1.0;
    }

    let mut expectation = Complex::ZERO;
    for (basis_index, amplitude) in state.iter().copied().enumerate() {
        let mut mapped_index = basis_index;
        let mut factor = Complex::ONE;

        for (qubit, pauli) in &term.paulis {
            let bit_is_one = ((basis_index >> qubit) & 1) == 1;
            match pauli {
                Pauli::I => {}
                Pauli::X => mapped_index ^= 1usize << qubit,
                Pauli::Y => {
                    mapped_index ^= 1usize << qubit;
                    factor = factor
                        * if bit_is_one {
                            Complex::new(0.0, -1.0)
                        } else {
                            Complex::new(0.0, 1.0)
                        };
                }
                Pauli::Z => {
                    if bit_is_one {
                        factor = factor * Complex::new(-1.0, 0.0);
                    }
                }
            }
        }

        expectation += amplitude.conj() * factor * state[mapped_index];
    }

    expectation.re
}

fn pauli_expectation_from_counts(result: &QuantumResult, paulis: &[(usize, Pauli)]) -> f64 {
    let mut expectation = 0.0;
    for (state, count) in result.counts() {
        let eigenvalue = paulis
            .iter()
            .fold(1.0, |value, (qubit, pauli)| match pauli {
                Pauli::I => value,
                Pauli::X | Pauli::Y | Pauli::Z => {
                    let bit = state.as_bytes().get(*qubit).copied().unwrap_or(b'0');
                    value * if bit == b'0' { 1.0 } else { -1.0 }
                }
            });
        expectation += eigenvalue * f64::from(*count) / f64::from(result.total_shots());
    }
    expectation
}

fn rotate_into_measurement_basis(
    circuit: &mut Circuit,
    paulis: &[(usize, Pauli)],
) -> WarosResult<()> {
    for (qubit, pauli) in paulis {
        match pauli {
            Pauli::I | Pauli::Z => {}
            Pauli::X => {
                circuit.h(*qubit)?;
            }
            Pauli::Y => {
                circuit.sdg(*qubit)?;
                circuit.h(*qubit)?;
            }
        }
    }
    Ok(())
}

fn numerical_gradient(
    hamiltonian: &Hamiltonian,
    ansatz: &AnsatzType,
    params: &[f64],
    epsilon: f64,
    shots: u32,
    simulator: &Simulator,
) -> WarosResult<Vec<f64>> {
    let mut gradient = vec![0.0; params.len()];
    for index in 0..params.len() {
        let mut params_plus = params.to_vec();
        let mut params_minus = params.to_vec();
        params_plus[index] += epsilon;
        params_minus[index] -= epsilon;

        let energy_plus =
            evaluate_expectation(hamiltonian, ansatz, &params_plus, shots, simulator)?;
        let energy_minus =
            evaluate_expectation(hamiltonian, ansatz, &params_minus, shots, simulator)?;
        gradient[index] = (energy_plus - energy_minus) / (2.0 * epsilon);
    }
    Ok(gradient)
}

fn validate_param_count(ansatz: AnsatzType, num_qubits: usize, provided: usize) -> WarosResult<()> {
    let expected = match ansatz {
        AnsatzType::RyLinear => num_qubits,
        AnsatzType::HardwareEfficient { layers } => 2 * num_qubits * layers,
        AnsatzType::UCCSD => 2,
    };

    if expected == provided {
        Ok(())
    } else {
        Err(WarosError::SimulationError(format!(
            "ansatz expects {expected} parameters, got {provided}"
        )))
    }
}

fn variance(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values
        .iter()
        .map(|value| {
            let delta = *value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64
}
