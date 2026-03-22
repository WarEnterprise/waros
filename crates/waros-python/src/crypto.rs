use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyModule};

use waros_crypto::kem::SecurityLevel;
use waros_crypto::sign::SignatureScheme;

use crate::{runtime_error, value_error};

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(kem_keygen, module)?)?;
    module.add_function(wrap_pyfunction!(kem_encapsulate, module)?)?;
    module.add_function(wrap_pyfunction!(kem_decapsulate, module)?)?;
    module.add_function(wrap_pyfunction!(sign_keygen, module)?)?;
    module.add_function(wrap_pyfunction!(sign, module)?)?;
    module.add_function(wrap_pyfunction!(verify, module)?)?;
    module.add_function(wrap_pyfunction!(sha3_256, module)?)?;
    module.add_function(wrap_pyfunction!(sha3_512, module)?)?;
    module.add_function(wrap_pyfunction!(shake128, module)?)?;
    module.add_function(wrap_pyfunction!(shake256, module)?)?;
    module.add_function(wrap_pyfunction!(random_bytes, module)?)?;
    module.add_function(wrap_pyfunction!(random_bits, module)?)?;
    module.add_function(wrap_pyfunction!(random_seed, module)?)?;
    module.add_function(wrap_pyfunction!(random_u64, module)?)?;
    Ok(())
}

#[pyfunction]
#[pyo3(signature = (level=None))]
fn kem_keygen(py: Python<'_>, level: Option<&str>) -> PyResult<(Py<PyBytes>, Py<PyBytes>)> {
    let level = parse_security_level(level)?;
    let (public_key, secret_key) = waros_crypto::kem::keygen_with_level(level);
    Ok((
        PyBytes::new_bound(py, &public_key.to_bytes()).unbind(),
        PyBytes::new_bound(py, &secret_key.to_bytes()).unbind(),
    ))
}

#[pyfunction]
fn kem_encapsulate(py: Python<'_>, public_key: &[u8]) -> PyResult<(Py<PyBytes>, Py<PyBytes>)> {
    let public_key =
        waros_crypto::kem::PublicKey::from_serialized(public_key).map_err(value_error)?;
    let (ciphertext, shared_secret) = waros_crypto::kem::encapsulate(&public_key);
    Ok((
        PyBytes::new_bound(py, &ciphertext.to_bytes()).unbind(),
        PyBytes::new_bound(py, shared_secret.as_bytes()).unbind(),
    ))
}

#[pyfunction]
fn kem_decapsulate(py: Python<'_>, secret_key: &[u8], ciphertext: &[u8]) -> PyResult<Py<PyBytes>> {
    let secret_key =
        waros_crypto::kem::SecretKey::from_serialized(secret_key).map_err(value_error)?;
    let ciphertext =
        waros_crypto::kem::Ciphertext::from_serialized(ciphertext).map_err(value_error)?;
    let shared_secret =
        waros_crypto::kem::decapsulate(&secret_key, &ciphertext).map_err(runtime_error)?;
    Ok(PyBytes::new_bound(py, shared_secret.as_bytes()).unbind())
}

#[pyfunction]
#[pyo3(signature = (scheme=None))]
fn sign_keygen(py: Python<'_>, scheme: Option<&str>) -> PyResult<(Py<PyBytes>, Py<PyBytes>)> {
    let scheme = parse_signature_scheme(scheme)?;
    let (public_key, secret_key) = waros_crypto::sign::keygen_with_scheme(scheme);
    Ok((
        PyBytes::new_bound(py, &public_key.to_bytes()).unbind(),
        PyBytes::new_bound(py, &secret_key.to_bytes()).unbind(),
    ))
}

#[pyfunction]
fn sign(py: Python<'_>, secret_key: &[u8], message: &[u8]) -> PyResult<Py<PyBytes>> {
    let secret_key =
        waros_crypto::sign::SignSecretKey::from_serialized(secret_key).map_err(value_error)?;
    let signature = waros_crypto::sign::sign(&secret_key, message);
    Ok(PyBytes::new_bound(py, &signature.to_bytes()).unbind())
}

