pub mod events;
pub mod logger;

use spin::Mutex;

use events::AuditEvent;
use logger::AuditLogger;

static LOGGER: Mutex<AuditLogger> = Mutex::new(AuditLogger::new());

pub fn init() {
    log_event(AuditEvent::SystemBoot {
        kernel_version: alloc::string::String::from(crate::KERNEL_VERSION),
    });
}

pub fn log_event(event: AuditEvent) {
    LOGGER.lock().log(event);
}

pub fn last_n(n: usize) -> alloc::vec::Vec<(u64, u64, alloc::string::String)> {
    let logger = LOGGER.lock();
    logger
        .last_n(n)
        .map(|e| (e.id, e.timestamp, alloc::format!("{}", e.event)))
        .collect()
}

pub fn total_count() -> u64 {
    LOGGER.lock().total_count()
}

pub fn stats() -> alloc::collections::BTreeMap<&'static str, u64> {
    LOGGER.lock().count_by_category()
}

pub fn auth_stats() -> (u64, u64) {
    LOGGER.lock().auth_stats()
}
