use crate::complex::Complex;
use std::f64::consts::{FRAC_1_SQRT_2, PI};

#[must_use]
#[derive(Debug, Clone)]
pub struct Gate {
    pub name: String,
    pub matrix: Vec<Complex>,
    pub num_qubits: usize,
}

impl Gate {
    pub fn single(name: &str, m: [[Complex; 2]; 2]) -> Self {
        Self {
            name: name.to_string(),
            matrix: vec![m[0][0], m[0][1], m[1][0], m[1][1]],
            num_qubits: 1,
        }
    }

    pub fn two_qubit(name: &str, m: [[Complex; 4]; 4]) -> Self {
        let mut matrix = Vec::with_capacity(16);
        for row in &m {
            for &val in row {
                matrix.push(val);
            }
        }
        Self {
            name: name.to_string(),
            matrix,
            num_qubits: 2,
        }
    }

    pub fn get(&self, row: usize, col: usize) -> Complex {
        let dim = 1 << self.num_qubits;
        self.matrix[row * dim + col]
    }

    /// Return the conjugate transpose of the gate matrix.
    pub fn dagger(&self) -> Self {
        let dim = 1usize << self.num_qubits;
        let mut matrix = vec![Complex::ZERO; self.matrix.len()];
        for row in 0..dim {
            for col in 0..dim {
                matrix[row * dim + col] = self.get(col, row).conj();
            }
        }

        Self {
            name: format!("{}^dagger", self.name),
            matrix,
            num_qubits: self.num_qubits,
        }
    }

    /// Return the inverse of the gate.
    pub fn inverse(&self) -> Self {
        self.dagger()
    }
}

// === Single-qubit gates ===

pub fn x() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("X", [[o, i], [i, o]])
}

pub fn y() -> Gate {
    let o = Complex::ZERO;
    Gate::single(
        "Y",
        [[o, Complex::new(0.0, -1.0)], [Complex::new(0.0, 1.0), o]],
    )
}

pub fn z() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("Z", [[i, o], [o, -i]])
}

pub fn h() -> Gate {
    let s = Complex::new(FRAC_1_SQRT_2, 0.0);
    Gate::single("H", [[s, s], [s, -s]])
}

pub fn s() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("S", [[i, o], [o, Complex::I]])
}

pub fn sdg() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("Sdg", [[i, o], [o, Complex::new(0.0, -1.0)]])
}

pub fn t() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("T", [[i, o], [o, Complex::from_polar(1.0, PI / 4.0)]])
}

pub fn tdg() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::single("Tdg", [[i, o], [o, Complex::from_polar(1.0, -PI / 4.0)]])
}

pub fn rx(theta: f64) -> Gate {
    let c = Complex::new((theta / 2.0).cos(), 0.0);
    let s = Complex::new(0.0, -(theta / 2.0).sin());
    Gate::single(&format!("Rx({theta:.3})"), [[c, s], [s, c]])
}

pub fn ry(theta: f64) -> Gate {
    let c = Complex::new((theta / 2.0).cos(), 0.0);
    let s = Complex::new((theta / 2.0).sin(), 0.0);
    Gate::single(&format!("Ry({theta:.3})"), [[c, -s], [s, c]])
}

pub fn rz(theta: f64) -> Gate {
    let o = Complex::ZERO;
    Gate::single(
        &format!("Rz({theta:.3})"),
        [
            [Complex::from_polar(1.0, -theta / 2.0), o],
            [o, Complex::from_polar(1.0, theta / 2.0)],
        ],
    )
}

pub fn sx() -> Gate {
    let a = Complex::new(0.5, 0.5);
    let b = Complex::new(0.5, -0.5);
    Gate::single("SX", [[a, b], [b, a]])
}

pub fn u3(theta: f64, phi: f64, lambda: f64) -> Gate {
    let ct = (theta / 2.0).cos();
    let st = (theta / 2.0).sin();
    Gate::single(
        &format!("U3({theta:.3},{phi:.3},{lambda:.3})"),
        [
            [Complex::new(ct, 0.0), Complex::from_polar(-st, lambda)],
            [
                Complex::from_polar(st, phi),
                Complex::from_polar(ct, phi + lambda),
            ],
        ],
    )
}

// === Two-qubit gates ===

pub fn cnot() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::two_qubit(
        "CNOT",
        [[i, o, o, o], [o, i, o, o], [o, o, o, i], [o, o, i, o]],
    )
}

pub fn cz() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::two_qubit(
        "CZ",
        [[i, o, o, o], [o, i, o, o], [o, o, i, o], [o, o, o, -i]],
    )
}

pub fn swap() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::two_qubit(
        "SWAP",
        [[i, o, o, o], [o, o, i, o], [o, i, o, o], [o, o, o, i]],
    )
}

pub fn cy() -> Gate {
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::two_qubit(
        "CY",
        [
            [i, o, o, o],
            [o, i, o, o],
            [o, o, o, Complex::new(0.0, -1.0)],
            [o, o, Complex::new(0.0, 1.0), o],
        ],
    )
}

pub fn rzz(theta: f64) -> Gate {
    let o = Complex::ZERO;
    let p1 = Complex::from_polar(1.0, -theta / 2.0);
    let p2 = Complex::from_polar(1.0, theta / 2.0);
    Gate::two_qubit(
        &format!("Rzz({theta:.3})"),
        [[p1, o, o, o], [o, p2, o, o], [o, o, p2, o], [o, o, o, p1]],
    )
}

pub fn crk(k: usize) -> Gate {
    let exponent = i32::try_from(k).unwrap_or(i32::MAX);
    let angle = 2.0 * PI / 2.0_f64.powi(exponent);
    let (o, i) = (Complex::ZERO, Complex::ONE);
    Gate::two_qubit(
        &format!("CR{k}"),
        [
            [i, o, o, o],
            [o, i, o, o],
            [o, o, i, o],
            [o, o, o, Complex::from_polar(1.0, angle)],
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_h_squared_is_identity() {
        let hg = h();
        for i in 0..2 {
            for j in 0..2 {
                let mut sum = Complex::ZERO;
                for k in 0..2 {
                    sum += hg.get(i, k) * hg.get(k, j);
                }
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((sum.re - expected).abs() < 1e-10);
                assert!(sum.im.abs() < 1e-10);
            }
        }
    }
}
