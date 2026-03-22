use pqcrypto_dilithium::dilithium3;
use pqcrypto_sphincsplus::sphincsshake128fsimple;
use pqcrypto_traits::sign::{DetachedSignature as _, PublicKey as _, SecretKey as _};

use crate::error::{CryptoError, CryptoResult};

/// Signature schemes supported by WarOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureScheme {
    /// ML-DSA / Dilithium.
    MlDsa,
    /// SLH-DSA / SPHINCS+.
    SlhDsa,
}

#[derive(Clone)]
enum SignPublicKeyKind {
    MlDsa(dilithium3::PublicKey),
    SlhDsa(sphincsshake128fsimple::PublicKey),
}

#[derive(Clone)]
enum SignSecretKeyKind {
    MlDsa(dilithium3::SecretKey),
    SlhDsa(sphincsshake128fsimple::SecretKey),
}

/// Public key for post-quantum signatures.
#[derive(Clone)]
pub struct SignPublicKey {
    kind: SignPublicKeyKind,
}

/// Secret key for post-quantum signatures.
#[derive(Clone)]
pub struct SignSecretKey {
    kind: SignSecretKeyKind,
}

/// Detached post-quantum signature.
#[derive(Clone)]
pub struct Signature {
    scheme: SignatureScheme,
    bytes: Vec<u8>,
}

impl SignPublicKey {
    /// Return the signature scheme associated with this key.
    #[must_use]
    pub fn scheme(&self) -> SignatureScheme {
        match self.kind {
            SignPublicKeyKind::MlDsa(_) => SignatureScheme::MlDsa,
            SignPublicKeyKind::SlhDsa(_) => SignatureScheme::SlhDsa,
        }
    }

    /// Return the serialized public key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.kind {
            SignPublicKeyKind::MlDsa(key) => key.as_bytes(),
            SignPublicKeyKind::SlhDsa(key) => key.as_bytes(),
        }
    }
}

impl SignSecretKey {
    /// Return the signature scheme associated with this key.
    #[must_use]
    pub fn scheme(&self) -> SignatureScheme {
        match self.kind {
            SignSecretKeyKind::MlDsa(_) => SignatureScheme::MlDsa,
            SignSecretKeyKind::SlhDsa(_) => SignatureScheme::SlhDsa,
        }
    }

    /// Return the serialized secret key bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.kind {
            SignSecretKeyKind::MlDsa(key) => key.as_bytes(),
            SignSecretKeyKind::SlhDsa(key) => key.as_bytes(),
        }
    }
}

impl Signature {
    /// Return the signature scheme associated with this signature.
    #[must_use]
    pub fn scheme(&self) -> SignatureScheme {
        self.scheme
    }

    /// Return the serialized signature bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Deserialize a signature from bytes.
    ///
    /// # Errors
    ///
    /// Returns [`CryptoError`] if the signature encoding is invalid.
    pub fn from_bytes(scheme: SignatureScheme, bytes: &[u8]) -> CryptoResult<Self> {
        match scheme {
            SignatureScheme::MlDsa => {
                let _ = dilithium3::DetachedSignature::from_bytes(bytes)
                    .map_err(|error| CryptoError::InvalidKeyMaterial(error.to_string()))?;
            }
            SignatureScheme::SlhDsa => {
                let _ = sphincsshake128fsimple::DetachedSignature::from_bytes(bytes)
                    .map_err(|error| CryptoError::InvalidKeyMaterial(error.to_string()))?;
            }
        }
        Ok(Self {
            scheme,
            bytes: bytes.to_vec(),
        })
    }
}

/// Generate a default ML-DSA keypair.
#[must_use]
pub fn keygen() -> (SignPublicKey, SignSecretKey) {
    keygen_with_scheme(SignatureScheme::MlDsa)
}

/// Generate a keypair for the requested signature scheme.
#[must_use]
pub fn keygen_with_scheme(scheme: SignatureScheme) -> (SignPublicKey, SignSecretKey) {
    match scheme {
        SignatureScheme::MlDsa => {
            let (public_key, secret_key) = dilithium3::keypair();
            (
                SignPublicKey {
                    kind: SignPublicKeyKind::MlDsa(public_key),
                },
                SignSecretKey {
                    kind: SignSecretKeyKind::MlDsa(secret_key),
                },
            )
        }
        SignatureScheme::SlhDsa => {
            let (public_key, secret_key) = sphincsshake128fsimple::keypair();
            (
                SignPublicKey {
                    kind: SignPublicKeyKind::SlhDsa(public_key),
                },
                SignSecretKey {
                    kind: SignSecretKeyKind::SlhDsa(secret_key),
                },
            )
        }
    }
}

/// Produce a detached signature over `message`.
#[must_use]
pub fn sign(sk: &SignSecretKey, message: &[u8]) -> Signature {
    match &sk.kind {
        SignSecretKeyKind::MlDsa(secret_key) => Signature {
            scheme: SignatureScheme::MlDsa,
            bytes: dilithium3::detached_sign(message, secret_key)
                .as_bytes()
                .to_vec(),
        },
        SignSecretKeyKind::SlhDsa(secret_key) => Signature {
            scheme: SignatureScheme::SlhDsa,
            bytes: sphincsshake128fsimple::detached_sign(message, secret_key)
                .as_bytes()
                .to_vec(),
        },
    }
}

/// Verify a detached signature.
#[must_use]
pub fn verify(pk: &SignPublicKey, message: &[u8], signature: &Signature) -> bool {
    if pk.scheme() != signature.scheme() {
        return false;
    }

    match (&pk.kind, signature.scheme()) {
        (SignPublicKeyKind::MlDsa(public_key), SignatureScheme::MlDsa) => {
            let Ok(signature) = dilithium3::DetachedSignature::from_bytes(signature.as_bytes())
            else {
                return false;
            };
            dilithium3::verify_detached_signature(&signature, message, public_key).is_ok()
        }
        (SignPublicKeyKind::SlhDsa(public_key), SignatureScheme::SlhDsa) => {
            let Ok(signature) =
                sphincsshake128fsimple::DetachedSignature::from_bytes(signature.as_bytes())
            else {
                return false;
            };
            sphincsshake128fsimple::verify_detached_signature(&signature, message, public_key)
                .is_ok()
        }
        _ => false,
    }
}
