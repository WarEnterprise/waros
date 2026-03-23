pub mod auth;
pub mod client;
pub mod jobs;
pub mod transpiler;
pub mod types;

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::backends::{BackendCapabilities, JobHandle, JobStatus, QuantumBackend};
use crate::circuit::Instruction;
use crate::error::WarosError;
use crate::{Circuit, QuantumResult, WarosResult};

use self::auth::ResolvedCredentials;
use self::client::IBMClient;
use self::types::{
    DynamicalDecouplingOptions, IBMBackendInfo, IBMJobResultEnvelope, IBMPrimitiveResult,
    JobCreateRequest, SamplerJobParams, SamplerOptions, SamplerPub, TwirlingOptions,
};

const DEFAULT_API_VERSION: &str = "2026-02-15";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IBMRegion {
    #[default]
    Global,
    EuDe,
}

impl IBMRegion {
    fn base_url(self) -> &'static str {
        match self {
            Self::Global => "https://quantum.cloud.ibm.com",
            Self::EuDe => "https://eu-de.quantum.cloud.ibm.com",
        }
    }
}

/// Connection and execution settings for the IBM Quantum Runtime backend.
#[derive(Debug, Clone)]
pub struct IBMConfig {
    api_key: Option<String>,
    instance_crn: Option<String>,
    region: IBMRegion,
    api_version: String,
    default_backend: String,
    request_timeout: Duration,
    initial_poll_interval: Duration,
    max_poll_interval: Duration,
    optimization_level: u8,
    resilience_level: u8,
}

impl IBMConfig {
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_instance_crn(mut self, instance_crn: impl Into<String>) -> Self {
        self.instance_crn = Some(instance_crn.into());
        self
    }

    #[must_use]
    pub fn with_region(mut self, region: IBMRegion) -> Self {
        self.region = region;
        self
    }

    #[must_use]
    pub fn with_default_backend(mut self, backend: impl Into<String>) -> Self {
        self.default_backend = backend.into();
        self
    }

    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_optimization_level(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    #[must_use]
    pub fn with_resilience_level(mut self, level: u8) -> Self {
        self.resilience_level = level.min(2);
        self
    }

    fn credentials(&self) -> WarosResult<ResolvedCredentials> {
        auth::resolve_credentials(self.api_key.as_deref(), self.instance_crn.as_deref())
    }
}

impl Default for IBMConfig {
    fn default() -> Self {
        Self {
            api_key: None,
            instance_crn: None,
            region: IBMRegion::Global,
            api_version: DEFAULT_API_VERSION.to_string(),
            default_backend: "ibm_brisbane".to_string(),
            request_timeout: Duration::from_secs(30),
            initial_poll_interval: Duration::from_secs(2),
            max_poll_interval: Duration::from_secs(30),
            optimization_level: 1,
            resilience_level: 1,
        }
    }
}

/// IBM Quantum backend backed by the Qiskit Runtime REST API.
pub struct IBMBackend {
    client: IBMClient,
    default_backend: String,
    optimization_level: u8,
    resilience_level: u8,
    initial_poll_interval: Duration,
    max_poll_interval: Duration,
    submitted_jobs: Mutex<HashMap<String, JobHandle>>,
}

impl IBMBackend {
    pub fn new(config: IBMConfig) -> WarosResult<Self> {
        let credentials = config.credentials()?;
        let client = IBMClient::new(
            credentials,
            config.region.base_url().to_string(),
            config.api_version,
            config.request_timeout,
        )?;

        Ok(Self {
            client,
            default_backend: config.default_backend,
            optimization_level: config.optimization_level,
            resilience_level: config.resilience_level,
            initial_poll_interval: config.initial_poll_interval,
            max_poll_interval: config.max_poll_interval,
            submitted_jobs: Mutex::new(HashMap::new()),
        })
    }

    pub fn from_env() -> WarosResult<Self> {
        Self::new(IBMConfig::default())
    }

    #[must_use]
    pub fn with_backend(mut self, backend: &str) -> Self {
        self.default_backend = backend.to_string();
        self
    }

    #[must_use]
    pub fn with_optimization(mut self, level: u8) -> Self {
        self.optimization_level = level.min(3);
        self
    }

