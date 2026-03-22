use std::f64::consts::PI;

use waros_quantum::gate::{self, Gate};
use waros_quantum::Complex;

const TOLERANCE: f64 = 1e-10;

fn assert_gate_is_unitary(gate: &Gate) {
    let dim = 1usize << gate.num_qubits;
    for row in 0..dim {
        for col in 0..dim {
            let mut value = Complex::ZERO;
            for k in 0..dim {
                value += gate.get(k, row).conj() * gate.get(k, col);
            }

            let expected = if row == col { 1.0 } else { 0.0 };
            assert!(
                (value.re - expected).abs() < TOLERANCE,
                "{} is not unitary at ({row}, {col}): real={} expected={expected}",
                gate.name,
                value.re
            );
            assert!(
                value.im.abs() < TOLERANCE,
                "{} is not unitary at ({row}, {col}): imag={}",
                gate.name,
                value.im
            );
        }
    }
}

macro_rules! unitarity_test {
    ($name:ident, $gate:expr) => {
        #[test]
        fn $name() {
            let gate = $gate;
            assert_gate_is_unitary(&gate);
        }
    };
}

unitarity_test!(x_is_unitary, gate::x());
unitarity_test!(y_is_unitary, gate::y());
unitarity_test!(z_is_unitary, gate::z());
unitarity_test!(h_is_unitary, gate::h());
unitarity_test!(s_is_unitary, gate::s());
unitarity_test!(sdg_is_unitary, gate::sdg());
unitarity_test!(t_is_unitary, gate::t());
unitarity_test!(tdg_is_unitary, gate::tdg());
unitarity_test!(sx_is_unitary, gate::sx());
unitarity_test!(rx_is_unitary, gate::rx(PI / 7.0));
unitarity_test!(ry_is_unitary, gate::ry(PI / 5.0));
unitarity_test!(rz_is_unitary, gate::rz(PI / 3.0));
unitarity_test!(u3_is_unitary, gate::u3(0.91, -0.43, 1.27));
unitarity_test!(cnot_is_unitary, gate::cnot());
unitarity_test!(cz_is_unitary, gate::cz());
unitarity_test!(swap_is_unitary, gate::swap());
unitarity_test!(cy_is_unitary, gate::cy());
unitarity_test!(rzz_is_unitary, gate::rzz(PI / 2.0));
