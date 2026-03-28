use crate::display::console::Colors;
use crate::{kprint_colored, kprintln};

use super::{with_manager, PackageInfo};

pub fn handle(args: &[&str]) {
    let Some(command) = args.first().copied() else {
        print_help();
        return;
    };

    match command {
        "update" => match with_manager(|manager| manager.update()) {
            Ok(()) => {
                kprint_colored!(Colors::GREEN, "[WarPkg] ");
                kprintln!("package index updated.");
            }
            Err(error) => report_error(error),
        },
        "search" => {
            let Some(query) = args.get(1).copied() else {
                kprintln!("Usage: warpkg search <query>");
                return;
            };
            match with_manager(|manager| Ok::<_, super::PkgError>(manager.search(query))) {
                Ok(results) => print_search_results(&results),
                Err(error) => report_error(error),
            }
        }
        "install" => {
            let Some(name) = args.get(1).copied() else {
                kprintln!("Usage: warpkg install <name>");
                return;
            };
            match with_manager(|manager| manager.install(name)) {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "[WarPkg] ");
                    kprintln!("{name} installed.");
                }
                Err(error) => report_error(error),
            }
        }
        "remove" => {
            let Some(name) = args.get(1).copied() else {
                kprintln!("Usage: warpkg remove <name>");
                return;
            };
            match with_manager(|manager| manager.remove(name)) {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "[WarPkg] ");
                    kprintln!("{name} removed.");
                }
                Err(error) => report_error(error),
            }
        }
        "list" => match with_manager(|manager| Ok::<_, super::PkgError>(manager.installed.clone())) {
            Ok(installed) => {
                if installed.is_empty() {
                    kprintln!("No packages installed.");
                    return;
                }
                kprintln!("Installed packages:");
                for package in installed {
                    kprintln!("  {:<20} {:<8} {}", package.name, package.version, package.description);
                }
            }
            Err(error) => report_error(error),
        },
        "info" => {
            let Some(name) = args.get(1).copied() else {
                kprintln!("Usage: warpkg info <name>");
                return;
            };
            match with_manager(|manager| manager.package_info(name).cloned().ok_or(super::PkgError::PackageNotFound)) {
                Ok(info) => {
                    kprintln!("Package: {}", info.name);
                    kprintln!("  Version:      {}", info.version);
                    kprintln!("  Description:  {}", info.description);
                    kprintln!("  Size:         {} bytes", info.size_bytes);
                    kprintln!("  Dependencies: {}", if info.dependencies.is_empty() { String::from("none") } else { info.dependencies.join(", ") });
                    kprintln!("  Signature:    {}", info.signature_scheme);
                    kprintln!("  Signed by:    {}", info.signed_by);
                    kprintln!("  Source:       {}", info.download_url);
                }
                Err(error) => report_error(error),
            }
        }
        "verify" => {
            let Some(name) = args.get(1).copied() else {
                kprintln!("Usage: warpkg verify <name>");
                return;
            };
            match with_manager(|manager| manager.verify(name)) {
                Ok(()) => {
                    kprint_colored!(Colors::GREEN, "[WarPkg] ");
                    kprintln!("{name} verified against the signed manifest and payload digests.");
                }
                Err(error) => report_error(error),
            }
        }
        _ => print_help(),
    }
}

use alloc::string::String;
fn print_help() {
    kprintln!("WarPkg commands:");
    kprintln!("  warpkg update");
    kprintln!("  warpkg search <query>");
    kprintln!("  warpkg install <name>");
    kprintln!("  warpkg remove <name>");
    kprintln!("  warpkg list");
    kprintln!("  warpkg info <name>");
    kprintln!("  warpkg verify <name>");
    kprintln!("  update uses the current kernel HTTP/TLS path when available and falls back to the seeded local index.");
    kprintln!("  install and remove require PKG_INSTALL.");
    kprintln!("  install and verify reject unsigned, tampered, or metadata-mismatched bundles.");
    kprintln!("  trust model: one embedded bootstrap ML-DSA root; no rotation or revocation yet.");
}

fn print_search_results(results: &[PackageInfo]) {
    if results.is_empty() {
        kprintln!("No packages found.");
        return;
    }
    kprintln!("Available packages:");
    for package in results {
        kprintln!(
            "  {:<20} {:<8} {}",
            package.name,
            package.version,
            package.description
        );
    }
}

fn report_error(error: super::PkgError) {
    kprint_colored!(Colors::RED, "[WarPkg] ");
    kprintln!("{}", error);
}
