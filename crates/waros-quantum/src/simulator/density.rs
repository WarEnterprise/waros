use crate::complex::Complex;
use crate::error::{WarosError, WarosResult};
use crate::Circuit;

use super::Simulator;

/// Exact density-matrix representation for small circuits.
#[derive(Debug, Clone)]
pub struct DensityMatrixSimulator {
    matrix: Vec<Vec<Complex>>,
    num_qubits: usize,
}

impl DensityMatrixSimulator {
    /// Build a density matrix from the final state of a circuit.
    ///
    /// # Errors
    ///
    /// Returns an error if the circuit is too wide for the exact matrix backend
    /// or if the simulator cannot provide a final state vector.
    pub fn from_circuit(circuit: &Circuit, simulator: &Simulator) -> WarosResult<Self> {
        if circuit.num_qubits() > 12 {
            return Err(WarosError::TooManyQubits(circuit.num_qubits(), 12));
        }
        let state = simulator.statevector(circuit)?;
        Self::from_statevector(&state, circuit.num_qubits())
    }

    /// Build a density matrix from a state vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the state width does not match `num_qubits`.
    pub fn from_statevector(state: &[Complex], num_qubits: usize) -> WarosResult<Self> {
        let dimension = 1usize << num_qubits;
        if state.len() != dimension {
            return Err(WarosError::SimulationError(
                "statevector width does not match qubit count".into(),
            ));
        }

        let mut matrix = vec![vec![Complex::ZERO; dimension]; dimension];
        for row in 0..dimension {
            for column in 0..dimension {
                matrix[row][column] = state[row] * state[column].conj();
            }
        }

        Ok(Self { matrix, num_qubits })
    }

    /// Return the density matrix entries.
    #[must_use]
    pub fn matrix(&self) -> &[Vec<Complex>] {
        &self.matrix
    }

    /// Return the represented qubit count.
    #[must_use]
    pub fn num_qubits(&self) -> usize {
        self.num_qubits
    }

    /// Return the diagonal probability of one basis state.
    ///
    /// # Errors
    ///
    /// Returns an error if `basis_state` is out of range.
    pub fn probability(&self, basis_state: usize) -> WarosResult<f64> {
        let dimension = 1usize << self.num_qubits;
        if basis_state >= dimension {
            return Err(WarosError::QubitOutOfRange(basis_state, dimension));
        }
        Ok(self.matrix[basis_state][basis_state].re)
    }
}

/// Reconstruct an ideal density matrix for a small circuit.
///
/// The current implementation uses the simulator's exact statevector and
/// returns the corresponding density operator. This keeps the API in place for
/// future measurement-driven tomography passes while remaining exact today.
///
/// # Errors
///
/// Returns an error if the simulator cannot provide an exact statevector.
pub fn quantum_state_tomography(
    circuit: &Circuit,
    simulator: &Simulator,
    _shots_per_basis: u32,
) -> WarosResult<DensityMatrixSimulator> {
    DensityMatrixSimulator::from_circuit(circuit, simulator)
}

#[cfg(test)]
mod tests {
    use super::{quantum_state_tomography, DensityMatrixSimulator};
    use crate::{Circuit, Simulator};

    #[test]
    fn density_matrix_tracks_bell_state_populations() {
        let mut circuit = Circuit::new(2).expect("valid bell circuit");
        circuit.h(0).expect("valid hadamard");
        circuit.cnot(0, 1).expect("valid cnot");

        let simulator = Simulator::with_seed(42);
        let density =
            DensityMatrixSimulator::from_circuit(&circuit, &simulator).expect("density matrix");

        assert!((density.probability(0).expect("basis 00") - 0.5).abs() < 1e-9);
        assert!((density.probability(3).expect("basis 11") - 0.5).abs() < 1e-9);
        assert!(density.probability(1).expect("basis 01") < 1e-9);
        assert!(density.probability(2).expect("basis 10") < 1e-9);
        assert!((density.matrix()[0][3].re - 0.5).abs() < 1e-9);
        assert!((density.matrix()[3][0].re - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tomography_matches_exact_density_matrix_for_small_circuit() {
        let mut circuit = Circuit::new(1).expect("valid single-qubit circuit");
        circuit.h(0).expect("valid hadamard");

        let simulator = Simulator::with_seed(7);
        let expected =
            DensityMatrixSimulator::from_circuit(&circuit, &simulator).expect("density matrix");
        let tomography = quantum_state_tomography(&circuit, &simulator, 1_000).expect("tomography");

        assert_eq!(expected.num_qubits(), tomography.num_qubits());
        for (expected_row, tomography_row) in expected.matrix().iter().zip(tomography.matrix()) {
            for (expected_entry, tomography_entry) in expected_row.iter().zip(tomography_row) {
                assert!((expected_entry.re - tomography_entry.re).abs() < 1e-9);
                assert!((expected_entry.im - tomography_entry.im).abs() < 1e-9);
            }
        }
    }
}
