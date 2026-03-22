use std::f64::consts::{FRAC_1_SQRT_2, PI};

use waros_quantum::{Circuit, Complex, Simulator};

const TOLERANCE: f64 = 1e-10;

fn complex_close(actual: Complex, expected: Complex) {
    assert!((actual.re - expected.re).abs() < TOLERANCE);
    assert!((actual.im - expected.im).abs() < TOLERANCE);
}

fn statevector(circuit: &Circuit) -> Vec<Complex> {
    Simulator::new()
        .statevector(circuit)
        .expect("statevector simulation succeeds")
}

fn rx_state(theta: f64) -> Vec<Complex> {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.rx(0, theta).expect("valid gate");
    statevector(&circuit)
}

fn ry_state(theta: f64) -> Vec<Complex> {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.ry(0, theta).expect("valid gate");
    statevector(&circuit)
}

fn rz_state(theta: f64) -> Vec<Complex> {
    let mut circuit = Circuit::new(1).expect("valid circuit");
    circuit.x(0).expect("valid gate");
    circuit.rz(0, theta).expect("valid gate");
    statevector(&circuit)
}

macro_rules! rotation_test {
    ($name:ident, $state_fn:ident, $theta:expr, $expected_zero:expr, $expected_one:expr) => {
        #[test]
        fn $name() {
            let state = $state_fn($theta);
            complex_close(state[0], $expected_zero);
            complex_close(state[1], $expected_one);
        }
    };
}

rotation_test!(rx_zero, rx_state, 0.0, Complex::ONE, Complex::ZERO);
rotation_test!(
    rx_pi_over_2,
    rx_state,
    PI / 2.0,
    Complex::new(FRAC_1_SQRT_2, 0.0),
    Complex::new(0.0, -FRAC_1_SQRT_2)
);
rotation_test!(rx_pi, rx_state, PI, Complex::ZERO, Complex::new(0.0, -1.0));
rotation_test!(
    rx_three_pi_over_2,
    rx_state,
    3.0 * PI / 2.0,
    Complex::new(-FRAC_1_SQRT_2, 0.0),
    Complex::new(0.0, -FRAC_1_SQRT_2)
);
rotation_test!(
    rx_two_pi,
    rx_state,
    2.0 * PI,
    Complex::new(-1.0, 0.0),
    Complex::ZERO
);

rotation_test!(ry_zero, ry_state, 0.0, Complex::ONE, Complex::ZERO);
rotation_test!(
    ry_pi_over_2,
    ry_state,
    PI / 2.0,
    Complex::new(FRAC_1_SQRT_2, 0.0),
    Complex::new(FRAC_1_SQRT_2, 0.0)
);
rotation_test!(ry_pi, ry_state, PI, Complex::ZERO, Complex::ONE);
rotation_test!(
    ry_three_pi_over_2,
    ry_state,
    3.0 * PI / 2.0,
    Complex::new(-FRAC_1_SQRT_2, 0.0),
    Complex::new(FRAC_1_SQRT_2, 0.0)
);
rotation_test!(
    ry_two_pi,
    ry_state,
    2.0 * PI,
    Complex::new(-1.0, 0.0),
    Complex::ZERO
);

rotation_test!(rz_zero, rz_state, 0.0, Complex::ZERO, Complex::ONE);
rotation_test!(
    rz_pi_over_2,
    rz_state,
    PI / 2.0,
    Complex::ZERO,
    Complex::from_polar(1.0, PI / 4.0)
);
rotation_test!(rz_pi, rz_state, PI, Complex::ZERO, Complex::I);
rotation_test!(
    rz_three_pi_over_2,
    rz_state,
    3.0 * PI / 2.0,
    Complex::ZERO,
    Complex::from_polar(1.0, 3.0 * PI / 4.0)
);
rotation_test!(
    rz_two_pi,
    rz_state,
    2.0 * PI,
    Complex::ZERO,
    Complex::new(-1.0, 0.0)
);
