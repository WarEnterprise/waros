use std::f64::consts::{FRAC_1_SQRT_2, PI};

use waros_quantum::{gate, Circuit, Complex, Simulator};

const TOLERANCE: f64 = 1e-10;

fn complex_close(actual: Complex, expected: Complex) {
    assert!(
        (actual.re - expected.re).abs() < TOLERANCE,
        "real parts differ: actual={} expected={}",
        actual.re,
        expected.re
    );
    assert!(
        (actual.im - expected.im).abs() < TOLERANCE,
        "imaginary parts differ: actual={} expected={}",
        actual.im,
        expected.im
    );
}

fn basis_index(bits: &[u8]) -> usize {
    bits.iter().enumerate().fold(0usize, |index, (qubit, bit)| {
        index | ((*bit as usize) << qubit)
    })
}

fn assert_basis_state(state: &[Complex], bits: &[u8], amplitude: Complex) {
    let expected_index = basis_index(bits);
    for (index, entry) in state.iter().enumerate() {
        if index == expected_index {
            complex_close(*entry, amplitude);
        } else {
            complex_close(*entry, Complex::ZERO);
        }
    }
}

fn statevector(circuit: &Circuit) -> Vec<Complex> {
    Simulator::new()
        .statevector(circuit)
        .expect("statevector simulation succeeds")
}

#[test]
fn x_maps_zero_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1], Complex::ONE);
}

#[test]
fn y_maps_zero_to_i_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.y(0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1], Complex::I);
}

#[test]
fn z_adds_phase_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.z(0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1], Complex::new(-1.0, 0.0));
}

#[test]
fn h_creates_plus_state() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    let state = statevector(&circuit);
    complex_close(state[0], Complex::new(FRAC_1_SQRT_2, 0.0));
    complex_close(state[1], Complex::new(FRAC_1_SQRT_2, 0.0));
}

#[test]
fn s_adds_quarter_phase_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.s(0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1], Complex::I);
}

#[test]
fn sdg_adds_negative_quarter_phase_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.sdg(0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1], Complex::new(0.0, -1.0));
}

#[test]
fn t_adds_eighth_phase_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.t(0).expect("valid gate");
    assert_basis_state(
        &statevector(&circuit),
        &[1],
        Complex::from_polar(1.0, PI / 4.0),
    );
}

#[test]
fn tdg_adds_negative_eighth_phase_to_one() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.tdg(0).expect("valid gate");
    assert_basis_state(
        &statevector(&circuit),
        &[1],
        Complex::from_polar(1.0, -PI / 4.0),
    );
}

#[test]
fn sx_squared_matches_x() {
    let mut square_root = Circuit::new(1).expect("valid circuit");
    square_root.sx(0).expect("valid gate");
    square_root.sx(0).expect("valid gate");

    let mut pauli_x = Circuit::new(1).expect("valid circuit");
    pauli_x.x(0).expect("valid gate");

    let left = statevector(&square_root);
    let right = statevector(&pauli_x);
    complex_close(left[0], right[0]);
    complex_close(left[1], right[1]);
}

#[test]
fn u3_rotates_zero_state_as_expected() {
    let theta = 1.2;
    let phi = -0.7;
    let lambda = 0.9;

    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.u3(0, theta, phi, lambda).expect("valid gate");

    let state = statevector(&circuit);
    complex_close(state[0], Complex::new((theta / 2.0).cos(), 0.0));
    complex_close(state[1], Complex::from_polar((theta / 2.0).sin(), phi));
}

#[test]
fn cnot_control_0_target_1_maps_to_11() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1, 1], Complex::ONE);
}

#[test]
fn cnot_control_1_target_0_maps_to_11() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(1).expect("valid gate");
    circuit.cnot(1, 0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1, 1], Complex::ONE);
}

#[test]
fn cz_order_0_1_adds_phase_to_11() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.h(1).expect("valid gate");
    circuit.cz(0, 1).expect("valid gate");

    let state = statevector(&circuit);
    for amplitude in &state[..3] {
        complex_close(*amplitude, Complex::new(0.5, 0.0));
    }
    complex_close(state[3], Complex::new(-0.5, 0.0));
}

#[test]
fn cz_order_1_0_adds_phase_to_11() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.h(1).expect("valid gate");
    circuit.cz(1, 0).expect("valid gate");

    let state = statevector(&circuit);
    for amplitude in &state[..3] {
        complex_close(*amplitude, Complex::new(0.5, 0.0));
    }
    complex_close(state[3], Complex::new(-0.5, 0.0));
}

#[test]
fn swap_0_1_moves_excitation() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.swap(0, 1).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[0, 1], Complex::ONE);
}

#[test]
fn swap_1_0_moves_excitation() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(1).expect("valid gate");
    circuit.swap(1, 0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1, 0], Complex::ONE);
}

#[test]
fn cy_order_0_1_applies_y_to_target() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.cy(0, 1).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1, 1], Complex::I);
}

#[test]
fn cy_order_1_0_applies_y_to_target() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(1).expect("valid gate");
    circuit.cy(1, 0).expect("valid gate");
    assert_basis_state(&statevector(&circuit), &[1, 1], Complex::I);
}

#[test]
fn rzz_adds_expected_phase_to_11() {
    let theta = PI / 3.0;
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.x(1).expect("valid gate");
    circuit.rzz(0, 1, theta).expect("valid gate");
    assert_basis_state(
        &statevector(&circuit),
        &[1, 1],
        Complex::from_polar(1.0, -theta / 2.0),
    );
}

