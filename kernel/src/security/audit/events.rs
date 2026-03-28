use alloc::string::String;

#[derive(Debug, Clone)]
pub enum AuditEvent {
    LoginSuccess { username: String, uid: u16 },
    LoginFailed { username: String, reason: String },
    Logout { username: String, uid: u16 },
    FileCreated { path: String, uid: u16 },
    FileModified { path: String, uid: u16 },
    FileDeleted { path: String, uid: u16 },
    FileAccessDenied { path: String, uid: u16, operation: String },
    ProcessSpawned { pid: u32, name: String, uid: u16 },
    ProcessExited { pid: u32, exit_code: i32 },
    ProcessExec {
        pid: u32,
        path: String,
        uid: u16,
        caps_before: String,
        caps_after: String,
    },
    CapabilityDenied { pid: u32, capability: String },
    FirewallMatch {
        rule_id: u32,
        direction: String,
        protocol: String,
        action: String,
        reason: String,
        src_ip: u32,
        dst_ip: u32,
        src_port: u16,
        dst_port: u16,
    },
    NetworkConnection {
        src_ip: u32,
        src_port: u16,
        dst_ip: u32,
        dst_port: u16,
        protocol: String,
    },
    TlsValidation {
        host: String,
        anchor: String,
        outcome: String,
        detail: String,
    },
    PackageVerification {
        name: String,
        version: String,
        outcome: String,
        reason: String,
    },
    PackageInstalled { name: String, version: String, uid: u16 },
    QuantumRegisterAllocated { pid: u32, qubits: u8 },
    SecurityPolicyChanged { change: String, uid: u16 },
    IntegrityViolation { path: String, expected_hash: String, actual_hash: String },
    SystemBoot { kernel_version: String },
}

impl core::fmt::Display for AuditEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::LoginSuccess { username, uid } => {
                write!(f, "LOGIN_OK user={} uid={}", username, uid)
            }
            Self::LoginFailed { username, reason } => {
                write!(f, "LOGIN_FAIL user={} reason={}", username, reason)
            }
            Self::Logout { username, uid } => {
                write!(f, "LOGOUT user={} uid={}", username, uid)
            }
            Self::FileCreated { path, uid } => {
                write!(f, "FILE_CREATE path={} uid={}", path, uid)
            }
            Self::FileModified { path, uid } => {
                write!(f, "FILE_MODIFY path={} uid={}", path, uid)
            }
            Self::FileDeleted { path, uid } => {
                write!(f, "FILE_DELETE path={} uid={}", path, uid)
            }
            Self::FileAccessDenied { path, uid, operation } => {
                write!(f, "FILE_DENIED path={} uid={} op={}", path, uid, operation)
            }
            Self::ProcessSpawned { pid, name, uid } => {
                write!(f, "PROC_SPAWN pid={} name={} uid={}", pid, name, uid)
            }
            Self::ProcessExited { pid, exit_code } => {
                write!(f, "PROC_EXIT pid={} code={}", pid, exit_code)
            }
            Self::ProcessExec {
                pid,
                path,
                uid,
                caps_before,
                caps_after,
            } => write!(
                f,
                "PROC_EXEC pid={} uid={} path={} caps_before={} caps_after={}",
                pid, uid, path, caps_before, caps_after
            ),
            Self::CapabilityDenied { pid, capability } => {
                write!(f, "CAP_DENIED pid={} cap={}", pid, capability)
            }
            Self::FirewallMatch {
                rule_id,
                direction,
                protocol,
                action,
                reason,
                src_ip,
                dst_ip,
                src_port,
                dst_port,
            } => write!(
                f,
                "FW_MATCH rule={} dir={} proto={} action={} reason={} src={} sport={} dst={} dport={}",
                rule_id, direction, protocol, action, reason, src_ip, src_port, dst_ip, dst_port
            ),
            Self::NetworkConnection {
                src_ip,
                src_port,
                dst_ip,
                dst_port,
                protocol,
            } => {
                write!(
                    f,
                    "NET_CONN src={} sport={} dst={} dport={} proto={}",
                    src_ip, src_port, dst_ip, dst_port, protocol
                )
            }
            Self::TlsValidation {
                host,
                anchor,
                outcome,
                detail,
            } => {
                write!(
                    f,
                    "TLS_VALIDATE host={} anchor={} outcome={} detail={}",
                    host, anchor, outcome, detail
                )
            }
            Self::PackageVerification {
                name,
                version,
                outcome,
                reason,
            } => {
                write!(
                    f,
                    "PKG_VERIFY name={} ver={} outcome={} reason={}",
                    name, version, outcome, reason
                )
            }
            Self::PackageInstalled { name, version, uid } => {
                write!(f, "PKG_INSTALL name={} ver={} uid={}", name, version, uid)
            }
            Self::QuantumRegisterAllocated { pid, qubits } => {
                write!(f, "QREG_ALLOC pid={} qubits={}", pid, qubits)
            }
            Self::SecurityPolicyChanged { change, uid } => {
                write!(f, "POLICY_CHANGE change={} uid={}", change, uid)
            }
            Self::IntegrityViolation { path, expected_hash, actual_hash } => {
                write!(f, "INTEGRITY_FAIL path={} expected={} actual={}", path, expected_hash, actual_hash)
            }
            Self::SystemBoot { kernel_version } => {
                write!(f, "SYSTEM_BOOT version={}", kernel_version)
            }
        }
    }
}

/// Classify event for stats counting.
pub fn event_category(event: &AuditEvent) -> &'static str {
    match event {
        AuditEvent::LoginSuccess { .. }
        | AuditEvent::LoginFailed { .. }
        | AuditEvent::Logout { .. } => "auth",
        AuditEvent::FileCreated { .. }
        | AuditEvent::FileModified { .. }
        | AuditEvent::FileDeleted { .. }
        | AuditEvent::FileAccessDenied { .. } => "file",
        AuditEvent::ProcessSpawned { .. }
        | AuditEvent::ProcessExec { .. }
        | AuditEvent::ProcessExited { .. } => "process",
        AuditEvent::CapabilityDenied { .. } => "capability",
        AuditEvent::FirewallMatch { .. }
        | AuditEvent::TlsValidation { .. }
        | AuditEvent::NetworkConnection { .. } => "network",
        AuditEvent::PackageVerification { .. }
        | AuditEvent::PackageInstalled { .. } => "package",
        AuditEvent::QuantumRegisterAllocated { .. } => "quantum",
        AuditEvent::SecurityPolicyChanged { .. } => "security",
        AuditEvent::IntegrityViolation { .. } => "integrity",
        AuditEvent::SystemBoot { .. } => "system",
    }
}
