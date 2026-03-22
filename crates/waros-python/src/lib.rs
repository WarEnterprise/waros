use std::fmt::Display;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyModule;

mod circuit;
mod crypto;
mod noise;
mod qasm;
mod result;
mod simulator;

pub(crate) fn value_error(error: impl Display) -> PyErr {
    PyValueError::new_err(error.to_string())
}

pub(crate) fn runtime_error(error: impl Display) -> PyErr {
    PyRuntimeError::new_err(error.to_string())
}

#[pymodule]
fn waros(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<circuit::PyCircuit>()?;
    m.add_class::<simulator::PySimulator>()?;
    m.add_class::<result::PyQuantumResult>()?;
    m.add_class::<noise::PyNoiseModel>()?;

    m.add_function(wrap_pyfunction!(qasm::parse_qasm, m)?)?;
    m.add_function(wrap_pyfunction!(qasm::to_qasm, m)?)?;

    let crypto_module = PyModule::new_bound(m.py(), "crypto")?;
    crypto::register(&crypto_module)?;
    m.add_submodule(&crypto_module)?;

    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
