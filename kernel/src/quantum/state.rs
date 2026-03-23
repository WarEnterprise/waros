use alloc::vec;
use alloc::vec::Vec;

/// Numerical tolerance used for probability and normalization checks.
pub const EPSILON: f64 = 1e-12;
pub const MAX_KERNEL_QUBITS: usize = 18;

/// Minimal complex number representation for the kernel simulator.
pub type Complex = (f64, f64);

/// Complex addition.
#[inline]
#[must_use]
pub fn cadd(lhs: Complex, rhs: Complex) -> Complex {
    (lhs.0 + rhs.0, lhs.1 + rhs.1)
}

/// Complex multiplication.
#[inline]
#[must_use]
pub fn cmul(lhs: Complex, rhs: Complex) -> Complex {
    (lhs.0 * rhs.0 - lhs.1 * rhs.1, lhs.0 * rhs.1 + lhs.1 * rhs.0)
}

/// Complex scalar multiplication.
#[inline]
#[must_use]
pub fn cscale(value: Complex, scalar: f64) -> Complex {
    (value.0 * scalar, value.1 * scalar)
}

/// Squared magnitude of a complex amplitude.
#[inline]
#[must_use]
pub fn norm_sq(value: Complex) -> f64 {
    value.0 * value.0 + value.1 * value.1
}

/// State vector for a kernel-managed quantum register.
pub struct QuantumState {
    pub num_qubits: usize,
    pub amplitudes: Vec<Complex>,
}

impl QuantumState {
    /// Allocate a new quantum register initialized to `|00...0>`.
    pub fn new(num_qubits: usize) -> Result<Self, &'static str> {
        if !(1..=MAX_KERNEL_QUBITS).contains(&num_qubits) {
            return Err("Qubit count must be between 1 and 18");
        }

        let dimension = 1usize << num_qubits;
        let mut amplitudes = vec![(0.0, 0.0); dimension];
        amplitudes[0] = (1.0, 0.0);

        Ok(Self {
            num_qubits,
            amplitudes,
        })
    }

    /// Reset the register back to `|00...0>`.
    pub fn reset(&mut self) {
        for amplitude in &mut self.amplitudes {
            *amplitude = (0.0, 0.0);
        }
        self.amplitudes[0] = (1.0, 0.0);
    }

    /// Number of basis amplitudes in the state vector.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.amplitudes.len()
    }

    /// Heap bytes occupied by the amplitude vector payload.
    #[must_use]
    pub fn bytes_used(&self) -> usize {
        self.amplitudes.len() * core::mem::size_of::<Complex>()
    }

    /// Probability of every basis state in order.
    #[must_use]
    pub fn probabilities(&self) -> Vec<f64> {
        self.amplitudes.iter().copied().map(norm_sq).collect()
    }

    /// Total probability mass in the register.
    #[must_use]
    pub fn total_probability(&self) -> f64 {
        self.amplitudes.iter().copied().map(norm_sq).sum()
    }
}
