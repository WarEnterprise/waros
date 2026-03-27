use alloc::vec::Vec;

use crate::auth::{FilePermissions, UserRole};
use crate::fs;

use super::{verify_package_bytes, InstalledPackage, PackageInfo, PackageManager, PkgError};

pub fn install(manager: &mut PackageManager, package: &PackageInfo) -> Result<(), PkgError> {
    if manager.is_installed(&package.name) {
        return Ok(());
    }

    let bundle_bytes = manager.fetch_package(&package.download_url)?;
    let verified = verify_package_bytes(package, &bundle_bytes)?;
    let manifest = &verified.bundle.signed_manifest.manifest;

    let mut installed_files = Vec::new();
    let mut filesystem = fs::FILESYSTEM.lock();
    for file in &manifest.files {
        let payload = verified
            .bundle
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
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        files: installed_files,
        installed_at: crate::arch::x86_64::interrupts::tick_count(),
        size_bytes: bundle_bytes.len() as u64,
        signed_by: verified.signed_by.clone(),
        signature_scheme: verified.signature_scheme.clone(),
    });
    manager.save_installed_list()?;

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::PackageInstalled {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            uid: crate::exec::current_uid(),
        },
    );

    Ok(())
}

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
