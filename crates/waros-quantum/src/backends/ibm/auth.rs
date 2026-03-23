use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::WarosError;
use crate::WarosResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCredentials {
    pub api_key: String,
    pub instance_crn: String,
}

pub fn resolve_credentials(
    explicit_api_key: Option<&str>,
    explicit_instance_crn: Option<&str>,
) -> WarosResult<ResolvedCredentials> {
    resolve_credentials_with_home(explicit_api_key, explicit_instance_crn, dirs::home_dir())
}

pub fn resolve_token(explicit_api_key: Option<&str>) -> WarosResult<String> {
    resolve_api_key(explicit_api_key, dirs::home_dir())
}

pub fn resolve_instance(explicit_instance_crn: Option<&str>) -> WarosResult<String> {
    resolve_instance_crn(explicit_instance_crn, dirs::home_dir())
}

pub fn save_token(api_key: &str) -> WarosResult<()> {
    save_value("ibm_token", api_key, dirs::home_dir())
}

pub fn save_instance(instance_crn: &str) -> WarosResult<()> {
    save_value("ibm_instance_crn", instance_crn, dirs::home_dir())
}

pub fn save_credentials(api_key: &str, instance_crn: &str) -> WarosResult<()> {
    save_token(api_key)?;
    save_instance(instance_crn)?;
    Ok(())
}

pub(crate) fn resolve_credentials_with_home(
    explicit_api_key: Option<&str>,
    explicit_instance_crn: Option<&str>,
    home_dir: Option<PathBuf>,
) -> WarosResult<ResolvedCredentials> {
    Ok(ResolvedCredentials {
        api_key: resolve_api_key(explicit_api_key, home_dir.clone())?,
        instance_crn: resolve_instance_crn(explicit_instance_crn, home_dir)?,
    })
}

fn resolve_api_key(
    explicit_api_key: Option<&str>,
    home_dir: Option<PathBuf>,
) -> WarosResult<String> {
    if let Some(api_key) = sanitize(explicit_api_key) {
        return Ok(api_key);
    }

    for variable in [
        "WAROS_IBM_TOKEN",
        "WAROS_IBM_API_KEY",
        "IQP_API_TOKEN",
        "QISKIT_IBM_TOKEN",
    ] {
        if let Some(value) = env_var(variable) {
            return Ok(value);
        }
    }

    if let Some(value) = read_waros_value("ibm_token", home_dir.clone())? {
        return Ok(value);
    }

    if let Some(credentials) = read_qiskit_credentials(home_dir)? {
        if !credentials.api_key.is_empty() {
            return Ok(credentials.api_key);
        }
    }

    Err(WarosError::AuthError(
        "IBM Quantum API key not found. Set WAROS_IBM_TOKEN or QISKIT_IBM_TOKEN, save credentials with IBMBackend.save_credentials(), or pass token directly.".into(),
    ))
}

fn resolve_instance_crn(
    explicit_instance_crn: Option<&str>,
    home_dir: Option<PathBuf>,
) -> WarosResult<String> {
    if let Some(instance_crn) = sanitize(explicit_instance_crn) {
        return Ok(instance_crn);
    }

    for variable in [
        "WAROS_IBM_INSTANCE_CRN",
        "WAROS_IBM_CRN",
        "QISKIT_IBM_INSTANCE",
    ] {
        if let Some(value) = env_var(variable) {
            return Ok(value);
        }
    }

    if let Some(value) = read_waros_value("ibm_instance_crn", home_dir.clone())? {
        return Ok(value);
    }

    if let Some(credentials) = read_qiskit_credentials(home_dir)? {
        if !credentials.instance_crn.is_empty() {
            return Ok(credentials.instance_crn);
        }
    }

    Err(WarosError::AuthError(
        "IBM Quantum service instance CRN not found. Set WAROS_IBM_INSTANCE_CRN or QISKIT_IBM_INSTANCE, save credentials with IBMBackend.save_credentials(), or pass instance_crn directly.".into(),
    ))
}

fn save_value(file_name: &str, value: &str, home_dir: Option<PathBuf>) -> WarosResult<()> {
    let base = waros_dir(home_dir)?;
    fs::create_dir_all(&base).map_err(io_error)?;
    fs::write(base.join(file_name), value.trim()).map_err(io_error)?;
    Ok(())
}

fn waros_dir(home_dir: Option<PathBuf>) -> WarosResult<PathBuf> {
    home_dir
        .ok_or_else(|| WarosError::IOError("Cannot determine home directory".into()))
        .map(|path| path.join(".waros"))
}

