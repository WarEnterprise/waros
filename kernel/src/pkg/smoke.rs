use super::{signature::sha256_hex, verify_package_bytes, PackageInfo, PkgError, PACKAGE_MANAGER};

pub fn run_signature_proof() -> Result<(), &'static str> {
    let (info, bytes) = {
        let manager = PACKAGE_MANAGER.lock();
        let index = manager.index.as_ref().ok_or("package index unavailable")?;
        let info = index
            .packages
            .iter()
            .find(|package| package.name == "hello-world")
            .cloned()
            .ok_or("hello-world package missing from index")?;
        let bytes = manager
            .fetch_package(&info.download_url)
            .map_err(|_| "failed to fetch signed package bundle")?;
        (info, bytes)
    };

    verify_package_bytes(&info, &bytes).map_err(|_| "valid signed package rejected")?;

    let mut tampered_bundle: super::manifest::WarPackBundle =
        serde_json::from_slice(&bytes).map_err(|_| "failed to parse signed package bundle")?;
    let Some(first_payload) = tampered_bundle.payloads.first_mut() else {
        return Err("hello-world payload missing");
    };
    first_payload.contents.push_str("# tampered\n");
    let tampered_bytes =
        serde_json::to_vec(&tampered_bundle).map_err(|_| "failed to serialize tampered bundle")?;
    let tampered_info = PackageInfo {
        sha256: sha256_hex(&tampered_bytes),
        ..info
    };

    match verify_package_bytes(&tampered_info, &tampered_bytes) {
        Err(PkgError::SignatureInvalid) => Ok(()),
        Err(_) => Err("tampered package rejected with wrong error"),
        Ok(_) => Err("tampered package unexpectedly verified"),
    }
}
