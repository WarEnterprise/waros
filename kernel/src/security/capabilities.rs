use alloc::string::String;

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

const ALL_CAPABILITIES: [(Capabilities, &str); 22] = [
    (Capabilities::USER_ADMIN, "USER_ADMIN"),
    (Capabilities::FS_ADMIN, "FS_ADMIN"),
    (Capabilities::FILE_READ_ALL, "FILE_READ_ALL"),
    (Capabilities::FILE_WRITE_ALL, "FILE_WRITE_ALL"),
    (Capabilities::PROCESS_ADMIN, "PROCESS_ADMIN"),
    (Capabilities::NET_ADMIN, "NET_ADMIN"),
    (Capabilities::NET_RAW, "NET_RAW"),
    (Capabilities::NET_BIND_SERVICE, "NET_BIND_SERVICE"),
    (Capabilities::HW_ACCESS, "HW_ACCESS"),
    (Capabilities::SYS_TIME, "SYS_TIME"),
    (Capabilities::SYS_POWER, "SYS_POWER"),
    (Capabilities::PKG_INSTALL, "PKG_INSTALL"),
    (Capabilities::QUANTUM_ALLOC, "QUANTUM_ALLOC"),
    (Capabilities::QUANTUM_EXECUTE, "QUANTUM_EXECUTE"),
    (Capabilities::QUANTUM_REMOTE, "QUANTUM_REMOTE"),
    (Capabilities::CRYPTO_PQ, "CRYPTO_PQ"),
    (Capabilities::CRYPTO_KEYGEN, "CRYPTO_KEYGEN"),
    (Capabilities::ENTROPY_ACCESS, "ENTROPY_ACCESS"),
    (Capabilities::AI_INFERENCE, "AI_INFERENCE"),
    (Capabilities::SECURITY_ADMIN, "SECURITY_ADMIN"),
    (Capabilities::AUDIT_READ, "AUDIT_READ"),
    (Capabilities::FIREWALL_BYPASS, "FIREWALL_BYPASS"),
];

/// Determine the default capability set for a given uid.
pub fn default_for_uid(uid: u16) -> Capabilities {
    if uid == 0 {
        ADMIN_DEFAULT
    } else {
        USER_DEFAULT
    }
}

#[must_use]
pub fn current_capabilities() -> Option<Capabilities> {
    let pid = crate::exec::current_pid()?;
    process_capabilities(pid)
}

#[must_use]
pub fn process_capabilities(pid: u32) -> Option<Capabilities> {
    let process_table = crate::exec::PROCESS_TABLE.lock();
    process_table.get(pid).map(|process| process.effective_capabilities)
}

#[must_use]
pub fn session_capabilities_for_uid(uid: u16) -> Capabilities {
    default_for_uid(uid)
}

#[must_use]
pub fn current_or_session_capabilities() -> Capabilities {
    current_capabilities()
        .unwrap_or_else(|| session_capabilities_for_uid(crate::auth::session::current_uid()))
}

/// Explicit session-to-process translation for a newly created shell/session process.
#[must_use]
pub fn shell_capabilities_for_uid(uid: u16) -> Capabilities {
    session_capabilities_for_uid(uid)
}

/// Explicit child launch transition: inherit from the parent effective set and never widen
/// beyond the baseline default for the target uid. If no parent process is available, fall
/// back to the current session's shell baseline for that uid.
#[must_use]
pub fn spawn_capabilities(parent_pid: u32, uid: u16) -> Capabilities {
    let baseline = default_for_uid(uid);
    process_capabilities(parent_pid)
        .unwrap_or_else(|| session_capabilities_for_uid(uid))
        & baseline
}

/// Exec keeps the current process identity and may only preserve or narrow capability bits.
#[must_use]
pub fn exec_capabilities(current: Capabilities, uid: u16) -> Capabilities {
    current & default_for_uid(uid)
}

