mod parser;
mod serializer;

use thiserror::Error;

use crate::Circuit;

/// Errors returned while parsing `OpenQASM` 2.0 source.
#[derive(Debug, Error, PartialEq)]
pub enum QasmError {
    /// Generic parse error with line information.
    #[error("Parse error at line {line}: {message}")]
    ParseError { line: usize, message: String },
    /// The source referenced an unsupported or unknown gate.
    #[error("Unknown gate '{0}'")]
    UnknownGate(String),
    /// The source referenced a register that was never declared.
    #[error("Register '{0}' not declared")]
    UndeclaredRegister(String),
    /// The source referenced a qubit outside the declared register bounds.
    #[error("Qubit index {index} out of range for register '{register}' (size {size})")]
    QubitOutOfRange {
        register: String,
        index: usize,
        size: usize,
    },
}

/// Parse `OpenQASM` 2.0 source into a [`Circuit`].
///
/// # Errors
///
/// Returns [`QasmError`] when the source is malformed or references
/// unsupported constructs.
pub fn parse_qasm(source: &str) -> Result<Circuit, QasmError> {
    parser::parse_qasm(source)
}

/// Serialize a circuit into `OpenQASM` 2.0 source.
#[must_use]
pub fn to_qasm(circuit: &Circuit) -> String {
    serializer::to_qasm(circuit)
}
