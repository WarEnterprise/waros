use std::f64::consts::PI;

use crate::circuit::Instruction;
use crate::error::WarosError;
use crate::optimizer::CircuitOptimizer;
use crate::{Circuit, WarosResult};

use super::types::IBMBackendInfo;

/// Convert a WarOS circuit into OpenQASM 3.0 accepted by Qiskit Runtime.
pub fn circuit_to_ibm_qasm(circuit: &Circuit) -> WarosResult<String> {
    let mut lines = vec![
        "OPENQASM 3.0;".to_string(),
        "include \"stdgates.inc\";".to_string(),
        format!("qubit[{}] q;", circuit.num_qubits()),
    ];

    if circuit.num_classical_bits() > 0 {
        lines.push(format!("bit[{}] c;", circuit.num_classical_bits()));
    }

    for instruction in circuit.instructions() {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                lines.extend(render_gate(&gate.name, targets)?);
            }
            Instruction::ConditionalGate {
                value,
                gate,
                targets,
                ..
            } => {
                for statement in render_gate(&gate.name, targets)? {
                    let statement = statement.trim_end_matches(';');
                    lines.push(format!("if (c == {value}) {statement};"));
                }
            }
            Instruction::Measure {
                qubit,
                classical_bit,
            } => lines.push(format!("c[{classical_bit}] = measure q[{qubit}];")),
            Instruction::Barrier { qubits } => {
                if qubits.is_empty() || qubits.len() == circuit.num_qubits() {
                    lines.push("barrier q;".to_string());
                } else {
                    let operands = qubits
                        .iter()
                        .map(|qubit| format!("q[{qubit}]"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    lines.push(format!("barrier {operands};"));
                }
            }
        }
    }

    Ok(lines.join("\n"))
}

pub fn validate_for_backend(
    circuit: &Circuit,
    backend: &IBMBackendInfo,
    shots: u32,
) -> WarosResult<()> {
    if circuit.num_qubits() > backend.num_qubits {
        return Err(WarosError::TooManyQubits(
            circuit.num_qubits(),
            backend.num_qubits,
        ));
    }

    if let Some(max_shots) = backend.max_shots {
        if shots > max_shots {
            return Err(WarosError::InvalidShots(shots));
        }
    }

    let has_measurements = circuit
        .instructions()
        .iter()
        .any(|instruction| matches!(instruction, Instruction::Measure { .. }));
    if !has_measurements {
        return Err(WarosError::HardwareError(
            "IBM Sampler requires at least one measurement in the circuit".into(),
        ));
    }

    Ok(())
}

pub fn optimize_for_ibm(circuit: &Circuit, optimization_level: u8) -> WarosResult<Circuit> {
    if optimization_level == 0 {
        return Ok(circuit.clone());
    }

    let mut current = circuit.clone();
    let passes = usize::from(optimization_level.min(3));
    for _ in 0..passes {
        let mut optimizer = CircuitOptimizer::new();
        let optimized = optimizer.optimize(&current)?;
        if optimized.gate_count() == current.gate_count() {
            return Ok(optimized);
        }
        current = optimized;
    }

    Ok(current)
}

