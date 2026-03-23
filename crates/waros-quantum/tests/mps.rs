use waros_quantum::{Backend, Circuit, Simulator};

#[test]
fn mps_bell_state_matches_expected_distribution() {
    let simulator = Simulator::builder()
        .backend(Backend::MPS { max_bond_dim: 64 })
        .seed(42)
        .build();

    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.measure_all().expect("valid measurement");

    let result = simulator
        .run(&circuit, 10_000)
        .expect("simulation succeeds");
    assert!((result.probability("00") - 0.5).abs() < 0.05);
    assert!((result.probability("11") - 0.5).abs() < 0.05);
}

#[test]
fn mps_matches_statevector_for_small_ghz_circuit() {
    let statevector_simulator = Simulator::builder()
        .backend(Backend::StateVector)
        .seed(7)
        .build();
    let mps_simulator = Simulator::builder()
        .backend(Backend::MPS { max_bond_dim: 128 })
        .seed(7)
        .build();

    let mut circuit = Circuit::new(5).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    for qubit in 0..4 {
        circuit.cnot(qubit, qubit + 1).expect("valid gate");
    }
    circuit.measure_all().expect("valid measurement");

    let statevector_result = statevector_simulator
        .run(&circuit, 10_000)
        .expect("statevector succeeds");
    let mps_result = mps_simulator.run(&circuit, 10_000).expect("mps succeeds");

    assert!(
        (statevector_result.probability("00000") - mps_result.probability("00000")).abs() < 0.05
    );
    assert!(
        (statevector_result.probability("11111") - mps_result.probability("11111")).abs() < 0.05
    );
}

#[test]
fn mps_handles_large_low_entanglement_circuits() {
    let simulator = Simulator::builder()
        .backend(Backend::MPS { max_bond_dim: 16 })
        .seed(19)
        .build();

    let mut circuit = Circuit::new(40).expect("valid circuit");
    for qubit in 0..40 {
        circuit.h(qubit).expect("valid gate");
    }
    circuit.measure_all().expect("valid measurement");

    let result = simulator.run(&circuit, 100).expect("simulation succeeds");
    assert_eq!(result.total_shots(), 100);
}

#[test]
fn auto_backend_uses_mps_for_large_circuits() {
    let simulator = Simulator::builder().backend(Backend::Auto).seed(29).build();

    let mut circuit = Circuit::new(40).expect("valid circuit");
    for qubit in 0..40 {
        circuit.h(qubit).expect("valid gate");
    }
    circuit.measure_all().expect("valid measurement");

    let result = simulator.run(&circuit, 64).expect("simulation succeeds");
    assert_eq!(result.total_shots(), 64);
}
