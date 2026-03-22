use pyo3::prelude::*;

use waros_quantum::NoiseModel as RustNoiseModel;

/// Noise model for realistic hardware-aware simulation.
#[pyclass(name = "NoiseModel", module = "waros")]
#[derive(Clone)]
pub struct PyNoiseModel {
    pub(crate) inner: RustNoiseModel,
}

#[pymethods]
impl PyNoiseModel {
    /// Create an ideal noise model.
    #[new]
    fn new() -> Self {
        Self::ideal()
    }

    /// Return a noiseless profile.
    #[staticmethod]
    fn ideal() -> Self {
        Self {
            inner: RustNoiseModel::ideal(),
        }
    }

    /// Return an IBM-like superconducting profile.
    #[staticmethod]
    fn ibm() -> Self {
        Self {
            inner: RustNoiseModel::ibm_like(),
        }
    }

    /// Return an IonQ-like trapped-ion profile.
    #[staticmethod]
    fn ionq() -> Self {
        Self {
            inner: RustNoiseModel::ionq_like(),
        }
    }

    /// Create a uniform depolarizing and readout profile.
    #[staticmethod]
    fn uniform(single_qubit_error: f64, two_qubit_error: f64, readout_error: f64) -> Self {
        Self {
            inner: RustNoiseModel::uniform(single_qubit_error, two_qubit_error, readout_error),
        }
    }

    /// Create a noise model from hardware parameters.
    #[staticmethod]
    fn from_hardware(
        t1_us: f64,
        t2_us: f64,
        gate_time_ns: f64,
        single_q_fidelity: f64,
        two_q_fidelity: f64,
        readout_fidelity: f64,
    ) -> Self {
        Self {
            inner: RustNoiseModel::from_hardware(
                t1_us,
                t2_us,
                gate_time_ns,
                single_q_fidelity,
                two_q_fidelity,
                readout_fidelity,
            ),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "NoiseModel(single_qubit_channels={}, two_qubit_channels={}, measurement_channels={})",
            self.inner.single_qubit_noise.len(),
            self.inner.two_qubit_noise.len(),
            self.inner.measurement_noise.len()
        )
    }

    fn __str__(&self) -> String {
        self.__repr__()
    }
}
