use pyo3::prelude::*;

use waros_quantum::Circuit as RustCircuit;

use crate::value_error;

/// Quantum circuit composed of gates, barriers, and measurements.
#[pyclass(name = "Circuit", module = "waros")]
#[derive(Clone)]
pub struct PyCircuit {
    pub(crate) inner: RustCircuit,
}

#[pymethods]
impl PyCircuit {
    /// Create a new circuit with `num_qubits` qubits initialized to `|0>`.
    #[new]
    fn new(num_qubits: usize) -> PyResult<Self> {
        Ok(Self {
            inner: RustCircuit::new(num_qubits).map_err(value_error)?,
        })
    }

    /// Number of qubits in the circuit.
    #[getter]
    fn num_qubits(&self) -> usize {
        self.inner.num_qubits()
    }

    /// Number of classical bits allocated by measurements.
    #[getter]
    fn num_classical_bits(&self) -> usize {
        self.inner.num_classical_bits()
    }

    /// Number of gate operations in the circuit.
    #[getter]
    fn gate_count(&self) -> usize {
        self.inner.gate_count()
    }

    /// Circuit depth measured in sequential gate layers.
    #[getter]
    fn depth(&self) -> usize {
        self.inner.depth()
    }

    /// Return a shallow copy of the circuit.
    fn copy(&self) -> Self {
        self.clone()
    }

    /// Apply a Hadamard gate.
    fn h(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.h(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a Pauli-X gate.
    fn x(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.x(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a Pauli-Y gate.
    fn y(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.y(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a Pauli-Z gate.
    fn z(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.z(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply an S gate.
    fn s(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.s(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply an S-dagger gate.
    fn sdg(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.sdg(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a T gate.
    fn t(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.t(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a T-dagger gate.
    fn tdg(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.tdg(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply an Rx rotation in radians.
    fn rx(&mut self, qubit: usize, theta: f64) -> PyResult<()> {
        self.inner.rx(qubit, theta).map(|_| ()).map_err(value_error)
    }

    /// Apply an Ry rotation in radians.
    fn ry(&mut self, qubit: usize, theta: f64) -> PyResult<()> {
        self.inner.ry(qubit, theta).map(|_| ()).map_err(value_error)
    }

    /// Apply an Rz rotation in radians.
    fn rz(&mut self, qubit: usize, theta: f64) -> PyResult<()> {
        self.inner.rz(qubit, theta).map(|_| ()).map_err(value_error)
    }

    /// Apply a square-root X gate.
    fn sx(&mut self, qubit: usize) -> PyResult<()> {
        self.inner.sx(qubit).map(|_| ()).map_err(value_error)
    }

    /// Apply a generic U3 rotation.
    fn u3(&mut self, qubit: usize, theta: f64, phi: f64, lambda: f64) -> PyResult<()> {
        self.inner
            .u3(qubit, theta, phi, lambda)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply a controlled-NOT gate.
    fn cnot(&mut self, control: usize, target: usize) -> PyResult<()> {
        self.inner
            .cnot(control, target)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Alias for `cnot`.
    fn cx(&mut self, control: usize, target: usize) -> PyResult<()> {
        self.cnot(control, target)
    }

    /// Apply a controlled-Y gate.
    fn cy(&mut self, control: usize, target: usize) -> PyResult<()> {
        self.inner
            .cy(control, target)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply a controlled-Z gate.
    fn cz(&mut self, control: usize, target: usize) -> PyResult<()> {
        self.inner
            .cz(control, target)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply a SWAP gate.
    fn swap(&mut self, q0: usize, q1: usize) -> PyResult<()> {
        self.inner.swap(q0, q1).map(|_| ()).map_err(value_error)
    }

    /// Apply an RZZ interaction in radians.
    fn rzz(&mut self, q0: usize, q1: usize, theta: f64) -> PyResult<()> {
        self.inner
            .rzz(q0, q1, theta)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply a Toffoli gate using the library decomposition.
    fn toffoli(&mut self, control_0: usize, control_1: usize, target: usize) -> PyResult<()> {
        self.inner
            .toffoli(control_0, control_1, target)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply a controlled phase rotation by `2*pi / 2^k`.
    fn crk(&mut self, control: usize, target: usize, k: usize) -> PyResult<()> {
        self.inner
            .crk(control, target, k)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Apply the Quantum Fourier Transform over the provided qubits.
    fn qft(&mut self, qubits: Vec<usize>) -> PyResult<()> {
        self.inner.qft(&qubits).map(|_| ()).map_err(value_error)
    }

    /// Apply the inverse Quantum Fourier Transform over the provided qubits.
    fn iqft(&mut self, qubits: Vec<usize>) -> PyResult<()> {
        self.inner.iqft(&qubits).map(|_| ()).map_err(value_error)
    }

    /// Measure a qubit into the next classical bit and return that bit index.
    fn measure(&mut self, qubit: usize) -> PyResult<usize> {
        self.inner.measure(qubit).map_err(value_error)
    }

    /// Measure a qubit into an explicit classical bit.
    fn measure_into(&mut self, qubit: usize, classical_bit: usize) -> PyResult<()> {
        self.inner
            .measure_into(qubit, classical_bit)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Measure every qubit in order.
    fn measure_all(&mut self) -> PyResult<()> {
        self.inner.measure_all().map(|_| ()).map_err(value_error)
    }

    /// Insert a logical barrier across the provided qubits.
    fn barrier(&mut self, qubits: Vec<usize>) -> PyResult<()> {
        self.inner.barrier(&qubits).map(|_| ()).map_err(value_error)
    }

    /// Insert a barrier across all qubits.
    fn barrier_all(&mut self) {
        self.inner.barrier_all();
    }

    /// Append another circuit with the same width.
    fn append(&mut self, other: PyRef<'_, Self>) -> PyResult<()> {
        self.inner
            .append(&other.inner)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Compose another circuit onto a mapped set of qubits.
    fn compose(&mut self, other: PyRef<'_, Self>, qubit_mapping: Vec<usize>) -> PyResult<()> {
        self.inner
            .compose(&other.inner, &qubit_mapping)
            .map(|_| ())
            .map_err(value_error)
    }

    /// Return the ASCII circuit diagram.
    fn draw(&self) -> String {
        self.inner.to_ascii()
    }

    /// Export the circuit as `OpenQASM` 2.0 source.
    fn to_qasm(&self) -> String {
        waros_quantum::to_qasm(&self.inner)
    }

    fn __copy__(&self) -> Self {
        self.clone()
    }

    fn __deepcopy__(&self, _memo: &Bound<'_, PyAny>) -> Self {
        self.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "Circuit({} qubits, {} gates, depth {})",
            self.inner.num_qubits(),
            self.inner.gate_count(),
            self.inner.depth()
        )
    }

    fn __str__(&self) -> String {
        self.draw()
    }

    fn __len__(&self) -> usize {
        self.inner.gate_count()
    }
}
