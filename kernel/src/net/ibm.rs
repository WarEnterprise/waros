use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use serde::de::DeserializeOwned;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::auth::{session, FilePermissions};
use crate::fs::{self, FILESYSTEM};
use crate::quantum;

use super::{http_get_with_headers, http_post_with_headers, NetError};

const IBM_IAM_TOKEN_URL: &str = "https://iam.cloud.ibm.com/identity/token";
const IBM_RUNTIME_BASE_URL: &str = "https://quantum.cloud.ibm.com";
const IBM_RUNTIME_API_VERSION: &str = "2026-02-15";
const IBM_API_KEY_PATH: &str = "/etc/ibm_token";
const IBM_INSTANCE_CRN_PATH: &str = "/etc/ibm_instance_crn";

#[derive(Debug, Clone)]
pub enum IBMError {
    MissingApiKey,
    MissingInstanceCrn,
    Filesystem(fs::FsError),
    Network(NetError),
    Parse(String),
    Api(String),
    Quantum(String),
}

impl core::fmt::Display for IBMError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingApiKey => formatter.write_str(
                "IBM Quantum API key not found. Run 'ibm login <api-key> [service-crn]'.",
            ),
            Self::MissingInstanceCrn => formatter.write_str(
                "IBM Quantum service CRN not found. Run 'ibm instance <service-crn>' or pass it to 'ibm login'.",
            ),
            Self::Filesystem(error) => write!(formatter, "{error}"),
            Self::Network(error) => write!(formatter, "{error}"),
            Self::Parse(error) => formatter.write_str(error),
            Self::Api(error) => formatter.write_str(error),
            Self::Quantum(error) => formatter.write_str(error),
        }
    }
}

impl From<fs::FsError> for IBMError {
    fn from(error: fs::FsError) -> Self {
        Self::Filesystem(error)
    }
}

impl From<NetError> for IBMError {
    fn from(error: NetError) -> Self {
        Self::Network(error)
    }
}

#[derive(Debug, Clone)]
pub struct SubmittedJob {
    pub job_id: String,
    pub backend: String,
    pub backend_qubits: usize,
    pub queue_length: usize,
    pub output_bits: usize,
}

