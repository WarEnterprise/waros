use std::io::{self, Write};

use waros_quantum::{to_qasm, Circuit, Simulator};

use crate::utils::{parse_angle, CliResult};

pub fn execute(qubits: usize) -> CliResult {
    let mut circuit = Circuit::new(qubits)?;
    println!("WarOS Quantum REPL ({qubits} qubits, StateVector backend)");
    println!("Type 'help' for commands, 'quit' to exit.");
    println!();

    loop {
        print!("waros> ");
        io::stdout().flush()?;

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if matches!(input, "quit" | "exit") {
            break;
        }
        if input == "help" {
            print_help();
            continue;
        }

        match handle_command(&mut circuit, qubits, input) {
            Ok(message) => println!("{message}"),
            Err(error) => println!("error: {error}"),
        }
    }

    Ok(())
}

fn handle_command(circuit: &mut Circuit, qubits: usize, input: &str) -> CliResult<String> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    match parts.as_slice() {
        ["h", qubit] => {
            circuit.h(parse_qubit(qubit)?)?;
            Ok(format!("Applied H on q{qubit}"))
        }
        ["x", qubit] => {
            circuit.x(parse_qubit(qubit)?)?;
            Ok(format!("Applied X on q{qubit}"))
        }
        ["y", qubit] => {
            circuit.y(parse_qubit(qubit)?)?;
            Ok(format!("Applied Y on q{qubit}"))
        }
        ["z", qubit] => {
            circuit.z(parse_qubit(qubit)?)?;
            Ok(format!("Applied Z on q{qubit}"))
        }
        ["s", qubit] => {
            circuit.s(parse_qubit(qubit)?)?;
            Ok(format!("Applied S on q{qubit}"))
        }
        ["t", qubit] => {
            circuit.t(parse_qubit(qubit)?)?;
            Ok(format!("Applied T on q{qubit}"))
        }
        ["cx", control, target] => {
            let control = parse_qubit(control)?;
            let target = parse_qubit(target)?;
            circuit.cx(control, target)?;
            Ok(format!("Applied CNOT on q{control} -> q{target}"))
        }
        ["cz", control, target] => {
            let control = parse_qubit(control)?;
            let target = parse_qubit(target)?;
            circuit.cz(control, target)?;
            Ok(format!("Applied CZ on q{control} <-> q{target}"))
        }
        ["swap", q0, q1] => {
            let q0 = parse_qubit(q0)?;
            let q1 = parse_qubit(q1)?;
            circuit.swap(q0, q1)?;
            Ok(format!("Applied SWAP on q{q0} <-> q{q1}"))
        }
        ["rx", qubit, angle] => {
            let qubit = parse_qubit(qubit)?;
            let angle = parse_angle(angle)?;
            circuit.rx(qubit, angle)?;
            Ok(format!("Applied Rx({angle:.4}) on q{qubit}"))
        }
        ["ry", qubit, angle] => {
            let qubit = parse_qubit(qubit)?;
            let angle = parse_angle(angle)?;
            circuit.ry(qubit, angle)?;
            Ok(format!("Applied Ry({angle:.4}) on q{qubit}"))
        }
        ["rz", qubit, angle] => {
            let qubit = parse_qubit(qubit)?;
            let angle = parse_angle(angle)?;
            circuit.rz(qubit, angle)?;
            Ok(format!("Applied Rz({angle:.4}) on q{qubit}"))
        }
        ["measure"] => {
            circuit.measure_all()?;
            Ok("Measured all qubits".into())
        }
        ["measure", qubit] => {
            let qubit = parse_qubit(qubit)?;
            let classical_bit = circuit.measure(qubit)?;
            Ok(format!("Measured q{qubit} into c{classical_bit}"))
        }
        ["run", shots] => {
            let shots = shots.parse::<u32>()?;
            let result = Simulator::new().run(circuit, shots)?;
            Ok(format!("Results ({shots} shots):\n{result}"))
        }
        ["show"] => Ok(circuit.to_ascii()),
        ["state"] => {
            let state = Simulator::new().statevector(circuit)?;
            let mut rows = Vec::new();
            for (index, amplitude) in state.iter().enumerate() {
                let probability = amplitude.norm_sq();
                if probability > 1e-12 {
                    rows.push(format!(
                        "  |{}> : {} ({:.1}%)",
                        basis_state(index, qubits),
                        amplitude,
                        probability * 100.0
                    ));
                }
            }
            Ok(format!("State vector:\n{}", rows.join("\n")))
        }
        ["reset"] => {
            *circuit = Circuit::new(qubits)?;
            Ok("Circuit reset. All qubits back to |0>.".into())
        }
        ["depth"] => Ok(format!("Circuit depth: {}", circuit.depth())),
        ["gates"] => Ok(format!("Gate count: {}", circuit.gate_count())),
        ["qasm"] => Ok(to_qasm(circuit)),
        _ => Ok("Unknown command. Type 'help' for supported commands.".into()),
    }
}

fn parse_qubit(raw: &str) -> Result<usize, Box<dyn std::error::Error>> {
    Ok(raw.parse::<usize>()?)
}

fn basis_state(index: usize, qubits: usize) -> String {
    (0..qubits)
        .map(|qubit| if (index >> qubit) & 1 == 1 { '1' } else { '0' })
        .collect()
}

fn print_help() {
    println!("Commands:");
    println!("  h/x/y/z/s/t Q");
    println!("  cx/cz/swap Q Q");
    println!("  rx/ry/rz Q ANGLE");
    println!("  measure [Q]");
    println!("  run SHOTS");
    println!("  show");
    println!("  state");
    println!("  reset");
    println!("  depth");
    println!("  gates");
    println!("  qasm");
    println!("  help");
    println!("  quit");
}
