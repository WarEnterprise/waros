use alloc::collections::VecDeque;

use super::events::AuditEvent;

const MAX_ENTRIES: usize = 10_000;

pub struct AuditEntry {
    pub id: u64,
    pub timestamp: u64,
    pub event: AuditEvent,
}

pub struct AuditLogger {
    entries: VecDeque<AuditEntry>,
    next_id: u64,
}

impl AuditLogger {
    pub const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            next_id: 1,
        }
    }

    pub fn log(&mut self, event: AuditEvent) {
        let id = self.next_id;
        self.next_id += 1;
        let timestamp = crate::arch::x86_64::interrupts::tick_count();

        crate::serial_println!("[AUDIT] #{}: {}", id, event);

        let entry = AuditEntry {
            id,
            timestamp,
            event,
        };

        if self.entries.len() >= MAX_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn last_n(&self, n: usize) -> impl Iterator<Item = &AuditEntry> {
        let skip = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(skip)
    }

    pub fn total_count(&self) -> u64 {
        self.next_id.saturating_sub(1)
    }

    pub fn current_count(&self) -> usize {
        self.entries.len()
    }

    pub fn count_by_category(&self) -> alloc::collections::BTreeMap<&'static str, u64> {
        use super::events::event_category;
        let mut counts = alloc::collections::BTreeMap::new();
        for entry in &self.entries {
            *counts.entry(event_category(&entry.event)).or_insert(0) += 1;
        }
        counts
    }

    pub fn auth_stats(&self) -> (u64, u64) {
        let mut ok = 0u64;
        let mut fail = 0u64;
        for entry in &self.entries {
            match &entry.event {
                AuditEvent::LoginSuccess { .. } => ok += 1,
                AuditEvent::LoginFailed { .. } => fail += 1,
                _ => {}
            }
        }
        (ok, fail)
    }
}
