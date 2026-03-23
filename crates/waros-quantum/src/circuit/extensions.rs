use super::{Circuit, Instruction};
use crate::error::{WarosError, WarosResult};
use crate::gate;

impl Circuit {
    fn validate_qubit_list(&self, qubits: &[usize]) -> WarosResult<()> {
        self.validate_distinct_qubits(qubits)
    }

    fn append_instruction(
        &mut self,
        instruction: &Instruction,
        qubit_mapping: Option<&[usize]>,
    ) -> WarosResult<()> {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                let mapped_targets: Vec<usize> = targets
                    .iter()
                    .map(|target| qubit_mapping.map_or(*target, |mapping| mapping[*target]))
                    .collect();
                self.instructions.push(Instruction::GateOp {
                    gate: gate.clone(),
                    targets: mapped_targets,
                });
            }
            Instruction::ConditionalGate {
                classical_bits,
                value,
                gate,
                targets,
            } => {
                let mapped_targets: Vec<usize> = targets
                    .iter()
                    .map(|target| qubit_mapping.map_or(*target, |mapping| mapping[*target]))
                    .collect();
                self.ensure_classical_bits(
                    classical_bits
                        .iter()
                        .copied()
                        .max()
                        .map_or(0, |bit| bit + 1),
                );
                self.instructions.push(Instruction::ConditionalGate {
                    classical_bits: classical_bits.clone(),
                    value: *value,
                    gate: gate.clone(),
                    targets: mapped_targets,
                });
            }
            Instruction::Measure { qubit, .. } => {
                self.measure(qubit_mapping.map_or(*qubit, |mapping| mapping[*qubit]))?;
            }
            Instruction::Barrier { qubits } => {
                let mapped_qubits: Vec<usize> = qubits
                    .iter()
                    .map(|qubit| qubit_mapping.map_or(*qubit, |mapping| mapping[*qubit]))
                    .collect();
                self.instructions.push(Instruction::Barrier {
                    qubits: mapped_qubits,
                });
            }
        }
        Ok(())
    }

    /// Apply a controlled phase rotation by `2*pi / 2^k`.
    ///
    /// # Errors
    ///
    /// Returns an error if either qubit is out of range or both indices are equal.
    pub fn crk(&mut self, control: usize, target: usize, k: usize) -> WarosResult<&mut Self> {
        self.add2(gate::crk(k), control, target)
    }

    /// Apply the Quantum Fourier Transform over `qubits`.
    ///
    /// # Errors
    ///
    /// Returns an error if any qubit is invalid or repeated.
    pub fn qft(&mut self, qubits: &[usize]) -> WarosResult<&mut Self> {
        self.validate_qubit_list(qubits)?;
        for (index, &target) in qubits.iter().enumerate() {
            self.h(target)?;
            for (offset, &control) in qubits.iter().enumerate().skip(index + 1) {
                self.crk(control, target, offset - index + 1)?;
            }
        }
        for index in 0..(qubits.len() / 2) {
            self.swap(qubits[index], qubits[qubits.len() - index - 1])?;
        }
        Ok(self)
    }

    /// Apply the inverse Quantum Fourier Transform over `qubits`.
    ///
    /// # Errors
    ///
    /// Returns an error if any qubit is invalid or repeated.
    pub fn iqft(&mut self, qubits: &[usize]) -> WarosResult<&mut Self> {
        self.validate_qubit_list(qubits)?;
        for index in 0..(qubits.len() / 2) {
            self.swap(qubits[index], qubits[qubits.len() - index - 1])?;
        }
        for index in (0..qubits.len()).rev() {
            let target = qubits[index];
            for other in ((index + 1)..qubits.len()).rev() {
                self.add2(gate::crk(other - index + 1).dagger(), qubits[other], target)?;
            }
            self.h(target)?;
        }
        Ok(self)
    }

    /// Append all instructions from another circuit with identical width.
    ///
    /// # Errors
    ///
    /// Returns an error if the two circuits have different qubit counts.
    pub fn append(&mut self, other: &Self) -> WarosResult<&mut Self> {
        if self.num_qubits != other.num_qubits {
            return Err(WarosError::CircuitQubitMismatch(
                self.num_qubits,
                other.num_qubits,
            ));
        }
        for instruction in other.instructions() {
            self.append_instruction(instruction, None)?;
        }
        Ok(self)
    }

    /// Compose another circuit onto the mapped qubits of this circuit.
    ///
    /// # Errors
    ///
    /// Returns an error if the mapping length is wrong or if any mapped qubit is invalid or repeated.
    pub fn compose(&mut self, other: &Self, qubit_mapping: &[usize]) -> WarosResult<&mut Self> {
        if qubit_mapping.len() != other.num_qubits {
            return Err(WarosError::InvalidQubitMapping(
                qubit_mapping.len(),
                other.num_qubits,
            ));
        }
        self.validate_qubit_list(qubit_mapping)?;
        for instruction in other.instructions() {
            self.append_instruction(instruction, Some(qubit_mapping))?;
        }
        Ok(self)
    }

    /// Return the gate depth of the circuit.
    #[must_use]
    pub fn depth(&self) -> usize {
        let mut layers = vec![0usize; self.num_qubits];
        let mut max_depth = 0usize;
        for instruction in &self.instructions {
            if let Instruction::GateOp { targets, .. }
            | Instruction::ConditionalGate { targets, .. } = instruction
            {
                let layer = targets
                    .iter()
                    .map(|target| layers[*target])
                    .max()
                    .unwrap_or(0)
                    + 1;
                for &target in targets {
                    layers[target] = layer;
                }
                max_depth = max_depth.max(layer);
            }
        }
        max_depth
    }

    /// Render the circuit as an ASCII diagram.
    #[must_use]
    pub fn to_ascii(&self) -> String {
        let mut rows: Vec<String> = (0..self.num_qubits)
            .map(|qubit| format!("q{qubit}: "))
            .collect();

        for instruction in &self.instructions {
            let width = match instruction {
                Instruction::GateOp { gate, .. } | Instruction::ConditionalGate { gate, .. } => {
                    (gate.name.len() + 2).max(5)
                }
                Instruction::Measure { .. } | Instruction::Barrier { .. } => 5,
            };
            let width = if width % 2 == 0 { width + 1 } else { width };
            let wire = "─".repeat(width);
            let mut column = vec![wire.clone(); self.num_qubits];

            match instruction {
                Instruction::GateOp { gate, targets } if gate.num_qubits == 1 => {
                    column[targets[0]] = format!("{:^width$}", format!("[{}]", gate.name));
                }
                Instruction::ConditionalGate { gate, targets, .. } if gate.num_qubits == 1 => {
                    column[targets[0]] = format!("{:^width$}", format!("[{}?]", gate.name));
                }
                Instruction::GateOp { gate, targets } if gate.name == "CNOT" => {
                    let (control, target) = (targets[0], targets[1]);
                    let start = control.min(target);
                    let end = control.max(target);
                    for cell in column.iter_mut().take(end).skip(start + 1) {
                        *cell = format!("{:^width$}", "│");
                    }
                    column[control] = format!("{:^width$}", "●");
                    column[target] = format!("{:^width$}", "⊕");
                }
                Instruction::ConditionalGate { gate, targets, .. } if gate.name == "CNOT" => {
                    let (control, target) = (targets[0], targets[1]);
                    let start = control.min(target);
                    let end = control.max(target);
                    for cell in column.iter_mut().take(end).skip(start + 1) {
                        *cell = format!("{:^width$}", "│");
                    }
                    column[control] = format!("{:^width$}", "●?");
                    column[target] = format!("{:^width$}", "⊕?");
                }
                Instruction::GateOp { gate, targets } => {
                    let start = targets[0].min(targets[1]);
                    let end = targets[0].max(targets[1]);
                    for cell in column.iter_mut().take(end).skip(start + 1) {
                        *cell = format!("{:^width$}", "│");
                    }
                    let label = format!("[{}]", gate.name);
                    column[targets[0]] = format!("{label:^width$}");
                    column[targets[1]] = format!("{label:^width$}");
                }
                Instruction::ConditionalGate { gate, targets, .. } => {
                    let start = targets[0].min(targets[1]);
                    let end = targets[0].max(targets[1]);
                    for cell in column.iter_mut().take(end).skip(start + 1) {
                        *cell = format!("{:^width$}", "│");
                    }
                    let label = format!("[{}?]", gate.name);
                    column[targets[0]] = format!("{label:^width$}");
                    column[targets[1]] = format!("{label:^width$}");
                }
                Instruction::Measure { qubit, .. } => {
                    column[*qubit] = format!("{:^width$}", "[M]");
                }
                Instruction::Barrier { qubits } => {
                    for &qubit in qubits {
                        column[qubit] = format!("{:^width$}", "┆");
                    }
                }
            }

            for (row, cell) in rows.iter_mut().zip(column) {
                row.push_str(&cell);
            }
        }

        rows.join("\n")
    }
}