/// Check whether the current process has a given capability.
pub fn has_capability(cap: Capabilities) -> bool {
    current_capabilities().is_some_and(|caps| caps.contains(cap))
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

/// Check a capability against the current shell/session context. When a shell process is
/// present this uses the live process capability set, so manual narrowing is respected.
pub fn session_require(cap: Capabilities) -> Result<(), &'static str> {
    let caps = current_or_session_capabilities();
    if caps.contains(cap) {
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

/// One-way drop: remove capabilities from the current process (cannot be regained through
/// spawn or exec under the current model).
pub fn drop_capabilities(caps: Capabilities) {
    if let Some(pid) = crate::exec::current_pid() {
        let mut process_table = crate::exec::PROCESS_TABLE.lock();
        if let Some(process) = process_table.get_mut(pid) {
            process.effective_capabilities.remove(caps);
            crate::security::audit::log_event(
                crate::security::audit::events::AuditEvent::SecurityPolicyChanged {
                    change: alloc::format!(
                        "cap_drop pid={} now={}",
                        pid,
                        summarize_capabilities(process.effective_capabilities)
                    ),
                    uid: process.uid,
                },
            );
        }
    }
}

pub fn set_process_capabilities(pid: u32, caps: Capabilities) {
    let mut process_table = crate::exec::PROCESS_TABLE.lock();
    if let Some(process) = process_table.get_mut(pid) {
        process.effective_capabilities = caps;
    }
}

#[must_use]
pub fn parse_capability(name: &str) -> Option<Capabilities> {
    ALL_CAPABILITIES
        .iter()
        .find(|(_, label)| label.eq_ignore_ascii_case(name))
        .map(|(capability, _)| *capability)
}

#[must_use]
pub fn all_capability_names() -> [&'static str; 22] {
    let mut names = [""; 22];
    let mut index = 0;
    while index < ALL_CAPABILITIES.len() {
        names[index] = ALL_CAPABILITIES[index].1;
        index += 1;
    }
    names
}

#[must_use]
pub fn summarize_capabilities(caps: Capabilities) -> String {
    let mut summary = String::new();
    for (capability, name) in &ALL_CAPABILITIES {
        if caps.contains(*capability) {
            if !summary.is_empty() {
                summary.push(',');
            }
            summary.push_str(name);
        }
    }
    if summary.is_empty() {
        summary.push_str("none");
    }
    summary
}

pub fn run_transition_proof() -> Result<(), &'static str> {
    let shell_pid = crate::exec::ensure_shell_process();
    let Some(original_caps) = process_capabilities(shell_pid) else {
        return Err("shell capability context missing");
    };

    let narrowed_caps = original_caps & !Capabilities::PKG_INSTALL;
    set_process_capabilities(shell_pid, narrowed_caps);

    let proof_result = (|| {
        let args = [crate::exec::smoke::SMOKE_ELF_PATH];
        let child_pid = crate::exec::loader::spawn_process(
            crate::exec::smoke::SMOKE_ELF_PATH,
            &args,
            &alloc::vec::Vec::new(),
            crate::auth::session::current_uid(),
            shell_pid,
            crate::exec::process::Priority::Normal,
        )
        .map_err(|_| "spawn proof failed")?;

        let child_caps = process_capabilities(child_pid).ok_or("spawned child missing")?;
        if child_caps.contains(Capabilities::PKG_INSTALL) {
            cleanup_spawned_process(child_pid);
            return Err("child unexpectedly gained PKG_INSTALL");
        }

        let exec_caps = exec_capabilities(child_caps, crate::auth::session::current_uid());
        if exec_caps.contains(Capabilities::PKG_INSTALL) {
            cleanup_spawned_process(child_pid);
            return Err("exec transition widened PKG_INSTALL");
        }

        cleanup_spawned_process(child_pid);

        match crate::pkg::with_manager(|manager| manager.install("hello-world")) {
            Err(crate::pkg::PkgError::PermissionDenied) => Ok(()),
            Err(_) => Err("capability denial proof returned wrong error"),
            Ok(()) => Err("capability narrowing did not deny package install"),
        }
    })();

    set_process_capabilities(shell_pid, original_caps);
    proof_result
}

/// Format capabilities as a human-readable checklist.
pub fn format_capabilities(caps: Capabilities) -> String {
    use alloc::format;
    use alloc::vec::Vec;

    let mut lines: Vec<String> = Vec::new();
    for (bit, name) in &ALL_CAPABILITIES {
        let mark = if caps.contains(*bit) { "[x]" } else { "[ ]" };
        lines.push(format!("  {} {}", mark, name));
    }
    lines.join("\n")
}

fn cleanup_spawned_process(pid: u32) {
    let _ = crate::exec::loader::teardown_process(pid);
    crate::exec::SCHEDULER.lock().dequeue(pid);
    crate::exec::PROCESS_TABLE.lock().remove(pid);
}
