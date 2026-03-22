use std::f64::consts::PI;

use waros_quantum::{Circuit, Simulator, WarosError};

fn assert_within_three_sigma(observed_count: u32, shots: u32, expected_probability: f64) {
    let observed_probability = f64::from(observed_count) / f64::from(shots);
    let sigma = (expected_probability * (1.0 - expected_probability) / f64::from(shots)).sqrt();
    assert!(
        (observed_probability - expected_probability).abs() <= 3.0 * sigma + 1e-3,
        "observed={observed_probability} expected={expected_probability} sigma={sigma}"
    );
}

fn chi_square_two_outcome(count_0: u32, count_1: u32, p0: f64, p1: f64) -> f64 {
    let total = f64::from(count_0 + count_1);
    let expected_0 = total * p0;
    let expected_1 = total * p1;
    (f64::from(count_0) - expected_0).powi(2) / expected_0
        + (f64::from(count_1) - expected_1).powi(2) / expected_1
}

fn teleportation_circuit(theta: f64) -> Circuit {
    let mut circuit = Circuit::new(3).expect("valid circuit");
    circuit.ry(0, theta).expect("valid gate");
    circuit.h(1).expect("valid gate");
    circuit.cnot(1, 2).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.h(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");
    circuit.measure(1).expect("valid measurement");
    circuit.cnot(1, 2).expect("valid gate");
    circuit.cz(0, 2).expect("valid gate");
    circuit.measure(2).expect("valid measurement");
    circuit
}

#[test]
fn hadamard_statistics_match_half_distribution() {
    let shots = 10_000;
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::with_seed(7)
        .run(&circuit, shots)
        .expect("simulation succeeds");
    let zero_count = *result.counts().get("0").unwrap_or(&0);
    let one_count = *result.counts().get("1").unwrap_or(&0);

    assert_within_three_sigma(zero_count, shots, 0.5);
    assert_within_three_sigma(one_count, shots, 0.5);
}

#[test]
fn bell_state_statistics_match_expected_distribution() {
    let shots = 10_000;
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.measure_all().expect("valid measurements");

    let result = Simulator::with_seed(11)
        .run(&circuit, shots)
        .expect("simulation succeeds");
    let count_00 = *result.counts().get("00").unwrap_or(&0);
    let count_11 = *result.counts().get("11").unwrap_or(&0);

    assert_within_three_sigma(count_00, shots, 0.5);
    assert_within_three_sigma(count_11, shots, 0.5);
    assert!(result.probability("01").abs() < f64::EPSILON);
    assert!(result.probability("10").abs() < f64::EPSILON);
}

#[test]
fn teleportation_fidelity_passes_chi_squared_check() {
    let shots = 10_000;
    let theta = PI / 3.0;
    let expected_p0 = (theta / 2.0).cos().powi(2);
    let expected_p1 = (theta / 2.0).sin().powi(2);

    let result = Simulator::with_seed(23)
        .run(&teleportation_circuit(theta), shots)
        .expect("simulation succeeds");

    let mut bob_0 = 0u32;
    let mut bob_1 = 0u32;
    for (bits, count) in result.counts() {
        match bits.as_bytes().get(2) {
            Some(b'0') => bob_0 += count,
            Some(b'1') => bob_1 += count,
            _ => {}
        }
    }

    let chi_square = chi_square_two_outcome(bob_0, bob_1, expected_p0, expected_p1);
    assert!(
        chi_square < 10.83,
        "teleportation chi-square too large: {chi_square}"
    );
}

#[test]
fn histogram_is_sorted_by_descending_count() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::with_seed(5)
        .run(&circuit, 1_000)
        .expect("simulation succeeds");
    let histogram = result.histogram();

    for pair in histogram.windows(2) {
        assert!(pair[0].1 >= pair[1].1);
    }
}

#[test]
fn most_probable_returns_the_only_measured_state() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::new()
        .run(&circuit, 128)
        .expect("simulation succeeds");
    assert_eq!(result.most_probable(), ("1", 128));
}

#[test]
fn expectation_z_matches_basis_state() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::new()
        .run(&circuit, 256)
        .expect("simulation succeeds");
    assert!((result.expectation_z(0).expect("valid qubit") + 1.0).abs() < f64::EPSILON);
}

#[test]
fn expectation_z_rejects_out_of_range_qubits() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::new()
        .run(&circuit, 16)
        .expect("simulation succeeds");
    let error = result
        .expectation_z(1)
        .expect_err("out-of-range qubit must fail");
    assert_eq!(error, WarosError::QubitOutOfRange(1, 1));
}

#[test]
fn probability_for_missing_state_is_zero() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.measure(0).expect("valid measurement");

    let result = Simulator::new()
        .run(&circuit, 8)
        .expect("simulation succeeds");
    assert!(result.probability("1").abs() < f64::EPSILON);
}

#[test]
fn same_seed_reproduces_mid_circuit_measurement_results() {
    let circuit = teleportation_circuit(PI / 5.0);

    let first = Simulator::with_seed(29)
        .run(&circuit, 2_000)
        .expect("simulation succeeds");
    let second = Simulator::with_seed(29)
        .run(&circuit, 2_000)
        .expect("simulation succeeds");
    assert_eq!(first.counts(), second.counts());
}

#[test]
fn zero_shot_runs_are_rejected() {
    let circuit = Circuit::new(1).expect("valid circuit");
    let error = Simulator::new()
        .run(&circuit, 0)
        .expect_err("zero shots must fail");
    assert_eq!(error, WarosError::InvalidShots(0));
}

#[test]
fn thirty_qubit_circuit_is_allowed() {
    let circuit = Circuit::new(30).expect("30 qubits is supported");
    assert_eq!(circuit.num_qubits(), 30);
}

#[test]
fn thirty_one_qubits_are_rejected() {
    let error = Circuit::new(31).expect_err("31 qubits exceeds the limit");
    assert_eq!(error, WarosError::TooManyQubits(31, 30));
}
