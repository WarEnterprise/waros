#![allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::unused_self,
    clippy::useless_conversion
)]

use std::fmt::Display;

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyModule;

mod algorithms;
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
fn _waros(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<circuit::PyCircuit>()?;
    m.add_class::<simulator::PySimulator>()?;
    m.add_class::<result::PyQuantumResult>()?;
    m.add_class::<noise::PyNoiseModel>()?;

    m.add_function(wrap_pyfunction!(qasm::parse_qasm, m)?)?;
    m.add_function(wrap_pyfunction!(qasm::to_qasm, m)?)?;
    algorithms::register_root(m)?;

    let crypto_module = PyModule::new_bound(m.py(), "crypto")?;
    crypto::register(&crypto_module)?;
    m.add("crypto", &crypto_module)?;
    m.add_submodule(&crypto_module)?;

    let algorithms_module = PyModule::new_bound(m.py(), "algorithms")?;
    algorithms::register(&algorithms_module)?;
    m.add_submodule(&algorithms_module)?;

    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
