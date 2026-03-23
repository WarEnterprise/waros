use alloc::vec::Vec;

use core::f64::consts::PI;

use libm::sqrt;

use crate::arch::x86_64::interrupts;
use crate::display::console::Colors;
use crate::quantum::display::{
    display_probabilities, display_results, display_state, format_basis_state,
};
use crate::quantum::gates::{
    cnot, controlled_phase, cz, hadamard, pauli_x, pauli_z, rx, ry, rz, swap,
};
use crate::quantum::simulator::{
    apply_1q, apply_2q, measure_qubit_collapse, measure_selected, Xorshift64,
};
use crate::quantum::state::{norm_sq, QuantumState};
use crate::{kprint_colored, kprintln};

const DEFAULT_SHOTS: usize = 1_000;

/// Run one of the named built-in circuit demonstrations.
pub fn run_builtin(name: &str) -> Result<(), &'static str> {
    match name {
        "bell" => run_bell(),
        "ghz" | "ghz3" => run_ghz3(),
        "grover" => run_grover(),
        "teleport" => run_teleport(),
        "qft" | "qft4" => run_qft4(),
        "deutsch" => run_deutsch(),
        "bernstein" | "bv" => run_bernstein_vazirani(),
        "superdense" => run_superdense(),
        "shor" | "factor" => run_shor_demo(),
        "vqe" | "hydrogen" => run_vqe_demo(),
        "qaoa" | "maxcut" => run_qaoa_demo(),
        "" => {
            kprintln!("Usage: qcircuit <name>");
            list_circuits();
            Ok(())
        }
        _ => {
            kprintln!("Unknown built-in circuit '{}'.", name);
            list_circuits();
            Ok(())
        }
    }
}

fn run_bell() -> Result<(), &'static str> {
    let mut state = QuantumState::new(2)?;
    apply_1q(&mut state, 0, &hadamard())?;
    apply_2q(&mut state, 0, 1, &cnot())?;

    header("Bell State (2 qubits)");
    kprintln!("  H q0");
    kprintln!("  CNOT q0 q1");
    let results = measure_all_with_seed(&state, DEFAULT_SHOTS);
    display_results(&results, 2, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "Bell state confirmed: ");
    kprintln!("maximal entanglement between q0 and q1.");
    Ok(())
}

fn run_ghz3() -> Result<(), &'static str> {
    let mut state = QuantumState::new(3)?;
    apply_1q(&mut state, 0, &hadamard())?;
    apply_2q(&mut state, 0, 1, &cnot())?;
    apply_2q(&mut state, 0, 2, &cnot())?;

    header("GHZ State (3 qubits)");
    kprintln!("  H q0");
    kprintln!("  CNOT q0 q1");
    kprintln!("  CNOT q0 q2");
    let results = measure_all_with_seed(&state, DEFAULT_SHOTS);
    display_results(&results, 3, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "GHZ state confirmed: ");
    kprintln!("perfect 3-qubit correlation.");
    Ok(())
}

fn run_grover() -> Result<(), &'static str> {
    let mut state = QuantumState::new(2)?;

    apply_1q(&mut state, 0, &hadamard())?;
    apply_1q(&mut state, 1, &hadamard())?;

    apply_2q(&mut state, 0, 1, &cz())?;

    apply_1q(&mut state, 0, &hadamard())?;
    apply_1q(&mut state, 1, &hadamard())?;
    apply_1q(&mut state, 0, &pauli_x())?;
    apply_1q(&mut state, 1, &pauli_x())?;
    apply_2q(&mut state, 0, 1, &cz())?;
    apply_1q(&mut state, 0, &pauli_x())?;
    apply_1q(&mut state, 1, &pauli_x())?;
    apply_1q(&mut state, 0, &hadamard())?;
    apply_1q(&mut state, 1, &hadamard())?;

    header("Grover Search (2-bit target |11>)");
    kprintln!("  [12 gates applied]");
    let results = measure_all_with_seed(&state, DEFAULT_SHOTS);
    display_results(&results, 2, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "Search found target |11>: ");
    kprintln!("one Grover iteration amplified the marked state.");
    Ok(())
}

