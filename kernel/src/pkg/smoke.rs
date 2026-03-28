use alloc::string::String;

use super::{
    registry::bundle_path, signature::sha256_hex, update, verify_package_bytes, PackageInfo,
    PkgError, PACKAGE_MANAGER,
};

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
    crate::serial_println!("[PROOF] WarPkg: valid signed bundle accepted");

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
        Err(PkgError::SignatureInvalid) => {
            crate::serial_println!("[PROOF] WarPkg: tampered bundle rejected");
            Ok(())
        }
        Err(_) => Err("tampered package rejected with wrong error"),
        Ok(_) => Err("tampered package unexpectedly verified"),
    }
}

pub fn run_offline_update_proof() -> Result<(), &'static str> {
    if !update::proof_available().map_err(|_| "offline update proof could not read update state")? {
        return Err("offline update proof requires an idle update state");
    }

    let source_path = bundle_path("hello-world");
    let mut tampered_path = None::<String>;
    let result = (|| -> Result<(), &'static str> {
        match update::apply_bundle_from_path(&source_path) {
            Ok(_) => crate::serial_println!("[PROOF] WarPkg update: valid offline apply accepted"),
            Err(PkgError::UpdateBusy) => {
                return Err("offline update proof blocked by active update state");
            }
            Err(error) => {
                crate::serial_println!(
                    "[INFO] WarPkg update proof: valid offline apply error={}",
                    error
                );
                return Err("valid offline update bundle rejected");
            }
        }

        let boot_report = update::prepare_boot();
        if !boot_report.summary.contains("pending confirmation") {
            return Err("offline update boot health did not enter pending-confirmation");
        }
        crate::serial_println!("[PROOF] WarPkg update: pending confirmation state reached");

        let shell_ready = update::note_shell_ready()
            .map_err(|_| "offline update proof failed to mark shell-ready")?
            .ok_or("offline update proof did not record shell-ready")?;
        if !shell_ready.contains("confirmation required") {
            return Err("offline update proof did not require confirmation after shell-ready");
        }
        crate::serial_println!("[PROOF] WarPkg update: shell-ready confirmation required");

        update::confirm_pending_update()
            .map_err(|_| "offline update proof failed to confirm boot")?;
        crate::serial_println!("[PROOF] WarPkg update: pending boot confirmed");

        update::rollback_current_update()
            .map_err(|_| "offline update proof failed to restore rollback snapshot")?;
        update::clear_completed_state()
            .map_err(|_| "offline update proof failed to clear completed state")?;
        crate::serial_println!("[PROOF] WarPkg update: rollback snapshot restored");

        update::request_recovery("offline update proof recovery demo")
            .map_err(|_| "offline update proof failed to request recovery")?;
        let recovery_status = update::status_report();
        if !update::should_enter_recovery() || !recovery_status.contains("Recovery request:  yes") {
            return Err("offline update proof did not expose recovery status");
        }
        crate::serial_println!("[PROOF] WarPkg update: recovery entry/status path observable");
        update::clear_recovery_request()
            .map_err(|_| "offline update proof failed to clear recovery request")?;

        let (path, tampered_bytes) = {
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
                .map_err(|_| "failed to fetch offline update proof bundle")?;
            let mut bundle: super::manifest::WarPackBundle =
                serde_json::from_slice(&bytes).map_err(|_| "failed to parse update proof bundle")?;
            let Some(first_payload) = bundle.payloads.first_mut() else {
                return Err("offline update proof payload missing");
            };
            first_payload.contents.push_str("# tampered\n");
            let tampered_bytes = serde_json::to_vec(&bundle)
                .map_err(|_| "failed to serialize tampered update bundle")?;
            (
                String::from("/var/pkg/staged/proof-tampered.warpack"),
                tampered_bytes,
            )
        };
        tampered_path = Some(path.clone());

        {
            let mut filesystem = crate::fs::FILESYSTEM.lock();
            filesystem
                .write_system(&path, &tampered_bytes, false)
                .map_err(|_| "failed to stage tampered proof bundle")?;
        }

        match update::apply_bundle_from_path(&path) {
            Err(PkgError::SignatureInvalid) => {
                crate::serial_println!("[PROOF] WarPkg update: tampered offline bundle rejected");
                Ok(())
            }
            Err(PkgError::UpdateBusy) => Err("tampered update proof blocked by active update state"),
            Err(_) => Err("tampered offline update rejected with wrong error"),
            Ok(_) => Err("tampered offline update unexpectedly applied"),
        }
    })();

    if let Some(path) = tampered_path {
        let mut filesystem = crate::fs::FILESYSTEM.lock();
        let _ = filesystem.delete(&path);
    }

    if result.is_err() {
        let _ = update::rollback_current_update();
        let _ = update::clear_completed_state();
        let _ = update::clear_recovery_request();
    }

    result
}
