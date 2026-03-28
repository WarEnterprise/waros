use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::auth::FilePermissions;
use crate::fs::{self, FILESYSTEM};

use super::installer;
use super::{verify_local_bundle_bytes, InstalledPackage, PkgError, PACKAGE_MANAGER};

pub const UPDATE_STATE_PATH: &str = "/var/pkg/update-state.json";
pub const STAGED_DIR_PATH: &str = "/var/pkg/staged";
pub const ROLLBACK_DIR_PATH: &str = "/var/pkg/rollback";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum UpdatePhase {
    Staged,
    PendingConfirmation,
    Confirmed,
    Failed,
    RolledBack,
}

impl UpdatePhase {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::PendingConfirmation => "pending-confirmation",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::RolledBack => "rolled-back",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTransaction {
    pub package_name: String,
    pub from_version: Option<String>,
    pub to_version: String,
    pub source_path: String,
    pub staged_path: Option<String>,
    pub rollback_path: String,
    pub phase: UpdatePhase,
    pub applied_at: Option<u64>,
    pub confirmed_at: Option<u64>,
    pub boot_started: bool,
    pub boot_observed: bool,
    pub failure_reason: Option<String>,
    pub last_boot_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateState {
    pub schema_version: u16,
    pub recovery_requested: bool,
    pub recovery_reason: Option<String>,
    pub current: Option<UpdateTransaction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollbackRecord {
    package_name: String,
    to_version: String,
    previous_installed: Option<InstalledPackage>,
    files: Vec<RollbackFileRecord>,
    captured_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RollbackFileRecord {
    path: String,
    existed: bool,
    data: Vec<u8>,
    owner_uid: u16,
    permissions_mode: String,
    readonly: bool,
    created_at: u64,
    modified_at: u64,
}

#[derive(Debug, Clone)]
pub struct BootHealthReport {
    pub recovery_requested: bool,
    pub summary: String,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            recovery_requested: false,
            recovery_reason: None,
            current: None,
        }
    }
}

pub fn ensure_state_file() -> Result<(), PkgError> {
    let exists = {
        let filesystem = FILESYSTEM.lock();
        filesystem.read(UPDATE_STATE_PATH).is_ok()
    };
    if exists {
        return Ok(());
    }
    save_state(&UpdateState::default())
}

pub fn stage_bundle_from_path(path: &str) -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let (resolved, bytes) = read_local_bundle(path)?;
    let verified = verify_local_bundle_bytes(&bytes)?;
    let manifest = &verified.bundle.signed_manifest.manifest;

    let mut state = load_state()?;
    ensure_state_replaceable(&state)?;

    let staged_path = staged_bundle_path(&manifest.name, &manifest.version);
    FILESYSTEM
        .lock()
        .write_system(&staged_path, &bytes, false)
        .map_err(|_| PkgError::InstallFailed)?;

    let transaction = UpdateTransaction {
        package_name: manifest.name.clone(),
        from_version: installed_version_for(&manifest.name),
        to_version: manifest.version.clone(),
        source_path: resolved,
        staged_path: Some(staged_path.clone()),
        rollback_path: rollback_record_path(&manifest.name),
        phase: UpdatePhase::Staged,
        applied_at: None,
        confirmed_at: None,
        boot_started: false,
        boot_observed: false,
        failure_reason: None,
        last_boot_note: Some(String::from("offline bundle staged for explicit apply")),
    };
    state.current = Some(transaction.clone());
    save_state(&state)?;
    log_update_state(
        &manifest.name,
        &manifest.version,
        "staged",
        staged_path.as_str(),
    );
    Ok(transaction)
}

pub fn apply_bundle_from_path(path: &str) -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let (resolved, bytes) = read_local_bundle(path)?;
    apply_verified_bundle_bytes(&resolved, &bytes, None)
}