    #[must_use]
    pub fn with_resilience(mut self, level: u8) -> Self {
        self.resilience_level = level.min(2);
        self
    }

    pub fn list_backends(&self) -> WarosResult<Vec<IBMBackendInfo>> {
        self.client
            .list_backends()?
            .into_iter()
            .map(|summary| Ok(IBMBackendInfo::from(summary)))
            .collect()
    }

    pub fn get_backend_info(&self, name: &str) -> WarosResult<IBMBackendInfo> {
        let summary = self
            .client
            .list_backends()?
            .into_iter()
            .find(|backend| backend.name == name)
            .ok_or_else(|| WarosError::APIError(format!("IBM backend '{name}' not found")))?;

        let mut info = IBMBackendInfo::from(summary);
        let configuration = self.client.get_backend_configuration(name)?;
        info.apply_configuration(&configuration);
        Ok(info)
    }

    pub fn run_on(
        &self,
        backend: &str,
        circuit: &Circuit,
        shots: u32,
    ) -> WarosResult<QuantumResult> {
        self.run_with_timeout(backend, circuit, shots, Duration::from_secs(600))
    }

    pub fn run_with_timeout(
        &self,
        backend: &str,
        circuit: &Circuit,
        shots: u32,
        timeout: Duration,
    ) -> WarosResult<QuantumResult> {
        let job = self.submit_to(backend, circuit, shots)?;
        self.wait_for_result(&job, timeout)
    }

    pub fn submit_to(
        &self,
        backend: &str,
        circuit: &Circuit,
        shots: u32,
    ) -> WarosResult<JobHandle> {
        let backend_info = self.get_backend_info(backend)?;
        transpiler::validate_for_backend(circuit, &backend_info, shots)?;

        let optimized = transpiler::optimize_for_ibm(circuit, self.optimization_level)?;
        let qasm = transpiler::circuit_to_ibm_qasm(&optimized)?;
        let request = JobCreateRequest {
            program_id: "sampler",
            backend: backend.to_string(),
            params: SamplerJobParams {
                pubs: vec![SamplerPub::new(qasm, Some(shots))],
                options: sampler_options(self.resilience_level),
                shots: None,
                support_qiskit: None,
                version: 2,
            },
        };

        let response = self.client.submit_job(&request)?;
        let handle = JobHandle {
            id: response.id,
            backend: if response.backend.is_empty() {
                backend.to_string()
            } else {
                response.backend
            },
            submitted_at: Instant::now(),
            shots: Some(shots),
            output_bits: Some(measured_output_width(circuit)),
        };
        self.remember_job(&handle)?;
        Ok(handle)
    }

    pub fn job_status_by_id(&self, job_id: &str) -> WarosResult<JobStatus> {
        let info = self.client.get_job(job_id)?;
        Ok(jobs::map_job_status(&info))
    }

    pub fn wait_for_job_id(&self, job_id: &str, timeout: Duration) -> WarosResult<QuantumResult> {
        let handle = self.known_job(job_id)?;
        self.wait_for_result(&handle, timeout)
    }

    pub fn wait_for_result(
        &self,
        job: &JobHandle,
        timeout: Duration,
    ) -> WarosResult<QuantumResult> {
        let started = Instant::now();
        let mut poll_interval = self.initial_poll_interval;

        loop {
            if started.elapsed() > timeout {
                return Err(WarosError::Timeout(format!(
                    "Job {} timed out after {:?}",
                    job.id, timeout
                )));
            }

            let info = self.client.get_job(&job.id)?;
            match jobs::map_job_status(&info) {
                JobStatus::Completed => {
                    let payload = self.client.get_job_results(&job.id)?;
                    return self.convert_ibm_result(payload, job.output_bits);
                }
                JobStatus::Failed { .. } => {
                    return Err(WarosError::HardwareError(jobs::failure_reason(&info)));
                }
                JobStatus::Cancelled => {
                    return Err(WarosError::HardwareError(format!(
                        "IBM job {} was cancelled",
                        job.id
                    )));
                }
                JobStatus::Queued { .. } | JobStatus::Running => {
                    thread::sleep(poll_interval);
                    poll_interval =
                        jobs::next_poll_interval(poll_interval).min(self.max_poll_interval);
                }
            }
        }
    }

