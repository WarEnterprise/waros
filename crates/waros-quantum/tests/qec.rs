use waros_quantum::{Circuit, QECCode, RepetitionCode, Simulator, SteaneCode};

#[test]
fn repetition_code_reports_expected_metadata() {
    let code = RepetitionCode::default();
    assert_eq!(code.physical_qubits(), 3);
    assert_eq!(code.logical_qubits(), 1);
    assert_eq!(code.distance(), 3);
}

#[test]
fn repetition_code_corrects_single_bit_flip() {
    let code = RepetitionCode::default();
    let mut circuit = Circuit::new(3).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    code.encode(&mut circuit, 0).expect("encoding succeeds");
    circuit.x(1).expect("inject error");
    code.correct(&mut circuit, &[1, 1])
        .expect("correction succeeds");
    for qubit in 0..3 {
        circuit.measure(qubit).expect("measurement succeeds");
    }

    let result = Simulator::with_seed(31)
        .run(&circuit, 256)
        .expect("simulation succeeds");
    assert!((result.probability("111") - 1.0).abs() < 1e-10);
}

#[test]
fn repetition_code_syndrome_uses_ancilla_qubits() {
    let code = RepetitionCode::default();
    let mut circuit = Circuit::new(5).expect("valid circuit");
    let syndrome = code
        .measure_syndrome(&mut circuit)
        .expect("syndrome extraction succeeds");
    assert_eq!(syndrome.len(), 2);
    assert_eq!(circuit.num_classical_bits(), 2);
}

#[test]
fn steane_code_reports_expected_metadata() {
    let code = SteaneCode;
    assert_eq!(code.physical_qubits(), 7);
    assert_eq!(code.logical_qubits(), 1);
    assert_eq!(code.distance(), 3);
}

#[test]
fn steane_code_builds_encoding_and_syndrome_circuits() {
    let code = SteaneCode;
    let mut circuit = Circuit::new(13).expect("valid circuit");
    code.encode(&mut circuit, 0).expect("encoding succeeds");
    let gate_count_after_encode = circuit.gate_count();
    let syndrome = code
        .measure_syndrome(&mut circuit)
        .expect("syndrome extraction succeeds");

    assert!(gate_count_after_encode > 0);
    assert_eq!(syndrome.len(), 6);
    assert_eq!(circuit.num_classical_bits(), 6);
}

#[test]
fn steane_code_applies_corrections_from_syndrome() {
    let code = SteaneCode;
    let mut circuit = Circuit::new(7).expect("valid circuit");
    code.correct(&mut circuit, &[1, 1, 1, 1, 0, 0])
        .expect("correction succeeds");
    assert_eq!(circuit.gate_count(), 2);
}
