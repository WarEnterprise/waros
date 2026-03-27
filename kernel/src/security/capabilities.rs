use bitflags::bitflags;

bitflags! {
    /// WarOS capability bitfield — bits 0-15 standard, bits 16-31 WarOS-exclusive.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Capabilities: u64 {
        // Standard capabilities (bits 0-15)
        const USER_ADMIN       = 1 << 0;
        const FS_ADMIN         = 1 << 1;
        const FILE_READ_ALL    = 1 << 2;
        const FILE_WRITE_ALL   = 1 << 3;
        const PROCESS_ADMIN    = 1 << 4;
        const NET_ADMIN        = 1 << 5;
        const NET_RAW          = 1 << 6;
        const NET_BIND_SERVICE = 1 << 7;
        const HW_ACCESS        = 1 << 8;
        const SYS_TIME         = 1 << 9;
        const SYS_POWER        = 1 << 10;
        const PKG_INSTALL      = 1 << 11;

        // WarOS-exclusive capabilities (bits 16-31)
        const QUANTUM_ALLOC    = 1 << 16;
        const QUANTUM_EXECUTE  = 1 << 17;
        const QUANTUM_REMOTE   = 1 << 18;
        const CRYPTO_PQ        = 1 << 19;
        const CRYPTO_KEYGEN    = 1 << 20;
        const ENTROPY_ACCESS   = 1 << 21;
        const AI_INFERENCE     = 1 << 22;
        const SECURITY_ADMIN   = 1 << 23;
        const AUDIT_READ       = 1 << 24;
        const FIREWALL_BYPASS  = 1 << 25;
    }
}

/// Default set for admin (uid 0) processes.
pub const ADMIN_DEFAULT: Capabilities = Capabilities::from_bits_truncate(
    Capabilities::USER_ADMIN.bits()
        | Capabilities::FS_ADMIN.bits()
        | Capabilities::FILE_READ_ALL.bits()
        | Capabilities::FILE_WRITE_ALL.bits()
        | Capabilities::PROCESS_ADMIN.bits()
        | Capabilities::NET_ADMIN.bits()
        | Capabilities::NET_RAW.bits()
        | Capabilities::NET_BIND_SERVICE.bits()
        | Capabilities::HW_ACCESS.bits()
        | Capabilities::SYS_TIME.bits()
        | Capabilities::SYS_POWER.bits()
        | Capabilities::PKG_INSTALL.bits()
        | Capabilities::QUANTUM_ALLOC.bits()
        | Capabilities::QUANTUM_EXECUTE.bits()
        | Capabilities::QUANTUM_REMOTE.bits()
        | Capabilities::CRYPTO_PQ.bits()
        | Capabilities::CRYPTO_KEYGEN.bits()
        | Capabilities::ENTROPY_ACCESS.bits()
        | Capabilities::AI_INFERENCE.bits()
        | Capabilities::SECURITY_ADMIN.bits()
        | Capabilities::AUDIT_READ.bits()
        | Capabilities::FIREWALL_BYPASS.bits(),
);

/// Default set for normal users.
pub const USER_DEFAULT: Capabilities = Capabilities::from_bits_truncate(
    Capabilities::QUANTUM_ALLOC.bits()
        | Capabilities::QUANTUM_EXECUTE.bits()
        | Capabilities::CRYPTO_PQ.bits()
        | Capabilities::CRYPTO_KEYGEN.bits()
        | Capabilities::AI_INFERENCE.bits()
        | Capabilities::NET_BIND_SERVICE.bits()
        | Capabilities::PKG_INSTALL.bits(),
);

/// Sandboxed processes get nothing.
pub const SANDBOXED: Capabilities = Capabilities::empty();

/// Check whether the current process has a given capability.
pub fn has_capability(cap: Capabilities) -> bool {
    let pid = crate::exec::current_pid();
    let process_table = crate::exec::PROCESS_TABLE.lock();
    pid.and_then(|pid| process_table.get(pid))
        .map_or(false, |p| p.effective_capabilities.contains(cap))
}

