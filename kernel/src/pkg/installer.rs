use alloc::vec::Vec;

use crate::auth::{FilePermissions, UserRole};
use crate::fs;

use super::manifest::WarPackBundle;
use super::signature::{sha256_hex, verify_bootstrap};
use super::{InstalledPackage, PackageInfo, PackageManager, PkgError};

pub fn install(manager: &mut PackageManager, package: &PackageInfo) -> Result<(), PkgError> {
    if manager.is_installed(&package.name) {
        return Ok(());
    }

    let bundle_bytes = manager.fetch_package(&package.download_url)?;
    let sha256 = sha256_hex(&bundle_bytes);
    if sha256 != package.sha256 {
        return Err(PkgError::ChecksumMismatch);
    }
    if !verify_bootstrap(&bundle_bytes, &package.signature) {
        return Err(PkgError::SignatureInvalid);
    }

    let bundle: WarPackBundle =
        serde_json::from_slice(&bundle_bytes).map_err(|_| PkgError::ExtractFailed)?;
    let mut installed_files = Vec::new();
    let mut filesystem = fs::FILESYSTEM.lock();
    for file in &bundle.manifest.files {
        let payload = bundle
            .payloads
            .iter()
            .find(|payload| payload.source == file.source)
            .ok_or(PkgError::ExtractFailed)?;
        filesystem
            .write_as(
                &file.path,
                payload.contents.as_bytes(),
                0,
                UserRole::Admin,
                FilePermissions::system(),
            )
            .map_err(|_| PkgError::InstallFailed)?;
        if file.executable {
            filesystem
                .chmod_as(&file.path, 0, UserRole::Admin, "rwrr")
                .map_err(|_| PkgError::InstallFailed)?;
        }
        installed_files.push(file.path.clone());
    }
    drop(filesystem);

    manager.installed.push(InstalledPackage {
        name: bundle.manifest.name,
        version: bundle.manifest.version,
        description: bundle.manifest.description,
        files: installed_files,
        installed_at: crate::arch::x86_64::interrupts::tick_count(),
        size_bytes: bundle_bytes.len() as u64,
        signed_by: String::from("War Enterprise bootstrap root"),
    });
    manager.save_installed_list()?;
    Ok(())
}

use alloc::string::String;

pub fn remove(manager: &mut PackageManager, name: &str) -> Result<(), PkgError> {
    let package = manager
        .installed
        .iter()
        .find(|package| package.name == name)
        .cloned()
        .ok_or(PkgError::PackageNotInstalled)?;
    let mut filesystem = fs::FILESYSTEM.lock();
    for file in &package.files {
        filesystem
            .delete_as(file, 0, UserRole::Admin)
            .map_err(|_| PkgError::InstallFailed)?;
    }
    drop(filesystem);
    manager.installed.retain(|package| package.name != name);
    manager.save_installed_list()?;
    Ok(())
}
