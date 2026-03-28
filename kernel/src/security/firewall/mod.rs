pub mod connection_track;
pub mod rules;

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use spin::Mutex;

use connection_track::ConnectionTracker;
use rules::{Action, Direction, FirewallRule, Protocol, default_rules};

pub struct FirewallStats {
    pub allowed: u64,
    pub denied: u64,
    pub inbound_allowed: u64,
    pub inbound_denied: u64,
    pub outbound_allowed: u64,
    pub outbound_denied: u64,
    pub tcp_allowed: u64,
    pub tcp_denied: u64,
    pub udp_allowed: u64,
    pub udp_denied: u64,
    pub icmp_allowed: u64,
    pub icmp_denied: u64,
    pub stateful_allows: u64,
}

pub struct WarGuard {
    rules: Vec<FirewallRule>,
    tracker: ConnectionTracker,
    enabled: bool,
    default_inbound: Action,
    default_outbound: Action,
    stats: FirewallStats,
    next_rule_id: u32,
}

static WARGUARD: Mutex<Option<WarGuard>> = Mutex::new(None);

impl WarGuard {
    fn new() -> Self {
        let rules = default_rules();
        let next_id = rules.iter().map(|r| r.id).max().unwrap_or(0) + 1;
        Self {
            rules,
            tracker: ConnectionTracker::new(),
            enabled: true,
            default_inbound: Action::Deny,
            default_outbound: Action::Allow,
            stats: FirewallStats {
                allowed: 0,
                denied: 0,
                inbound_allowed: 0,
                inbound_denied: 0,
                outbound_allowed: 0,
                outbound_denied: 0,
                tcp_allowed: 0,
                tcp_denied: 0,
                udp_allowed: 0,
                udp_denied: 0,
                icmp_allowed: 0,
                icmp_denied: 0,
                stateful_allows: 0,
            },
            next_rule_id: next_id,
        }
    }

    fn record_decision(
        &mut self,
        direction: Direction,
        protocol: Protocol,
        action: Action,
        stateful: bool,
    ) {
        match action {
            Action::Allow => {
                self.stats.allowed += 1;
                match direction {
                    Direction::Inbound => self.stats.inbound_allowed += 1,
                    Direction::Outbound => self.stats.outbound_allowed += 1,
                }
                match protocol {
                    Protocol::Tcp => self.stats.tcp_allowed += 1,
                    Protocol::Udp => self.stats.udp_allowed += 1,
                    Protocol::Icmp => self.stats.icmp_allowed += 1,
                    Protocol::Any => {}
                }
                if stateful {
                    self.stats.stateful_allows += 1;
                }
            }
            Action::Deny => {
                self.stats.denied += 1;
                match direction {
                    Direction::Inbound => self.stats.inbound_denied += 1,
                    Direction::Outbound => self.stats.outbound_denied += 1,
                }
                match protocol {
                    Protocol::Tcp => self.stats.tcp_denied += 1,
                    Protocol::Udp => self.stats.udp_denied += 1,
                    Protocol::Icmp => self.stats.icmp_denied += 1,
                    Protocol::Any => {}
                }
            }
        }
    }

    fn protocol_number(protocol: Protocol) -> u8 {
        match protocol {
            Protocol::Tcp => 6,
            Protocol::Udp => 17,
            Protocol::Icmp => 1,
            Protocol::Any => 0,
        }
    }

    pub fn process_packet(
        &mut self,
        direction: Direction,
        protocol: Protocol,
        src_ip: u32,
        dst_ip: u32,
        src_port: u16,
        dst_port: u16,
    ) -> Action {
        if !self.enabled {
            return Action::Allow;
        }

        // Stateful: allow established responses
        if direction == Direction::Inbound {
            let proto_num = Self::protocol_number(protocol);
            if self.tracker.is_established_response(src_ip, dst_ip, src_port, dst_port, proto_num) {
                self.record_decision(direction, protocol, Action::Allow, true);
                log_firewall_decision(
                    0,
                    direction,
                    protocol,
                    Action::Allow,
                    "stateful-response",
                    src_ip,
                    dst_ip,
                    src_port,
                    dst_port,
                );
                return Action::Allow;
            }
        }

        // Match against rules (first match wins)
        for rule in &mut self.rules {
            if rule.matches(direction, protocol, dst_port) {
                rule.hit_count += 1;
                log_firewall_decision(
                    rule.id,
                    direction,
                    protocol,
                    rule.action,
                    &rule.description,
                    src_ip,
                    dst_ip,
                    src_port,
                    dst_port,
                );
                match rule.action {
                    Action::Allow => {
                        self.record_decision(direction, protocol, Action::Allow, false);
                        // Track outbound for stateful matching
                        if direction == Direction::Outbound {
                            let proto_num = Self::protocol_number(protocol);
                            self.tracker.track_outbound(src_ip, dst_ip, src_port, dst_port, proto_num);
                        }
                        return Action::Allow;
                    }
                    Action::Deny => {
                        self.record_decision(direction, protocol, Action::Deny, false);
                        return Action::Deny;
                    }
                }
            }
        }

        // Default policy
        let default = match direction {
            Direction::Inbound => self.default_inbound,
            Direction::Outbound => self.default_outbound,
        };
        self.record_decision(direction, protocol, default, false);
        log_firewall_decision(
            0,
            direction,
            protocol,
            default,
            "default-policy",
            src_ip,
            dst_ip,
            src_port,
            dst_port,
        );
        default
    }