pub fn apply_staged_bundle() -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let state = load_state()?;
    let staged_path = state
        .current
        .as_ref()
        .and_then(|transaction| transaction.staged_path.clone())
        .ok_or(PkgError::NoStagedBundle)?;
    let bytes = FILESYSTEM
        .lock()
        .read(&staged_path)
        .map(|bytes| bytes.to_vec())
        .map_err(|_| PkgError::NoStagedBundle)?;
    apply_verified_bundle_bytes(&staged_path.clone(), &bytes, Some(staged_path))
}

pub fn confirm_pending_update() -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let mut state = load_state()?;
    let transaction = state.current.as_mut().ok_or(PkgError::NoPendingUpdate)?;
    if transaction.phase != UpdatePhase::PendingConfirmation {
        return Err(PkgError::NoPendingUpdate);
    }
    if !transaction.boot_observed {
        return Err(PkgError::BootConfirmationRequired);
    }

    transaction.phase = UpdatePhase::Confirmed;
    transaction.confirmed_at = Some(now());
    transaction.boot_started = false;
    transaction.boot_observed = false;
    transaction.failure_reason = None;
    transaction.last_boot_note = Some(String::from("update confirmed after shell-ready boot"));
    state.recovery_requested = false;
    state.recovery_reason = None;
    let snapshot = transaction.clone();
    save_state(&state)?;
    log_update_state(
        &snapshot.package_name,
        &snapshot.to_version,
        "confirmed",
        "boot health acknowledged",
    );
    Ok(snapshot)
}

pub fn reject_pending_update(reason: &str) -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let mut state = load_state()?;
    let transaction = state.current.as_mut().ok_or(PkgError::NoPendingUpdate)?;
    if transaction.phase != UpdatePhase::PendingConfirmation {
        return Err(PkgError::NoPendingUpdate);
    }

    transaction.phase = UpdatePhase::Failed;
    transaction.failure_reason = Some(reason.into());
    transaction.last_boot_note = Some(String::from("update explicitly rejected"));
    state.recovery_requested = true;
    state.recovery_reason = Some(String::from("pending update rejected"));
    let snapshot = transaction.clone();
    save_state(&state)?;
    log_update_state(&snapshot.package_name, &snapshot.to_version, "failed", reason);
    Ok(snapshot)
}

pub fn rollback_current_update() -> Result<UpdateTransaction, PkgError> {
    super::require_pkg_install()?;
    let mut state = load_state()?;
    let transaction = state.current.as_mut().ok_or(PkgError::RollbackUnavailable)?;
    let rollback_path = transaction.rollback_path.clone();
    let rollback = load_rollback_record(&rollback_path)?;

    restore_rollback_files(&rollback)?;
    {
        let mut manager = PACKAGE_MANAGER.lock();
        manager
            .installed
            .retain(|package| package.name != rollback.package_name);
        if let Some(previous) = rollback.previous_installed.clone() {
            manager.installed.push(previous);
        }
        manager.save_installed_list()?;
    }

    transaction.phase = UpdatePhase::RolledBack;
    transaction.boot_started = false;
    transaction.boot_observed = false;
    transaction.confirmed_at = Some(now());
    transaction.failure_reason = None;
    transaction.last_boot_note = Some(String::from("rollback restored pre-apply filesystem state"));
    state.recovery_requested = false;
    state.recovery_reason = None;
    let snapshot = transaction.clone();
    save_state(&state)?;
    log_update_state(
        &snapshot.package_name,
        &snapshot.to_version,
        "rolled-back",
        rollback_path.as_str(),
    );
    Ok(snapshot)
}

pub fn clear_completed_state() -> Result<(), PkgError> {
    super::require_pkg_install()?;
    let mut state = load_state()?;
    if state.current.as_ref().is_some_and(|transaction| {
        matches!(
            transaction.phase,
            UpdatePhase::PendingConfirmation | UpdatePhase::Failed
        )
    }) {
        return Err(PkgError::UpdateBusy);
    }
    state.current = None;
    if !state.recovery_requested {
        state.recovery_reason = None;
    }
    save_state(&state)?;
    Ok(())
}