#[derive(Debug, Clone)]
pub struct JobStatusSnapshot {
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IBMJobResult {
    pub counts: Vec<(String, u32)>,
    pub total_shots: u32,
}

#[derive(Debug, Clone)]
struct ResolvedCredentials {
    api_key: String,
    instance_crn: String,
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
    pub queue_length: usize,
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

#[derive(Debug, Deserialize)]
struct IBMBackendListResponse {
    devices: Vec<IBMBackendSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct JobCreateRequest {
    program_id: &'static str,
    backend: String,
    params: SamplerJobParams,
}

#[derive(Debug, Clone, Serialize)]
struct SamplerJobParams {
    pubs: Vec<SamplerPub>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<SamplerOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shots: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    support_qiskit: Option<bool>,
    version: u8,
}

#[derive(Debug, Clone)]
struct SamplerPub {
    circuit: String,
    shots: Option<u32>,
}

impl SamplerPub {
    fn new(circuit: String, shots: Option<u32>) -> Self {
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
struct SamplerOptions {}

#[derive(Debug, Clone, Deserialize)]
struct JobCreateResponse {
    id: String,
    #[serde(default)]
    backend: String,
}

#[derive(Debug, Clone, Deserialize)]
struct JobInfoResponse {
    id: String,
    #[serde(default)]
    backend: Option<String>,
    state: JobState,
}

#[derive(Debug, Clone, Deserialize)]
struct JobState {
    status: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    reason_solution: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct IAMTokenResponse {
    access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IBMJobResultEnvelope {
    results: Vec<IBMPrimitiveResult>,
}

#[derive(Debug, Clone, Deserialize)]
struct IBMPrimitiveResult {
    data: Value,
}

pub fn save_api_key(api_key: &str) -> Result<(), IBMError> {
    save_private_value(IBM_API_KEY_PATH, api_key)
}

pub fn save_instance_crn(instance_crn: &str) -> Result<(), IBMError> {
    save_private_value(IBM_INSTANCE_CRN_PATH, instance_crn)
}

#[must_use]
pub fn load_saved_api_key() -> Option<String> {
    read_private_value(IBM_API_KEY_PATH)
}

#[must_use]
pub fn load_saved_instance_crn() -> Option<String> {
    read_private_value(IBM_INSTANCE_CRN_PATH)
}

pub fn list_backends() -> Result<Vec<IBMBackendSummary>, IBMError> {
    let response: IBMBackendListResponse =
        runtime_get_json("/api/v1/backends?fields=wait_time_seconds")?;
    Ok(response.devices)
}

pub fn submit_current_job(backend: &str, shots: u32) -> Result<SubmittedJob, IBMError> {
    let (qasm, output_bits) = quantum::current_ibm_qasm().ok_or_else(|| {
        IBMError::Quantum("No active quantum register. Use qalloc + qrun first.".into())
    })?;

    let backend_info = list_backends()?
        .into_iter()
        .find(|candidate| candidate.name == backend)
        .ok_or_else(|| IBMError::Api(format!("IBM backend '{backend}' not found.")))?;

    let backend_qubits = backend_info.qubits.unwrap_or_default();
    if backend_qubits != 0 && output_bits > backend_qubits {
        return Err(IBMError::Quantum(format!(
            "Circuit requires {} qubits but backend '{}' reports {} qubits.",
            output_bits, backend, backend_qubits
        )));
    }

    let request = JobCreateRequest {
        program_id: "sampler",
        backend: backend.to_string(),
        params: SamplerJobParams {
            pubs: vec![SamplerPub::new(qasm, Some(shots))],
            options: None,
            shots: None,
            support_qiskit: None,
            version: 2,
        },
    };
    let response: JobCreateResponse = runtime_post_json("/api/v1/jobs", &request)?;

    Ok(SubmittedJob {
        job_id: response.id,
        backend: if response.backend.is_empty() {
            backend.to_string()
        } else {
            response.backend
        },
        backend_qubits: backend_qubits.max(output_bits),
        queue_length: backend_info.queue_length,
        output_bits,
    })
}

pub fn job_status(job_id: &str) -> Result<JobStatusSnapshot, IBMError> {
    let info: JobInfoResponse = runtime_get_json(&format!("/api/v1/jobs/{job_id}"))?;
    let mut reason_parts = Vec::new();
    if let Some(reason) = info.state.reason.as_deref() {
        if !reason.trim().is_empty() {
            reason_parts.push(reason.trim().to_string());
        }
    }
    if let Some(solution) = info.state.reason_solution.as_deref() {
        if !solution.trim().is_empty() {
            reason_parts.push(solution.trim().to_string());
        }
    }
    Ok(JobStatusSnapshot {
        status: info.state.status,
        reason: if reason_parts.is_empty() {
            None
        } else {
            Some(reason_parts.join(" "))
        },
    })
}

pub fn job_result(job_id: &str, output_bits: usize) -> Result<IBMJobResult, IBMError> {
    let payload: Value = runtime_get_json(&format!("/api/v1/jobs/{job_id}/results"))?;
    convert_ibm_result(payload, Some(output_bits))
}

fn save_private_value(path: &str, value: &str) -> Result<(), IBMError> {
    let uid = session::current_uid();
    let role = session::current_role();
    let permissions = FilePermissions::private(uid);
    FILESYSTEM
        .lock()
        .write_as(path, value.trim().as_bytes(), uid, role, permissions)?;
    Ok(())
}

fn read_private_value(path: &str) -> Option<String> {
    let uid = session::current_uid();
    let role = session::current_role();
    FILESYSTEM
        .lock()
        .read_as(path, uid, role)
        .ok()
        .and_then(|data| core::str::from_utf8(data).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_credentials() -> Result<ResolvedCredentials, IBMError> {
    Ok(ResolvedCredentials {
        api_key: load_saved_api_key().ok_or(IBMError::MissingApiKey)?,
        instance_crn: load_saved_instance_crn().ok_or(IBMError::MissingInstanceCrn)?,
    })
}

fn fetch_bearer_token(credentials: &ResolvedCredentials) -> Result<String, IBMError> {
    let body = format!(
        "grant_type=urn:ibm:params:oauth:grant-type:apikey&apikey={}",
        percent_encode(&credentials.api_key)
    );
    let response = http_post_with_headers(
        IBM_IAM_TOKEN_URL,
        "application/x-www-form-urlencoded",
        body.as_bytes(),
        &[("Accept", "application/json")],
    )?;
    if response.status_code / 100 != 2 {
        let message = String::from_utf8_lossy(&response.body).trim().to_string();
        return Err(IBMError::Api(format!(
            "IBM IAM token exchange failed (HTTP {}): {}",
            response.status_code, message
        )));
    }
    let payload: IAMTokenResponse =
        serde_json::from_slice(&response.body).map_err(|error| IBMError::Parse(error.to_string()))?;
    Ok(payload.access_token)
}

fn runtime_get_json<T>(path: &str) -> Result<T, IBMError>
where
    T: DeserializeOwned,
{
    let credentials = resolve_credentials()?;
    let bearer = fetch_bearer_token(&credentials)?;
    let authorization = format!("Bearer {bearer}");
    let url = format!("{IBM_RUNTIME_BASE_URL}{path}");
    let response = http_get_with_headers(
        &url,
        &[
            ("Accept", "application/json"),
            ("Authorization", authorization.as_str()),
            ("Service-CRN", credentials.instance_crn.as_str()),
            ("IBM-API-Version", IBM_RUNTIME_API_VERSION),
        ],
    )?;

    decode_json_response(response.status_code, &response.body)
}

fn runtime_post_json<T, B>(path: &str, body: &B) -> Result<T, IBMError>
where
    T: DeserializeOwned,
    B: Serialize + ?Sized,
{
    let credentials = resolve_credentials()?;
    let bearer = fetch_bearer_token(&credentials)?;
    let authorization = format!("Bearer {bearer}");
    let url = format!("{IBM_RUNTIME_BASE_URL}{path}");
    let payload =
        serde_json::to_vec(body).map_err(|error| IBMError::Parse(error.to_string()))?;
    let response = http_post_with_headers(
        &url,
        "application/json",
        &payload,
        &[
            ("Accept", "application/json"),
            ("Authorization", authorization.as_str()),
            ("Service-CRN", credentials.instance_crn.as_str()),
            ("IBM-API-Version", IBM_RUNTIME_API_VERSION),
        ],
    )?;

    decode_json_response(response.status_code, &response.body)
}

fn decode_json_response<T>(status_code: u16, body: &[u8]) -> Result<T, IBMError>
where
    T: DeserializeOwned,
{
    if status_code / 100 != 2 {
        let message = String::from_utf8_lossy(body).trim().to_string();
        return Err(IBMError::Api(format!(
            "IBM Runtime request failed (HTTP {}): {}",
            status_code, message
        )));
    }

    serde_json::from_slice(body).map_err(|error| IBMError::Parse(error.to_string()))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0F));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value & 0x0F {
        0..=9 => (b'0' + (value & 0x0F)) as char,
        10..=15 => (b'A' + ((value & 0x0F) - 10)) as char,
        _ => '0',
    }
}

fn convert_ibm_result(payload: Value, output_bits: Option<usize>) -> Result<IBMJobResult, IBMError> {
    let envelope: IBMJobResultEnvelope =
        serde_json::from_value(payload).map_err(|error| IBMError::Parse(error.to_string()))?;
    let primitive = envelope
        .results
        .first()
        .ok_or_else(|| IBMError::Parse("IBM result payload did not contain any PUB results.".into()))?;

    let counts = if let Some(counts) = extract_counts(&primitive.data, output_bits) {
        counts
    } else {
        let samples = extract_samples(primitive)?;
        let width = output_bits
            .or_else(|| find_num_bits(&primitive.data))
            .unwrap_or_else(|| infer_width(&samples));
        let mut tallied = BTreeMap::new();
        for sample in &samples {
            let state = normalize_ibm_state(sample, width);
            *tallied.entry(state).or_insert(0) += 1;
        }
        tallied
    };

    let total_shots = counts.values().copied().sum();
    Ok(IBMJobResult {
        counts: counts.into_iter().collect(),
        total_shots,
    })
}

fn extract_samples(result: &IBMPrimitiveResult) -> Result<Vec<String>, IBMError> {
    find_samples(&result.data).ok_or_else(|| {
        IBMError::Parse("IBM sampler result did not contain a samples array.".into())
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

fn extract_counts(value: &Value, output_bits: Option<usize>) -> Option<BTreeMap<String, u32>> {
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
                let mut parsed = BTreeMap::new();
                for (state, count) in counts {
                    let count = u32::try_from(count.as_u64()?).ok()?;
                    parsed.insert(normalize_ibm_state(state, width), count);
                }
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
