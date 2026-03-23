use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct IBMBackendListResponse {
    pub devices: Vec<IBMBackendSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IBMBackendSummary {
    pub name: String,
    pub status: BackendStatus,
    #[serde(default)]
    pub is_simulator: bool,
    #[serde(default)]
    pub qubits: Option<usize>,
    #[serde(default)]
    pub clops: Option<Clops>,
    #[serde(default)]
    pub processor_type: Option<ProcessorType>,
    #[serde(default)]
    pub queue_length: usize,
    #[serde(default)]
    pub wait_time_seconds: Option<WaitTimeSeconds>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BackendStatus {
    pub name: String,
    #[serde(default)]
    pub reason: Option<String>,
}

impl BackendStatus {
    #[must_use]
    pub fn is_operational(&self) -> bool {
        self.name.eq_ignore_ascii_case("online")
    }

    #[must_use]
    pub fn message(&self) -> &str {
        self.reason.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessorType {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub segment: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Clops {
    #[serde(rename = "type")]
    pub kind: String,
    pub value: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WaitTimeSeconds {
    pub average: i32,
    pub p50: i32,
    pub p95: i32,
}

#[derive(Debug, Clone)]
pub struct IBMBackendInfo {
    pub name: String,
    pub num_qubits: usize,
    pub status: BackendStatus,
    pub queue_length: usize,
    pub is_simulator: bool,
    pub processor_type: Option<ProcessorType>,
    pub clops: Option<Clops>,
    pub wait_time_seconds: Option<WaitTimeSeconds>,
    pub basis_gates: Vec<String>,
    pub coupling_map: Option<Vec<[usize; 2]>>,
    pub max_shots: Option<u32>,
    pub max_experiments: Option<usize>,
    pub supported_features: Vec<String>,
    pub supported_instructions: Vec<String>,
    pub conditional: bool,
    pub memory: bool,
}

impl From<IBMBackendSummary> for IBMBackendInfo {
    fn from(summary: IBMBackendSummary) -> Self {
        Self {
            name: summary.name,
            num_qubits: summary.qubits.unwrap_or_default(),
            status: summary.status,
            queue_length: summary.queue_length,
            is_simulator: summary.is_simulator,
            processor_type: summary.processor_type,
            clops: summary.clops,
            wait_time_seconds: summary.wait_time_seconds,
            basis_gates: Vec::new(),
            coupling_map: None,
            max_shots: None,
            max_experiments: None,
            supported_features: Vec::new(),
            supported_instructions: Vec::new(),
            conditional: false,
            memory: false,
        }
    }
}

impl IBMBackendInfo {
    pub(crate) fn apply_configuration(&mut self, configuration: &IBMBackendConfiguration) {
        self.num_qubits = configuration.n_qubits;
        self.basis_gates = configuration.basis_gates.clone();
        self.coupling_map = Some(configuration.coupling_pairs());
        self.max_shots = configuration
            .max_shots
            .and_then(|value| u32::try_from(value).ok());
        self.max_experiments = configuration
            .max_experiments
            .and_then(|value| usize::try_from(value).ok());
        self.supported_features = configuration.supported_features.clone().unwrap_or_default();
        self.supported_instructions = configuration
            .supported_instructions
            .clone()
            .unwrap_or_default();
        self.conditional = configuration.conditional;
        self.memory = configuration.memory;
        if let Some(processor_type) = configuration.processor_type.clone() {
            self.processor_type = Some(processor_type);
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct IBMBackendConfiguration {
    pub backend_name: String,
    pub basis_gates: Vec<String>,
    pub coupling_map: Vec<Vec<usize>>,
    pub n_qubits: usize,
    #[serde(default)]
    pub max_shots: Option<u64>,
    #[serde(default)]
    pub max_experiments: Option<u64>,
    #[serde(default)]
    pub supported_features: Option<Vec<String>>,
    #[serde(default)]
    pub supported_instructions: Option<Vec<String>>,
    #[serde(default)]
    pub conditional: bool,
    #[serde(default)]
    pub memory: bool,
    #[serde(default)]
    pub processor_type: Option<ProcessorType>,
}

impl IBMBackendConfiguration {
    #[must_use]
    pub fn coupling_pairs(&self) -> Vec<[usize; 2]> {
        self.coupling_map
            .iter()
            .filter_map(|pair| match pair.as_slice() {
                [lhs, rhs, ..] => Some([*lhs, *rhs]),
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobCreateRequest {
    pub program_id: &'static str,
    pub backend: String,
    pub params: SamplerJobParams,
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplerJobParams {
    pub pubs: Vec<SamplerPub>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SamplerOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shots: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_qiskit: Option<bool>,
    pub version: u8,
}

#[derive(Debug, Clone)]
pub struct SamplerPub {
    pub circuit: String,
    pub shots: Option<u32>,
}

impl SamplerPub {
    #[must_use]
    pub fn new(circuit: String, shots: Option<u32>) -> Self {
        Self { circuit, shots }
    }
}

impl Serialize for SamplerPub {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let len = if self.shots.is_some() { 3 } else { 1 };
        let mut sequence = serializer.serialize_seq(Some(len))?;
        sequence.serialize_element(&self.circuit)?;
        if let Some(shots) = self.shots {
            sequence.serialize_element(&Option::<Value>::None)?;
            sequence.serialize_element(&shots)?;
        }
        sequence.end()
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SamplerOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamical_decoupling: Option<DynamicalDecouplingOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twirling: Option<TwirlingOptions>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DynamicalDecouplingOptions {
    pub enable: bool,
    pub sequence_type: String,
    pub extra_slack_distribution: String,
    pub scheduling_method: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TwirlingOptions {
    pub enable_gates: bool,
    pub enable_measure: bool,
    pub num_randomizations: String,
    pub shots_per_randomization: String,
    pub strategy: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobCreateResponse {
    pub id: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub private: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobState {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub reason_code: Option<i32>,
    #[serde(default)]
    pub reason_solution: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobInfoResponse {
    pub id: String,
    #[serde(default)]
    pub backend: Option<String>,
    pub status: String,
    pub state: JobState,
    pub created: String,
    #[serde(default)]
    pub estimated_running_time_seconds: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IBMJobResultEnvelope {
    pub results: Vec<IBMPrimitiveResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IBMPrimitiveResult {
    pub data: Value,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IAMTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub expiration: Option<u64>,
}
