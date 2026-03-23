use std::time::Duration;

use pyo3::exceptions::{PyConnectionError, PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use waros_quantum::backends::ibm::{auth, IBMBackend, IBMConfig, IBMRegion};
use waros_quantum::{QuantumBackend, WarosError};

use crate::circuit::PyCircuit;
use crate::result::PyQuantumResult;

#[pyclass(name = "IBMBackend", module = "waros")]
pub struct PyIBMBackend {
    inner: IBMBackend,
}

#[pymethods]
impl PyIBMBackend {
    #[new]
    #[pyo3(signature = (token=None, instance_crn=None, region="global"))]
    fn new(token: Option<&str>, instance_crn: Option<&str>, region: &str) -> PyResult<Self> {
        let mut config = token.map_or_else(IBMConfig::default, IBMConfig::new);
        if let Some(instance_crn) = instance_crn {
            config = config.with_instance_crn(instance_crn);
        }
        config = config.with_region(parse_region(region)?);

        Ok(Self {
            inner: IBMBackend::new(config).map_err(map_ibm_error)?,
        })
    }

    fn backends(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let backends = self.inner.list_backends().map_err(map_ibm_error)?;
        let mut result = Vec::with_capacity(backends.len());
        for backend in backends {
            let entry = PyDict::new_bound(py);
            entry.set_item("name", &backend.name)?;
            entry.set_item("num_qubits", backend.num_qubits)?;
            entry.set_item("operational", backend.status.is_operational())?;
            entry.set_item("pending_jobs", backend.queue_length)?;
            entry.set_item("status", backend.status.message())?;
            entry.set_item("is_simulator", backend.is_simulator)?;
            if let Some(processor_type) = backend.processor_type {
                entry.set_item("family", processor_type.family)?;
                entry.set_item("revision", processor_type.revision)?;
                entry.set_item("segment", processor_type.segment)?;
            }
            result.push(entry.into_py(py));
        }
        Ok(result)
    }

    #[pyo3(signature = (circuit, shots=1000, backend=None, timeout_secs=600))]
    fn run(
        &self,
        circuit: PyRef<'_, PyCircuit>,
        shots: u32,
        backend: Option<&str>,
        timeout_secs: u64,
    ) -> PyResult<PyQuantumResult> {
        let timeout = Duration::from_secs(timeout_secs);
        let result = match backend {
            Some(backend) => self
                .inner
                .run_with_timeout(backend, &circuit.inner, shots, timeout),
            None => self.inner.run(&circuit.inner, shots),
        }
        .map_err(map_ibm_error)?;

        Ok(PyQuantumResult { inner: result })
    }

    #[pyo3(signature = (circuit, shots=1000, backend=None))]
    fn submit(
        &self,
        circuit: PyRef<'_, PyCircuit>,
        shots: u32,
        backend: Option<&str>,
    ) -> PyResult<String> {
        let handle = match backend {
            Some(backend) => self.inner.submit_to(backend, &circuit.inner, shots),
            None => self.inner.submit(&circuit.inner, shots),
        }
        .map_err(map_ibm_error)?;
        Ok(handle.id)
    }

    fn status(&self, job_id: &str) -> PyResult<String> {
        Ok(format!(
            "{:?}",
            self.inner.job_status_by_id(job_id).map_err(map_ibm_error)?
        ))
    }

    #[pyo3(signature = (job_id, timeout_secs=600))]
    fn result(&self, job_id: &str, timeout_secs: u64) -> PyResult<PyQuantumResult> {
        let result = self
            .inner
            .wait_for_job_id(job_id, Duration::from_secs(timeout_secs))
            .map_err(map_ibm_error)?;
        Ok(PyQuantumResult { inner: result })
    }

    #[staticmethod]
    fn save_token(token: &str) -> PyResult<()> {
        auth::save_token(token).map_err(map_ibm_error)
    }

    #[staticmethod]
    fn save_instance(instance_crn: &str) -> PyResult<()> {
        auth::save_instance(instance_crn).map_err(map_ibm_error)
    }

    #[staticmethod]
    fn save_credentials(token: &str, instance_crn: &str) -> PyResult<()> {
        auth::save_credentials(token, instance_crn).map_err(map_ibm_error)
    }

    fn __repr__(&self) -> String {
        "IBMBackend(IBM Quantum Runtime)".to_string()
    }
}

fn parse_region(region: &str) -> PyResult<IBMRegion> {
    match region.trim().to_ascii_lowercase().as_str() {
        "" | "global" | "us" | "default" => Ok(IBMRegion::Global),
        "eu" | "eu-de" => Ok(IBMRegion::EuDe),
        other => Err(PyValueError::new_err(format!(
            "Unsupported IBM region '{other}'. Use 'global' or 'eu-de'."
        ))),
    }
}

fn map_ibm_error(error: WarosError) -> PyErr {
    match error {
        WarosError::AuthError(message) => PyValueError::new_err(message),
        WarosError::Timeout(message) => PyTimeoutError::new_err(message),
        WarosError::NetworkError(message)
        | WarosError::APIError(message)
        | WarosError::HardwareError(message)
        | WarosError::ParseError(message)
        | WarosError::IOError(message) => PyConnectionError::new_err(message),
        other => PyValueError::new_err(other.to_string()),
    }
}
