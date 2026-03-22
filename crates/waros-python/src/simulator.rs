use pyo3::prelude::*;

use waros_quantum::Simulator as RustSimulator;

use crate::circuit::PyCircuit;
use crate::noise::PyNoiseModel;
use crate::result::PyQuantumResult;
use crate::value_error;

/// State-vector quantum simulator with optional Monte Carlo noise.
#[pyclass(name = "Simulator", module = "waros")]
pub struct PySimulator {
    inner: RustSimulator,
}

#[pymethods]
impl PySimulator {
    /// Create a simulator.
    #[new]
    #[pyo3(signature = (seed=None, noise=None, parallel=true))]
    fn new(
        py: Python<'_>,
        seed: Option<u64>,
        noise: Option<Py<PyNoiseModel>>,
        parallel: bool,
    ) -> Self {
        let mut builder = RustSimulator::builder().parallel(parallel);
        if let Some(seed) = seed {
            builder = builder.seed(seed);
        }
        if let Some(noise) = noise {
            let noise = noise.bind(py).borrow();
            builder = builder.noise(noise.inner.clone());
        }

        Self {
            inner: builder.build(),
        }
    }

    /// Run a circuit for `shots` executions.
    #[pyo3(signature = (circuit, shots=1000))]
    fn run(&self, circuit: PyRef<'_, PyCircuit>, shots: u32) -> PyResult<PyQuantumResult> {
        Ok(PyQuantumResult {
            inner: self.inner.run(&circuit.inner, shots).map_err(value_error)?,
        })
    }

    /// Return the final noiseless state vector as `(real, imag)` tuples.
    fn statevector(&self, circuit: PyRef<'_, PyCircuit>) -> PyResult<Vec<(f64, f64)>> {
        let state = self
            .inner
            .statevector(&circuit.inner)
            .map_err(value_error)?;
        Ok(state
            .into_iter()
            .map(|amplitude| (amplitude.re, amplitude.im))
            .collect())
    }

    /// Return the basis-state probabilities for a noiseless circuit.
    fn probabilities(&self, circuit: PyRef<'_, PyCircuit>) -> PyResult<Vec<f64>> {
        let state = self
            .inner
            .statevector(&circuit.inner)
            .map_err(value_error)?;
        Ok(state
            .into_iter()
            .map(|amplitude| amplitude.norm_sq())
            .collect())
    }

    fn __repr__(&self) -> String {
        "Simulator(StateVector)".to_string()
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }
}
