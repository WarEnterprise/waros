//! Advanced quantum algorithms built on top of the `WarOS` circuit and simulator APIs.

mod qaoa;
mod qpe;
mod random_walk;
mod shor;
mod simon;
mod vqe;

pub use qaoa::{classical_maxcut, maxcut_cost, qaoa_maxcut, Graph, QAOAResult};
pub use qpe::{quantum_phase_estimation, QPEResult};
pub use random_walk::{quantum_random_walk, RandomWalkResult};
pub use shor::{continued_fraction_period, gcd, mod_pow, shor_factor, ShorResult};
pub use simon::{apply_hidden_xor_oracle, simon_algorithm, solve_gf2, SimonResult};
pub use vqe::{evaluate_expectation, vqe, AnsatzType, Hamiltonian, Pauli, PauliTerm, VQEResult};
