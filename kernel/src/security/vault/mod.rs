pub mod integrity;

use alloc::string::String;
use alloc::vec::Vec;

use spin::Mutex;

use integrity::{IntegrityEntry, IntegrityViolation, compute_hash, hash_to_hex, verify_entry};

/// Critical system paths to monitor.
const MONITORED_PATHS: &[&str] = &[
    "/etc/users.db",
    "/etc/firewall.conf",
    "/var/pkg/index.json",
];

pub struct WarVault {
    entries: Vec<IntegrityEntry>,
}

static VAULT: Mutex<Option<WarVault>> = Mutex::new(None);

impl WarVault {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn build(&mut self) {
        self.entries.clear();
        let fs = crate::fs::FILESYSTEM.lock();
        for &path in MONITORED_PATHS {
            if let Ok(data) = fs.read(path) {
                let hash = compute_hash(data);
                self.entries.push(IntegrityEntry {
                    path: String::from(path),
                    sha3_hash: hash,
                    size: data.len(),
                });
                crate::serial_println!(
                    "[WarVault] Hashed {} ({} bytes) = {}",
                    path,
                    data.len(),
                    hash_to_hex(&hash)
                );
            }
        }
    }

    fn verify(&self) -> Vec<IntegrityViolation> {
        let mut violations = Vec::new();
        for entry in &self.entries {
            if let Err(violation) = verify_entry(entry) {
                crate::security::audit::log_event(
                    crate::security::audit::events::AuditEvent::IntegrityViolation {
                        path: violation.path.clone(),
                        expected_hash: violation.expected.clone(),
                        actual_hash: violation.actual.clone(),
                    },
                );
                violations.push(violation);
            }
        }
        violations
    }
}

pub fn init() {
    *VAULT.lock() = Some(WarVault::new());
}

pub fn build_database() -> usize {
    let mut vault = VAULT.lock();
    if let Some(v) = vault.as_mut() {
        v.build();
        v.entries.len()
    } else {
        0
    }
}

pub fn verify_all() -> Vec<IntegrityViolation> {
    let vault = VAULT.lock();
    vault.as_ref().map_or_else(Vec::new, |v| v.verify())
}

pub fn monitored_count() -> usize {
    VAULT.lock().as_ref().map_or(0, |v| v.entries.len())
}
