//! Minimal firmware registry/lookup plumbing for staged network drivers.
//!
//! WarOS still does not implement a generic DMA/upload path for firmware-
//! driven PCIe devices, but drivers need a truthful way to answer:
//! - is the required blob name known?
//! - is a matching blob present in WarFS?
//! - has any load/upload attempt happened?

use alloc::string::String;
use alloc::vec::Vec;

use crate::fs::{self, FILESYSTEM};

const FIRMWARE_SEARCH_ROOTS: [&str; 3] = ["/lib/firmware", "/lib/firmware/intel", "/firmware"];

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareLoadState {
    Missing,
    LoaderUnavailable,
    Ready,
}

impl FirmwareLoadState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::LoaderUnavailable => "loader-unavailable",
            Self::Ready => "ready",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FirmwareRequirement {
    pub name_pattern: &'static str,
    pub state: FirmwareLoadState,
    pub provider: &'static str,
    pub reason: &'static str,
    pub blob_found: bool,
    pub blob_path: Option<String>,
    pub load_attempted: bool,
    pub upload_state: &'static str,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FirmwareBlob {
    pub path: String,
    pub data: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareLoadError {
    Missing,
    ReadFailed,
}

impl FirmwareRequirement {
    #[must_use]
    pub fn missing(name_pattern: &'static str) -> Self {
        Self {
            name_pattern,
            state: FirmwareLoadState::Missing,
            provider: "warfs-firmware-registry",
            reason: "no matching firmware blob was found in the WarFS firmware roots",
            blob_found: false,
            blob_path: None,
            load_attempted: false,
            upload_state: "not-attempted",
        }
    }

    #[must_use]
    fn available(name_pattern: &'static str, path: String) -> Self {
        Self {
            name_pattern,
            state: FirmwareLoadState::Ready,
            provider: "warfs-firmware-registry",
            reason: "matching firmware blob is present and readable; upload plumbing is still staged",
            blob_found: true,
            blob_path: Some(path),
            load_attempted: false,
            upload_state: "not-attempted",
        }
    }
}

#[must_use]
pub fn firmware_search_roots() -> &'static [&'static str] {
    &FIRMWARE_SEARCH_ROOTS
}

#[must_use]
pub fn require_blob(name_pattern: &'static str) -> FirmwareRequirement {
    find_blob_path(name_pattern)
        .map(|path| FirmwareRequirement::available(name_pattern, path))
        .unwrap_or_else(|| FirmwareRequirement::missing(name_pattern))
}

#[allow(dead_code)]
pub fn load_blob(name_pattern: &'static str) -> Result<FirmwareBlob, FirmwareLoadError> {
    let Some(path) = find_blob_path(name_pattern) else {
        return Err(FirmwareLoadError::Missing);
    };
    let (_, data) = fs::read_current(&path).map_err(|_| FirmwareLoadError::ReadFailed)?;
    Ok(FirmwareBlob { path, data })
}

fn find_blob_path(name_pattern: &str) -> Option<String> {
    let filesystem = FILESYSTEM.lock();
    for root in firmware_search_roots() {
        for entry in filesystem.list() {
            if !entry.name.starts_with(root) {
                continue;
            }
            if matches_pattern(fs::basename(&entry.name), name_pattern) {
                return Some(entry.name.clone());
            }
        }
    }
    None
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return name == pattern;
    };
    name.len() >= prefix.len().saturating_add(suffix.len())
        && name.starts_with(prefix)
        && name.ends_with(suffix)
}
