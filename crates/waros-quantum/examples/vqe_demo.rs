use waros_quantum::{vqe, AnsatzType, Hamiltonian, Simulator};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("============================================================");
    println!("  WarOS Quantum - VQE: Hydrogen Molecule");
    println!("  Finding the ground-state energy of H2");
    println!("============================================================");
    println!();

    let simulator = Simulator::with_seed(42);
    let hamiltonian = Hamiltonian::hydrogen_molecule();
    let result = vqe(
        &hamiltonian,
        AnsatzType::RyLinear,
        &[0.0, 0.0],
        30,
        1_000,
        &simulator,
    )?;

    println!("Hamiltonian: H2 at 0.735 A bond length (STO-3G basis)");
    println!("  H = -1.0524 I x I + 0.3979 I x Z - 0.3979 Z x I - 0.0112 Z x Z + 0.1809 X x X");
    println!("  Ansatz: Ry-Linear (2 parameters)");
    println!();

    for (index, energy) in result.energy_history.iter().enumerate() {
        if matches!(index, 0 | 4 | 9 | 19 | 29) || index + 1 == result.iterations {
            println!("  Iteration {:>2}: E = {energy:.4} Ha", index + 1);
        }
    }

    let exact = -1.1373;
    let error = (result.energy - exact).abs();
    println!();
    println!("  OK Ground-state energy: {:.4} Hartree", result.energy);
    println!("  OK Exact reference:     {exact:.4} Hartree");
    println!("  OK Absolute error:      {error:.4} Hartree");
    println!();
    println!("  This is the canonical hybrid quantum-classical chemistry workflow.");

    Ok(())
}
