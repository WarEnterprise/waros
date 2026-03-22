use crate::error::{WarosError, WarosResult};
use crate::gate::{self, Gate};

mod extensions;

/// Maximum number of qubits supported by the circuit model.
///
/// The exact backend may impose stricter runtime limits, but circuit
/// construction supports wider systems so tensor-network backends can
/// simulate low-entanglement workloads beyond the state-vector range.
pub const MAX_QUBITS: usize = 128;

/// A single quantum instruction inside a circuit.
#[derive(Debug, Clone)]
pub enum Instruction {
    /// Apply a quantum gate to the provided target qubits.
    GateOp { gate: Gate, targets: Vec<usize> },
    /// Apply a quantum gate when a classical register equals a specific value.
    ConditionalGate {
        classical_bits: Vec<usize>,
        value: usize,
        gate: Gate,
        targets: Vec<usize>,
    },
    /// Measure a qubit into a classical bit.
    Measure { qubit: usize, classical_bit: usize },
    /// Insert a logical barrier across the provided qubits.
    Barrier { qubits: Vec<usize> },
}

/// A quantum circuit: a sequence of gates, barriers, and measurements.
///
/// ```rust
/// use waros_quantum::{Circuit, WarosError};
///
/// # fn main() -> Result<(), WarosError> {
/// let mut c = Circuit::new(2)?;
/// c.h(0)?;
/// c.cnot(0, 1)?;
/// c.measure_all()?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct Circuit {
    num_qubits: usize,
    num_classical_bits: usize,
    instructions: Vec<Instruction>,
}

impl Circuit {
    /// Create a circuit with the requested number of qubits.
    ///
    /// # Errors
    ///
    /// Returns an error if `num_qubits` is zero or exceeds [`MAX_QUBITS`].
    pub fn new(num_qubits: usize) -> WarosResult<Self> {
        if num_qubits == 0 {
            return Err(WarosError::ZeroQubits);
        }
        if num_qubits > MAX_QUBITS {
            return Err(WarosError::TooManyQubits(num_qubits, MAX_QUBITS));
        }

        Ok(Self {
            num_qubits,
            num_classical_bits: 0,
            instructions: Vec::new(),
        })
    }

    /// Create a circuit with pre-allocated classical bits.
    ///
    /// # Errors
    ///
    /// Returns an error if `num_qubits` is zero or exceeds [`MAX_QUBITS`].
    pub fn with_classical_bits(num_qubits: usize, num_classical_bits: usize) -> WarosResult<Self> {
        let mut circuit = Self::new(num_qubits)?;
        circuit.num_classical_bits = num_classical_bits;
        Ok(circuit)
    }

    /// Return the number of qubits in the circuit.
    #[must_use]
    pub fn num_qubits(&self) -> usize {
        self.num_qubits
    }

    /// Return the number of classical bits allocated by measurements.
    #[must_use]
    pub fn num_classical_bits(&self) -> usize {
        self.num_classical_bits
    }

    /// Return the instruction list in insertion order.
    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    /// Return the number of quantum gates in the circuit.
    #[must_use]
    pub fn gate_count(&self) -> usize {
        self.instructions
            .iter()
            .filter(|instruction| {
                matches!(
                    instruction,
                    Instruction::GateOp { .. } | Instruction::ConditionalGate { .. }
                )
            })
            .count()
    }

    fn validate_qubit(&self, qubit: usize) -> WarosResult<()> {
        if qubit >= self.num_qubits {
            return Err(WarosError::QubitOutOfRange(qubit, self.num_qubits));
        }
        Ok(())
    }

    fn validate_pair(&self, q0: usize, q1: usize) -> WarosResult<()> {
        self.validate_qubit(q0)?;
        self.validate_qubit(q1)?;
        if q0 == q1 {
            return Err(WarosError::SameQubit(q0, q1));
        }
        Ok(())
    }

    fn validate_triple(&self, q0: usize, q1: usize, q2: usize) -> WarosResult<()> {
        self.validate_qubit(q0)?;
        self.validate_qubit(q1)?;
        self.validate_qubit(q2)?;
        if q0 == q1 || q0 == q2 {
            return Err(WarosError::SameQubit(q0, q1.max(q2)));
        }
        if q1 == q2 {
            return Err(WarosError::SameQubit(q1, q2));
        }
        Ok(())
    }

    fn validate_distinct_qubits(&self, qubits: &[usize]) -> WarosResult<()> {
        for (index, qubit) in qubits.iter().copied().enumerate() {
            self.validate_qubit(qubit)?;
            for other in qubits.iter().skip(index + 1).copied() {
                if qubit == other {
                    return Err(WarosError::SameQubit(qubit, other));
                }
            }
        }
        Ok(())
    }

    fn ensure_classical_bits(&mut self, required: usize) {
        self.num_classical_bits = self.num_classical_bits.max(required);
    }