/// Require a capability or return a security error string.
pub fn require_capability(cap: Capabilities) -> Result<(), &'static str> {
    if has_capability(cap) {
        Ok(())
    } else {
        crate::security::audit::log_event(
            crate::security::audit::events::AuditEvent::CapabilityDenied {
                pid: crate::exec::current_pid().unwrap_or(0),
                capability: alloc::format!("{:?}", cap),
            },
        );
        Err("permission denied: missing capability")
    }
}

/// One-way drop: remove capabilities from the current process (cannot be regained).
pub fn drop_capabilities(caps: Capabilities) {
    let pid = crate::exec::current_pid();
    let mut process_table = crate::exec::PROCESS_TABLE.lock();
    if let Some(process) = pid.and_then(|pid| process_table.get_mut(pid)) {
        process.effective_capabilities.remove(caps);
    }
}

/// Determine the default capability set for a given uid.
pub fn default_for_uid(uid: u16) -> Capabilities {
    if uid == 0 {
        ADMIN_DEFAULT
    } else {
        USER_DEFAULT
    }
}

/// Check a capability against the current login session (for shell commands that
/// run in kernel context without a process-table entry).
pub fn session_require(cap: Capabilities) -> Result<(), &'static str> {
    let uid = crate::auth::session::current_uid();
    let caps = default_for_uid(uid);
    if caps.contains(cap) {
        Ok(())
    } else {
        crate::security::audit::log_event(
            crate::security::audit::events::AuditEvent::CapabilityDenied {
                pid: 0,
                capability: alloc::format!("{:?}", cap),
            },
        );
        Err("permission denied: missing capability")
    }
}

use alloc::string::String;

/// Format capabilities as a human-readable checklist.
pub fn format_capabilities(caps: Capabilities) -> String {
    use alloc::format;
    use alloc::vec::Vec;

    let all = [
        (Capabilities::USER_ADMIN,       "USER_ADMIN"),
        (Capabilities::FS_ADMIN,         "FS_ADMIN"),
        (Capabilities::FILE_READ_ALL,    "FILE_READ_ALL"),
        (Capabilities::FILE_WRITE_ALL,   "FILE_WRITE_ALL"),
        (Capabilities::PROCESS_ADMIN,    "PROCESS_ADMIN"),
        (Capabilities::NET_ADMIN,        "NET_ADMIN"),
        (Capabilities::NET_RAW,          "NET_RAW"),
        (Capabilities::NET_BIND_SERVICE, "NET_BIND_SERVICE"),
        (Capabilities::HW_ACCESS,        "HW_ACCESS"),
        (Capabilities::SYS_TIME,         "SYS_TIME"),
        (Capabilities::SYS_POWER,        "SYS_POWER"),
        (Capabilities::PKG_INSTALL,      "PKG_INSTALL"),
        (Capabilities::QUANTUM_ALLOC,    "QUANTUM_ALLOC"),
        (Capabilities::QUANTUM_EXECUTE,  "QUANTUM_EXECUTE"),
        (Capabilities::QUANTUM_REMOTE,   "QUANTUM_REMOTE"),
        (Capabilities::CRYPTO_PQ,        "CRYPTO_PQ"),
        (Capabilities::CRYPTO_KEYGEN,    "CRYPTO_KEYGEN"),
        (Capabilities::ENTROPY_ACCESS,   "ENTROPY_ACCESS"),
        (Capabilities::AI_INFERENCE,     "AI_INFERENCE"),
        (Capabilities::SECURITY_ADMIN,   "SECURITY_ADMIN"),
        (Capabilities::AUDIT_READ,       "AUDIT_READ"),
        (Capabilities::FIREWALL_BYPASS,  "FIREWALL_BYPASS"),
    ];

    let mut lines: Vec<String> = Vec::new();
    for (bit, name) in &all {
        let mark = if caps.contains(*bit) { "[x]" } else { "[ ]" };
        lines.push(format!("  {} {}", mark, name));
    }
    lines.join("\n")
}
