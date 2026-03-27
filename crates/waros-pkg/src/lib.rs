#![no_std]

extern crate alloc;

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::convert::TryFrom;

use ml_dsa::{
    signature::{Signer, Verifier},
    EncodedVerifyingKey, KeyGen, MlDsa65, Signature as MlDsaSignature, VerifyingKey,
};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

pub const WARPKG_SIGNATURE_SCHEME: &str = "ml-dsa-dilithium3-v1";
pub const WARPKG_BOOTSTRAP_KEY_ID: &str = "waros-bootstrap-root-v1";
const SIGNING_DOMAIN: &[u8] = b"WarPkgSignedManifest-v1\x00";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub files: Vec<ManifestFile>,
    pub dependencies: Vec<String>,
    pub min_waros_version: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub source: String,
    pub executable: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WarPackPayload {
    pub source: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadDigest {
    pub source: String,
    pub size: u64,
    pub sha3_256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignatureEnvelope {
    pub scheme: String,
    pub key_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedManifest {
    pub manifest: Manifest,
    pub payloads: Vec<PayloadDigest>,
    pub signature: SignatureEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WarPackBundle {
    pub signed_manifest: SignedManifest,
    pub payloads: Vec<WarPackPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustRoot {
    pub key_id: &'static str,
    pub scheme: &'static str,
    pub public_key_hex: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    SignatureMissing,
    UnsupportedSignatureScheme,
    UnknownTrustRoot,
    InvalidPublicKey,
    InvalidSignatureEncoding,
    SignatureMismatch,
    CanonicalEncodingFailed,
    DuplicatePayload(String),
    PayloadMissing(String),
    UnexpectedPayload(String),
    PayloadDigestMismatch(String),
    PayloadSizeMismatch(String),
    FilePayloadMissing(String),
    FileSizeMismatch(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignError {
    UnsupportedSignatureScheme,
    InvalidSecretKey,
    CanonicalEncodingFailed,
}

#[derive(Serialize)]
struct CanonicalSignedManifest<'a> {
    manifest: &'a Manifest,
    payloads: &'a [PayloadDigest],
}

pub fn embedded_bootstrap_root() -> TrustRoot {
    TrustRoot {
        key_id: WARPKG_BOOTSTRAP_KEY_ID,
        scheme: WARPKG_SIGNATURE_SCHEME,
        public_key_hex: include_str!("bootstrap_root_v1.pk.hex"),
    }
}

#[must_use]
pub fn sha256_hex(payload: &[u8]) -> String {
    hex_encode(&sha2::Sha256::digest(payload))
}

#[must_use]
pub fn sha3_256_hex(payload: &[u8]) -> String {
    hex_encode(&sha3::Sha3_256::digest(payload))
}

pub fn canonical_signed_message(
    manifest: &Manifest,
    payloads: &[PayloadDigest],
) -> Result<Vec<u8>, VerifyError> {
    let canonical = CanonicalSignedManifest { manifest, payloads };
    let json = serde_json::to_vec(&canonical).map_err(|_| VerifyError::CanonicalEncodingFailed)?;
    let mut message = Vec::with_capacity(SIGNING_DOMAIN.len() + json.len());
    message.extend_from_slice(SIGNING_DOMAIN);
    message.extend_from_slice(&json);
    Ok(message)
}

pub fn sign_manifest(
    manifest: &Manifest,
    payloads: &[PayloadDigest],
    key_id: &str,
    scheme: &str,
    secret_key_hex: &str,
) -> Result<SignatureEnvelope, SignError> {
    if scheme != WARPKG_SIGNATURE_SCHEME {
        return Err(SignError::UnsupportedSignatureScheme);
    }
    let message = canonical_signed_message(manifest, payloads)
        .map_err(|_| SignError::CanonicalEncodingFailed)?;
    let secret_key_bytes = parse_hex(secret_key_hex).ok_or(SignError::InvalidSecretKey)?;
    let secret_seed = ml_dsa::Seed::try_from(secret_key_bytes.as_slice())
        .map_err(|_| SignError::InvalidSecretKey)?;
    let signing_key = MlDsa65::from_seed(&secret_seed);
    let signature = signing_key.sign(&message).encode();
    Ok(SignatureEnvelope {
        scheme: scheme.to_string(),
        key_id: key_id.to_string(),
        signature: hex_encode(signature.as_ref()),
    })
}

#[must_use]
pub fn payload_digests(payloads: &[WarPackPayload]) -> Vec<PayloadDigest> {
    payloads
        .iter()
        .map(|payload| PayloadDigest {
            source: payload.source.clone(),
            size: payload.contents.len() as u64,
            sha3_256: sha3_256_hex(payload.contents.as_bytes()),
        })
        .collect()
}

pub fn verify_bundle(bundle: &WarPackBundle, root: TrustRoot) -> Result<(), VerifyError> {
    let signature = &bundle.signed_manifest.signature;
    if signature.signature.is_empty() {
        return Err(VerifyError::SignatureMissing);
    }
    if signature.scheme != root.scheme {
        return Err(VerifyError::UnsupportedSignatureScheme);
    }
    if signature.key_id != root.key_id {
        return Err(VerifyError::UnknownTrustRoot);
    }

    validate_payloads(bundle)?;

    let public_key_bytes = parse_hex(root.public_key_hex).ok_or(VerifyError::InvalidPublicKey)?;
    let signature_bytes =
        parse_hex(&signature.signature).ok_or(VerifyError::InvalidSignatureEncoding)?;
    let encoded_public_key = EncodedVerifyingKey::<MlDsa65>::try_from(public_key_bytes.as_slice())
        .map_err(|_| VerifyError::InvalidPublicKey)?;
    let public_key = VerifyingKey::<MlDsa65>::decode(&encoded_public_key);
    let message = canonical_signed_message(
        &bundle.signed_manifest.manifest,
        &bundle.signed_manifest.payloads,
    )?;

    let signature = MlDsaSignature::<MlDsa65>::try_from(signature_bytes.as_slice())
        .map_err(|_| VerifyError::InvalidSignatureEncoding)?;
    public_key
        .verify(&message, &signature)
        .map_err(|_| VerifyError::SignatureMismatch)
}

pub fn verify_bundle_with_embedded_root(bundle: &WarPackBundle) -> Result<(), VerifyError> {
    verify_bundle(bundle, embedded_bootstrap_root())
}

#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(hex_digit(byte >> 4));
        output.push(hex_digit(byte & 0x0F));
    }
    output
}

#[must_use]
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let bytes = text.trim().as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }

    let mut output = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let high = decode_digit(bytes[index])?;
        let low = decode_digit(bytes[index + 1])?;
        output.push((high << 4) | low);
        index += 2;
    }
    Some(output)
}

fn validate_payloads(bundle: &WarPackBundle) -> Result<(), VerifyError> {
    let mut seen_signed = Vec::<&str>::new();
    for digest in &bundle.signed_manifest.payloads {
        let source = digest.source.as_str();
        if seen_signed.iter().any(|existing| *existing == source) {
            return Err(VerifyError::DuplicatePayload(digest.source.clone()));
        }
        seen_signed.push(source);

        let payload = bundle
            .payloads
            .iter()
            .find(|payload| payload.source == digest.source)
            .ok_or_else(|| VerifyError::PayloadMissing(digest.source.clone()))?;
        let actual_size = payload.contents.len() as u64;
        if actual_size != digest.size {
            return Err(VerifyError::PayloadSizeMismatch(digest.source.clone()));
        }
        let actual_digest = sha3_256_hex(payload.contents.as_bytes());
        if actual_digest != digest.sha3_256 {
            return Err(VerifyError::PayloadDigestMismatch(digest.source.clone()));
        }
    }

    let mut seen_payloads = Vec::<&str>::new();
    for payload in &bundle.payloads {
        let source = payload.source.as_str();
        if seen_payloads.iter().any(|existing| *existing == source) {
            return Err(VerifyError::DuplicatePayload(payload.source.clone()));
        }
        seen_payloads.push(source);
        if !bundle
            .signed_manifest
            .payloads
            .iter()
            .any(|digest| digest.source == payload.source)
        {
            return Err(VerifyError::UnexpectedPayload(payload.source.clone()));
        }
    }

    for file in &bundle.signed_manifest.manifest.files {
        let digest = bundle
            .signed_manifest
            .payloads
            .iter()
            .find(|digest| digest.source == file.source)
            .ok_or_else(|| VerifyError::FilePayloadMissing(file.source.clone()))?;
        if digest.size != file.size {
            return Err(VerifyError::FileSizeMismatch(file.path.clone()));
        }
    }

    Ok(())
}

fn hex_digit(value: u8) -> char {
    match value & 0x0F {
        0..=9 => char::from(b'0' + (value & 0x0F)),
        10..=15 => char::from(b'a' + ((value & 0x0F) - 10)),
        _ => '0',
    }
}

fn decode_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use alloc::boxed::Box;
    use alloc::vec;

    use super::*;
    use ml_dsa::signature::Keypair;

    const TEST_SEED_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn test_root() -> TrustRoot {
        let seed = ml_dsa::Seed::try_from(parse_hex(TEST_SEED_HEX).unwrap().as_slice()).unwrap();
        let signing_key = MlDsa65::from_seed(&seed);
        let public_key = signing_key.verifying_key().encode();
        TrustRoot {
            key_id: "test-root",
            scheme: WARPKG_SIGNATURE_SCHEME,
            public_key_hex: Box::leak(hex_encode(public_key.as_ref()).into_boxed_str()),
        }
    }

    fn sample_bundle() -> WarPackBundle {
        let payloads = vec![WarPackPayload {
            source: String::from("hello.txt"),
            contents: String::from("hello warpkg\n"),
        }];
        let manifest = Manifest {
            name: String::from("hello"),
            version: String::from("1.0.0"),
            description: String::from("test bundle"),
            author: String::from("WarOS"),
            license: String::from("Apache-2.0"),
            files: vec![ManifestFile {
                path: String::from("/usr/share/hello.txt"),
                source: String::from("hello.txt"),
                executable: false,
                size: payloads[0].contents.len() as u64,
            }],
            dependencies: Vec::new(),
            min_waros_version: String::from("0.2.0"),
            category: String::from("docs"),
        };
        let digests = payload_digests(&payloads);
        let signature = sign_manifest(
            &manifest,
            &digests,
            "test-root",
            WARPKG_SIGNATURE_SCHEME,
            TEST_SEED_HEX,
        )
        .unwrap();
        WarPackBundle {
            signed_manifest: SignedManifest {
                manifest,
                payloads: digests,
                signature,
            },
            payloads,
        }
    }

    #[test]
    fn signed_bundle_verifies() {
        let bundle = sample_bundle();
        assert_eq!(verify_bundle(&bundle, test_root()), Ok(()));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let mut bundle = sample_bundle();
        bundle.payloads[0].contents.push_str("tampered");
        assert!(matches!(
            verify_bundle(&bundle, test_root()),
            Err(VerifyError::PayloadDigestMismatch(_) | VerifyError::PayloadSizeMismatch(_))
        ));
    }

    #[test]
    fn unsigned_bundle_is_rejected() {
        let mut bundle = sample_bundle();
        bundle.signed_manifest.signature.signature.clear();
        assert_eq!(
            verify_bundle(&bundle, test_root()),
            Err(VerifyError::SignatureMissing)
        );
    }
}