    fn convert_ibm_result(
        &self,
        payload: Value,
        output_bits: Option<usize>,
    ) -> WarosResult<QuantumResult> {
        let envelope: IBMJobResultEnvelope = serde_json::from_value(payload)
            .map_err(|error| WarosError::ParseError(error.to_string()))?;
        let primitive = envelope.results.first().ok_or_else(|| {
            WarosError::ParseError("IBM result payload did not contain any PUB results".into())
        })?;

        if let Some(counts) = extract_counts(&primitive.data, output_bits) {
            let shots = counts.values().copied().sum();
            let width = output_bits
                .unwrap_or_else(|| counts.keys().map(|key| key.len()).max().unwrap_or_default());
            return Ok(QuantumResult::new(width, shots, counts));
        }

        let samples = extract_samples(primitive)?;
        let width = output_bits
            .or_else(|| find_num_bits(&primitive.data))
            .unwrap_or_else(|| infer_width(&samples));
        let mut counts = HashMap::new();
        for sample in &samples {
            let state = normalize_ibm_state(sample, width);
            *counts.entry(state).or_insert(0) += 1;
        }

        let total_shots = u32::try_from(samples.len())
            .map_err(|_| WarosError::ParseError("IBM result sample count overflowed u32".into()))?;
        Ok(QuantumResult::new(width, total_shots, counts))
    }

    fn remember_job(&self, handle: &JobHandle) -> WarosResult<()> {
        self.submitted_jobs
            .lock()
            .map_err(|_| WarosError::HardwareError("IBM job cache poisoned".into()))?
            .insert(handle.id.clone(), handle.clone());
        Ok(())
    }

    fn known_job(&self, job_id: &str) -> WarosResult<JobHandle> {
        Ok(self
            .submitted_jobs
            .lock()
            .map_err(|_| WarosError::HardwareError("IBM job cache poisoned".into()))?
            .get(job_id)
            .cloned()
            .unwrap_or(JobHandle {
                id: job_id.to_string(),
                backend: self.default_backend.clone(),
                submitted_at: Instant::now(),
                shots: None,
                output_bits: None,
            }))
    }
}

impl QuantumBackend for IBMBackend {
    fn name(&self) -> &str {
        "ibm-quantum"
    }

    fn max_qubits(&self) -> usize {
        127
    }

    fn is_hardware(&self) -> bool {
        true
    }

    fn run(&self, circuit: &Circuit, shots: u32) -> WarosResult<QuantumResult> {
        self.run_on(&self.default_backend, circuit, shots)
    }

    fn submit(&self, circuit: &Circuit, shots: u32) -> WarosResult<JobHandle> {
        self.submit_to(&self.default_backend, circuit, shots)
    }

    fn job_status(&self, job: &JobHandle) -> WarosResult<JobStatus> {
        self.job_status_by_id(&job.id)
    }

