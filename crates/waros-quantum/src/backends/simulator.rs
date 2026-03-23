use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::circuit::Instruction;
use crate::error::WarosError;
use crate::simulator::Simulator;
use crate::{Circuit, QuantumResult, WarosResult};

use super::{BackendCapabilities, JobHandle, JobStatus, QuantumBackend};

/// Backend adapter that exposes the in-process simulator through [`QuantumBackend`].
pub struct SimulatorBackend {
    inner: Simulator,
    jobs: Arc<Mutex<HashMap<String, QuantumResult>>>,
    next_job_id: Arc<AtomicU64>,
}

impl SimulatorBackend {
    #[must_use]
    pub fn new() -> Self {
        Self::from_simulator(Simulator::new())
    }

    #[must_use]
    pub fn from_simulator(simulator: Simulator) -> Self {
        Self {
            inner: simulator,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            next_job_id: Arc::new(AtomicU64::new(1)),
        }
    }

    #[must_use]
    pub fn simulator(&self) -> &Simulator {
        &self.inner
    }

    fn next_handle(&self, circuit: &Circuit, shots: u32) -> JobHandle {
        let id = format!("sim-{}", self.next_job_id.fetch_add(1, Ordering::Relaxed));
        JobHandle {
            id,
            backend: self.name().to_string(),
            submitted_at: Instant::now(),
            shots: Some(shots),
            output_bits: Some(measured_output_width(circuit)),
        }
    }
}

impl Default for SimulatorBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantumBackend for SimulatorBackend {
    fn name(&self) -> &str {
        "simulator"
    }

    fn max_qubits(&self) -> usize {
        crate::circuit::MAX_QUBITS
    }

    fn is_hardware(&self) -> bool {
        false
    }

    fn run(&self, circuit: &Circuit, shots: u32) -> WarosResult<QuantumResult> {
        self.inner.run(circuit, shots)
    }

    fn submit(&self, circuit: &Circuit, shots: u32) -> WarosResult<JobHandle> {
        let result = self.inner.run(circuit, shots)?;
        let handle = self.next_handle(circuit, shots);
        self.jobs
            .lock()
            .map_err(|_| WarosError::SimulationError("simulator job store poisoned".into()))?
            .insert(handle.id.clone(), result);
        Ok(handle)
    }

    fn job_status(&self, job: &JobHandle) -> WarosResult<JobStatus> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| WarosError::SimulationError("simulator job store poisoned".into()))?;
        if jobs.contains_key(&job.id) {
            Ok(JobStatus::Completed)
        } else {
            Err(WarosError::SimulationError(format!(
                "unknown simulator job '{}'",
                job.id
            )))
        }
    }

    fn get_result(&self, job: &JobHandle, _timeout: Duration) -> WarosResult<QuantumResult> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| WarosError::SimulationError("simulator job store poisoned".into()))?;
        jobs.get(&job.id).cloned().ok_or_else(|| {
            WarosError::SimulationError(format!("unknown simulator job '{}'", job.id))
        })
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            max_qubits: crate::circuit::MAX_QUBITS,
            max_shots: u32::MAX,
            max_circuit_depth: None,
            native_gates: vec![
                "h", "x", "y", "z", "s", "sdg", "t", "tdg", "sx", "rx", "ry", "rz", "u3", "cx",
                "cy", "cz", "swap", "rzz", "crk",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            coupling_map: None,
            is_simulator: true,
            supports_mid_circuit_measurement: true,
        }
    }
}

fn measured_output_width(circuit: &Circuit) -> usize {
    let measured = circuit
        .instructions()
        .iter()
        .filter(|instruction| matches!(instruction, Instruction::Measure { .. }))
        .count();
    if measured == 0 {
        circuit.num_qubits()
    } else {
        measured
    }
}