pub fn proof_available() -> Result<bool, PkgError> {
    let state = load_state()?;
    Ok(state.current.is_none() && !state.recovery_requested)
}

pub fn request_recovery(reason: &str) -> Result<(), PkgError> {
    let mut state = load_state()?;
    state.recovery_requested = true;
    state.recovery_reason = Some(reason.into());
    save_state(&state)?;
    log_recovery_action("enter", reason);
    Ok(())
}

pub fn clear_recovery_request() -> Result<(), PkgError> {
    let mut state = load_state()?;
    if state.current.as_ref().is_some_and(|transaction| {
        matches!(
            transaction.phase,
            UpdatePhase::PendingConfirmation | UpdatePhase::Failed
        )
    }) {
        return Err(PkgError::RecoveryBlocked);
    }
    state.recovery_requested = false;
    state.recovery_reason = None;
    save_state(&state)?;
    log_recovery_action("resume", "recovery request cleared");
    Ok(())
}

#[must_use]
pub fn should_enter_recovery() -> bool {
    match load_state() {
        Ok(state) => state.recovery_requested,
        Err(PkgError::StateCorrupted) => true,
        Err(_) => false,
    }
}

pub fn prepare_boot() -> BootHealthReport {
    let mut state = match load_state() {
        Ok(state) => state,
        Err(PkgError::StateCorrupted) => {
            return BootHealthReport {
                recovery_requested: true,
                summary: String::from(
                    "update state metadata is corrupted; recovery mode requested",
                ),
            };
        }
        Err(_) => UpdateState::default(),
    };

    let mut summary = String::from("idle");
    if let Some(transaction) = state.current.as_mut() {
        summary = match transaction.phase {
            UpdatePhase::PendingConfirmation => {
                if transaction.boot_started && !transaction.boot_observed {
                    transaction.phase = UpdatePhase::Failed;
                    transaction.failure_reason =
                        Some(String::from("post-update boot did not reach shell-ready"));
                    state.recovery_requested = true;
                    state.recovery_reason =
                        Some(String::from("post-update boot failed before shell-ready"));
                    log_update_state(
                        &transaction.package_name,
                        &transaction.to_version,
                        "failed",
                        "post-update boot did not reach shell-ready",
                    );
                    alloc::format!(
                        "{} {} failed health check before shell-ready; recovery requested",
                        transaction.package_name, transaction.to_version
                    )
                } else if transaction.boot_observed {
                    transaction.phase = UpdatePhase::Failed;
                    transaction.failure_reason = Some(String::from(
                        "post-update boot was not confirmed before the next boot",
                    ));
                    state.recovery_requested = true;
                    state.recovery_reason =
                        Some(String::from("post-update boot was not confirmed"));
                    log_update_state(
                        &transaction.package_name,
                        &transaction.to_version,
                        "failed",
                        "post-update boot was not confirmed before the next boot",
                    );
                    alloc::format!(
                        "{} {} was not confirmed before reboot; recovery requested",
                        transaction.package_name, transaction.to_version
                    )
                } else {
                    transaction.boot_started = true;
                    transaction.last_boot_note =
                        Some(String::from("boot started; awaiting confirmation"));
                    alloc::format!(
                        "{} {} pending confirmation",
                        transaction.package_name, transaction.to_version
                    )
                }
            }
            UpdatePhase::Staged => alloc::format!(
                "{} {} staged for explicit apply",
                transaction.package_name, transaction.to_version
            ),
            UpdatePhase::Confirmed => alloc::format!(
                "{} {} confirmed",
                transaction.package_name, transaction.to_version
            ),
            UpdatePhase::Failed => {
                state.recovery_requested = true;
                if state.recovery_reason.is_none() {
                    state.recovery_reason = Some(String::from("failed update requires recovery"));
                }
                alloc::format!(
                    "{} {} failed and requires recovery",
                    transaction.package_name, transaction.to_version
                )
            }
            UpdatePhase::RolledBack => alloc::format!(
                "{} {} rolled back",
                transaction.package_name, transaction.to_version
            ),
        };
    }

    let _ = save_state(&state);
    BootHealthReport {
        recovery_requested: state.recovery_requested,
        summary,
    }
}

