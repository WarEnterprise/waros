//! # `WarOS` Quantum SDK
//!
//! The first building block of the `WarOS` hybrid quantum-classical operating system.
//!
//! ```rust
//! use waros_quantum::{Circuit, Simulator, WarosError};
//!
//! # fn main() -> Result<(), WarosError> {
//! let mut circuit = Circuit::new(2)?;
//! circuit.h(0)?;
//! circuit.cnot(0, 1)?;
//! circuit.measure_all()?;
//!
//! let sim = Simulator::new();
//! let result = sim.run(&circuit, 1000)?;
//! result.print_histogram();
//! # Ok(())
//! # }
//! ```

pub mod algorithms;
pub mod circuit;
pub mod complex;
pub mod error;
pub mod gate;
pub mod noise;
pub mod qasm;
pub mod result;
pub mod simulator;

pub use algorithms::{
    apply_hidden_xor_oracle, classical_maxcut, continued_fraction_period, evaluate_expectation,
    gcd, maxcut_cost, mod_pow, qaoa_maxcut, quantum_phase_estimation, quantum_random_walk,
    shor_factor, simon_algorithm, solve_gf2, vqe, AnsatzType, Graph, Hamiltonian, Pauli, PauliTerm,
    QAOAResult, QPEResult, RandomWalkResult, ShorResult, SimonResult, VQEResult,
};
pub use circuit::Circuit;
pub use complex::Complex;
pub use error::{WarosError, WarosResult};
pub use noise::{NoiseChannel, NoiseModel};
pub use qasm::{parse_qasm, to_qasm, QasmError};
pub use result::QuantumResult;
pub use simulator::{Backend, Simulator, SimulatorBuilder};
