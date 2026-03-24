use super::ENOSYS;

pub fn sys_ioctl(_fd: u32, _request: u64, _arg: u64) -> i64 {
    ENOSYS
}

pub fn sys_lsdev(_buffer: *mut u8, _len: usize) -> i64 {
    ENOSYS
}
