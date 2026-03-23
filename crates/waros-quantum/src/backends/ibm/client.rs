use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;

use crate::backends::ibm::auth::ResolvedCredentials;
use crate::error::WarosError;
use crate::WarosResult;

use super::types::{
    IAMTokenResponse, IBMBackendConfiguration, IBMBackendListResponse, IBMBackendSummary,
    JobCreateRequest, JobCreateResponse, JobInfoResponse,
};

const IAM_TOKEN_URL: &str = "https://iam.cloud.ibm.com/identity/token";

pub struct IBMClient {
    client: Client,
    credentials: ResolvedCredentials,
    base_url: String,
    api_version: String,
    bearer_token: Mutex<Option<CachedBearerToken>>,
}

#[derive(Debug, Clone)]
struct CachedBearerToken {
    value: String,
    expires_at: SystemTime,
}

impl IBMClient {
    pub fn new(
        credentials: ResolvedCredentials,
        base_url: String,
        api_version: String,
        timeout: Duration,
    ) -> WarosResult<Self> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| WarosError::NetworkError(error.to_string()))?;

        Ok(Self {
            client,
            credentials,
            base_url,
            api_version,
            bearer_token: Mutex::new(None),
        })
    }

    pub fn list_backends(&self) -> WarosResult<Vec<IBMBackendSummary>> {
        let response: IBMBackendListResponse =
            self.get("/api/v1/backends?fields=wait_time_seconds")?;
        Ok(response.devices)
    }

    pub fn get_backend_configuration(&self, backend: &str) -> WarosResult<IBMBackendConfiguration> {
        self.get(&format!("/api/v1/backends/{backend}/configuration"))
    }

    pub fn submit_job(&self, request: &JobCreateRequest) -> WarosResult<JobCreateResponse> {
        self.post_json("/api/v1/jobs", request)
    }

    pub fn get_job(&self, job_id: &str) -> WarosResult<JobInfoResponse> {
        self.get(&format!("/api/v1/jobs/{job_id}"))
    }

    pub fn get_job_results(&self, job_id: &str) -> WarosResult<serde_json::Value> {
        let response = self.runtime_request(
            self.client
                .get(format!("{}/api/v1/jobs/{job_id}/results", self.base_url)),
        )?;
        let body = response
            .text()
            .map_err(|error| WarosError::NetworkError(error.to_string()))?;
        serde_json::from_str(&body).map_err(|error| {
            WarosError::ParseError(format!(
                "Unable to parse IBM job result payload: {error}. Raw body: {body}"
            ))
        })
    }

    fn get<T>(&self, path: &str) -> WarosResult<T>
    where
        T: DeserializeOwned,
    {
        let response =
            self.runtime_request(self.client.get(format!("{}{}", self.base_url, path)))?;
        response
            .json()
            .map_err(|error| WarosError::ParseError(error.to_string()))
    }

    fn post_json<T, B>(&self, path: &str, body: &B) -> WarosResult<T>
    where
        T: DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let response = self.runtime_request(
            self.client
                .post(format!("{}{}", self.base_url, path))
                .header(CONTENT_TYPE, "application/json")
                .json(body),
        )?;
        response
            .json()
            .map_err(|error| WarosError::ParseError(error.to_string()))
    }

    fn runtime_request(&self, request: RequestBuilder) -> WarosResult<Response> {
        let token = self.bearer_token()?;
        let response = request
            .header(ACCEPT, "application/json")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header("Service-CRN", &self.credentials.instance_crn)
            .header("IBM-API-Version", &self.api_version)
            .send()
            .map_err(|error| WarosError::NetworkError(error.to_string()))?;

        if response.status().is_success() {
            return Ok(response);
        }

        Err(map_runtime_error(response))
    }

    fn bearer_token(&self) -> WarosResult<String> {
        let mut cache = self
            .bearer_token
            .lock()
            .map_err(|_| WarosError::AuthError("IBM auth cache poisoned".into()))?;

        let now = SystemTime::now();
        if let Some(token) = cache.as_ref() {
            let refresh_deadline = token
                .expires_at
                .checked_sub(Duration::from_secs(60))
                .unwrap_or(token.expires_at);
            if now < refresh_deadline {
                return Ok(token.value.clone());
            }
        }

        let refreshed = self.fetch_bearer_token()?;
        let token = refreshed.value.clone();
        *cache = Some(refreshed);
        Ok(token)
    }

    fn fetch_bearer_token(&self) -> WarosResult<CachedBearerToken> {
        let response = self
            .client
            .post(IAM_TOKEN_URL)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .form(&[
                ("grant_type", "urn:ibm:params:oauth:grant-type:apikey"),
                ("apikey", self.credentials.api_key.as_str()),
            ])
            .send()
            .map_err(|error| WarosError::NetworkError(error.to_string()))?;

        if !response.status().is_success() {
            return Err(map_iam_error(response));
        }

        let payload: IAMTokenResponse = response
            .json()
            .map_err(|error| WarosError::ParseError(error.to_string()))?;
        let expires_at = payload
            .expiration
            .map(|epoch| UNIX_EPOCH + Duration::from_secs(epoch))
            .unwrap_or_else(|| SystemTime::now() + Duration::from_secs(payload.expires_in));

        Ok(CachedBearerToken {
            value: payload.access_token,
            expires_at,
        })
    }
}

fn map_runtime_error(response: Response) -> WarosError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response.text().unwrap_or_default();

    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => WarosError::AuthError(format!(
            "IBM Runtime authentication failed ({status}): {body}"
        )),
        StatusCode::TOO_MANY_REQUESTS => {
            let suffix = retry_after
                .map(|value| format!(" Retry-After: {value}."))
                .unwrap_or_default();
            WarosError::APIError(format!(
                "IBM Runtime rate limit exceeded ({status}).{suffix} {body}"
            ))
        }
        _ => WarosError::APIError(format!("IBM Runtime error ({status}): {body}")),
    }
}

fn map_iam_error(response: Response) -> WarosError {
    let status = response.status();
    let body = response.text().unwrap_or_default();
    WarosError::AuthError(format!("IBM IAM token exchange failed ({status}): {body}"))
}
