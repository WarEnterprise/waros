use pyo3::prelude::*;

use crate::circuit::PyCircuit;
use crate::value_error;

/// Parse OpenQASM 2.0 source into a circuit.
#[pyfunction]
pub fn parse_qasm(source: &str) -> PyResult<PyCircuit> {
    Ok(PyCircuit {
        inner: waros_quantum::parse_qasm(source).map_err(value_error)?,
    })
}

/// Serialize a circuit into OpenQASM 2.0 source.
#[pyfunction]
pub fn to_qasm(circuit: PyRef<'_, PyCircuit>) -> String {
    waros_quantum::to_qasm(&circuit.inner)
}
