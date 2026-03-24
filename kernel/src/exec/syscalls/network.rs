use super::ENOSYS;

pub fn sys_socket(_domain: u32, _socket_type: u32, _protocol: u32) -> i64 {
    ENOSYS
}

pub fn sys_connect(_fd: u32, _address: *const u8, _len: u32) -> i64 {
    ENOSYS
}

pub fn sys_send(_fd: u32, _buffer: *const u8, _len: usize) -> i64 {
    ENOSYS
}

pub fn sys_recv(_fd: u32, _buffer: *mut u8, _len: usize) -> i64 {
    ENOSYS
}

pub fn sys_bind(_fd: u32, _address: *const u8, _len: u32) -> i64 {
    ENOSYS
}

pub fn sys_listen(_fd: u32, _backlog: u32) -> i64 {
    ENOSYS
}

pub fn sys_accept(_fd: u32, _address: *mut u8, _len: *mut u32) -> i64 {
    ENOSYS
}

pub fn sys_dns_resolve(_name: *const u8, _result: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_https_get(_url: *const u8, _buffer: *mut u8, _len: usize) -> i64 {
    ENOSYS
}