pub fn note_shell_ready() -> Result<Option<String>, PkgError> {
    let mut state = load_state()?;
    let Some(transaction) = state.current.as_mut() else {
        return Ok(None);
    };
    if transaction.phase != UpdatePhase::PendingConfirmation || !transaction.boot_started {
        return Ok(None);
    }
    if transaction.boot_observed {
        return Ok(None);
    }

    transaction.boot_observed = true;
    transaction.last_boot_note = Some(String::from("shell-ready reached; confirmation required"));
    let message = alloc::format!(
        "{} {} reached shell-ready; confirmation required",
        transaction.package_name, transaction.to_version
    );
    save_state(&state)?;
    Ok(Some(message))
}

pub fn status_report() -> String {
    match load_state() {
        Ok(state) => render_state(&state),
        Err(PkgError::StateCorrupted) => String::from(
            "WarPkg Offline Update / Recovery:\n\
\n  State:             metadata-corrupted\n\
\n  Recovery request:  yes\n\
\n  Recovery reason:   update state metadata is corrupted",
        ),
        Err(_) => render_state(&UpdateState::default()),
    }
}

#[must_use]
pub fn short_status() -> String {
    match load_state() {
        Ok(state) => {
            if let Some(transaction) = state.current {
                alloc::format!(
                    "{} {} ({}){}",
                    transaction.package_name,
                    transaction.to_version,
                    transaction.phase.as_str(),
                    if state.recovery_requested {
                        ", recovery requested"
                    } else {
                        ""
                    }
                )
            } else if state.recovery_requested {
                alloc::format!(
                    "manual recovery requested ({})",
                    state.recovery_reason
                        .unwrap_or_else(|| String::from("unspecified"))
                )
            } else {
                String::from("idle")
            }
        }
        Err(PkgError::StateCorrupted) => String::from("metadata-corrupted (recovery requested)"),
        Err(_) => String::from("idle"),
    }
}

fn apply_verified_bundle_bytes(
    source_path: &str,
    bytes: &[u8],
    staged_override: Option<String>,
) -> Result<UpdateTransaction, PkgError> {
    let mut state = load_state()?;
    ensure_state_replaceable(&state)?;

    let verified = verify_local_bundle_bytes(bytes)?;
    let manifest = &verified.bundle.signed_manifest.manifest;
    let previous_installed = {
        let manager = PACKAGE_MANAGER.lock();
        manager
            .installed
            .iter()
            .find(|package| package.name == manifest.name)
            .cloned()
    };

    let rollback_path = rollback_record_path(&manifest.name);
    let rollback = capture_rollback_record(manifest, previous_installed.clone());
    store_rollback_record(&rollback_path, &rollback)?;

    {
        let mut manager = PACKAGE_MANAGER.lock();
        let _ = installer::apply_verified_bundle(
            &mut manager,
            &verified,
            bytes.len() as u64,
            crate::exec::current_uid(),
        )?;
    }

    let transaction = UpdateTransaction {
        package_name: manifest.name.clone(),
        from_version: previous_installed.as_ref().map(|package| package.version.clone()),
        to_version: manifest.version.clone(),
        source_path: source_path.into(),
        staged_path: staged_override.or_else(|| {
            if source_path.starts_with(STAGED_DIR_PATH) {
                Some(source_path.into())
            } else {
                None
            }
        }),
        rollback_path: rollback_path.clone(),
        phase: UpdatePhase::PendingConfirmation,
        applied_at: Some(now()),
        confirmed_at: None,
        boot_started: false,
        boot_observed: false,
        failure_reason: None,
        last_boot_note: Some(String::from("offline apply completed; awaiting boot confirmation")),
    };
    state.current = Some(transaction.clone());
    state.recovery_requested = false;
    state.recovery_reason = None;
    save_state(&state)?;
    log_update_state(
        &transaction.package_name,
        &transaction.to_version,
        "pending-confirmation",
        source_path,
    );
    Ok(transaction)
}

