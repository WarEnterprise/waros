//! Error types for the `WarOS` quantum simulator.

use thiserror::Error;

/// Result alias used across the public `WarOS` quantum API.
pub type WarosResult<T> = Result<T, WarosError>;

/// Errors returned by the `WarOS` quantum simulator and circuit builder.
#[derive(Debug, Error, PartialEq)]
pub enum WarosError {
    /// The caller requested a circuit with no qubits.
    #[error("Circuit must have at least 1 qubit")]
    ZeroQubits,

    /// A qubit index exceeded the size of the circuit.
    #[error("Qubit index {0} out of range (circuit has {1} qubits)")]
    QubitOutOfRange(usize, usize),

    /// A multi-qubit operation was asked to reuse the same qubit twice.
    #[error("Two-qubit gate requires different qubits, got {0} and {1}")]
    SameQubit(usize, usize),

    /// The requested qubit count exceeds the supported state-vector limit.
    #[error("Circuit exceeds maximum qubit count ({0} > {1})")]
    TooManyQubits(usize, usize),

    /// Two circuits require different qubit counts for the requested operation.
    #[error("Circuit qubit count mismatch ({0} != {1})")]
    CircuitQubitMismatch(usize, usize),

    /// The provided qubit mapping length does not match the source circuit width.
    #[error("Qubit mapping length mismatch ({0} != {1})")]
    InvalidQubitMapping(usize, usize),

    /// The requested simulation could not reserve enough memory.
    #[error("Insufficient memory for {0}-qubit simulation (need {1} bytes)")]
    InsufficientMemory(usize, usize),

    /// The caller requested an invalid shot count.
    #[error("Shot count must be greater than zero, got {0}")]
    InvalidShots(u32),

    /// A numerical error would make the simulation physically invalid.
    #[error("Numerical instability while {0}")]
    NumericalInstability(&'static str),

    /// A generic simulation error.
    #[error("Simulation error: {0}")]
    SimulationError(String),

    /// A network request to a remote backend failed.
    #[error("Network error: {0}")]
    NetworkError(String),

    /// A remote API returned an error response.
    #[error("IBM API error: {0}")]
    APIError(String),

    /// Authentication or credential resolution failed.
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// A remote quantum hardware execution failed.
    #[error("Hardware error: {0}")]
    HardwareError(String),

    /// A remote job exceeded the allowed wait time.
    #[error("Job timeout: {0}")]
    Timeout(String),

    /// Structured data from an external source could not be parsed.
    #[error("Parse error: {0}")]
    ParseError(String),

    /// Local filesystem I/O failed.
    #[error("IO error: {0}")]
    IOError(String),
}
