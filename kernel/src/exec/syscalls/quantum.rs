use crate::exec::PROCESS_TABLE;
use crate::quantum;
use crate::quantum::state::QuantumState;

use super::{copy_to_user_ptr, current_pid, ENOSYS};

pub fn sys_qalloc(num_qubits: u32) -> i64 {
    if num_qubits == 0 || num_qubits > 18 {
        return -22; // EINVAL
    }
    let state = match QuantumState::new(num_qubits as usize) {
        Ok(s) => s,
        Err(_) => return -12, // ENOMEM
    };
    let handle = quantum::alloc_process_register(state);
    // Track handle in process
    if let Some(pid) = current_pid() {
        if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
            process.quantum_registers.push(handle);
        }
    }
    i64::from(handle)
}

pub fn sys_qfree(handle: u32) -> i64 {
    quantum::free_process_register(handle);
    if let Some(pid) = current_pid() {
        if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
            process.quantum_registers.retain(|&h| h != handle);
        }
    }
    0
}

pub fn sys_qgate(handle: u32, gate: u32, target: u64, control: u64, param: u64) -> i64 {
    match quantum::apply_gate_to_register(handle, gate, target as usize, control as usize, param) {
        Ok(()) => 0,
        Err(_) => -22,
    }
}

pub fn sys_qmeasure(handle: u32, shots: u32, result_buf: *mut u8) -> i64 {
    if result_buf.is_null() {
        return -22;
    }
    let shots = shots.max(1) as usize;
    match quantum::measure_register(handle, shots) {
        Ok(text) => {
            let bytes = text.as_bytes();
            // SAFETY: result_buf is a userspace buffer provided by the caller.
            unsafe { copy_to_user_ptr(result_buf, bytes) as i64 }
        }
        Err(_) => -22,
    }
}

pub fn sys_qstate(handle: u32, result_buf: *mut u8, len: usize) -> i64 {
    if result_buf.is_null() {
        return -22;
    }
    match quantum::state_vector_text(handle, len) {
        Ok(text) => {
            let bytes = text.as_bytes();
            let to_write = bytes.len().min(len.saturating_sub(1));
            // SAFETY: result_buf is a userspace buffer provided by the caller.
            unsafe { copy_to_user_ptr(result_buf, &bytes[..to_write]) as i64 }
        }
        Err(_) => -22,
    }
}

pub fn sys_qcircuit(_path: *const u8) -> i64 {
    ENOSYS
}

pub fn sys_ibm_submit(_backend: *const u8, _shots: u32, _result: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_ibm_status(_job_id: *const u8, _result: *mut u8) -> i64 {
    ENOSYS
}

pub fn sys_qkd_bb84(_bits: u32, _result: *mut u8) -> i64 {
    ENOSYS
}