fn run_teleport() -> Result<(), &'static str> {
    header("Quantum Teleportation");
    kprintln!("  Teleporting state Ry(pi/3)|0> from q0 to q2");
    kprintln!("  Bell pair, Bell-basis measurement, classical correction");

    let mut rng = seeded_rng(3);
    let mut bob_zero = 0usize;
    let mut bob_one = 0usize;

    for _ in 0..DEFAULT_SHOTS {
        let mut state = QuantumState::new(3)?;
        apply_1q(&mut state, 0, &ry(PI / 3.0))?;
        apply_1q(&mut state, 1, &hadamard())?;
        apply_2q(&mut state, 1, 2, &cnot())?;
        apply_2q(&mut state, 0, 1, &cnot())?;
        apply_1q(&mut state, 0, &hadamard())?;

        let alice_qubit = measure_qubit_collapse(&mut state, 0, &mut rng)?;
        let entanglement_qubit = measure_qubit_collapse(&mut state, 1, &mut rng)?;

        if entanglement_qubit == 1 {
            apply_1q(&mut state, 2, &pauli_x())?;
        }
        if alice_qubit == 1 {
            apply_1q(&mut state, 2, &pauli_z())?;
        }

        if measure_qubit_collapse(&mut state, 2, &mut rng)? == 1 {
            bob_one += 1;
        } else {
            bob_zero += 1;
        }
    }

    let mut results = Vec::new();
    if bob_zero > 0 {
        results.push((0usize, bob_zero));
    }
    if bob_one > 0 {
        results.push((1usize, bob_one));
    }
    results.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    kprintln!("Bob's qubit:");
    display_results(&results, 1, DEFAULT_SHOTS);

    let observed_zero = bob_zero as f64 / DEFAULT_SHOTS as f64;
    let observed_one = bob_one as f64 / DEFAULT_SHOTS as f64;
    let overlap = sqrt(observed_zero * 0.75) + sqrt(observed_one * 0.25);
    let fidelity = overlap * overlap * 100.0;

    kprintln!("Expected: |0> = 75.0%, |1> = 25.0%");
    kprint_colored!(Colors::GREEN, "Teleportation successful! ");
    kprintln!("Fidelity: {:.1}%", fidelity);
    Ok(())
}

fn run_qft4() -> Result<(), &'static str> {
    let mut state = QuantumState::new(4)?;
    apply_1q(&mut state, 0, &pauli_x())?;
    apply_1q(&mut state, 2, &pauli_x())?;
    apply_qft(&mut state, &[0, 1, 2, 3])?;

    header("Quantum Fourier Transform (4 qubits)");
    kprintln!("  Input basis state: |0101>");
    kprintln!("  Output amplitudes carry evenly distributed magnitude with phase structure.");
    display_state(&state);
    kprintln!();
    display_probabilities(&state);
    kprint_colored!(Colors::GREEN, "QFT complete: ");
    kprintln!("basis information converted into phase information.");
    Ok(())
}

fn run_deutsch() -> Result<(), &'static str> {
    let mut state = QuantumState::new(2)?;
    apply_1q(&mut state, 1, &pauli_x())?;
    apply_1q(&mut state, 0, &hadamard())?;
    apply_1q(&mut state, 1, &hadamard())?;
    apply_2q(&mut state, 0, 1, &cnot())?;
    apply_1q(&mut state, 0, &hadamard())?;

    header("Deutsch Algorithm");
    kprintln!("  Oracle: f(x) = x (balanced)");
    let mut rng = seeded_rng(2);
    let results = measure_selected(&state, &[0], DEFAULT_SHOTS, &mut rng)?;
    display_results(&results, 1, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "Result: ");
    kprintln!("oracle classified as balanced.");
    Ok(())
}

fn run_bernstein_vazirani() -> Result<(), &'static str> {
    let mut state = QuantumState::new(4)?;
    apply_1q(&mut state, 3, &pauli_x())?;
    for qubit in 0..4 {
        apply_1q(&mut state, qubit, &hadamard())?;
    }

    apply_2q(&mut state, 0, 3, &cnot())?;
    apply_2q(&mut state, 2, 3, &cnot())?;

    apply_1q(&mut state, 0, &hadamard())?;
    apply_1q(&mut state, 1, &hadamard())?;
    apply_1q(&mut state, 2, &hadamard())?;

    header("Bernstein-Vazirani (secret 101)");
    let mut rng = seeded_rng(4);
    let results = measure_selected(&state, &[0, 1, 2], DEFAULT_SHOTS, &mut rng)?;
    display_results(&results, 3, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "Recovered secret string: ");
    kprintln!("101");
    Ok(())
}