fn read_waros_value(file_name: &str, home_dir: Option<PathBuf>) -> WarosResult<Option<String>> {
    let Some(home_dir) = home_dir else {
        return Ok(None);
    };
    let path = home_dir.join(".waros").join(file_name);
    read_text_if_exists(&path)
}

fn read_qiskit_credentials(home_dir: Option<PathBuf>) -> WarosResult<Option<ResolvedCredentials>> {
    let Some(home_dir) = home_dir else {
        return Ok(None);
    };
    let path = home_dir.join(".qiskit").join("qiskit-ibm.json");
    let Some(content) = read_text_if_exists(&path)? else {
        return Ok(None);
    };
    let json: Value = serde_json::from_str(&content)
        .map_err(|error| WarosError::ParseError(error.to_string()))?;
    Ok(find_qiskit_credentials(&json))
}

fn find_qiskit_credentials(value: &Value) -> Option<ResolvedCredentials> {
    match value {
        Value::Object(map) => {
            let token = map
                .get("token")
                .and_then(Value::as_str)
                .and_then(|value| sanitize(Some(value)));
            let instance = map
                .get("instance")
                .or_else(|| map.get("crn"))
                .and_then(Value::as_str)
                .and_then(|value| sanitize(Some(value)));
            let channel = map
                .get("channel")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default();

            if let Some(api_key) = token {
                let valid_channel =
                    channel.is_empty() || matches!(channel, "ibm_cloud" | "ibm_quantum_platform");
                if valid_channel {
                    return Some(ResolvedCredentials {
                        api_key,
                        instance_crn: instance.unwrap_or_default(),
                    });
                }
            }

            map.values().find_map(find_qiskit_credentials)
        }
        Value::Array(values) => values.iter().find_map(find_qiskit_credentials),
        _ => None,
    }
}

fn read_text_if_exists(path: &Path) -> WarosResult<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path).map_err(io_error)?;
    Ok(sanitize(Some(&content)))
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(|value| sanitize(Some(&value)))
}

fn sanitize(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn io_error(error: std::io::Error) -> WarosError {
    WarosError::IOError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_home() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock available")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("waros-ibm-auth-{nonce}"));
        fs::create_dir_all(&path).expect("temp home created");
        path
    }

    #[test]
    fn explicit_token_overrides_environment() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("WAROS_IBM_TOKEN", "env-token");
        let token = resolve_api_key(Some("explicit-token"), None).expect("token resolves");
        std::env::remove_var("WAROS_IBM_TOKEN");
        assert_eq!(token, "explicit-token");
    }

    #[test]
    fn resolve_token_from_environment() {
        let _guard = env_lock().lock().expect("env lock");
        std::env::set_var("WAROS_IBM_TOKEN", "test-token-123");
        let token = resolve_api_key(None, None).expect("token resolves");
        std::env::remove_var("WAROS_IBM_TOKEN");
        assert_eq!(token, "test-token-123");
    }

    #[test]
    fn save_and_load_waros_credentials() {
        let home = temp_home();
        save_value("ibm_token", "token-1", Some(home.clone())).expect("token saved");
        save_value(
            "ibm_instance_crn",
            "crn:v1:bluemix:public:quantum",
            Some(home.clone()),
        )
        .expect("instance saved");

        let credentials =
            resolve_credentials_with_home(None, None, Some(home)).expect("credentials resolve");
        assert_eq!(credentials.api_key, "token-1");
        assert_eq!(credentials.instance_crn, "crn:v1:bluemix:public:quantum");
    }

    #[test]
    fn read_qiskit_credentials_from_nested_json() {
        let home = temp_home();
        let qiskit_dir = home.join(".qiskit");
        fs::create_dir_all(&qiskit_dir).expect("qiskit dir created");
        fs::write(
            qiskit_dir.join("qiskit-ibm.json"),
            r#"{
  "default-ibm-quantum-platform": {
    "channel": "ibm_quantum_platform",
    "token": "qiskit-token",
    "instance": "qiskit-crn"
  }
}"#,
        )
        .expect("qiskit file written");

        let credentials = read_qiskit_credentials(Some(home))
            .expect("qiskit read succeeds")
            .expect("credentials found");
        assert_eq!(credentials.api_key, "qiskit-token");
        assert_eq!(credentials.instance_crn, "qiskit-crn");
    }
}
