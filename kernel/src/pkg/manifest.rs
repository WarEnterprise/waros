use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub source: String,
    pub executable: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarPackPayload {
    pub source: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarPackBundle {
    pub manifest: Manifest,
    pub payloads: Vec<WarPackPayload>,
}
