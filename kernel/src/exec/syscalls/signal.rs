use super::ENOSYS;

pub fn sys_kill(_pid: i32, _signal: i32) -> i64 {
    ENOSYS
}

pub fn sys_sigaction(_signal: i32, _action: *const u8, _old_action: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_sigreturn() -> i64 {
    ENOSYS
}