fn capture_rollback_record(
    manifest: &crate::pkg::manifest::Manifest,
    previous_installed: Option<InstalledPackage>,
) -> RollbackRecord {
    let mut paths = Vec::<String>::new();
    for file in &manifest.files {
        push_unique_path(&mut paths, &file.path);
    }
    if let Some(previous) = &previous_installed {
        for path in &previous.files {
            push_unique_path(&mut paths, path);
        }
    }

    let filesystem = FILESYSTEM.lock();
    let files = paths
        .into_iter()
        .map(|path| match filesystem.stat(&path) {
            Ok(entry) => RollbackFileRecord {
                path,
                existed: true,
                data: entry.data.clone(),
                owner_uid: entry.owner_uid,
                permissions_mode: entry.permissions.mode_string(),
                readonly: entry.readonly,
                created_at: entry.created_at,
                modified_at: entry.modified_at,
            },
            Err(_) => RollbackFileRecord {
                path,
                existed: false,
                data: Vec::new(),
                owner_uid: 0,
                permissions_mode: String::from("rw--"),
                readonly: false,
                created_at: 0,
                modified_at: 0,
            },
        })
        .collect();

    RollbackRecord {
        package_name: manifest.name.clone(),
        to_version: manifest.version.clone(),
        previous_installed,
        files,
        captured_at: now(),
    }
}

fn restore_rollback_files(rollback: &RollbackRecord) -> Result<(), PkgError> {
    let mut filesystem = FILESYSTEM.lock();
    for file in &rollback.files {
        if file.existed {
            let mut permissions = FilePermissions::default_for(file.owner_uid);
            if !permissions.apply_mode_string(&file.permissions_mode) {
                return Err(PkgError::RollbackUnavailable);
            }
            filesystem
                .load_file_from_disk(
                    &file.path,
                    &file.data,
                    file.owner_uid,
                    permissions,
                    file.readonly,
                    file.created_at,
                    file.modified_at,
                )
                .map_err(|_| PkgError::InstallFailed)?;
            if let Some(entry) = filesystem.find(&file.path) {
                crate::disk::maybe_sync_file_entry(entry);
            }
        } else {
            match filesystem.delete(&file.path) {
                Ok(()) | Err(fs::FsError::FileNotFound) => {}
                Err(_) => return Err(PkgError::InstallFailed),
            }
        }
    }
    Ok(())
}

fn read_local_bundle(path: &str) -> Result<(String, Vec<u8>), PkgError> {
    fs::read_current(path).map_err(|_| PkgError::BundleUnavailable)
}

fn save_state(state: &UpdateState) -> Result<(), PkgError> {
    let bytes = serde_json::to_vec(state).map_err(|_| PkgError::InstallFailed)?;
    FILESYSTEM
        .lock()
        .write_system(UPDATE_STATE_PATH, &bytes, false)
        .map_err(|_| PkgError::InstallFailed)
}

fn load_state() -> Result<UpdateState, PkgError> {
    let bytes = {
        let filesystem = FILESYSTEM.lock();
        match filesystem.read(UPDATE_STATE_PATH) {
            Ok(bytes) => bytes.to_vec(),
            Err(fs::FsError::FileNotFound) => return Ok(UpdateState::default()),
            Err(_) => return Err(PkgError::InstallFailed),
        }
    };
    serde_json::from_slice(&bytes).map_err(|_| PkgError::StateCorrupted)
}