    fn validate_classical_bits(&self, classical_bits: &[usize]) -> WarosResult<()> {
        for &classical_bit in classical_bits {
            if classical_bit >= self.num_classical_bits {
                return Err(WarosError::SimulationError(format!(
                    "classical bit {classical_bit} out of range (circuit has {} classical bits)",
                    self.num_classical_bits
                )));
            }
        }
        Ok(())
    }

    fn add1(&mut self, gate: Gate, qubit: usize) -> WarosResult<&mut Self> {
        self.validate_qubit(qubit)?;
        self.instructions.push(Instruction::GateOp {
            gate,
            targets: vec![qubit],
        });
        Ok(self)
    }

    fn add2(&mut self, gate: Gate, q0: usize, q1: usize) -> WarosResult<&mut Self> {
        self.validate_pair(q0, q1)?;
        self.instructions.push(Instruction::GateOp {
            gate,
            targets: vec![q0, q1],
        });
        Ok(self)
    }

    /// Apply a custom 1- or 2-qubit gate.
    ///
    /// This expert-oriented entry point is used by higher-level algorithms that
    /// need to synthesize controlled or derived unitaries from the validated gate
    /// library.
    ///
    /// # Errors
    ///
    /// Returns an error if the gate width does not match `targets`, if the gate
    /// width is unsupported by the simulator, or if any target is invalid.
    pub fn custom_gate(&mut self, gate: Gate, targets: &[usize]) -> WarosResult<&mut Self> {
        if gate.num_qubits != targets.len() {
            return Err(WarosError::SimulationError(format!(
                "gate '{}' expects {} targets, got {}",
                gate.name,
                gate.num_qubits,
                targets.len()
            )));
        }
        if !(1..=2).contains(&gate.num_qubits) {
            return Err(WarosError::SimulationError(format!(
                "gate '{}' uses unsupported width {}",
                gate.name, gate.num_qubits
            )));
        }

        self.validate_distinct_qubits(targets)?;
        self.instructions.push(Instruction::GateOp {
            gate,
            targets: targets.to_vec(),
        });
        Ok(self)
    }

    /// Apply a custom gate conditioned on a classical register value.
    ///
    /// # Errors
    ///
    /// Returns an error if the gate or target list is invalid, or if any
    /// classical bit lies outside the allocated classical register.
    pub fn conditional_gate(
        &mut self,
        classical_bits: &[usize],
        value: usize,
        gate: Gate,
        targets: &[usize],
    ) -> WarosResult<&mut Self> {
        if classical_bits.is_empty() {
            return Err(WarosError::SimulationError(
                "conditional gates require at least one classical bit".into(),
            ));
        }
        self.validate_classical_bits(classical_bits)?;
        if gate.num_qubits != targets.len() {
            return Err(WarosError::SimulationError(format!(
                "gate '{}' expects {} targets, got {}",
                gate.name,
                gate.num_qubits,
                targets.len()
            )));
        }
        self.validate_distinct_qubits(targets)?;
        self.instructions.push(Instruction::ConditionalGate {
            classical_bits: classical_bits.to_vec(),
            value,
            gate,
            targets: targets.to_vec(),
        });
        Ok(self)
    }

