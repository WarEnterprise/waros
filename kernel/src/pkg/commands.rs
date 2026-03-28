use alloc::string::String;

use crate::display::console::Colors;
use crate::{kprint_colored, kprintln};

use super::{update, with_manager, PackageInfo, PkgError};

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
        "status" => {
            kprintln!("{}", update::status_report());
        }
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
        "stage" => {
            let Some(path) = args.get(1).copied() else {
                kprintln!("Usage: warpkg stage <bundle-path>");
                return;
            };
            match update::stage_bundle_from_path(path) {
                Ok(transaction) => {
                    kprint_colored!(Colors::GREEN, "[WarPkg] ");
                    kprintln!(
                        "{} {} staged from {}.",
                        transaction.package_name,
                        transaction.to_version,
                        transaction.source_path
                    );
                }
                Err(error) => report_error(error),
            }
        }
        "apply" => {
            let Some(source) = args.get(1).copied() else {
                kprintln!("Usage: warpkg apply <bundle-path|staged>");
                return;
            };
            let result = if source == "staged" {
                update::apply_staged_bundle()
            } else {
                update::apply_bundle_from_path(source)
            };
            match result {
                Ok(transaction) => {
                    kprint_colored!(Colors::GREEN, "[WarPkg] ");
                    kprintln!(
                        "{} {} applied offline. Reboot, reach shell, then confirm with 'warpkg confirm' or 'recovery confirm'.",
                        transaction.package_name,
                        transaction.to_version
                    );
                }
                Err(error) => report_error(error),
            }
        }
        "confirm" => match update::confirm_pending_update() {
            Ok(transaction) => {
                kprint_colored!(Colors::GREEN, "[WarPkg] ");
                kprintln!(
                    "{} {} marked confirmed.",
                    transaction.package_name,
                    transaction.to_version
                );
            }
            Err(error) => report_error(error),
        },
        "reject" => {
            let reason = args
                .get(1..)
                .filter(|parts| !parts.is_empty())
                .map(|parts| parts.join(" "))
                .unwrap_or_else(|| String::from("operator rejected pending update"));
            match update::reject_pending_update(&reason) {
                Ok(transaction) => {
                    kprint_colored!(Colors::YELLOW, "[WarPkg] ");
                    kprintln!(
                        "{} {} marked failed and recovery requested.",
                        transaction.package_name,
                        transaction.to_version
                    );
                }
                Err(error) => report_error(error),
            }
        }
        "rollback" => match update::rollback_current_update() {
            Ok(transaction) => {
                kprint_colored!(Colors::GREEN, "[WarPkg] ");
                kprintln!(
                    "{} {} rolled back to the pre-apply filesystem state.",
                    transaction.package_name,
                    transaction.to_version
                );
            }
            Err(error) => report_error(error),
        },
        "proof" => match super::smoke::run_offline_update_proof() {
            Ok(()) => {
                kprint_colored!(Colors::GREEN, "[WarPkg] ");
                kprintln!("offline update proof passed.");
            }
            Err(error) => {
                kprint_colored!(Colors::RED, "[WarPkg] ");
                kprintln!("{}", error);
            }
        },
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
                    kprintln!(
                        "  {:<20} {:<8} {}",
                        package.name,
                        package.version,
                        package.description
                    );
                }
            }
            Err(error) => report_error(error),
        },
        "info" => {
            let Some(name) = args.get(1).copied() else {
                kprintln!("Usage: warpkg info <name>");
                return;
            };
            match with_manager(|manager| {
                manager
                    .package_info(name)
                    .cloned()
                    .ok_or(super::PkgError::PackageNotFound)
            }) {
                Ok(info) => {
                    kprintln!("Package: {}", info.name);
                    kprintln!("  Version:      {}", info.version);
                    kprintln!("  Description:  {}", info.description);
                    kprintln!("  Size:         {} bytes", info.size_bytes);
                    kprintln!(
                        "  Dependencies: {}",
                        if info.dependencies.is_empty() {
                            String::from("none")
                        } else {
                            info.dependencies.join(", ")
                        }
                    );
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
                    kprintln!(
                        "{name} verified against the signed bundle manifest and payload digests."
                    );
                }
                Err(error) => report_error(error),
            }
        }
        _ => print_help(),
    }
}

fn print_help() {
    kprintln!("WarPkg commands:");
    kprintln!("  warpkg update");
    kprintln!("  warpkg search <query>");
    kprintln!("  warpkg list");
    kprintln!("  warpkg info <name>");
    kprintln!("  warpkg verify <name>");
    kprintln!("  warpkg install <name>");
    kprintln!("  warpkg remove <name>");
    kprintln!("  warpkg status");
    kprintln!("  warpkg stage <bundle-path>");
    kprintln!("  warpkg apply <bundle-path|staged>");
    kprintln!("  warpkg confirm");
    kprintln!("  warpkg reject [reason]");
    kprintln!("  warpkg rollback");
    kprintln!("  warpkg proof");
    kprintln!("  update uses the validated kernel HTTPS path for supported hosts and falls back to the seeded local index.");
    kprintln!("  stage/apply are the narrow offline update path: local signed bundle only, explicit operator action, no auto-update service.");
    kprintln!("  apply, confirm, reject, rollback, install, and remove require PKG_INSTALL.");
    kprintln!("  apply and verify reject unsigned, tampered, or metadata-mismatched bundles.");
    kprintln!("  post-apply boots require shell-ready plus explicit confirmation; unconfirmed boots are marked failed and request recovery.");
    kprintln!("  proof runs a controlled local self-test of apply, health confirmation, rollback preparation, and tamper rejection.");
    kprintln!("  HTTPS trust is narrow: embedded roots + hostname checks for supported hosts only; no RTC-backed expiry check.");
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

fn report_error(error: PkgError) {
    kprint_colored!(Colors::RED, "[WarPkg] ");
    kprintln!("{}", error);
}
