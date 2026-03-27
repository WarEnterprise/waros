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
    CapabilityDenied { pid: u32, capability: String },
    FirewallMatch { rule_id: u32, action: String, src_ip: u32, dst_port: u16 },
    NetworkConnection { src_ip: u32, dst_ip: u32, dst_port: u16, protocol: String },
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
            Self::CapabilityDenied { pid, capability } => {
                write!(f, "CAP_DENIED pid={} cap={}", pid, capability)
            }
            Self::FirewallMatch { rule_id, action, src_ip, dst_port } => {
                write!(f, "FW_MATCH rule={} action={} src={} dport={}", rule_id, action, src_ip, dst_port)
            }
            Self::NetworkConnection { src_ip, dst_ip, dst_port, protocol } => {
                write!(f, "NET_CONN src={} dst={} dport={} proto={}", src_ip, dst_ip, dst_port, protocol)
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
        | AuditEvent::ProcessExited { .. } => "process",
        AuditEvent::CapabilityDenied { .. } => "capability",
        AuditEvent::FirewallMatch { .. }
        | AuditEvent::NetworkConnection { .. } => "network",
        AuditEvent::PackageInstalled { .. } => "package",
        AuditEvent::QuantumRegisterAllocated { .. } => "quantum",
        AuditEvent::SecurityPolicyChanged { .. } => "security",
        AuditEvent::IntegrityViolation { .. } => "integrity",
        AuditEvent::SystemBoot { .. } => "system",
    }
}
