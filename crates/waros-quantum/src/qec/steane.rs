#![allow(clippy::trivially_copy_pass_by_ref, clippy::unused_self)]

use crate::error::{WarosError, WarosResult};
use crate::qec::QECCode;
use crate::Circuit;

const DATA_QUBITS: usize = 7;
const ANCILLA_QUBITS: usize = 6;
const X_STABILIZERS: [[usize; 4]; 3] = [[0, 1, 2, 3], [0, 1, 4, 5], [0, 2, 4, 6]];

/// Steane `[[7,1,3]]` code.
#[derive(Debug, Clone, Copy, Default)]
pub struct SteaneCode;

impl SteaneCode {
    fn ensure_layout(&self, circuit: &Circuit, required: usize) -> WarosResult<()> {
        if circuit.num_qubits() < required {
            return Err(WarosError::QubitOutOfRange(
                required - 1,
                circuit.num_qubits(),
            ));
        }
        Ok(())
    }

    fn syndrome_to_qubit(bits: &[usize]) -> Option<usize> {
        match bits {
            [1, 1, 1] => Some(0),
            [1, 1, 0] => Some(1),
            [1, 0, 1] => Some(2),
            [0, 1, 1] => Some(3),
            [1, 0, 0] => Some(4),
            [0, 1, 0] => Some(5),
            [0, 0, 1] => Some(6),
            _ => None,
        }
    }
}

impl QECCode for SteaneCode {
    fn physical_qubits(&self) -> usize {
        DATA_QUBITS
    }

    fn logical_qubits(&self) -> usize {
        1
    }

    fn distance(&self) -> usize {
        3
    }

    fn encode(&self, circuit: &mut Circuit, logical_qubit: usize) -> WarosResult<()> {
        self.ensure_layout(circuit, logical_qubit + DATA_QUBITS)?;
        let q = logical_qubit;

        circuit.h(q + 4)?;
        circuit.h(q + 5)?;
        circuit.h(q + 6)?;

        circuit.cnot(q + 6, q)?;
        circuit.cnot(q + 6, q + 1)?;
        circuit.cnot(q + 6, q + 2)?;

        circuit.cnot(q + 5, q)?;
        circuit.cnot(q + 5, q + 1)?;
        circuit.cnot(q + 5, q + 3)?;

        circuit.cnot(q + 4, q)?;
        circuit.cnot(q + 4, q + 2)?;
        circuit.cnot(q + 4, q + 3)?;

        circuit.cnot(q, q + 1)?;
        circuit.cnot(q, q + 2)?;
        circuit.cnot(q, q + 4)?;
        Ok(())
    }

    fn measure_syndrome(&self, circuit: &mut Circuit) -> WarosResult<Vec<usize>> {
        self.ensure_layout(circuit, DATA_QUBITS + ANCILLA_QUBITS)?;
        let mut classical_bits = Vec::with_capacity(ANCILLA_QUBITS);

        for (offset, stabilizer) in X_STABILIZERS.iter().enumerate() {
            let ancilla = DATA_QUBITS + offset;
            for &data_qubit in stabilizer {
                circuit.cnot(data_qubit, ancilla)?;
            }
            classical_bits.push(circuit.measure(ancilla)?);
        }

        for (offset, stabilizer) in X_STABILIZERS.iter().enumerate() {
            let ancilla = DATA_QUBITS + 3 + offset;
            circuit.h(ancilla)?;
            for &data_qubit in stabilizer {
                circuit.cnot(ancilla, data_qubit)?;
            }
            circuit.h(ancilla)?;
            classical_bits.push(circuit.measure(ancilla)?);
        }

        Ok(classical_bits)
    }

    fn correct(&self, circuit: &mut Circuit, syndrome: &[usize]) -> WarosResult<()> {
        if syndrome.len() != ANCILLA_QUBITS {
            return Err(WarosError::SimulationError(
                "Steane code expects a 6-bit syndrome".into(),
            ));
        }

        if let Some(qubit) = Self::syndrome_to_qubit(&syndrome[..3]) {
            circuit.x(qubit)?;
        }
        if let Some(qubit) = Self::syndrome_to_qubit(&syndrome[3..]) {
            circuit.z(qubit)?;
        }
        if syndrome.iter().any(|bit| *bit > 1) {
            return Err(WarosError::SimulationError(
                "syndrome bits must be 0 or 1".into(),
            ));
        }
        Ok(())
    }
}
