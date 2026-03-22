use std::fmt::Write as _;

use std::collections::HashMap;

use pyo3::prelude::*;

use waros_quantum::QuantumResult as RustQuantumResult;

use crate::value_error;

/// Measurement result collected from repeated circuit execution.
#[pyclass(name = "QuantumResult", module = "waros")]
#[derive(Clone)]
pub struct PyQuantumResult {
    pub(crate) inner: RustQuantumResult,
}

#[pymethods]
impl PyQuantumResult {
    /// Mapping from measured basis state to observation count.
    #[getter]
    fn counts(&self) -> HashMap<String, u32> {
        self.inner.counts().clone()
    }

    /// Total number of shots accumulated in this result.
    #[getter]
    fn total_shots(&self) -> u32 {
        self.inner.total_shots()
    }

    /// Return the observed probability of a basis state.
    fn probability(&self, state: &str) -> f64 {
        self.inner.probability(state)
    }

    /// Return the most probable observed state as `(state, count)`.
    fn most_probable(&self) -> (String, u32) {
        let (state, count) = self.inner.most_probable();
        (state.to_string(), count)
    }

    /// Return histogram rows as `(state, count, probability)`.
    fn histogram_data(&self) -> Vec<(String, u32, f64)> {
        self.inner
            .histogram()
            .into_iter()
            .map(|(state, count, probability)| (state.to_string(), count, probability))
            .collect()
    }

    /// Print an ASCII histogram to standard output.
    fn histogram(&self) {
        self.inner.print_histogram();
    }

    /// Return the expectation value of Z on the requested measured output bit.
    fn expectation_z(&self, qubit: usize) -> PyResult<f64> {
        self.inner.expectation_z(qubit).map_err(value_error)
    }

    /// Support dictionary-style access: `result["00"]`.
    fn __getitem__(&self, state: &str) -> u32 {
        self.inner.counts().get(state).copied().unwrap_or(0)
    }

    fn __repr__(&self) -> String {
        format!(
            "QuantumResult({} shots, {} outcomes)",
            self.inner.total_shots(),
            self.inner.counts().len()
        )
    }

    fn __str__(&self) -> String {
        let mut output = format!("QuantumResult ({} shots):\n", self.inner.total_shots());
        for (state, count, probability) in self.inner.histogram() {
            let _ = writeln!(
                output,
                "  |{}> : {} ({:.1}%)",
                state,
                count,
                probability * 100.0
            );
        }
        output.trim_end().to_string()
    }

    fn __len__(&self) -> usize {
        self.inner.counts().len()
    }
}
