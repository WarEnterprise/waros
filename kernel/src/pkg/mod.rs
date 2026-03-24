use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use spin::{Lazy, Mutex};

use crate::fs;
use crate::net;

pub mod commands;
pub mod installer;
pub mod manifest;
pub mod registry;
pub mod resolver;
pub mod signature;

use installer::{install as install_package, remove as remove_package};
use manifest::{Manifest, ManifestFile, WarPackBundle, WarPackPayload};
use registry::{bundle_path, fetch_index, load_local_index};
use resolver::resolve_dependencies;
use signature::{sha256_hex, sign_bootstrap};

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
    pub signature: String,
    pub dependencies: Vec<String>,
    pub download_url: String,
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
    SignatureInvalid,
    DependencyCycle,
    DependencyNotFound(String),
    DownloadFailed,
    ExtractFailed,
    InstallFailed,
    NetworkError,
}

impl core::fmt::Display for PkgError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PackageNotFound => formatter.write_str("package not found"),
            Self::PackageNotInstalled => formatter.write_str("package is not installed"),
            Self::ChecksumMismatch => formatter.write_str("package checksum mismatch"),
            Self::SignatureInvalid => formatter.write_str("package signature invalid"),
            Self::DependencyCycle => formatter.write_str("dependency cycle detected"),
            Self::DependencyNotFound(name) => {
                write!(formatter, "dependency '{}' missing from package index", name)
            }
            Self::DownloadFailed => formatter.write_str("package download failed"),
            Self::ExtractFailed => formatter.write_str("package extraction failed"),
            Self::InstallFailed => formatter.write_str("package installation failed"),
            Self::NetworkError => formatter.write_str("network error while updating package index"),
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
        if sha256_hex(&bytes) != info.sha256 {
            return Err(PkgError::ChecksumMismatch);
        }
        if !signature::verify_bootstrap(&bytes, &info.signature) {
            return Err(PkgError::SignatureInvalid);
        }
        Ok(())
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
    Ok(())
}

pub fn with_manager<T>(f: impl FnOnce(&mut PackageManager) -> Result<T, PkgError>) -> Result<T, PkgError> {
    let mut manager = PACKAGE_MANAGER.lock();
    f(&mut manager)
}

fn load_installed() -> Option<Vec<InstalledPackage>> {
    let filesystem = fs::FILESYSTEM.lock();
    let data = filesystem.read(LOCAL_INSTALLED_PATH).ok()?;
    serde_json::from_slice(data).ok()
}

fn seed_repository() -> Result<(), PkgError> {
    let packages = built_in_packages();
    let mut index_packages = Vec::new();
    let mut filesystem = fs::FILESYSTEM.lock();

    for bundle in packages {
        let bytes = serde_json::to_vec(&bundle).map_err(|_| PkgError::ExtractFailed)?;
        let name = bundle.manifest.name.clone();
        let path = bundle_path(&name);
        filesystem
            .write_system(&path, &bytes, false)
            .map_err(|_| PkgError::InstallFailed)?;
        index_packages.push(PackageInfo {
            name: bundle.manifest.name.clone(),
            version: bundle.manifest.version.clone(),
            description: bundle.manifest.description.clone(),
            size_bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            signature: sign_bootstrap(&bytes),
            dependencies: bundle.manifest.dependencies.clone(),
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

fn built_in_packages() -> Vec<WarPackBundle> {
    vec![
        package(
            "quantum-examples",
            "Quantum example circuits",
            "quantum",
            vec![
                file("/usr/share/quantum/bell.qasm", "bell.qasm", false, include_str!("../../../examples/qasm/bell.qasm")),
                file("/usr/share/quantum/ghz5.qasm", "ghz5.qasm", false, include_str!("../../../examples/qasm/ghz5.qasm")),
                file("/usr/share/quantum/grover2.qasm", "grover2.qasm", false, include_str!("../../../examples/qasm/grover2.qasm")),
                file("/usr/share/quantum/qft4.qasm", "qft4.qasm", false, include_str!("../../../examples/qasm/qft4.qasm")),
            ],
            Vec::new(),
        ),
        package(
            "crypto-tools",
            "Post-quantum crypto command helpers",
            "crypto",
            vec![file(
                "/usr/bin/crypto-tools",
                "crypto-tools",
                true,
                "#!warsh\ncrypto\n",
            )],
            Vec::new(),
        ),
        package(
            "network-utils",
            "Network helper scripts",
            "binary",
            vec![
                file("/usr/bin/net-status", "net-status", true, "#!warsh\nnet status\n"),
                file("/usr/bin/net-dns", "net-dns", true, "#!warsh\ndns warenterprise.com\n"),
            ],
            Vec::new(),
        ),
        package(
            "system-monitor",
            "Top-style system dashboard launcher",
            "binary",
            vec![file("/usr/bin/system-monitor", "system-monitor", true, "#!warsh\ntop\n")],
            Vec::new(),
        ),
        package(
            "waros-docs",
            "Offline WarOS documentation",
            "docs",
            vec![file(
                "/usr/share/doc/waros/README.txt",
                "README.txt",
                false,
                "WarOS package repository bootstrap.\nUse 'warpkg list' and 'warpkg info <name>'.\n",
            )],
            Vec::new(),
        ),
        package(
            "quantum-benchmarks",
            "Quantum benchmark scripts",
            "quantum",
            vec![file(
                "/usr/share/quantum/benchmarks.txt",
                "benchmarks.txt",
                false,
                "bell: 2 qubits\nqft4: 4 qubits\nghz5: 5 qubits\n",
            )],
            vec![String::from("quantum-examples")],
        ),
        package(
            "hello-world",
            "WarExec hello-world launcher",
            "binary",
            vec![file("/usr/bin/hello", "hello", true, "#!warsh\necho Hello from WarOS package manager\n")],
            Vec::new(),
        ),
        package(
            "war-shell-plugins",
            "Shell helper launchers",
            "binary",
            vec![
                file("/usr/bin/waros-help", "waros-help", true, "#!warsh\nhelp\n"),
                file("/usr/bin/waros-version", "waros-version", true, "#!warsh\nversion --all\n"),
            ],
            Vec::new(),
        ),
    ]
}

fn package(
    name: &str,
    description: &str,
    category: &str,
    files: Vec<(ManifestFile, WarPackPayload)>,
    dependencies: Vec<String>,
) -> WarPackBundle {
    let (manifest_files, payloads): (Vec<_>, Vec<_>) = files.into_iter().unzip();
    WarPackBundle {
        manifest: Manifest {
            name: name.into(),
            version: String::from("0.1.0"),
            description: description.into(),
            author: String::from("War Enterprise"),
            license: String::from("Proprietary"),
            files: manifest_files,
            dependencies,
            min_waros_version: crate::KERNEL_VERSION.into(),
            category: category.into(),
        },
        payloads,
    }
}

fn file(path: &str, source: &str, executable: bool, contents: &str) -> (ManifestFile, WarPackPayload) {
    (
        ManifestFile {
            path: path.into(),
            source: source.into(),
            executable,
            size: contents.len() as u64,
        },
        WarPackPayload {
            source: source.into(),
            contents: contents.into(),
        },
    )
}
