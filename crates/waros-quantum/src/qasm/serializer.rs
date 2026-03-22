use crate::circuit::Instruction;
use crate::Circuit;

pub(super) fn to_qasm(circuit: &Circuit) -> String {
    let mut lines = vec![
        "OPENQASM 2.0;".to_string(),
        "include \"qelib1.inc\";".to_string(),
        format!("qreg q[{}];", circuit.num_qubits()),
    ];
    if circuit.num_classical_bits() > 0 {
        lines.push(format!("creg c[{}];", circuit.num_classical_bits()));
    }

    for instruction in circuit.instructions() {
        match instruction {
            Instruction::GateOp { gate, targets } => {
                lines.push(format_gate(gate.name.as_str(), targets));
            }
            Instruction::ConditionalGate {
                value,
                gate,
                targets,
                ..
            } => {
                let gate_statement = format_gate(gate.name.as_str(), targets);
                lines.push(format!(
                    "if(c=={value}) {}",
                    gate_statement.trim_end_matches(';')
                ));
            }
            Instruction::Measure {
                qubit,
                classical_bit,
            } => lines.push(format!("measure q[{qubit}] -> c[{classical_bit}];")),
            Instruction::Barrier { qubits } => {
                let all_qubits: Vec<usize> = (0..circuit.num_qubits()).collect();
                if qubits == &all_qubits {
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

    lines.join("\n")
}

fn format_gate(name: &str, targets: &[usize]) -> String {
    match (name, targets) {
        ("H", [q0]) => format!("h q[{q0}];"),
        ("X", [q0]) => format!("x q[{q0}];"),
        ("Y", [q0]) => format!("y q[{q0}];"),
        ("Z", [q0]) => format!("z q[{q0}];"),
        ("S", [q0]) => format!("s q[{q0}];"),
        ("Sdg", [q0]) => format!("sdg q[{q0}];"),
        ("T", [q0]) => format!("t q[{q0}];"),
        ("Tdg", [q0]) => format!("tdg q[{q0}];"),
        ("CNOT", [q0, q1]) => format!("cx q[{q0}], q[{q1}];"),
        ("CZ", [q0, q1]) => format!("cz q[{q0}], q[{q1}];"),
        ("SWAP", [q0, q1]) => format!("swap q[{q0}], q[{q1}];"),
        _ if name.starts_with("CR") && targets.len() == 2 => {
            format!("crk({}) q[{}], q[{}];", &name[2..], targets[0], targets[1])
        }
        _ if name.starts_with("Rx(") && targets.len() == 1 => {
            format!("rx{} q[{}];", &name[2..], targets[0])
        }
        _ if name.starts_with("Ry(") && targets.len() == 1 => {
            format!("ry{} q[{}];", &name[2..], targets[0])
        }
        _ if name.starts_with("Rz(") && targets.len() == 1 => {
            format!("rz{} q[{}];", &name[2..], targets[0])
        }
        _ if name.starts_with("U3(") && targets.len() == 1 => {
            format!("u3{} q[{}];", &name[2..], targets[0])
        }
        _ => format!("// unsupported gate {name} on {targets:?}"),
    }
}
