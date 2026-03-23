use waros_quantum::backends::ibm::IBMBackend;
use waros_quantum::{Circuit, WarosError};

fn main() -> Result<(), WarosError> {
    println!("============================================================");
    println!("  WarOS Quantum - Running on REAL IBM Quantum Hardware");
    println!("============================================================\n");

    println!("Credentials are resolved from:");
    println!("  1. WAROS_IBM_TOKEN / WAROS_IBM_INSTANCE_CRN");
    println!("  2. QISKIT_IBM_TOKEN / QISKIT_IBM_INSTANCE");
    println!("  3. ~/.waros/ibm_token and ~/.waros/ibm_instance_crn");
    println!("  4. ~/.qiskit/qiskit-ibm.json\n");

    let ibm = IBMBackend::from_env()?;

    println!("Available backends:");
    for backend in ibm.list_backends()? {
        println!(
            "  {} - {} qubits ({}, queue={})",
            backend.name,
            backend.num_qubits,
            backend.status.message(),
            backend.queue_length
        );
    }
    println!();

    let mut circuit = Circuit::new(2)?;
    circuit.h(0)?;
    circuit.cnot(0, 1)?;
    circuit.measure_all()?;

    println!("Submitting Bell state to ibm_brisbane...\n");
    let result = ibm.run_on("ibm_brisbane", &circuit, 1000)?;

    println!("Results from IBM Quantum hardware:");
    result.print_histogram();
    Ok(())
}
