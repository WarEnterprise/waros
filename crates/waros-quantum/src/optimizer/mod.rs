use crate::circuit::{Circuit, Instruction};
use crate::error::WarosResult;
use crate::gate::Gate;

const ROTATION_EPSILON: f64 = 1e-9;

/// Peephole optimizer for small-to-medium `WarOS` circuits.
#[derive(Debug, Clone, Default)]
pub struct CircuitOptimizer {
    stats: OptimizationStats,
}

/// Summary of one optimization run.
#[derive(Debug, Clone, Copy, Default)]
pub struct OptimizationStats {
    pub original_gate_count: usize,
    pub optimized_gate_count: usize,
    pub gates_removed: usize,
    pub reduction_percent: f64,
}

impl CircuitOptimizer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Optimize a circuit with simple cancellation and gate-merging rules.
    ///
    /// # Errors
    ///
    /// Returns an error if the optimized circuit cannot be reconstructed.
    pub fn optimize(&mut self, circuit: &Circuit) -> WarosResult<Circuit> {
        let optimized_instructions = optimize_instructions(circuit.instructions());
        let optimized = rebuild_circuit(circuit, &optimized_instructions)?;
        self.stats = OptimizationStats::from_counts(circuit.gate_count(), optimized.gate_count());
        Ok(optimized)
    }

    /// Return the statistics from the most recent optimization run.
    #[must_use]
    pub fn stats(&self) -> OptimizationStats {
        self.stats
    }
}

impl OptimizationStats {
    fn from_counts(original_gate_count: usize, optimized_gate_count: usize) -> Self {
        let gates_removed = original_gate_count.saturating_sub(optimized_gate_count);
        let reduction_percent = if original_gate_count == 0 {
            0.0
        } else {
            (count_to_f64(gates_removed) / count_to_f64(original_gate_count)) * 100.0
        };

        Self {
            original_gate_count,
            optimized_gate_count,
            gates_removed,
            reduction_percent,
        }
    }
}

fn optimize_instructions(instructions: &[Instruction]) -> Vec<Instruction> {
    let mut optimized = Vec::new();

    for instruction in instructions {
        if let Some(last) = optimized.last_mut() {
            if try_cancel_pair(last, instruction) {
                optimized.pop();
                continue;
            }
            if try_merge_rotations(last, instruction) {
                continue;
            }
        }
        optimized.push(instruction.clone());
    }

    optimized
}

fn try_cancel_pair(previous: &mut Instruction, current: &Instruction) -> bool {
    match (&*previous, current) {
        (
            Instruction::GateOp {
                gate: previous_gate,
                targets: previous_targets,
            },
            Instruction::GateOp {
                gate: current_gate,
                targets: current_targets,
            },
        ) if previous_targets == current_targets => {
            is_self_inverse(previous_gate, current_gate)
                || is_inverse_pair(previous_gate, current_gate)
        }
        _ => false,
    }
}

fn try_merge_rotations(previous: &mut Instruction, current: &Instruction) -> bool {
    let (
        Instruction::GateOp {
            gate: previous_gate,
            targets: previous_targets,
        },
        Instruction::GateOp {
            gate: current_gate,
            targets: current_targets,
        },
    ) = (&mut *previous, current)
    else {
        return false;
    };

    if previous_targets != current_targets
        || previous_gate.num_qubits != 1
        || current_gate.num_qubits != 1
    {
        return false;
    }

    let Some((axis, previous_angle)) = rotation_axis_and_angle(previous_gate) else {
        return false;
    };
    let Some((current_axis, current_angle)) = rotation_axis_and_angle(current_gate) else {
        return false;
    };
    if axis != current_axis {
        return false;
    }

    let merged_angle = previous_angle + current_angle;
    if merged_angle.abs() <= ROTATION_EPSILON {
        *previous = Instruction::Barrier { qubits: Vec::new() };
        return true;
    }

    *previous_gate = rotation_gate(axis, merged_angle);
    true
}

fn rebuild_circuit(template: &Circuit, instructions: &[Instruction]) -> WarosResult<Circuit> {
    let mut circuit =
        Circuit::with_classical_bits(template.num_qubits(), template.num_classical_bits())?;

    for instruction in instructions {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                circuit.custom_gate(gate.clone(), targets)?;
            }
            Instruction::ConditionalGate {
                classical_bits,
                value,
                gate,
                targets,
            } => {
                circuit.conditional_gate(classical_bits, *value, gate.clone(), targets)?;
            }
            Instruction::Measure {
                qubit,
                classical_bit,
            } => {
                circuit.measure_into(*qubit, *classical_bit)?;
            }
            Instruction::Barrier { qubits } if !qubits.is_empty() => {
                circuit.barrier(qubits)?;
            }
            Instruction::Barrier { .. } => {}
        }
    }

    Ok(circuit)
}

fn is_self_inverse(previous: &Gate, current: &Gate) -> bool {
    previous.name == current.name
        && matches!(
            previous.name.as_str(),
            "H" | "X" | "Y" | "Z" | "CNOT" | "CZ" | "SWAP"
        )
}

fn is_inverse_pair(previous: &Gate, current: &Gate) -> bool {
    matches!(
        (previous.name.as_str(), current.name.as_str()),
        ("S", "Sdg") | ("Sdg", "S") | ("T", "Tdg") | ("Tdg", "T")
    )
}

fn rotation_axis_and_angle(gate: &Gate) -> Option<(char, f64)> {
    for axis in ['x', 'y', 'z'] {
        let prefix = format!("R{axis}(");
        if let Some(angle_text) = gate
            .name
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(')'))
        {
            return angle_text.parse::<f64>().ok().map(|angle| (axis, angle));
        }
    }
    None
}

fn rotation_gate(axis: char, angle: f64) -> Gate {
    match axis {
        'x' => crate::gate::rx(angle),
        'y' => crate::gate::ry(angle),
        'z' => crate::gate::rz(angle),
        _ => unreachable!("optimizer only merges Rx/Ry/Rz rotations"),
    }
}

fn count_to_f64(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod tests {
    use super::CircuitOptimizer;
    use crate::Circuit;

    #[test]
    fn optimizer_cancels_inverse_pairs() {
        let mut circuit = Circuit::new(1).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.h(0).expect("valid gate");

        let mut optimizer = CircuitOptimizer::new();
        let optimized_circuit = optimizer.optimize(&circuit).expect("optimization succeeds");
        assert_eq!(optimized_circuit.gate_count(), 0);
        assert_eq!(optimizer.stats().gates_removed, 2);
    }

    #[test]
    fn optimizer_merges_rotations() {
        let mut circuit = Circuit::new(1).expect("valid circuit");
        circuit.rz(0, 0.25).expect("valid gate");
        circuit.rz(0, 0.50).expect("valid gate");

        let mut optimizer = CircuitOptimizer::new();
        let optimized_circuit = optimizer.optimize(&circuit).expect("optimization succeeds");
        assert_eq!(optimized_circuit.gate_count(), 1);
    }
}
