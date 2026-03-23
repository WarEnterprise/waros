use std::f64::consts::PI;

use waros_quantum::{parse_qasm, to_qasm, QasmError, Simulator};

#[test]
fn parse_empty_circuit() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[2];
        creg c[2];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    assert_eq!(circuit.num_qubits(), 2);
    assert_eq!(circuit.gate_count(), 0);
    assert_eq!(circuit.num_classical_bits(), 2);
}

#[test]
fn parse_bell_state_circuit() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[2];
        creg c[2];
        h q[0];
        cx q[0], q[1];
        measure q[0] -> c[0];
        measure q[1] -> c[1];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    assert_eq!(circuit.gate_count(), 2);
    assert_eq!(circuit.num_classical_bits(), 2);
}

#[test]
fn parse_ghz_state() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[3];
        creg c[3];
        h q[0];
        cx q[0], q[1];
        cx q[1], q[2];
        barrier q;
        measure q[0] -> c[0];
        measure q[1] -> c[1];
        measure q[2] -> c[2];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    assert_eq!(circuit.gate_count(), 3);
}

#[test]
fn parse_circuit_with_supported_gates() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[3];
        h q[0];
        x q[1];
        y q[2];
        z q[0];
        s q[1];
        sdg q[1];
        t q[2];
        tdg q[2];
        rx(pi/2) q[0];
        ry(0.75) q[1];
        rz(-pi/3) q[2];
        u3(pi/2, pi/4, -pi/8) q[0];
        cx q[0], q[1];
        cz q[1], q[2];
        swap q[0], q[2];
        ccx q[0], q[1], q[2];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    assert!(circuit.gate_count() >= 15);
}

#[test]
fn parse_parameterized_gates() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        rx(pi/2) q[0];
        ry(3*pi/2) q[0];
        rz(-pi/4) q[0];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let state = Simulator::new()
        .statevector(&circuit)
        .expect("statevector succeeds");
    let probability: f64 = state.iter().map(|amplitude| amplitude.norm_sq()).sum();
    assert!((probability - 1.0).abs() < 1e-10);
}

#[test]
fn round_trip_preserves_gate_count() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[2];
        creg c[2];
        h q[0];
        cx q[0], q[1];
        measure q[0] -> c[0];
        measure q[1] -> c[1];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let serialized = to_qasm(&circuit);
    let round_tripped = parse_qasm(&serialized).expect("serialized qasm parses");
    assert_eq!(round_tripped.gate_count(), circuit.gate_count());
}

#[test]
fn error_unknown_gate_name() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        foo q[0];
    "#;
    let error = parse_qasm(source).expect_err("unknown gate must fail");
    assert_eq!(error, QasmError::UnknownGate("foo".into()));
}

#[test]
fn error_undeclared_register() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        h r[0];
    "#;
    let error = parse_qasm(source).expect_err("undeclared register must fail");
    assert_eq!(error, QasmError::UndeclaredRegister("r".into()));
}

#[test]
fn error_qubit_index_out_of_range() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        x q[3];
    "#;
    let error = parse_qasm(source).expect_err("out-of-range index must fail");
    assert_eq!(
        error,
        QasmError::QubitOutOfRange {
            register: "q".into(),
            index: 3,
            size: 1,
        }
    );
}

#[test]
fn error_missing_semicolon() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1]
        h q[0];
    "#;
    let error = parse_qasm(source).expect_err("missing semicolon must fail");
    assert!(matches!(error, QasmError::ParseError { .. }));
}

#[test]
fn error_malformed_header() {
    let source = r"
        OPENQASM 3.0;
        qreg q[1];
    ";
    let error = parse_qasm(source).expect_err("malformed header must fail");
    assert!(matches!(error, QasmError::ParseError { .. }));
}

#[test]
fn parse_and_execute_qasm() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[2];
        creg c[2];
        h q[0];
        cx q[0], q[1];
        measure q[0] -> c[0];
        measure q[1] -> c[1];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let result = Simulator::with_seed(101)
        .run(&circuit, 10_000)
        .expect("simulation succeeds");
    assert!((result.probability("00") - 0.5).abs() < 0.03);
    assert!((result.probability("11") - 0.5).abs() < 0.03);
}

#[test]
fn parse_multiple_registers() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg a[1];
        qreg b[1];
        creg c[2];
        x a[0];
        cx a[0], b[0];
        measure a[0] -> c[0];
        measure b[0] -> c[1];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let result = Simulator::with_seed(109)
        .run(&circuit, 100)
        .expect("simulation succeeds");
    assert!((result.probability("11") - 1.0).abs() < f64::EPSILON);
}

#[test]
fn qasm_expression_parser_handles_nested_terms() {
    let source = format!(
        r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        rz((pi + pi/2) / 3) q[0];
        rx({}) q[0];
    "#,
        PI / 7.0
    );
    let circuit = parse_qasm(&source).expect("qasm parses");
    assert_eq!(circuit.gate_count(), 2);
}

#[test]
fn parse_qiskit_style_u_gates_and_identity() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        u1(pi/2) q[0];
        u2(0, pi) q[0];
        id q[0];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    assert_eq!(circuit.gate_count(), 2);
}

#[test]
fn parse_custom_gate_definition() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        gate myh q { u2(0, pi) q; }
        qreg q[1];
        creg c[1];
        myh q[0];
        measure q[0] -> c[0];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let result = Simulator::with_seed(17)
        .run(&circuit, 10_000)
        .expect("simulation succeeds");
    assert!((result.probability("0") - 0.5).abs() < 0.05);
    assert!((result.probability("1") - 0.5).abs() < 0.05);
}

#[test]
fn parse_conditional_execution() {
    let source = r#"
        OPENQASM 2.0;
        include "qelib1.inc";
        qreg q[1];
        creg c[1];
        x q[0];
        measure q[0] -> c[0];
        if(c==1) x q[0];
        measure q[0] -> c[0];
    "#;
    let circuit = parse_qasm(source).expect("qasm parses");
    let result = Simulator::with_seed(23)
        .run(&circuit, 256)
        .expect("simulation succeeds");
    assert!((result.probability("0") - 1.0).abs() < 1e-10);
}
