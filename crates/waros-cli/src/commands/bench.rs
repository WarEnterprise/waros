use std::time::Instant;

use waros_quantum::{Circuit, Simulator, WarosError};

use crate::utils::CliResult;

pub fn execute(qubits: usize) -> CliResult {
    let simulator = Simulator::builder().parallel(true).build();
    println!("WarOS benchmark probe ({qubits} qubits)");

    benchmark("Hadamard layer", || hadamard_circuit(qubits), &simulator)?;
    benchmark("Bell chain", || bell_chain_circuit(qubits), &simulator)?;
    benchmark("QFT", || qft_circuit(qubits), &simulator)?;

    Ok(())
}

fn benchmark<F>(
    label: &str,
    builder: F,
    simulator: &Simulator,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: FnOnce() -> Result<Circuit, WarosError>,
{
    let circuit = builder()?;
    let start = Instant::now();
    let _state = simulator.statevector(&circuit)?;
    let elapsed = start.elapsed();
    println!("  {label:<14} {:>8.3} ms", elapsed.as_secs_f64() * 1_000.0);
    Ok(())
}

fn hadamard_circuit(qubits: usize) -> Result<Circuit, WarosError> {
    let mut circuit = Circuit::new(qubits)?;
    for qubit in 0..qubits {
        circuit.h(qubit)?;
    }
    Ok(circuit)
}

fn bell_chain_circuit(qubits: usize) -> Result<Circuit, WarosError> {
    let mut circuit = Circuit::new(qubits)?;
    circuit.h(0)?;
    for qubit in 1..qubits {
        circuit.cnot(qubit - 1, qubit)?;
    }
    Ok(circuit)
}

fn qft_circuit(qubits: usize) -> Result<Circuit, WarosError> {
    let mut circuit = Circuit::new(qubits)?;
    let register: Vec<usize> = (0..qubits).collect();
    circuit.qft(&register)?;
    Ok(circuit)
}
