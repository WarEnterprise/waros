#![allow(clippy::trivially_copy_pass_by_ref, clippy::unused_self)]

use crate::error::{WarosError, WarosResult};
use crate::qec::QECCode;
use crate::Circuit;

/// Odd-distance repetition code for correcting bit-flip errors.
#[derive(Debug, Clone, Copy)]
pub struct RepetitionCode {
    distance: usize,
}

impl RepetitionCode {
    /// Create a new odd-distance repetition code.
    ///
    /// # Errors
    ///
    /// Returns an error if `distance` is even or smaller than `3`.
    pub fn new(distance: usize) -> WarosResult<Self> {
        if distance < 3 || distance.is_multiple_of(2) {
            return Err(WarosError::SimulationError(
                "repetition code distance must be an odd integer >= 3".into(),
            ));
        }
        Ok(Self { distance })
    }

    fn ancilla_start(&self) -> usize {
        self.distance
    }

    fn ensure_layout(&self, circuit: &Circuit, required: usize) -> WarosResult<()> {
        if circuit.num_qubits() < required {
            return Err(WarosError::QubitOutOfRange(
                required - 1,
                circuit.num_qubits(),
            ));
        }
        Ok(())
    }
}

impl Default for RepetitionCode {
    fn default() -> Self {
        Self { distance: 3 }
    }
}

impl QECCode for RepetitionCode {
    fn physical_qubits(&self) -> usize {
        self.distance
    }

    fn logical_qubits(&self) -> usize {
        1
    }

    fn distance(&self) -> usize {
        self.distance
    }

    fn encode(&self, circuit: &mut Circuit, logical_qubit: usize) -> WarosResult<()> {
        self.ensure_layout(circuit, logical_qubit + self.distance)?;
        for offset in 1..self.distance {
            circuit.cnot(logical_qubit, logical_qubit + offset)?;
        }
        Ok(())
    }

    fn measure_syndrome(&self, circuit: &mut Circuit) -> WarosResult<Vec<usize>> {
        let ancilla_start = self.ancilla_start();
        self.ensure_layout(circuit, ancilla_start + self.distance - 1)?;

        let mut classical_bits = Vec::with_capacity(self.distance - 1);
        for offset in 0..(self.distance - 1) {
            let ancilla = ancilla_start + offset;
            circuit.cnot(offset, ancilla)?;
            circuit.cnot(offset + 1, ancilla)?;
            classical_bits.push(circuit.measure(ancilla)?);
        }
        Ok(classical_bits)
    }

    fn correct(&self, circuit: &mut Circuit, syndrome: &[usize]) -> WarosResult<()> {
        if self.distance != 3 {
            return Err(WarosError::SimulationError(
                "repetition correction is implemented for the 3-qubit code".into(),
            ));
        }
        if syndrome.len() != 2 {
            return Err(WarosError::SimulationError(
                "3-qubit repetition code expects a 2-bit syndrome".into(),
            ));
        }

        match syndrome {
            [1, 0] => {
                circuit.x(0)?;
            }
            [1, 1] => {
                circuit.x(1)?;
            }
            [0, 1] => {
                circuit.x(2)?;
            }
            [0, 0] => {}
            _ => {
                return Err(WarosError::SimulationError(
                    "syndrome bits must be 0 or 1".into(),
                ));
            }
        }

        Ok(())
    }
}
