use alloc::string::String;
use alloc::vec::Vec;

use super::{PackageIndex, PkgError};

pub fn resolve_dependencies(index: &PackageIndex, root: &str) -> Result<Vec<String>, PkgError> {
    let mut ordered = Vec::new();
    let mut visiting = Vec::new();
    visit(index, root, &mut visiting, &mut ordered)?;
    Ok(ordered)
}

fn visit(
    index: &PackageIndex,
    package_name: &str,
    visiting: &mut Vec<String>,
    ordered: &mut Vec<String>,
) -> Result<(), PkgError> {
    if ordered.iter().any(|name| name == package_name) {
        return Ok(());
    }
    if visiting.iter().any(|name| name == package_name) {
        return Err(PkgError::DependencyCycle);
    }

    let package = index
        .packages
        .iter()
        .find(|package| package.name == package_name)
        .ok_or_else(|| PkgError::DependencyNotFound(package_name.into()))?;
    visiting.push(package_name.into());
    for dependency in &package.dependencies {
        visit(index, dependency, visiting, ordered)?;
    }
    visiting.retain(|name| name != package_name);
    ordered.push(package_name.into());
    Ok(())
}