fn render_gate(name: &str, targets: &[usize]) -> WarosResult<Vec<String>> {
    Ok(match (name, targets) {
        ("H", [q0]) => vec![format!("h q[{q0}];")],
        ("X", [q0]) => vec![format!("x q[{q0}];")],
        ("Y", [q0]) => vec![format!("y q[{q0}];")],
        ("Z", [q0]) => vec![format!("z q[{q0}];")],
        ("S", [q0]) => vec![format!("s q[{q0}];")],
        ("Sdg", [q0]) => vec![format!("sdg q[{q0}];")],
        ("T", [q0]) => vec![format!("t q[{q0}];")],
        ("Tdg", [q0]) => vec![format!("tdg q[{q0}];")],
        ("SX", [q0]) => vec![format!("sx q[{q0}];")],
        ("CNOT", [q0, q1]) => vec![format!("cx q[{q0}], q[{q1}];")],
        ("CY", [q0, q1]) => vec![format!("cy q[{q0}], q[{q1}];")],
        ("CZ", [q0, q1]) => vec![format!("cz q[{q0}], q[{q1}];")],
        ("SWAP", [q0, q1]) => vec![format!("swap q[{q0}], q[{q1}];")],
        _ if name.starts_with("Rx(") && targets.len() == 1 => {
            vec![format!(
                "rx({}) q[{}];",
                single_angle(name, "Rx(")?,
                targets[0]
            )]
        }
        _ if name.starts_with("Ry(") && targets.len() == 1 => {
            vec![format!(
                "ry({}) q[{}];",
                single_angle(name, "Ry(")?,
                targets[0]
            )]
        }
        _ if name.starts_with("Rz(") && targets.len() == 1 => {
            vec![format!(
                "rz({}) q[{}];",
                single_angle(name, "Rz(")?,
                targets[0]
            )]
        }
        _ if name.starts_with("U3(") && targets.len() == 1 => {
            let [theta, phi, lambda] = three_angles(name, "U3(")?;
            vec![format!("U({theta}, {phi}, {lambda}) q[{}];", targets[0])]
        }
        _ if name.starts_with("Rzz(") && targets.len() == 2 => {
            let theta = single_angle(name, "Rzz(")?;
            vec![
                format!("cx q[{}], q[{}];", targets[0], targets[1]),
                format!("rz({theta}) q[{}];", targets[1]),
                format!("cx q[{}], q[{}];", targets[0], targets[1]),
            ]
        }
        _ if name.starts_with("CR") && targets.len() == 2 => {
            let k = name
                .trim_start_matches("CR")
                .parse::<u32>()
                .map_err(|error| WarosError::ParseError(error.to_string()))?;
            let angle = 2.0 * PI / 2.0_f64.powi(i32::try_from(k).unwrap_or(i32::MAX));
            vec![format!("cp({angle}) q[{}], q[{}];", targets[0], targets[1])]
        }
        _ => {
            return Err(WarosError::HardwareError(format!(
                "Gate '{name}' is not supported by the IBM transpiler"
            )));
        }
    })
}

fn single_angle(name: &str, prefix: &str) -> WarosResult<f64> {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| WarosError::ParseError(format!("Cannot parse gate angle from '{name}'")))?
        .parse::<f64>()
        .map_err(|error| WarosError::ParseError(error.to_string()))
}

fn three_angles(name: &str, prefix: &str) -> WarosResult<[f64; 3]> {
    let raw = name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| WarosError::ParseError(format!("Cannot parse gate angles from '{name}'")))?;

    let values = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| WarosError::ParseError(error.to_string()))?;

    match values.as_slice() {
        [theta, phi, lambda] => Ok([*theta, *phi, *lambda]),
        _ => Err(WarosError::ParseError(format!(
            "Expected three angles in '{name}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::ibm::types::BackendStatus;

    #[test]
    fn emits_openqasm_three_measurements() {
        let mut circuit = Circuit::new(2).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        circuit.cnot(0, 1).expect("valid gate");
        circuit.measure_all().expect("valid measurements");

        let qasm = circuit_to_ibm_qasm(&circuit).expect("qasm emitted");
        assert!(qasm.contains("OPENQASM 3.0;"));
        assert!(qasm.contains("bit[2] c;"));
        assert!(qasm.contains("c[0] = measure q[0];"));
    }

    #[test]
    fn validate_rejects_too_many_qubits() {
        let circuit = Circuit::new(5).expect("valid circuit");
        let backend = IBMBackendInfo {
            name: "test".into(),
            num_qubits: 3,
            status: BackendStatus {
                name: "online".into(),
                reason: None,
            },
            queue_length: 0,
            is_simulator: false,
            processor_type: None,
            clops: None,
            wait_time_seconds: None,
            basis_gates: vec![],
            coupling_map: None,
            max_shots: Some(1000),
            max_experiments: None,
            supported_features: vec![],
            supported_instructions: vec![],
            conditional: false,
            memory: true,
        };
        assert!(validate_for_backend(&circuit, &backend, 10).is_err());
    }

    #[test]
    fn optimization_level_zero_keeps_circuit() {
        let mut circuit = Circuit::new(1).expect("valid circuit");
        circuit.h(0).expect("valid gate");
        let optimized = optimize_for_ibm(&circuit, 0).expect("optimize succeeds");
        assert_eq!(optimized.gate_count(), circuit.gate_count());
    }
}
