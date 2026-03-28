use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::{Lazy, Mutex};

use crate::fs;
use crate::net;
use crate::security::capabilities::{self, Capabilities};

pub mod bootstrap;
pub mod commands;
pub mod installer;
pub mod manifest;
pub mod registry;
pub mod resolver;
pub mod signature;
pub mod smoke;
pub mod update;

use installer::{install as install_package, remove as remove_package};
use manifest::WarPackBundle;
use registry::{bundle_path, fetch_index, load_local_index};
use resolver::resolve_dependencies;
use signature::{format_trust_root, sha256_hex};

pub const DEFAULT_REPO_URL: &str = "https://warenterprise.com/packages";
pub const LOCAL_INDEX_PATH: &str = "/var/pkg/index.json";
const LOCAL_INSTALLED_PATH: &str = "/var/pkg/installed.json";

pub static PACKAGE_MANAGER: Lazy<Mutex<PackageManager>> =
    Lazy::new(|| Mutex::new(PackageManager::new(DEFAULT_REPO_URL)));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub name: String,
    pub version: String,
    pub description: String,
    pub files: Vec<String>,
    pub installed_at: u64,
    pub size_bytes: u64,
    pub signed_by: String,
    pub signature_scheme: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageIndex {
    pub packages: Vec<PackageInfo>,
    pub last_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub signed_by: String,
    pub signature_scheme: String,
    pub dependencies: Vec<String>,
    pub download_url: String,
}

#[derive(Debug, Clone)]
pub struct VerifiedPackage {
    pub bundle: WarPackBundle,
    pub signed_by: String,
    pub signature_scheme: String,
}

pub struct PackageManager {
    pub installed: Vec<InstalledPackage>,
    pub index: Option<PackageIndex>,
    repo_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PkgError {
    PackageNotFound,
    PackageNotInstalled,
    ChecksumMismatch,
    UnsignedPackage,
    SignatureInvalid,
    MetadataMismatch,
    PermissionDenied,
    DependencyCycle,
    DependencyNotFound(String),
    DownloadFailed,
    ExtractFailed,
    InstallFailed,
    NetworkError,
    BundleUnavailable,
    UpdateBusy,
    NoStagedBundle,
    NoPendingUpdate,
    BootConfirmationRequired,
    RollbackUnavailable,
    RecoveryBlocked,
    StateCorrupted,
}

impl core::fmt::Display for PkgError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PackageNotFound => formatter.write_str("package not found"),
            Self::PackageNotInstalled => formatter.write_str("package is not installed"),
            Self::ChecksumMismatch => formatter.write_str("package checksum mismatch"),
            Self::UnsignedPackage => formatter.write_str("package is unsigned"),
            Self::SignatureInvalid => formatter.write_str("package signature invalid"),
            Self::MetadataMismatch => formatter.write_str("package metadata does not match signed manifest"),
            Self::PermissionDenied => formatter.write_str("permission denied: PKG_INSTALL capability required"),
            Self::DependencyCycle => formatter.write_str("dependency cycle detected"),
            Self::DependencyNotFound(name) => {
                write!(formatter, "dependency '{}' missing from package index", name)
            }
            Self::DownloadFailed => formatter.write_str("package download failed"),
            Self::ExtractFailed => formatter.write_str("package extraction failed"),
            Self::InstallFailed => formatter.write_str("package installation failed"),
            Self::NetworkError => formatter.write_str("network error while updating package index"),
            Self::BundleUnavailable => {
                formatter.write_str("offline bundle path could not be read")
            }
            Self::UpdateBusy => {
                formatter.write_str("an update is already pending confirmation or recovery handling")
            }
            Self::NoStagedBundle => formatter.write_str("no staged update bundle is available"),
            Self::NoPendingUpdate => formatter.write_str("no pending applied update exists"),
            Self::BootConfirmationRequired => formatter.write_str(
                "pending update has not reached shell-ready yet; boot it and confirm after shell-ready",
            ),
            Self::RollbackUnavailable => {
                formatter.write_str("no rollback record is available for the current update")
            }
            Self::RecoveryBlocked => formatter.write_str(
                "recovery cannot be cleared while an update is still pending or failed",
            ),
            Self::StateCorrupted => formatter.write_str("update state metadata is corrupted"),
        }
    }
}

impl PackageManager {
    #[must_use]
    pub fn new(repo_url: &str) -> Self {
        Self {
            installed: load_installed().unwrap_or_default(),
            index: load_local_index().ok(),
            repo_url: repo_url.into(),
        }
    }

