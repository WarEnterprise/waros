use alloc::string::String;

pub use waros_pkg::{
    payload_digests, sha256_hex, verify_bundle_with_embedded_root, VerifyError,
    WARPKG_BOOTSTRAP_KEY_ID, WARPKG_SIGNATURE_SCHEME,
};

use super::manifest::WarPackBundle;

#[must_use]
pub fn format_trust_root() -> String {
    alloc::format!(
        "{} ({})",
        WARPKG_BOOTSTRAP_KEY_ID,
        WARPKG_SIGNATURE_SCHEME
    )
}

pub fn verify_bootstrap_bundle(bundle: &WarPackBundle) -> Result<(), VerifyError> {
    verify_bundle_with_embedded_root(bundle)
}