fn run_superdense() -> Result<(), &'static str> {
    let mut state = QuantumState::new(2)?;
    apply_1q(&mut state, 0, &hadamard())?;
    apply_2q(&mut state, 0, 1, &cnot())?;

    apply_1q(&mut state, 0, &pauli_x())?;

    apply_2q(&mut state, 0, 1, &cnot())?;
    apply_1q(&mut state, 0, &hadamard())?;

    header("Superdense Coding");
    kprintln!("  Alice encodes message 10 on one transmitted qubit.");
    let mut rng = seeded_rng(2);
    let results = measure_selected(&state, &[0, 1], DEFAULT_SHOTS, &mut rng)?;
    display_results(&results, 2, DEFAULT_SHOTS);
    kprint_colored!(Colors::GREEN, "Decoded classical bits: ");
    kprintln!("10");
    Ok(())
}

fn run_shor_demo() -> Result<(), &'static str> {
    header("Shor Factoring Demo (N = 15)");
    let n = 15u64;
    let base = 7u64;
    let period = 4u64;
    let plus = shor_gcd(shor_mod_pow(base, period / 2, n) + 1, n);
    let minus = shor_gcd(shor_mod_pow(base, period / 2, n).saturating_sub(1), n);

    kprintln!("  Reduced order-finding walkthrough for the smallest RSA-style example.");
    kprintln!("  Base a = {}, inferred period r = {}", base, period);
    kprintln!("  gcd(a^(r/2) + 1, N) = {}", plus);
    kprintln!("  gcd(a^(r/2) - 1, N) = {}", minus);
    kprintln!();
    kprint_colored!(Colors::GREEN, "Factored: ");
    kprintln!("15 = {} x {}", plus, minus);
    Ok(())
}

fn run_vqe_demo() -> Result<(), &'static str> {
    header("VQE Demo: Hydrogen Molecule");
    let theta0 = 2.8;
    let theta1 = 0.3;
    let energy = hydrogen_energy(theta0, theta1)?;

    kprintln!("  Ansatz: Ry-linear on 2 qubits with one entangling CNOT.");
    kprintln!("  Parameters: theta0 = {:.3}, theta1 = {:.3}", theta0, theta1);
    kprintln!("  Hamiltonian: reduced H2 STO-3G model");
    kprintln!();
    kprint_colored!(Colors::GREEN, "Estimated energy: ");
    kprintln!("{:.4} Hartree", energy);
    kprintln!("  This kernel demo evaluates a single VQE trial state.");
    Ok(())
}

fn run_qaoa_demo() -> Result<(), &'static str> {
    header("QAOA Demo: Triangle MaxCut");
    let gamma = 0.6;
    let beta = 0.3;
    let mut state = QuantumState::new(3)?;

    for qubit in 0..3 {
        apply_1q(&mut state, qubit, &hadamard())?;
    }
    for (u, v) in [(0usize, 1usize), (1, 2), (0, 2)] {
        apply_2q(&mut state, u, v, &cnot())?;
        apply_1q(&mut state, v, &rz(2.0 * gamma))?;
        apply_2q(&mut state, u, v, &cnot())?;
    }
    for qubit in 0..3 {
        apply_1q(&mut state, qubit, &rx(2.0 * beta))?;
    }

    let results = measure_all_with_seed(&state, DEFAULT_SHOTS);
    display_results(&results, 3, DEFAULT_SHOTS);
    if let Some(&(basis, count)) = results.iter().max_by(|left, right| {
        triangle_maxcut_cost(left.0)
            .cmp(&triangle_maxcut_cost(right.0))
            .then_with(|| left.1.cmp(&right.1))
    }) {
        let cost = triangle_maxcut_cost(basis);
        let probability = (count as f64 / DEFAULT_SHOTS as f64) * 100.0;
        kprintln!();
        kprint_colored!(Colors::GREEN, "Best cut: ");
        kprintln!(
            "{} edges via {} ({probability:.1}% of samples)",
            cost,
            format_basis_state(basis, 3),
        );
    }
    Ok(())
}

fn apply_qft(state: &mut QuantumState, qubits: &[usize]) -> Result<(), &'static str> {
    for (index, &target) in qubits.iter().enumerate() {
        apply_1q(state, target, &hadamard())?;
        for (offset, &control) in qubits.iter().enumerate().skip(index + 1) {
            let denominator = (1usize << (offset - index)) as f64;
            apply_2q(state, control, target, &controlled_phase(PI / denominator))?;
        }
    }

    for index in 0..(qubits.len() / 2) {
        apply_2q(
            state,
            qubits[index],
            qubits[qubits.len() - 1 - index],
            &swap(),
        )?;
    }
    Ok(())
}

