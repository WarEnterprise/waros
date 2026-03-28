use alloc::vec::Vec;

use crate::auth::{FilePermissions, UserRole};
use crate::fs;

use super::{
    verify_package_bytes, InstalledPackage, PackageInfo, PackageManager, PkgError, VerifiedPackage,
};

pub fn install(manager: &mut PackageManager, package: &PackageInfo) -> Result<(), PkgError> {
    if manager.is_installed(&package.name) {
        return Ok(());
    }

    let bundle_bytes = manager.fetch_package(&package.download_url)?;
    let verified = verify_package_bytes(package, &bundle_bytes)?;
    let _ = apply_verified_bundle(
        manager,
        &verified,
        bundle_bytes.len() as u64,
        crate::exec::current_uid(),
    )?;
    Ok(())
}

pub fn apply_verified_bundle(
    manager: &mut PackageManager,
    verified: &VerifiedPackage,
    bundle_size: u64,
    uid: u16,
) -> Result<InstalledPackage, PkgError> {
    let manifest = &verified.bundle.signed_manifest.manifest;
    let previous = manager
        .installed
        .iter()
        .find(|package| package.name == manifest.name)
        .cloned();

    let mut installed_files = Vec::new();
    let mut filesystem = fs::FILESYSTEM.lock();
    if let Some(previous) = &previous {
        for file in &previous.files {
            if manifest.files.iter().all(|next| next.path != *file) {
                match filesystem.delete_as(file, 0, UserRole::Admin) {
                    Ok(()) | Err(fs::FsError::FileNotFound) => {}
                    Err(error) => {
                        crate::serial_println!(
                            "[INFO] WarPkg install: failed to remove stale file {} ({})",
                            file,
                            error
                        );
                        return Err(PkgError::InstallFailed);
                    }
                }
            }
        }
    }

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
            .map_err(|error| {
                crate::serial_println!(
                    "[INFO] WarPkg install: failed to write {} ({})",
                    file.path,
                    error
                );
                PkgError::InstallFailed
            })?;
        if file.executable {
            filesystem
                .chmod_as(&file.path, 0, UserRole::Admin, "rwr-")
                .map_err(|error| {
                    crate::serial_println!(
                        "[INFO] WarPkg install: failed to chmod {} ({})",
                        file.path,
                        error
                    );
                    PkgError::InstallFailed
                })?;
        }
        installed_files.push(file.path.clone());
    }
    drop(filesystem);

    let installed = InstalledPackage {
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        description: manifest.description.clone(),
        files: installed_files,
        installed_at: crate::arch::x86_64::interrupts::tick_count(),
        size_bytes: bundle_size,
        signed_by: verified.signed_by.clone(),
        signature_scheme: verified.signature_scheme.clone(),
    };
    manager.installed.retain(|package| package.name != manifest.name);
    manager.installed.push(installed.clone());
    manager.save_installed_list().map_err(|error| {
        crate::serial_println!(
            "[INFO] WarPkg install: failed to persist installed package list ({})",
            error
        );
        error
    })?;

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::PackageInstalled {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            uid,
        },
    );

    Ok(installed)
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
