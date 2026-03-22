use pqcrypto_mlkem::{mlkem1024, mlkem512, mlkem768};
use pqcrypto_traits::kem::{Ciphertext as _, PublicKey as _, SecretKey as _, SharedSecret as _};

use crate::error::{CryptoError, CryptoResult};
use crate::hash;

/// Security levels matching the standardized ML-KEM parameter sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityLevel {
    /// ML-KEM-512 (NIST level 1).
    Level1,
    /// ML-KEM-768 (NIST level 3).
    Level3,
    /// ML-KEM-1024 (NIST level 5).
    Level5,
}

#[derive(Clone)]
enum PublicKeyKind {
    Level1(mlkem512::PublicKey),
    Level3(mlkem768::PublicKey),
    Level5(mlkem1024::PublicKey),
}

#[derive(Clone)]
enum SecretKeyKind {
    Level1(mlkem512::SecretKey),
    Level3(mlkem768::SecretKey),
    Level5(mlkem1024::SecretKey),
}

#[derive(Clone)]
enum CiphertextKind {
    Level1(mlkem512::Ciphertext),
    Level3(mlkem768::Ciphertext),
    Level5(mlkem1024::Ciphertext),
}

/// ML-KEM public key.
#[derive(Clone)]
pub struct PublicKey {
    kind: PublicKeyKind,
}

/// ML-KEM secret key.
#[derive(Clone)]
pub struct SecretKey {
    kind: SecretKeyKind,
}

/// ML-KEM ciphertext with an integrity tag used by the wrapper API.
#[derive(Clone)]
pub struct Ciphertext {
    kind: CiphertextKind,
    tag: [u8; 32],
}

/// 32-byte ML-KEM shared secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSecret([u8; 32]);

impl PublicKey {
    /// Return the parameter set used by this key.
    #[must_use]
    pub fn security_level(&self) -> SecurityLevel {
        match self.kind {
            PublicKeyKind::Level1(_) => SecurityLevel::Level1,
            PublicKeyKind::Level3(_) => SecurityLevel::Level3,
            PublicKeyKind::Level5(_) => SecurityLevel::Level5,
        }
    }

    /// Return the serialized key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.kind {
            PublicKeyKind::Level1(key) => key.as_bytes(),
            PublicKeyKind::Level3(key) => key.as_bytes(),
            PublicKeyKind::Level5(key) => key.as_bytes(),
        }
    }

    /// Deserialize a public key from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError`] if the byte slice is invalid.
    pub fn from_bytes(level: SecurityLevel, bytes: &[u8]) -> CryptoResult<Self> {
        Ok(Self {
            kind: match level {
                SecurityLevel::Level1 => PublicKeyKind::Level1(
                    mlkem512::PublicKey::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level3 => PublicKeyKind::Level3(
                    mlkem768::PublicKey::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level5 => PublicKeyKind::Level5(
                    mlkem1024::PublicKey::from_bytes(bytes).map_err(key_error)?,
                ),
            },
        })
    }
}

impl SecretKey {
    /// Return the parameter set used by this key.
    #[must_use]
    pub fn security_level(&self) -> SecurityLevel {
        match self.kind {
            SecretKeyKind::Level1(_) => SecurityLevel::Level1,
            SecretKeyKind::Level3(_) => SecurityLevel::Level3,
            SecretKeyKind::Level5(_) => SecurityLevel::Level5,
        }
    }

    /// Return the serialized key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.kind {
            SecretKeyKind::Level1(key) => key.as_bytes(),
            SecretKeyKind::Level3(key) => key.as_bytes(),
            SecretKeyKind::Level5(key) => key.as_bytes(),
        }
    }

    /// Deserialize a secret key from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError`] if the byte slice is invalid.
    pub fn from_bytes(level: SecurityLevel, bytes: &[u8]) -> CryptoResult<Self> {
        Ok(Self {
            kind: match level {
                SecurityLevel::Level1 => SecretKeyKind::Level1(
                    mlkem512::SecretKey::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level3 => SecretKeyKind::Level3(
                    mlkem768::SecretKey::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level5 => SecretKeyKind::Level5(
                    mlkem1024::SecretKey::from_bytes(bytes).map_err(key_error)?,
                ),
            },
        })
    }
}

impl Ciphertext {
    /// Return the parameter set used by this ciphertext.
    #[must_use]
    pub fn security_level(&self) -> SecurityLevel {
        match self.kind {
            CiphertextKind::Level1(_) => SecurityLevel::Level1,
            CiphertextKind::Level3(_) => SecurityLevel::Level3,
            CiphertextKind::Level5(_) => SecurityLevel::Level5,
        }
    }

    /// Return the serialized ciphertext bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.kind {
            CiphertextKind::Level1(ciphertext) => ciphertext.as_bytes(),
            CiphertextKind::Level3(ciphertext) => ciphertext.as_bytes(),
            CiphertextKind::Level5(ciphertext) => ciphertext.as_bytes(),
        }
    }

    /// Return the integrity tag associated with this ciphertext.
    #[must_use]
    pub fn integrity_tag(&self) -> [u8; 32] {
        self.tag
    }

    /// Deserialize a ciphertext from bytes and an integrity tag.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError`] if the byte slice is invalid.
    pub fn from_bytes(level: SecurityLevel, bytes: &[u8], tag: [u8; 32]) -> CryptoResult<Self> {
        Ok(Self {
            kind: match level {
                SecurityLevel::Level1 => CiphertextKind::Level1(
                    mlkem512::Ciphertext::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level3 => CiphertextKind::Level3(
                    mlkem768::Ciphertext::from_bytes(bytes).map_err(key_error)?,
                ),
                SecurityLevel::Level5 => CiphertextKind::Level5(
                    mlkem1024::Ciphertext::from_bytes(bytes).map_err(key_error)?,
                ),
            },
            tag,
        })
    }
}

