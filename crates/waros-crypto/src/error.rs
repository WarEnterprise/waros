use thiserror::Error;

/// Result alias for WarOS cryptography APIs.
pub type CryptoResult<T> = Result<T, CryptoError>;

/// Errors returned by the cryptography modules.
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Invalid or mismatched key material was supplied.
    #[error("invalid key material: {0}")]
    InvalidKeyMaterial(String),
    /// A ciphertext integrity check failed after decapsulation.
    #[error("ciphertext integrity check failed")]
    IntegrityCheckFailed,
}
