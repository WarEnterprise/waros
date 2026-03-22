use waros_quantum::algorithms::{
    apply_hidden_xor_oracle, classical_maxcut, continued_fraction_period, evaluate_expectation,
    maxcut_cost, qaoa_maxcut, quantum_phase_estimation, quantum_random_walk, shor_factor,
    simon_algorithm, solve_gf2, vqe, AnsatzType, Graph, Hamiltonian, Pauli, PauliTerm,
};
use waros_quantum::gate::Gate;
use waros_quantum::{gate, Circuit, Complex, Simulator};

#[test]
fn qpe_t_gate_phase() {
    let simulator = Simulator::with_seed(42);
    let result = quantum_phase_estimation(
        &gate::t(),
        |circuit| {
            circuit.x(3).unwrap();
        },
        3,
        1_024,
        &simulator,
    )
    .expect("QPE succeeds");

    assert!((result.phase - 0.125).abs() < 0.01);
}

#[test]
fn qpe_s_gate_phase() {
    let simulator = Simulator::with_seed(42);
    let result = quantum_phase_estimation(
        &gate::s(),
        |circuit| {
            circuit.x(2).unwrap();
        },
        2,
        1_024,
        &simulator,
    )
    .expect("QPE succeeds");

    assert!((result.phase - 0.25).abs() < 0.01);
}

#[test]
fn qpe_z_gate_phase() {
    let simulator = Simulator::with_seed(42);
    let result = quantum_phase_estimation(
        &gate::z(),
        |circuit| {
            circuit.x(2).unwrap();
        },
        2,
        1_024,
        &simulator,
    )
    .expect("QPE succeeds");

    assert!((result.phase - 0.5).abs() < 0.01);
}

#[test]
fn qpe_identity_phase() {
    let simulator = Simulator::with_seed(42);
    let identity = Gate::single(
        "I",
        [[Complex::ONE, Complex::ZERO], [Complex::ZERO, Complex::ONE]],
    );
    let result = quantum_phase_estimation(&identity, |_circuit| {}, 3, 512, &simulator)
        .expect("QPE succeeds");

    assert!(result.phase.abs() < 0.001);
}

#[test]
fn qpe_counts_track_integer_outcomes() {
    let simulator = Simulator::with_seed(7);
    let result = quantum_phase_estimation(
        &gate::s(),
        |circuit| {
            circuit.x(3).unwrap();
        },
        3,
        256,
        &simulator,
    )
    .expect("QPE succeeds");

    assert!(result.counts.values().sum::<u32>() == 256);
}

#[test]
fn shor_factor_15() {
    let simulator = Simulator::with_seed(42);
    let result = shor_factor(15, &simulator).expect("Shor succeeds");

    assert!(result.success);
    assert_eq!(result.factors.0 * result.factors.1, 15);
    assert!(matches!(result.factors.0, 3 | 5));
}

#[test]
fn shor_factor_21() {
    let simulator = Simulator::with_seed(42);
    let result = shor_factor(21, &simulator).expect("Shor completes");

    if result.success {
        assert_eq!(result.factors.0 * result.factors.1, 21);
    } else {
        assert_eq!(result.factors, (1, 21));
    }
}

#[test]
fn shor_mod_pow_correctness() {
    assert_eq!(waros_quantum::mod_pow(7, 2, 15), 4);
    assert_eq!(waros_quantum::mod_pow(2, 10, 1_000), 24);
}

#[test]
fn shor_gcd_correctness() {
    assert_eq!(waros_quantum::gcd(15, 6), 3);
    assert_eq!(waros_quantum::gcd(17, 13), 1);
    assert_eq!(waros_quantum::gcd(100, 75), 25);
}

#[test]
fn shor_continued_fraction_recovers_quarter() {
    assert_eq!(continued_fraction_period(0.25, 32), 4);
}

#[test]
fn shor_even_number_short_circuits() {
    let simulator = Simulator::with_seed(42);
    let result = shor_factor(12, &simulator).expect("Shor completes");
    assert!(result.success);
    assert_eq!(result.factors.0, 2);
}

#[test]
fn shor_prime_power_short_circuits() {
    let simulator = Simulator::with_seed(42);
    let result = shor_factor(9, &simulator).expect("Shor completes");
    assert!(result.success);
    assert_eq!(result.factors, (3, 3));
}

