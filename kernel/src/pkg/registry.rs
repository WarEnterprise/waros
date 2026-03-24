use alloc::string::String;

use crate::fs;
use crate::net;

use super::{PackageIndex, PkgError, LOCAL_INDEX_PATH};

pub fn load_local_index() -> Result<PackageIndex, PkgError> {
    let filesystem = fs::FILESYSTEM.lock();
    let data = filesystem
        .read(LOCAL_INDEX_PATH)
        .map_err(|_| PkgError::PackageNotFound)?;
    serde_json::from_slice(data).map_err(|_| PkgError::ExtractFailed)
}

pub fn fetch_index(repo_url: &str) -> Result<PackageIndex, PkgError> {
    let url = alloc::format!("{repo_url}/index.json");
    let response = net::http_get(&url).map_err(|_| PkgError::NetworkError)?;
    serde_json::from_slice(&response.body).map_err(|_| PkgError::ExtractFailed)
}

pub fn bundle_path(name: &str) -> String {
    alloc::format!("/var/pkg/repo/{name}.warpack")
}
