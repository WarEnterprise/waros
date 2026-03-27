use alloc::string::String;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct FirewallRule {
    pub id: u32,
    pub direction: Direction,
    pub protocol: Protocol,
    pub port: Option<u16>,
    pub action: Action,
    pub description: String,
    pub hit_count: u64,
}

impl FirewallRule {
    pub fn matches(&self, direction: Direction, protocol: Protocol, port: u16) -> bool {
        if self.direction != direction {
            return false;
        }
        if self.protocol != Protocol::Any && self.protocol != protocol {
            return false;
        }
        if let Some(rule_port) = self.port {
            if rule_port != port {
                return false;
            }
        }
        true
    }
}

impl core::fmt::Display for Direction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Inbound => write!(f, "IN"),
            Self::Outbound => write!(f, "OUT"),
        }
    }
}

impl core::fmt::Display for Protocol {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Tcp => write!(f, "TCP"),
            Self::Udp => write!(f, "UDP"),
            Self::Icmp => write!(f, "ICMP"),
            Self::Any => write!(f, "ANY"),
        }
    }
}

impl core::fmt::Display for Action {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Allow => write!(f, "ALLOW"),
            Self::Deny => write!(f, "DENY"),
        }
    }
}

/// Create the default secure ruleset.
pub fn default_rules() -> alloc::vec::Vec<FirewallRule> {
    use alloc::string::ToString;
    alloc::vec![
        FirewallRule {
            id: 1,
            direction: Direction::Outbound,
            protocol: Protocol::Tcp,
            port: Some(443),
            action: Action::Allow,
            description: "HTTPS outbound".to_string(),
            hit_count: 0,
        },
        FirewallRule {
            id: 2,
            direction: Direction::Outbound,
            protocol: Protocol::Tcp,
            port: Some(80),
            action: Action::Allow,
            description: "HTTP outbound".to_string(),
            hit_count: 0,
        },
        FirewallRule {
            id: 3,
            direction: Direction::Outbound,
            protocol: Protocol::Udp,
            port: Some(53),
            action: Action::Allow,
            description: "DNS outbound".to_string(),
            hit_count: 0,
        },
        FirewallRule {
            id: 4,
            direction: Direction::Outbound,
            protocol: Protocol::Icmp,
            port: None,
            action: Action::Allow,
            description: "ICMP outbound (ping)".to_string(),
            hit_count: 0,
        },
        FirewallRule {
            id: 5,
            direction: Direction::Outbound,
            protocol: Protocol::Any,
            port: None,
            action: Action::Allow,
            description: "Allow all outbound".to_string(),
            hit_count: 0,
        },
        FirewallRule {
            id: 6,
            direction: Direction::Inbound,
            protocol: Protocol::Any,
            port: None,
            action: Action::Deny,
            description: "Deny all inbound".to_string(),
            hit_count: 0,
        },
    ]
}
