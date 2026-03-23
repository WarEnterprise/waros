//! Quantum error-correction helpers for `WarOS`.

mod repetition;
mod steane;

use crate::error::WarosResult;
use crate::Circuit;

pub use repetition::RepetitionCode;
pub use steane::SteaneCode;

/// Shared interface for simple quantum error-correction codes.
pub trait QECCode {
    /// Number of physical qubits required per logical qubit.
    fn physical_qubits(&self) -> usize;

    /// Number of logical qubits encoded by the code.
    fn logical_qubits(&self) -> usize;

    /// Code distance.
    fn distance(&self) -> usize;

    /// Append the encoding circuit for `logical_qubit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the circuit does not have enough qubits.
    fn encode(&self, circuit: &mut Circuit, logical_qubit: usize) -> WarosResult<()>;

    /// Append syndrome extraction and return the allocated classical bit indices.
    ///
    /// # Errors
    ///
    /// Returns an error if the circuit does not have enough data and ancilla
    /// qubits for the code.
    fn measure_syndrome(&self, circuit: &mut Circuit) -> WarosResult<Vec<usize>>;

    /// Append a correction derived from the provided syndrome bits.
    ///
    /// # Errors
    ///
    /// Returns an error if the syndrome length is invalid.
    fn correct(&self, circuit: &mut Circuit, syndrome: &[usize]) -> WarosResult<()>;
}
