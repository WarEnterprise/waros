use waros_quantum::{Circuit, NoiseModel, Simulator, WarosError};

fn main() -> Result<(), WarosError> {
    println!("╔══════════════════════════════════════════════╗");
    println!("║  WarOS Quantum - Noise Simulation           ║");
    println!("║  Comparing ideal vs realistic hardware      ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();
    println!("Circuit: 5-qubit GHZ state (H + 4 CNOTs)");
    println!();

    let circuit = ghz_circuit(5)?;
    run_profile("Ideal simulation", &circuit, None)?;
    run_profile("IBM-like noise", &circuit, Some(NoiseModel::ibm_like()))?;
    run_profile("IonQ-like noise", &circuit, Some(NoiseModel::ionq_like()))?;

    println!("Conclusion: trapped-ion style noise retains higher GHZ fidelity");
    println!("than superconducting two-qubit error profiles for this circuit.");
    Ok(())
}

fn ghz_circuit(num_qubits: usize) -> Result<Circuit, WarosError> {
    let mut circuit = Circuit::new(num_qubits)?;
    circuit.h(0)?;
    for target in 1..num_qubits {
        circuit.cnot(0, target)?;
    }
    circuit.measure_all()?;
    Ok(circuit)
}

fn run_profile(
    label: &str,
    circuit: &Circuit,
    noise: Option<NoiseModel>,
) -> Result<(), WarosError> {
    let simulator = match noise {
        Some(model) => Simulator::builder().seed(42).noise(model).build(),
        None => Simulator::builder().seed(42).build(),
    };
    let result = simulator.run(circuit, 10_000)?;
    let fidelity = result.probability("00000") + result.probability("11111");

    println!("-- {label} --");
    for (state, count, probability) in result.histogram().into_iter().take(6) {
        println!("  |{state}> : {count:>4} ({:>4.1}%)", probability * 100.0);
    }
    println!("  Fidelity: {:>5.1}%", fidelity * 100.0);
    println!();
    Ok(())
}