    fn get_result(&self, job: &JobHandle, timeout: Duration) -> WarosResult<QuantumResult> {
        self.wait_for_result(job, timeout)
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            max_qubits: 127,
            max_shots: 100_000,
            max_circuit_depth: None,
            native_gates: vec!["cx", "cz", "id", "rz", "sx", "x"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            coupling_map: None,
            is_simulator: false,
            supports_mid_circuit_measurement: true,
        }
    }
}

fn sampler_options(resilience_level: u8) -> Option<SamplerOptions> {
    match resilience_level.min(2) {
        0 => None,
        1 => Some(SamplerOptions {
            twirling: Some(TwirlingOptions {
                enable_gates: true,
                enable_measure: true,
                num_randomizations: "auto".into(),
                shots_per_randomization: "auto".into(),
                strategy: "active-accum".into(),
            }),
            dynamical_decoupling: None,
        }),
        _ => Some(SamplerOptions {
            twirling: Some(TwirlingOptions {
                enable_gates: true,
                enable_measure: true,
                num_randomizations: "auto".into(),
                shots_per_randomization: "auto".into(),
                strategy: "active-accum".into(),
            }),
            dynamical_decoupling: Some(DynamicalDecouplingOptions {
                enable: true,
                sequence_type: "XpXm".into(),
                extra_slack_distribution: "middle".into(),
                scheduling_method: "alap".into(),
            }),
        }),
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

fn extract_samples(result: &IBMPrimitiveResult) -> WarosResult<Vec<String>> {
    find_samples(&result.data).ok_or_else(|| {
        WarosError::ParseError(format!(
            "IBM sampler result did not contain a samples array: {}",
            result.data
        ))
    })
}

fn find_samples(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Object(map) => {
            if let Some(samples) = map.get("samples").and_then(Value::as_array) {
                let parsed = samples
                    .iter()
                    .map(|sample| sample.as_str().map(str::to_string))
                    .collect::<Option<Vec<_>>>()?;
                return Some(parsed);
            }
            map.values().find_map(find_samples)
        }
        Value::Array(values) => values.iter().find_map(find_samples),
        _ => None,
    }
}

fn extract_counts(value: &Value, output_bits: Option<usize>) -> Option<HashMap<String, u32>> {
    match value {
        Value::Object(map) => {
            if let Some(counts) = map.get("counts").and_then(Value::as_object) {
                let width = output_bits.unwrap_or_else(|| {
                    counts
                        .keys()
                        .map(|state| normalize_ibm_state(state, 0).len())
                        .max()
                        .unwrap_or_default()
                });
                let parsed = counts
                    .iter()
                    .map(|(state, count)| {
                        Some((
                            normalize_ibm_state(state, width),
                            u32::try_from(count.as_u64()?).ok()?,
                        ))
                    })
                    .collect::<Option<HashMap<_, _>>>()?;
                return Some(parsed);
            }
            map.values()
                .find_map(|nested| extract_counts(nested, output_bits))
        }
        Value::Array(values) => values
            .iter()
            .find_map(|nested| extract_counts(nested, output_bits)),
        _ => None,
    }
}

fn infer_width(samples: &[String]) -> usize {
    samples
        .iter()
        .map(|sample| {
            if let Some(hex) = sample
                .strip_prefix("0x")
                .or_else(|| sample.strip_prefix("0X"))
            {
                u128::from_str_radix(hex, 16)
                    .ok()
                    .map(|value| {
                        let bits = 128usize.saturating_sub(value.leading_zeros() as usize);
                        bits.max(1)
                    })
                    .unwrap_or_else(|| hex.len() * 4)
            } else {
                sample.trim().len()
            }
        })
        .max()
        .unwrap_or_default()
}

fn find_num_bits(value: &Value) -> Option<usize> {
    match value {
        Value::Object(map) => {
            if let Some(bits) = map
                .get("num_bits")
                .or_else(|| map.get("numbits"))
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            {
                return Some(bits);
            }
            map.values().find_map(find_num_bits)
        }
        Value::Array(values) => values.iter().find_map(find_num_bits),
        _ => None,
    }
}

fn normalize_ibm_state(state: &str, width: usize) -> String {
    let trimmed = state.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        let value = u128::from_str_radix(hex, 16).unwrap_or_default();
        let bits = if width == 0 {
            format!("{value:b}")
        } else {
            format!("{value:0width$b}")
        };
        return bits.chars().rev().collect();
    }

    let mut binary = trimmed.replace(' ', "");
    if width > binary.len() {
        binary = format!("{binary:0>width$}");
    }
    binary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ibm_state_hex_to_waros_bit_order() {
        assert_eq!(normalize_ibm_state("0x0", 2), "00");
        assert_eq!(normalize_ibm_state("0x1", 2), "10");
        assert_eq!(normalize_ibm_state("0x3", 2), "11");
    }

    #[test]
    fn extract_sampler_samples_from_named_register() {
        let primitive = IBMPrimitiveResult {
            data: serde_json::json!({
                "meas": {
                    "samples": ["0x0", "0x3", "0x3"]
                }
            }),
            metadata: Value::Null,
        };

        let samples = extract_samples(&primitive).expect("samples extracted");
        assert_eq!(samples, vec!["0x0", "0x3", "0x3"]);
    }

    #[test]
    fn extract_counts_fallback_reads_nested_counts() {
        let counts = extract_counts(
            &serde_json::json!({
                "meas": {
                    "counts": {
                        "0x0": 2,
                        "0x3": 1
                    }
                }
            }),
            Some(2),
        )
        .expect("counts extracted");

        assert_eq!(counts.get("00"), Some(&2));
        assert_eq!(counts.get("11"), Some(&1));
    }
}
