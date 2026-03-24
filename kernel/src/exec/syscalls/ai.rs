use super::ENOSYS;

pub fn sys_ai_load_model(_path: *const u8) -> i64 {
    ENOSYS
}

pub fn sys_ai_inference(_handle: u64, _input: *const u8, _output: *mut u8) -> i64 {
    ENOSYS
}
