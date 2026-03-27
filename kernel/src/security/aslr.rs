use core::sync::atomic::{AtomicBool, Ordering};

use super::crypt::entropy;

static ASLR_ENABLED: AtomicBool = AtomicBool::new(true);

/// Enable or disable ASLR globally.
pub fn set_enabled(enabled: bool) {
    ASLR_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn is_enabled() -> bool {
    ASLR_ENABLED.load(Ordering::Relaxed)
}

/// Randomize the stack top address for a new process.
/// Returns a page-aligned downward offset in bytes.
/// Uses 8-bit entropy (256 pages = 1 MiB max) to stay safely within the
/// gap between USER_MMAP_TOP and USER_STACK_TOP without risking collisions.
pub fn randomize_stack_offset() -> u64 {
    if !is_enabled() {
        return 0;
    }
    let r = entropy::random_u64();
    let pages = r & 0xFF; // 8-bit: 0-255 pages, 0-1 MiB
    let offset = pages * 0x1000;
    crate::serial_println!("[ASLR] Stack offset: 0x{:X} ({} pages)", offset, pages);
    offset
}

/// Randomize the heap base address for a new process.
/// Returns a page-aligned upward offset (0 to 2^13 pages).
pub fn randomize_heap_offset() -> u64 {
    if !is_enabled() {
        return 0;
    }
    let r = entropy::random_u64();
    let pages = r & 0x1FFF;
    let offset = pages * 0x1000;
    crate::serial_println!("[ASLR] Heap offset: 0x{:X} ({} pages)", offset, pages);
    offset
}

/// Randomize the mmap region base for a new process.
/// Returns a page-aligned downward offset (0 to 2^14 pages).
pub fn randomize_mmap_offset() -> u64 {
    if !is_enabled() {
        return 0;
    }
    let r = entropy::random_u64();
    let pages = r & 0x3FFF;
    pages * 0x1000
}
