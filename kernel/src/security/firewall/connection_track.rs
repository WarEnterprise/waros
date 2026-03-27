use alloc::vec::Vec;

#[derive(Debug, Clone)]
pub struct TrackedConnection {
    pub src_ip: u32,
    pub dst_ip: u32,
    pub src_port: u16,
    pub dst_port: u16,
    pub protocol: u8, // 6 = TCP, 17 = UDP
    pub established_at: u64,
    pub last_seen: u64,
    pub packets: u64,
}

const CONNECTION_TIMEOUT_TICKS: u64 = 30_000; // ~5 min at 100Hz
const MAX_CONNECTIONS: usize = 256;

pub struct ConnectionTracker {
    connections: Vec<TrackedConnection>,
}

impl ConnectionTracker {
    pub const fn new() -> Self {
        Self {
            connections: Vec::new(),
        }
    }

    /// Record an outbound connection to allow matching inbound responses.
    pub fn track_outbound(&mut self, src_ip: u32, dst_ip: u32, src_port: u16, dst_port: u16, protocol: u8) {
        let now = crate::arch::x86_64::interrupts::tick_count();

        // Update existing
        for conn in &mut self.connections {
            if conn.src_ip == src_ip
                && conn.dst_ip == dst_ip
                && conn.src_port == src_port
                && conn.dst_port == dst_port
                && conn.protocol == protocol
            {
                conn.last_seen = now;
                conn.packets += 1;
                return;
            }
        }

        // Evict expired before adding new
        self.expire();

        if self.connections.len() >= MAX_CONNECTIONS {
            self.connections.remove(0);
        }

        self.connections.push(TrackedConnection {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
            established_at: now,
            last_seen: now,
            packets: 1,
        });
    }

    /// Check if an inbound packet matches an existing outbound connection (response).
    pub fn is_established_response(&self, src_ip: u32, dst_ip: u32, src_port: u16, dst_port: u16, protocol: u8) -> bool {
        let now = crate::arch::x86_64::interrupts::tick_count();
        self.connections.iter().any(|conn| {
            conn.dst_ip == src_ip
                && conn.src_ip == dst_ip
                && conn.dst_port == src_port
                && conn.src_port == dst_port
                && conn.protocol == protocol
                && now.saturating_sub(conn.last_seen) < CONNECTION_TIMEOUT_TICKS
        })
    }

    pub fn expire(&mut self) {
        let now = crate::arch::x86_64::interrupts::tick_count();
        self.connections
            .retain(|conn| now.saturating_sub(conn.last_seen) < CONNECTION_TIMEOUT_TICKS);
    }

    pub fn active_count(&self) -> usize {
        self.connections.len()
    }

    pub fn connections(&self) -> &[TrackedConnection] {
        &self.connections
    }
}