fn ensure_state_replaceable(state: &UpdateState) -> Result<(), PkgError> {
    if state.current.as_ref().is_some_and(|transaction| {
        matches!(
            transaction.phase,
            UpdatePhase::PendingConfirmation | UpdatePhase::Failed
        )
    }) {
        return Err(PkgError::UpdateBusy);
    }
    Ok(())
}

fn store_rollback_record(path: &str, rollback: &RollbackRecord) -> Result<(), PkgError> {
    let bytes = serde_json::to_vec(rollback).map_err(|_| PkgError::InstallFailed)?;
    FILESYSTEM
        .lock()
        .write_system(path, &bytes, false)
        .map_err(|_| PkgError::InstallFailed)
}

fn load_rollback_record(path: &str) -> Result<RollbackRecord, PkgError> {
    let bytes = {
        let filesystem = FILESYSTEM.lock();
        filesystem
            .read(path)
            .map(|bytes| bytes.to_vec())
            .map_err(|_| PkgError::RollbackUnavailable)?
    };
    serde_json::from_slice(&bytes).map_err(|_| PkgError::RollbackUnavailable)
}

fn installed_version_for(name: &str) -> Option<String> {
    PACKAGE_MANAGER
        .lock()
        .installed
        .iter()
        .find(|package| package.name == name)
        .map(|package| package.version.clone())
}

fn render_state(state: &UpdateState) -> String {
    let mut lines = Vec::<String>::new();
    lines.push(String::from("WarPkg Offline Update / Recovery:"));
    lines.push(String::new());
    if let Some(transaction) = &state.current {
        lines.push(alloc::format!(
            "  Package:           {} {}",
            transaction.package_name, transaction.to_version
        ));
        if let Some(from_version) = &transaction.from_version {
            lines.push(alloc::format!("  Previous version:  {}", from_version));
        }
        lines.push(alloc::format!(
            "  State:             {}",
            transaction.phase.as_str()
        ));
        lines.push(alloc::format!("  Source:            {}", transaction.source_path));
        if let Some(staged_path) = &transaction.staged_path {
            lines.push(alloc::format!("  Staged bundle:     {}", staged_path));
        }
        lines.push(alloc::format!("  Rollback record:   {}", transaction.rollback_path));
        lines.push(alloc::format!(
            "  Boot health:       started={} shell-ready={}",
            transaction.boot_started, transaction.boot_observed
        ));
        if let Some(reason) = &transaction.failure_reason {
            lines.push(alloc::format!("  Failure reason:    {}", reason));
        }
        if let Some(note) = &transaction.last_boot_note {
            lines.push(alloc::format!("  Last note:         {}", note));
        }
    } else {
        lines.push(String::from("  State:             idle"));
    }
    lines.push(alloc::format!(
        "  Recovery request:  {}",
        if state.recovery_requested { "yes" } else { "no" }
    ));
    if let Some(reason) = &state.recovery_reason {
        lines.push(alloc::format!("  Recovery reason:   {}", reason));
    }
    lines.join("\n")
}

fn push_unique_path(paths: &mut Vec<String>, path: &str) {
    if paths.iter().all(|existing| existing != path) {
        paths.push(path.into());
    }
}

fn staged_bundle_path(name: &str, version: &str) -> String {
    alloc::format!("{STAGED_DIR_PATH}/{name}-{version}.warpack")
}

fn rollback_record_path(name: &str) -> String {
    alloc::format!("{ROLLBACK_DIR_PATH}/{name}.json")
}

fn log_update_state(name: &str, version: &str, state: &str, detail: &str) {
    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::SecurityPolicyChanged {
            change: alloc::format!(
                "update_state name={} ver={} state={} detail={}",
                name, version, state, detail
            ),
            uid: crate::exec::current_uid(),
        },
    );
}

fn log_recovery_action(action: &str, detail: &str) {
    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::SecurityPolicyChanged {
            change: alloc::format!("recovery action={} detail={}", action, detail),
            uid: crate::exec::current_uid(),
        },
    );
}

fn now() -> u64 {
    crate::arch::x86_64::interrupts::tick_count()
}