#[pyfunction]
fn verify(public_key: &[u8], message: &[u8], signature: &[u8]) -> PyResult<bool> {
    let public_key =
        waros_crypto::sign::SignPublicKey::from_serialized(public_key).map_err(value_error)?;
    let signature =
        waros_crypto::sign::Signature::from_serialized(signature).map_err(value_error)?;
    Ok(waros_crypto::sign::verify(&public_key, message, &signature))
}

#[pyfunction]
fn sha3_256(py: Python<'_>, data: &[u8]) -> Py<PyBytes> {
    let digest = waros_crypto::hash::sha3_256(data);
    PyBytes::new_bound(py, &digest).unbind()
}

#[pyfunction]
fn sha3_512(py: Python<'_>, data: &[u8]) -> Py<PyBytes> {
    let digest = waros_crypto::hash::sha3_512(data);
    PyBytes::new_bound(py, &digest).unbind()
}

#[pyfunction]
fn shake128(py: Python<'_>, data: &[u8], output_len: usize) -> Py<PyBytes> {
    let digest = waros_crypto::hash::shake128(data, output_len);
    PyBytes::new_bound(py, &digest).unbind()
}

#[pyfunction]
fn shake256(py: Python<'_>, data: &[u8], output_len: usize) -> Py<PyBytes> {
    let digest = waros_crypto::hash::shake256(data, output_len);
    PyBytes::new_bound(py, &digest).unbind()
}

#[pyfunction]
fn random_bytes(py: Python<'_>, n: usize) -> Py<PyBytes> {
    let bytes = waros_crypto::qrng::random_bytes(n);
    PyBytes::new_bound(py, &bytes).unbind()
}

#[pyfunction]
fn random_bits(n: usize) -> Vec<bool> {
    waros_crypto::qrng::random_bits(n)
}

#[pyfunction]
fn random_seed(py: Python<'_>) -> Py<PyBytes> {
    let seed = waros_crypto::qrng::random_seed();
    PyBytes::new_bound(py, &seed).unbind()
}

#[pyfunction]
fn random_u64() -> u64 {
    waros_crypto::qrng::random_u64()
}

fn parse_security_level(level: Option<&str>) -> PyResult<SecurityLevel> {
    match level.map(str::trim).map(str::to_ascii_lowercase) {
        None => Ok(SecurityLevel::Level3),
        Some(value)
            if matches!(
                value.as_str(),
                "1" | "level1" | "level_1" | "ml-kem-512" | "mlkem512"
            ) =>
        {
            Ok(SecurityLevel::Level1)
        }
        Some(value)
            if matches!(
                value.as_str(),
                "3" | "level3" | "level_3" | "ml-kem-768" | "mlkem768"
            ) =>
        {
            Ok(SecurityLevel::Level3)
        }
        Some(value)
            if matches!(
                value.as_str(),
                "5" | "level5" | "level_5" | "ml-kem-1024" | "mlkem1024"
            ) =>
        {
            Ok(SecurityLevel::Level5)
        }
        Some(value) => Err(value_error(format!(
            "unknown ML-KEM level '{value}'; expected level1, level3, or level5"
        ))),
    }
}

fn parse_signature_scheme(scheme: Option<&str>) -> PyResult<SignatureScheme> {
    match scheme.map(str::trim).map(str::to_ascii_lowercase) {
        None => Ok(SignatureScheme::MlDsa),
        Some(value) if matches!(value.as_str(), "ml_dsa" | "ml-dsa" | "mldsa" | "dilithium") => {
            Ok(SignatureScheme::MlDsa)
        }
        Some(value)
            if matches!(
                value.as_str(),
                "slh_dsa" | "slh-dsa" | "slhdsa" | "sphincs" | "sphincs+"
            ) =>
        {
            Ok(SignatureScheme::SlhDsa)
        }
        Some(value) => Err(value_error(format!(
            "unknown signature scheme '{value}'; expected ml_dsa or slh_dsa"
        ))),
    }
}