    /// Apply a Hadamard gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn h(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::h(), qubit)
    }

    /// Apply a Pauli-X gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn x(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::x(), qubit)
    }

    /// Apply a Pauli-Y gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn y(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::y(), qubit)
    }

    /// Apply a Pauli-Z gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn z(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::z(), qubit)
    }

    /// Apply an S gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn s(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::s(), qubit)
    }

    /// Apply an S-dagger gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn sdg(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::sdg(), qubit)
    }

    /// Apply a T gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn t(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::t(), qubit)
    }

    /// Apply a T-dagger gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn tdg(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::tdg(), qubit)
    }

    /// Apply an Rx rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn rx(&mut self, qubit: usize, theta: f64) -> WarosResult<&mut Self> {
        self.add1(gate::rx(theta), qubit)
    }

    /// Apply an Ry rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn ry(&mut self, qubit: usize, theta: f64) -> WarosResult<&mut Self> {
        self.add1(gate::ry(theta), qubit)
    }

    /// Apply an Rz rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn rz(&mut self, qubit: usize, theta: f64) -> WarosResult<&mut Self> {
        self.add1(gate::rz(theta), qubit)
    }

    /// Apply the square-root of X gate.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn sx(&mut self, qubit: usize) -> WarosResult<&mut Self> {
        self.add1(gate::sx(), qubit)
    }

    /// Apply a generic U3 rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn u3(
        &mut self,
        qubit: usize,
        theta: f64,
        phi: f64,
        lambda: f64,
    ) -> WarosResult<&mut Self> {
        self.add1(gate::u3(theta, phi, lambda), qubit)
    }

    /// Apply a controlled-NOT gate.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn cnot(&mut self, control: usize, target: usize) -> WarosResult<&mut Self> {
        self.add2(gate::cnot(), control, target)
    }

    /// Alias for [`Circuit::cnot`].
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn cx(&mut self, control: usize, target: usize) -> WarosResult<&mut Self> {
        self.cnot(control, target)
    }

    /// Apply a controlled-Z gate.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn cz(&mut self, q0: usize, q1: usize) -> WarosResult<&mut Self> {
        self.add2(gate::cz(), q0, q1)
    }

    /// Apply a controlled-Y gate.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn cy(&mut self, q0: usize, q1: usize) -> WarosResult<&mut Self> {
        self.add2(gate::cy(), q0, q1)
    }

    /// Apply a SWAP gate.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn swap(&mut self, q0: usize, q1: usize) -> WarosResult<&mut Self> {
        self.add2(gate::swap(), q0, q1)
    }

    /// Apply a two-qubit ZZ rotation.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn rzz(&mut self, q0: usize, q1: usize, theta: f64) -> WarosResult<&mut Self> {
        self.add2(gate::rzz(theta), q0, q1)
    }

    /// Apply a Toffoli (CCX) gate using a standard decomposition.
    ///
    /// # Errors
    ///
    /// Returns an error if any qubit is out of range or if any two qubits are equal.
    pub fn toffoli(
        &mut self,
        control_0: usize,
        control_1: usize,
        target: usize,
    ) -> WarosResult<&mut Self> {
        self.validate_triple(control_0, control_1, target)?;

        self.h(target)?;
        self.cnot(control_1, target)?;
        self.tdg(target)?;
        self.cnot(control_0, target)?;
        self.t(target)?;
        self.cnot(control_1, target)?;
        self.tdg(target)?;
        self.cnot(control_0, target)?;
        self.t(control_1)?;
        self.t(target)?;
        self.h(target)?;
        self.cnot(control_0, control_1)?;
        self.t(control_0)?;
        self.tdg(control_1)?;
        self.cnot(control_0, control_1)?;
        Ok(self)
    }

    /// Measure a qubit and return the allocated classical bit index.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn measure(&mut self, qubit: usize) -> WarosResult<usize> {
        self.validate_qubit(qubit)?;
        let classical_bit = self.num_classical_bits;
        self.num_classical_bits += 1;
        self.instructions.push(Instruction::Measure {
            qubit,
            classical_bit,
        });
        Ok(classical_bit)
    }

    /// Measure a qubit into an explicit classical bit index.
    ///
    /// # Errors
    ///
    /// Returns an error if `qubit` is out of range.
    pub fn measure_into(&mut self, qubit: usize, classical_bit: usize) -> WarosResult<&mut Self> {
        self.validate_qubit(qubit)?;
        self.ensure_classical_bits(classical_bit + 1);
        self.instructions.push(Instruction::Measure {
            qubit,
            classical_bit,
        });
        Ok(self)
    }

    /// Measure every qubit into a new classical register.
    ///
    /// # Errors
    ///
    /// Returns an error if any qubit is invalid.
    pub fn measure_all(&mut self) -> WarosResult<&mut Self> {
        for qubit in 0..self.num_qubits {
            self.measure(qubit)?;
        }
        Ok(self)
    }

    /// Insert a logical barrier across the provided qubits.
    ///
    /// # Errors
    ///
    /// Returns an error if any qubit is invalid or repeated.
    pub fn barrier(&mut self, qubits: &[usize]) -> WarosResult<&mut Self> {
        self.validate_distinct_qubits(qubits)?;
        self.instructions.push(Instruction::Barrier {
            qubits: qubits.to_vec(),
        });
        Ok(self)
    }

    /// Insert a barrier spanning all qubits.
    pub fn barrier_all(&mut self) -> &mut Self {
        let qubits: Vec<usize> = (0..self.num_qubits).collect();
        self.instructions.push(Instruction::Barrier { qubits });
        self
    }
}

impl std::fmt::Display for Circuit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let measurement_count = self
            .instructions
            .iter()
            .filter(|instruction| matches!(instruction, Instruction::Measure { .. }))
            .count();
        write!(
            f,
            "Circuit({} qubits, {} gates, {} measurements)",
            self.num_qubits,
            self.gate_count(),
            measurement_count
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_tracks_gate_and_classical_bit_counts() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.cnot(0, 1).expect("valid gate");
        circuit.measure_all().expect("valid measurements");

        assert_eq!(circuit.gate_count(), 2);
        assert_eq!(circuit.num_classical_bits(), 2);
    }

    #[test]
    fn circuit_rejects_out_of_range_qubits() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        let error = circuit.h(5).expect_err("out-of-range gate must fail");
        assert_eq!(error, WarosError::QubitOutOfRange(5, 2));
    }

    #[test]
    fn circuit_rejects_same_qubit_two_qubit_gates() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        let error = circuit.cnot(0, 0).expect_err("same-qubit gate must fail");
        assert_eq!(error, WarosError::SameQubit(0, 0));
    }
}
