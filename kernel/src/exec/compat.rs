/// Compatibility helpers for Linux-style syscall numbering.
#[must_use]
pub fn linux_to_waros_syscall(number: u64) -> u64 {
    number
}
