use std::time::{Duration, Instant};

use crate::{Circuit, QuantumResult, WarosResult};

pub mod simulator;

#[cfg(feature = "ibm")]
pub mod ibm;

/// Unified backend trait for quantum circuit execution.
pub trait QuantumBackend {
    /// Human-readable backend name.
    fn name(&self) -> &str;

    /// Maximum qubits supported by the backend.
    fn max_qubits(&self) -> usize;

    /// Whether this backend executes on real hardware.
    fn is_hardware(&self) -> bool;

    /// Execute a circuit synchronously.
    fn run(&self, circuit: &Circuit, shots: u32) -> WarosResult<QuantumResult>;

    /// Submit a circuit asynchronously.
    fn submit(&self, circuit: &Circuit, shots: u32) -> WarosResult<JobHandle>;

    /// Query the state of a submitted job.
    fn job_status(&self, job: &JobHandle) -> WarosResult<JobStatus>;

    /// Block until the job completes or the timeout elapses.
    fn get_result(&self, job: &JobHandle, timeout: Duration) -> WarosResult<QuantumResult>;

    /// Advertise backend capabilities.
    fn capabilities(&self) -> BackendCapabilities;
}

/// Static or slowly changing backend capability metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendCapabilities {
    pub max_qubits: usize,
    pub max_shots: u32,
    pub max_circuit_depth: Option<usize>,
    pub native_gates: Vec<String>,
    pub coupling_map: Option<Vec<(usize, usize)>>,
    pub is_simulator: bool,
    pub supports_mid_circuit_measurement: bool,
}

/// Lightweight handle for a submitted backend job.
#[derive(Debug, Clone)]
pub struct JobHandle {
    pub id: String,
    pub backend: String,
    pub submitted_at: Instant,
    pub shots: Option<u32>,
    pub output_bits: Option<usize>,
}

/// Runtime state of a backend job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued { position: Option<usize> },
    Running,
    Completed,
    Failed { error: String },
    Cancelled,
}