#[test]
fn vqe_hydrogen_ground_state_is_reasonable() {
    let simulator = Simulator::with_seed(42);
    let hydrogen = Hamiltonian::hydrogen_molecule();
    let result = vqe(
        &hydrogen,
        AnsatzType::RyLinear,
        &[0.0, 0.0],
        40,
        1_000,
        &simulator,
    )
    .expect("VQE succeeds");

    assert!(result.energy < -1.0);
    assert!(result.energy > -1.3);
}

#[test]
fn vqe_ising_energy_is_negative() {
    let simulator = Simulator::with_seed(42);
    let ising = Hamiltonian::ising_chain(2, 1.0, 0.5);
    let result = vqe(
        &ising,
        AnsatzType::HardwareEfficient { layers: 2 },
        &[0.0; 8],
        20,
        1_000,
        &simulator,
    )
    .expect("VQE succeeds");

    assert!(result.energy < 0.0);
}

#[test]
fn vqe_energy_decreases_from_initial_guess() {
    let simulator = Simulator::with_seed(42);
    let hydrogen = Hamiltonian::hydrogen_molecule();
    let result = vqe(
        &hydrogen,
        AnsatzType::RyLinear,
        &[0.0, 0.0],
        20,
        1_000,
        &simulator,
    )
    .expect("VQE succeeds");

    assert!(result.energy <= result.energy_history[0]);
}

#[test]
fn vqe_hamiltonian_construction_is_correct() {
    let hydrogen = Hamiltonian::hydrogen_molecule();
    assert_eq!(hydrogen.num_qubits, 2);
    assert_eq!(hydrogen.terms.len(), 5);
}

#[test]
fn vqe_pauli_expectation_for_z_on_zero_is_one() {
    let simulator = Simulator::with_seed(42);
    let hamiltonian = Hamiltonian {
        num_qubits: 1,
        terms: vec![PauliTerm {
            coefficient: 1.0,
            paulis: vec![(0, Pauli::Z)],
        }],
    };

    let energy = evaluate_expectation(&hamiltonian, &AnsatzType::RyLinear, &[0.0], 256, &simulator)
        .expect("expectation succeeds");
    assert!((energy - 1.0).abs() < 1e-10);
}

#[test]
fn qaoa_triangle_maxcut_optimal_value() {
    assert_eq!(classical_maxcut(&Graph::triangle()), 2.0);
}

#[test]
fn qaoa_square_maxcut_optimal_value() {
    assert_eq!(classical_maxcut(&Graph::square()), 4.0);
}

#[test]
fn qaoa_cost_function_scores_cut_edges() {
    let graph = Graph::triangle();
    let assignment = vec![false, true, false];
    assert_eq!(maxcut_cost(&graph, &assignment), 2.0);
}

#[test]
fn qaoa_returns_parameter_vectors_of_expected_depth() {
    let simulator = Simulator::with_seed(42);
    let result = qaoa_maxcut(&Graph::triangle(), 2, 5, 256, &simulator).expect("QAOA succeeds");
    assert_eq!(result.optimal_gamma.len(), 2);
    assert_eq!(result.optimal_beta.len(), 2);
}

#[test]
fn qaoa_approximation_ratio_is_nontrivial() {
    let simulator = Simulator::with_seed(42);
    let result = qaoa_maxcut(&Graph::square(), 2, 10, 256, &simulator).expect("QAOA succeeds");
    assert!(result.approximation_ratio.expect("ratio present") > 0.5);
}

#[test]
fn simon_recovers_known_secret() {
    let simulator = Simulator::with_seed(42);
    let secret = vec![false, true, true];
    let result = simon_algorithm(
        |circuit: &mut Circuit, input_start, output_start| {
            apply_hidden_xor_oracle(circuit, input_start, output_start, &secret)
        },
        3,
        &simulator,
    )
    .expect("Simon succeeds");

    assert_eq!(result.secret, secret);
}

#[test]
fn simon_gf2_solver_finds_hidden_xor() {
    let equations = vec![vec![true, false, false], vec![false, true, true]];
    let solution = solve_gf2(&equations, 3);
    assert_eq!(solution, vec![false, true, true]);
}

#[test]
fn random_walk_probabilities_are_normalized() {
    let result = quantum_random_walk(4);
    let total_probability: f64 = result.probabilities.iter().sum();
    assert!((total_probability - 1.0).abs() < 1e-10);
}

#[test]
fn random_walk_distribution_is_symmetric() {
    let result = quantum_random_walk(4);
    for offset in 0..result.positions.len() / 2 {
        let left = result.probabilities[offset];
        let right = result.probabilities[result.probabilities.len() - 1 - offset];
        assert!((left - right).abs() < 1e-10);
    }
}
