/// Placeholder helper for WarOS's experimental Linux-numbered syscall subset.
///
/// WarOS does not currently provide a Linux compatibility layer. The kernel
/// reuses selected x86_64 Linux syscall numbers for a narrow experimental ABI,
/// and unsupported paths still return `ENOSYS`.
#[must_use]
pub fn linux_to_waros_syscall(number: u64) -> u64 {
    number
}
