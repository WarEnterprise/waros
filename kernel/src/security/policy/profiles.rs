use core::sync::atomic::{AtomicU8, Ordering};

/// Security profile levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SecurityProfile {
    Minimal  = 0,
    Standard = 1,
    Server   = 2,
    Paranoid = 3,
}

static CURRENT_PROFILE: AtomicU8 = AtomicU8::new(SecurityProfile::Standard as u8);

impl SecurityProfile {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "minimal" => Some(Self::Minimal),
            "standard" => Some(Self::Standard),
            "server" => Some(Self::Server),
            "paranoid" => Some(Self::Paranoid),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Minimal => "Minimal",
            Self::Standard => "Standard",
            Self::Server => "Server",
            Self::Paranoid => "Paranoid",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Minimal => "ASLR + outbound TCP firewall hook (development)",
            Self::Standard => "Minimal + audit hooks + loader W^X + capability gates (default)",
            Self::Server => "Standard hooks and controls (no extra server-only enforcement yet)",
            Self::Paranoid => "Standard + WarVault database build on apply",
        }
    }
}

pub fn current() -> SecurityProfile {
    match CURRENT_PROFILE.load(Ordering::Relaxed) {
        0 => SecurityProfile::Minimal,
        1 => SecurityProfile::Standard,
        2 => SecurityProfile::Server,
        3 => SecurityProfile::Paranoid,
        _ => SecurityProfile::Standard,
    }
}

/// Apply a security profile, configuring all subsystems accordingly.
pub fn apply(profile: SecurityProfile) {
    use crate::security::{aslr, firewall};

    match profile {
        SecurityProfile::Minimal => {
            aslr::set_enabled(true);
            firewall::set_enabled(true);
        }
        SecurityProfile::Standard => {
            aslr::set_enabled(true);
            firewall::set_enabled(true);
        }
        SecurityProfile::Server => {
            aslr::set_enabled(true);
            firewall::set_enabled(true);
        }
        SecurityProfile::Paranoid => {
            aslr::set_enabled(true);
            firewall::set_enabled(true);
            // Build integrity database
            crate::security::vault::build_database();
        }
    }

    CURRENT_PROFILE.store(profile as u8, Ordering::Relaxed);

    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::SecurityPolicyChanged {
            change: alloc::format!("profile set to {}", profile.name()),
            uid: crate::exec::current_uid(),
        },
    );

    crate::serial_println!("[WarShield] Security profile applied: {}", profile.name());
}