#[test]
fn display_reports_qubits_gates_and_measurements() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.measure_all().expect("valid measurements");
    assert_eq!(
        format!("{circuit}"),
        "Circuit(2 qubits, 2 gates, 2 measurements)"
    );
}

#[test]
fn measure_returns_incrementing_classical_bits() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    let first = circuit.measure(1).expect("valid measurement");
    let second = circuit.measure(0).expect("valid measurement");
    assert_eq!(first, 0);
    assert_eq!(second, 1);
}

#[test]
fn statevector_ignores_terminal_measurements() {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.measure(0).expect("valid measurement");
    let state = statevector(&circuit);
    complex_close(state[0], Complex::new(FRAC_1_SQRT_2, 0.0));
    complex_close(state[1], Complex::new(FRAC_1_SQRT_2, 0.0));
}

macro_rules! toffoli_case {
    ($name:ident, [$b0:expr, $b1:expr, $b2:expr], [$e0:expr, $e1:expr, $e2:expr]) => {
        #[test]
        fn $name() {
            let mut circuit = Circuit::new(3).expect("valid circuit");
            if $b0 == 1 {
                circuit.x(0).expect("valid gate");
            }
            if $b1 == 1 {
                circuit.x(1).expect("valid gate");
            }
            if $b2 == 1 {
                circuit.x(2).expect("valid gate");
            }
            circuit.toffoli(0, 1, 2).expect("valid gate");
            assert_basis_state(&statevector(&circuit), &[$e0, $e1, $e2], Complex::ONE);
        }
    };
}

toffoli_case!(toffoli_preserves_000, [0, 0, 0], [0, 0, 0]);
toffoli_case!(toffoli_preserves_001, [0, 0, 1], [0, 0, 1]);
toffoli_case!(toffoli_preserves_010, [0, 1, 0], [0, 1, 0]);
toffoli_case!(toffoli_preserves_011, [0, 1, 1], [0, 1, 1]);
toffoli_case!(toffoli_preserves_100, [1, 0, 0], [1, 0, 0]);
toffoli_case!(toffoli_preserves_101, [1, 0, 1], [1, 0, 1]);
toffoli_case!(toffoli_flips_110_to_111, [1, 1, 0], [1, 1, 1]);
toffoli_case!(toffoli_flips_111_to_110, [1, 1, 1], [1, 1, 0]);

#[test]
fn gate_inverse_matches_known_adjoint() {
    assert_eq!(gate::t().inverse().matrix, gate::tdg().matrix);
    assert_eq!(gate::s().dagger().matrix, gate::sdg().matrix);
}

#[test]
fn crk_k1_matches_controlled_z() {
    let mut crk_circuit = Circuit::new(2).expect("valid circuit");
    crk_circuit.h(0).expect("valid gate");
    crk_circuit.h(1).expect("valid gate");
    crk_circuit.crk(0, 1, 1).expect("valid gate");

    let mut cz_circuit = Circuit::new(2).expect("valid circuit");
    cz_circuit.h(0).expect("valid gate");
    cz_circuit.h(1).expect("valid gate");
    cz_circuit.cz(0, 1).expect("valid gate");

    let left = statevector(&crk_circuit);
    let right = statevector(&cz_circuit);
    for (left_amplitude, right_amplitude) in left.into_iter().zip(right) {
        complex_close(left_amplitude, right_amplitude);
    }
}

#[test]
fn qft_followed_by_iqft_restores_basis_state() {
    let mut circuit = Circuit::new(3).expect("valid circuit");
    circuit.x(1).expect("valid gate");
    circuit.qft(&[0, 1, 2]).expect("valid qft");
    circuit.iqft(&[0, 1, 2]).expect("valid iqft");
    assert_basis_state(&statevector(&circuit), &[0, 1, 0], Complex::ONE);
}

#[test]
fn depth_counts_parallel_single_qubit_layers() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.h(1).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    assert_eq!(circuit.depth(), 2);
}

#[test]
fn append_combines_instruction_streams() {
    let mut left = Circuit::new(2).expect("valid circuit");
    left.h(0).expect("valid gate");

    let mut right = Circuit::new(2).expect("valid circuit");
    right.cnot(0, 1).expect("valid gate");
    right.measure_all().expect("valid measurements");

    left.append(&right).expect("append succeeds");
    assert_eq!(left.gate_count(), 2);
    assert_eq!(left.num_classical_bits(), 2);
}

#[test]
fn compose_remaps_qubits() {
    let mut base = Circuit::new(3).expect("valid circuit");
    base.x(1).expect("valid gate");

    let mut other = Circuit::new(2).expect("valid circuit");
    other.cnot(0, 1).expect("valid gate");

    base.compose(&other, &[1, 2]).expect("compose succeeds");
    assert_basis_state(&statevector(&base), &[0, 1, 1], Complex::ONE);
}

#[test]
fn to_ascii_renders_controls_targets_and_measurements() {
    let mut circuit = Circuit::new(2).expect("valid circuit");
    circuit.h(0).expect("valid gate");
    circuit.cnot(0, 1).expect("valid gate");
    circuit.measure_all().expect("valid measurements");

    let ascii = circuit.to_ascii();
    assert!(ascii.contains("q0:"));
    assert!(ascii.contains("q1:"));
    assert!(ascii.contains('@') || ascii.contains('●'));
    assert!(ascii.contains('X') || ascii.contains('⊕'));
    assert!(ascii.contains("[M]"));
}