    pub fn add_rule(&mut self, direction: Direction, protocol: Protocol, port: Option<u16>, action: Action, description: String) -> u32 {
        let id = self.next_rule_id;
        self.next_rule_id += 1;
        // Insert before the last catch-all rule
        let pos = if self.rules.len() > 1 {
            self.rules.len() - 1
        } else {
            self.rules.len()
        };
        self.rules.insert(pos, FirewallRule {
            id,
            direction,
            protocol,
            port,
            action,
            description,
            hit_count: 0,
        });
        id
    }

    pub fn remove_rule(&mut self, id: u32) -> bool {
        let before = self.rules.len();
        self.rules.retain(|r| r.id != id);
        self.rules.len() < before
    }
}

// Public API using global lock

pub fn init() {
    *WARGUARD.lock() = Some(WarGuard::new());
}

pub fn is_enabled() -> bool {
    WARGUARD.lock().as_ref().map_or(false, |g| g.enabled)
}

pub fn set_enabled(enabled: bool) {
    if let Some(g) = WARGUARD.lock().as_mut() {
        g.enabled = enabled;
    }
}

pub fn process_packet(
    direction: Direction,
    protocol: Protocol,
    src_ip: u32,
    dst_ip: u32,
    src_port: u16,
    dst_port: u16,
) -> Action {
    WARGUARD
        .lock()
        .as_mut()
        .map_or(Action::Allow, |g| g.process_packet(direction, protocol, src_ip, dst_ip, src_port, dst_port))
}

pub fn add_rule(direction: Direction, protocol: Protocol, port: Option<u16>, action: Action, description: String) -> u32 {
    WARGUARD
        .lock()
        .as_mut()
        .map_or(0, |g| g.add_rule(direction, protocol, port, action, description))
}

pub fn remove_rule(id: u32) -> bool {
    WARGUARD
        .lock()
        .as_mut()
        .map_or(false, |g| g.remove_rule(id))
}

pub fn rule_count() -> usize {
    WARGUARD.lock().as_ref().map_or(0, |g| g.rules.len())
}

pub fn active_connections() -> usize {
    WARGUARD.lock().as_ref().map_or(0, |g| g.tracker.active_count())
}

pub fn stats() -> (u64, u64) {
    WARGUARD
        .lock()
        .as_ref()
        .map_or((0, 0), |g| (g.stats.allowed, g.stats.denied))
}

pub fn format_rules() -> String {
    use alloc::format;
    let guard = WARGUARD.lock();
    let Some(g) = guard.as_ref() else {
        return String::from("  Firewall not initialized\n");
    };
    let mut out = String::new();
    for rule in &g.rules {
        let port_str = rule.port.map_or(String::from("*"), |p| alloc::format!("{}", p));
        out.push_str(&format!(
            "  {:>3}  {} {:>5} {:>4} port {:>5}  hits:{:<6}  {}\n",
            rule.id, rule.action, rule.direction, rule.protocol, port_str, rule.hit_count, rule.description
        ));
    }
    out
}

pub fn format_status() -> String {
    use alloc::format;
    let guard = WARGUARD.lock();
    let Some(g) = guard.as_ref() else {
        return String::from("  Firewall not initialized\n");
    };
    format!(
        "    State:       {}\n    Rules:       {} active\n    Connections: {} tracked\n    Coverage:    TCP connect + inbound response, UDP send/response, DNS egress, ICMP ping/reply\n    Allowed:     {}  Denied: {}\n    Inbound:     allow {}  deny {}\n    Outbound:    allow {}  deny {}\n    TCP:         allow {}  deny {}\n    UDP:         allow {}  deny {}\n    ICMP:        allow {}  deny {}\n    Stateful:    {} response(s) allowed",
        if g.enabled { "enabled" } else { "disabled" },
        g.rules.len(),
        g.tracker.active_count(),
        g.stats.allowed,
        g.stats.denied,
        g.stats.inbound_allowed,
        g.stats.inbound_denied,
        g.stats.outbound_allowed,
        g.stats.outbound_denied,
        g.stats.tcp_allowed,
        g.stats.tcp_denied,
        g.stats.udp_allowed,
        g.stats.udp_denied,
        g.stats.icmp_allowed,
        g.stats.icmp_denied,
        g.stats.stateful_allows,
    )
}

fn log_firewall_decision(
    rule_id: u32,
    direction: Direction,
    protocol: Protocol,
    action: Action,
    reason: &str,
    src_ip: u32,
    dst_ip: u32,
    src_port: u16,
    dst_port: u16,
) {
    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::FirewallMatch {
            rule_id,
            direction: direction.to_string(),
            protocol: protocol.to_string(),
            action: action.to_string(),
            reason: reason.to_string(),
            src_ip,
            dst_ip,
            src_port,
            dst_port,
        },
    );
}