impl SharedSecret {
    /// Return the shared secret bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Generate an ML-KEM keypair using the recommended level-3 parameter set.
#[must_use]
pub fn keygen() -> (PublicKey, SecretKey) {
    keygen_with_level(SecurityLevel::Level3)
}

/// Generate an ML-KEM keypair for the requested security level.
#[must_use]
pub fn keygen_with_level(level: SecurityLevel) -> (PublicKey, SecretKey) {
    match level {
        SecurityLevel::Level1 => {
            let (public_key, secret_key) = mlkem512::keypair();
            (
                PublicKey {
                    kind: PublicKeyKind::Level1(public_key),
                },
                SecretKey {
                    kind: SecretKeyKind::Level1(secret_key),
                },
            )
        }
        SecurityLevel::Level3 => {
            let (public_key, secret_key) = mlkem768::keypair();
            (
                PublicKey {
                    kind: PublicKeyKind::Level3(public_key),
                },
                SecretKey {
                    kind: SecretKeyKind::Level3(secret_key),
                },
            )
        }
        SecurityLevel::Level5 => {
            let (public_key, secret_key) = mlkem1024::keypair();
            (
                PublicKey {
                    kind: PublicKeyKind::Level5(public_key),
                },
                SecretKey {
                    kind: SecretKeyKind::Level5(secret_key),
                },
            )
        }
    }
}

/// Encapsulate a fresh shared secret to `pk`.
#[must_use]
pub fn encapsulate(pk: &PublicKey) -> (Ciphertext, SharedSecret) {
    match &pk.kind {
        PublicKeyKind::Level1(public_key) => {
            let (shared_secret, ciphertext) = mlkem512::encapsulate(public_key);
            let shared_secret = SharedSecret(copy_shared_secret(shared_secret.as_bytes()));
            let tag = integrity_tag(SecurityLevel::Level1, &shared_secret, ciphertext.as_bytes());
            (
                Ciphertext {
                    kind: CiphertextKind::Level1(ciphertext),
                    tag,
                },
                shared_secret,
            )
        }
        PublicKeyKind::Level3(public_key) => {
            let (shared_secret, ciphertext) = mlkem768::encapsulate(public_key);
            let shared_secret = SharedSecret(copy_shared_secret(shared_secret.as_bytes()));
            let tag = integrity_tag(SecurityLevel::Level3, &shared_secret, ciphertext.as_bytes());
            (
                Ciphertext {
                    kind: CiphertextKind::Level3(ciphertext),
                    tag,
                },
                shared_secret,
            )
        }
        PublicKeyKind::Level5(public_key) => {
            let (shared_secret, ciphertext) = mlkem1024::encapsulate(public_key);
            let shared_secret = SharedSecret(copy_shared_secret(shared_secret.as_bytes()));
            let tag = integrity_tag(SecurityLevel::Level5, &shared_secret, ciphertext.as_bytes());
            (
                Ciphertext {
                    kind: CiphertextKind::Level5(ciphertext),
                    tag,
                },
                shared_secret,
            )
        }
    }
}

/// Decapsulate `ct` with `sk`.
///
/// # Errors
///
/// Returns [`CryptoError`] when the key levels do not match or when the
/// integrity tag does not validate against the recovered shared secret.
pub fn decapsulate(sk: &SecretKey, ct: &Ciphertext) -> CryptoResult<SharedSecret> {
    if sk.security_level() != ct.security_level() {
        return Err(CryptoError::InvalidKeyMaterial(
            "key and ciphertext security levels do not match".into(),
        ));
    }

    let shared_secret = match (&sk.kind, &ct.kind) {
        (SecretKeyKind::Level1(secret_key), CiphertextKind::Level1(ciphertext)) => SharedSecret(
            copy_shared_secret(mlkem512::decapsulate(ciphertext, secret_key).as_bytes()),
        ),
        (SecretKeyKind::Level3(secret_key), CiphertextKind::Level3(ciphertext)) => SharedSecret(
            copy_shared_secret(mlkem768::decapsulate(ciphertext, secret_key).as_bytes()),
        ),
        (SecretKeyKind::Level5(secret_key), CiphertextKind::Level5(ciphertext)) => SharedSecret(
            copy_shared_secret(mlkem1024::decapsulate(ciphertext, secret_key).as_bytes()),
        ),
        _ => {
            return Err(CryptoError::InvalidKeyMaterial(
                "key and ciphertext security levels do not match".into(),
            ))
        }
    };

    let expected_tag = integrity_tag(ct.security_level(), &shared_secret, ct.as_bytes());
    if expected_tag != ct.tag {
        return Err(CryptoError::IntegrityCheckFailed);
    }
    Ok(shared_secret)
}

fn copy_shared_secret(bytes: &[u8]) -> [u8; 32] {
    let mut output = [0u8; 32];
    let count = bytes.len().min(output.len());
    output[..count].copy_from_slice(&bytes[..count]);
    output
}

fn integrity_tag(
    level: SecurityLevel,
    shared_secret: &SharedSecret,
    ciphertext: &[u8],
) -> [u8; 32] {
    let mut buffer = Vec::with_capacity(1 + shared_secret.0.len() + ciphertext.len());
    buffer.push(match level {
        SecurityLevel::Level1 => 1,
        SecurityLevel::Level3 => 3,
        SecurityLevel::Level5 => 5,
    });
    buffer.extend_from_slice(shared_secret.as_bytes());
    buffer.extend_from_slice(ciphertext);
    hash::sha3_256(&buffer)
}

fn key_error(error: impl std::fmt::Display) -> CryptoError {
    CryptoError::InvalidKeyMaterial(error.to_string())
}
