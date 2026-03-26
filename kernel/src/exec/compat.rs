/// Identity helper for WarOS's Linux-numbered experimental syscall subset.
///
/// WarOS does not currently translate or emulate a Linux ABI. The kernel only
/// reuses selected x86_64 Linux syscall numbers for a narrow experimental ABI,
/// and unsupported paths return `ENOSYS`.
#[must_use]
pub fn linux_to_waros_syscall(number: u64) -> u64 {
    number
}