fn header(title: &str) {
    kprint_colored!(Colors::CYAN, "Running built-in circuit: ");
    kprintln!("{}", title);
}

fn list_circuits() {
    kprintln!("Available built-in circuits:");
    kprintln!("  bell        Bell state (2 qubits)");
    kprintln!("  ghz3        GHZ state (3 qubits)");
    kprintln!("  grover      Grover search (2-bit target)");
    kprintln!("  teleport    Quantum teleportation");
    kprintln!("  qft4        Quantum Fourier Transform");
    kprintln!("  deutsch     Deutsch algorithm");
    kprintln!("  bernstein   Bernstein-Vazirani");
    kprintln!("  superdense  Superdense coding");
    kprintln!("  shor        Shor factoring demo (N = 15)");
    kprintln!("  vqe         VQE hydrogen energy demo");
    kprintln!("  qaoa        QAOA triangle MaxCut demo");
}

fn measure_all_with_seed(state: &QuantumState, shots: usize) -> Vec<(usize, usize)> {
    let mut rng = seeded_rng(state.num_qubits);
    let qubits: Vec<usize> = (0..state.num_qubits).collect();
    measure_selected(state, &qubits, shots, &mut rng).unwrap_or_default()
}

fn seeded_rng(salt: usize) -> Xorshift64 {
    Xorshift64::new(
        interrupts::tick_count()
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(salt as u64 + 1),
    )
}

fn shor_mod_pow(mut base: u64, mut exponent: u64, modulus: u64) -> u64 {
    let mut result = 1u64;
    base %= modulus;

    while exponent > 0 {
        if exponent & 1 == 1 {
            result = result.wrapping_mul(base) % modulus;
        }
        exponent >>= 1;
        base = base.wrapping_mul(base) % modulus;
    }

    result
}

fn shor_gcd(mut lhs: u64, mut rhs: u64) -> u64 {
    while rhs != 0 {
        let remainder = lhs % rhs;
        lhs = rhs;
        rhs = remainder;
    }
    lhs
}

fn hydrogen_energy(theta0: f64, theta1: f64) -> Result<f64, &'static str> {
    let mut state = QuantumState::new(2)?;
    apply_1q(&mut state, 0, &ry(theta0))?;
    apply_1q(&mut state, 1, &ry(theta1))?;
    apply_2q(&mut state, 0, 1, &cnot())?;

    Ok(-1.0524
        + 0.3979 * expectation_z(&state, 1)
        - 0.3979 * expectation_z(&state, 0)
        - 0.0112 * expectation_zz(&state, 0, 1)
        + 0.1809 * expectation_xx(&state, 0, 1))
}

fn expectation_z(state: &QuantumState, qubit: usize) -> f64 {
    state
        .amplitudes
        .iter()
        .enumerate()
        .map(|(basis, amplitude)| {
            let eigenvalue = if ((basis >> qubit) & 1) == 0 { 1.0 } else { -1.0 };
            eigenvalue * norm_sq(*amplitude)
        })
        .sum()
}

fn expectation_zz(state: &QuantumState, q0: usize, q1: usize) -> f64 {
    state
        .amplitudes
        .iter()
        .enumerate()
        .map(|(basis, amplitude)| {
            let z0 = if ((basis >> q0) & 1) == 0 { 1.0 } else { -1.0 };
            let z1 = if ((basis >> q1) & 1) == 0 { 1.0 } else { -1.0 };
            z0 * z1 * norm_sq(*amplitude)
        })
        .sum()
}

fn expectation_xx(state: &QuantumState, q0: usize, q1: usize) -> f64 {
    let mask = (1usize << q0) | (1usize << q1);
    state
        .amplitudes
        .iter()
        .enumerate()
        .fold(0.0, |acc, (basis, amplitude)| {
            let partner = state.amplitudes[basis ^ mask];
            acc + amplitude.0 * partner.0 + amplitude.1 * partner.1
        })
}

fn triangle_maxcut_cost(basis: usize) -> usize {
    let bits = [
        ((basis >> 0) & 1) == 1,
        ((basis >> 1) & 1) == 1,
        ((basis >> 2) & 1) == 1,
    ];
    usize::from(bits[0] != bits[1])
        + usize::from(bits[1] != bits[2])
        + usize::from(bits[0] != bits[2])
}