    pub fn update(&mut self) -> Result<(), PkgError> {
        require_pkg_install()?;
        let index = fetch_index(&self.repo_url).or_else(|_| load_local_index())?;
        let bytes = serde_json::to_vec(&index).map_err(|_| PkgError::ExtractFailed)?;
        fs::FILESYSTEM
            .lock()
            .write_system(LOCAL_INDEX_PATH, &bytes, false)
            .map_err(|_| PkgError::InstallFailed)?;
        self.index = Some(index);
        Ok(())
    }

    #[must_use]
    pub fn search(&self, query: &str) -> Vec<PackageInfo> {
        let query = query.to_ascii_lowercase();
        self.index
            .as_ref()
            .map(|index| {
                index
                    .packages
                    .iter()
                    .filter(|package| {
                        package.name.to_ascii_lowercase().contains(&query)
                            || package.description.to_ascii_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn install(&mut self, name: &str) -> Result<(), PkgError> {
        require_pkg_install()?;
        let index = self.index.clone().ok_or(PkgError::PackageNotFound)?;
        let ordered = resolve_dependencies(&index, name)?;
        for package_name in ordered {
            let package = index
                .packages
                .iter()
                .find(|package| package.name == package_name)
                .ok_or(PkgError::PackageNotFound)?;
            install_package(self, package)?;
        }
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Result<(), PkgError> {
        require_pkg_install()?;
        remove_package(self, name)
    }

    #[must_use]
    pub fn is_installed(&self, name: &str) -> bool {
        self.installed.iter().any(|package| package.name == name)
    }

    #[must_use]
    pub fn package_info(&self, name: &str) -> Option<&PackageInfo> {
        self.index
            .as_ref()
            .and_then(|index| index.packages.iter().find(|package| package.name == name))
    }

    pub fn verify(&self, name: &str) -> Result<(), PkgError> {
        let info = self.package_info(name).ok_or(PkgError::PackageNotFound)?;
        let bytes = self.fetch_package(&info.download_url)?;
        verify_package_bytes(info, &bytes).map(|_| ())
    }

    pub fn save_installed_list(&self) -> Result<(), PkgError> {
        let bytes = serde_json::to_vec(&self.installed).map_err(|_| PkgError::ExtractFailed)?;
        fs::FILESYSTEM
            .lock()
            .write_system(LOCAL_INSTALLED_PATH, &bytes, false)
            .map_err(|_| PkgError::InstallFailed)
    }

    pub fn fetch_package(&self, location: &str) -> Result<Vec<u8>, PkgError> {
        if location.starts_with('/') {
            return fs::FILESYSTEM
                .lock()
                .read(location)
                .map(|bytes| bytes.to_vec())
                .map_err(|_| PkgError::DownloadFailed);
        }

        let response = net::http_get(location).map_err(|_| PkgError::DownloadFailed)?;
        Ok(response.body)
    }
}

pub fn init() -> Result<(), PkgError> {
    seed_repository()?;
    *PACKAGE_MANAGER.lock() = PackageManager::new(DEFAULT_REPO_URL);
    update::ensure_state_file()?;
    Ok(())
}

pub fn with_manager<T>(
    f: impl FnOnce(&mut PackageManager) -> Result<T, PkgError>,
) -> Result<T, PkgError> {
    let mut manager = PACKAGE_MANAGER.lock();
    f(&mut manager)
}

pub fn verify_package_bytes(
    package: &PackageInfo,
    bytes: &[u8],
) -> Result<VerifiedPackage, PkgError> {
    if sha256_hex(bytes) != package.sha256 {
        log_package_verification(package, "deny", "checksum-mismatch");
        return Err(PkgError::ChecksumMismatch);
    }

    let bundle: WarPackBundle = match serde_json::from_slice(bytes) {
        Ok(bundle) => bundle,
        Err(_) => {
            log_package_verification(package, "deny", "extract-failed");
            return Err(PkgError::ExtractFailed);
        }
    };
    if let Err(error) = signature::verify_bootstrap_bundle(&bundle) {
        log_package_verification(package, "deny", verify_error_reason(&error));
        return Err(map_verify_error(error));
    }

    let manifest = &bundle.signed_manifest.manifest;
    if manifest.name != package.name
        || manifest.version != package.version
        || manifest.description != package.description
        || manifest.dependencies != package.dependencies
    {
        log_package_verification(package, "deny", "metadata-mismatch");
        return Err(PkgError::MetadataMismatch);
    }

    log_package_verification(package, "allow", "verified");
    Ok(VerifiedPackage {
        signed_by: bundle.signed_manifest.signature.key_id.clone(),
        signature_scheme: bundle.signed_manifest.signature.scheme.clone(),
        bundle,
    })
}

pub fn verify_local_bundle_bytes(bytes: &[u8]) -> Result<VerifiedPackage, PkgError> {
    let bundle: WarPackBundle = match serde_json::from_slice(bytes) {
        Ok(bundle) => bundle,
        Err(_) => {
            crate::security::audit::log_event(
                crate::security::audit::events::AuditEvent::PackageVerification {
                    name: String::from("<local>"),
                    version: String::from("unknown"),
                    outcome: String::from("deny"),
                    reason: String::from("extract-failed"),
                },
            );
            return Err(PkgError::ExtractFailed);
        }
    };

    let manifest = &bundle.signed_manifest.manifest;
    if let Err(error) = signature::verify_bootstrap_bundle(&bundle) {
        crate::security::audit::log_event(
            crate::security::audit::events::AuditEvent::PackageVerification {
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                outcome: String::from("deny"),
                reason: String::from(verify_error_reason(&error)),
            },
        );
        return Err(map_verify_error(error));
    }

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::PackageVerification {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            outcome: String::from("allow"),
            reason: String::from("verified"),
        },
    );
    Ok(VerifiedPackage {
        signed_by: bundle.signed_manifest.signature.key_id.clone(),
        signature_scheme: bundle.signed_manifest.signature.scheme.clone(),
        bundle,
    })
}

#[must_use]
pub fn trust_root_summary() -> String {
    format_trust_root()
}

fn load_installed() -> Option<Vec<InstalledPackage>> {
    let filesystem = fs::FILESYSTEM.lock();
    let data = filesystem.read(LOCAL_INSTALLED_PATH).ok()?;
    serde_json::from_slice(data).ok()
}

fn seed_repository() -> Result<(), PkgError> {
    let packages = bootstrap::built_in_packages();
    let mut index_packages = Vec::new();
    let mut filesystem = fs::FILESYSTEM.lock();

    for bundle in packages {
        let bytes = serde_json::to_vec(&bundle).map_err(|_| PkgError::ExtractFailed)?;
        let manifest = &bundle.signed_manifest.manifest;
        let path = bundle_path(&manifest.name);
        filesystem
            .write_system(&path, &bytes, false)
            .map_err(|_| PkgError::InstallFailed)?;
        index_packages.push(PackageInfo {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            signed_by: bundle.signed_manifest.signature.key_id.clone(),
            signature_scheme: bundle.signed_manifest.signature.scheme.clone(),
            dependencies: manifest.dependencies.clone(),
            download_url: path,
        });
    }

    let index = PackageIndex {
        packages: index_packages,
        last_updated: crate::arch::x86_64::interrupts::tick_count(),
    };
    let index_bytes = serde_json::to_vec(&index).map_err(|_| PkgError::ExtractFailed)?;
    filesystem
        .write_system(LOCAL_INDEX_PATH, &index_bytes, false)
        .map_err(|_| PkgError::InstallFailed)?;
    if filesystem.read(LOCAL_INSTALLED_PATH).is_err() {
        filesystem
            .write_system(LOCAL_INSTALLED_PATH, b"[]", false)
            .map_err(|_| PkgError::InstallFailed)?;
    }
    Ok(())
}

pub(crate) fn require_pkg_install() -> Result<(), PkgError> {
    capabilities::session_require(Capabilities::PKG_INSTALL).map_err(|_| PkgError::PermissionDenied)
}

fn log_package_verification(package: &PackageInfo, outcome: &str, reason: &str) {
    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::PackageVerification {
            name: package.name.clone(),
            version: package.version.clone(),
            outcome: outcome.into(),
            reason: reason.into(),
        },
    );
}

fn map_verify_error(error: signature::VerifyError) -> PkgError {
    match error {
        signature::VerifyError::SignatureMissing => PkgError::UnsignedPackage,
        _ => PkgError::SignatureInvalid,
    }
}

fn verify_error_reason(error: &signature::VerifyError) -> &'static str {
    match error {
        signature::VerifyError::SignatureMissing => "signature-missing",
        signature::VerifyError::UnsupportedSignatureScheme => "unsupported-signature-scheme",
        signature::VerifyError::UnknownTrustRoot => "unknown-trust-root",
        signature::VerifyError::InvalidPublicKey => "invalid-public-key",
        signature::VerifyError::InvalidSignatureEncoding => "invalid-signature-encoding",
        signature::VerifyError::SignatureMismatch => "signature-mismatch",
        signature::VerifyError::CanonicalEncodingFailed => "canonical-encoding-failed",
        signature::VerifyError::DuplicatePayload(_) => "duplicate-payload",
        signature::VerifyError::PayloadMissing(_) => "payload-missing",
        signature::VerifyError::UnexpectedPayload(_) => "unexpected-payload",
        signature::VerifyError::PayloadDigestMismatch(_) => "payload-digest-mismatch",
        signature::VerifyError::PayloadSizeMismatch(_) => "payload-size-mismatch",
        signature::VerifyError::FilePayloadMissing(_) => "file-payload-missing",
        signature::VerifyError::FileSizeMismatch(_) => "file-size-mismatch",
    }
}
