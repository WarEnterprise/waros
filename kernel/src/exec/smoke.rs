use alloc::string::String;
use alloc::vec::Vec;

use crate::auth::session;

use super::{loader, process::Priority, run_user_process, ExecError};

pub const SMOKE_ELF_PATH: &str = "/bin/warexec-smoke.elf";
pub const SMOKE_ELF_EXIT_CODE: i32 = 42;
pub const SMOKE_ELF_STDOUT: &str = "warexec smoke user program\n";
pub const ABI_READ_SMOKE_ELF_PATH: &str = "/bin/warexec-read-smoke.elf";
pub const ABI_READ_SMOKE_ELF_EXIT_CODE: i32 = 43;
pub const ABI_READ_SMOKE_FILE_PATH: &str = "/abi/waros-abi-proof.txt";
pub const ABI_READ_SMOKE_FILE_CONTENT: &str = "warexec abi file proof\n";
pub const ABI_OFFSET_SMOKE_ELF_PATH: &str = "/bin/warexec-offset-smoke.elf";
pub const ABI_OFFSET_SMOKE_ELF_EXIT_CODE: i32 = 44;
pub const ABI_OFFSET_SMOKE_FILE_PATH: &str = "/abi/waros-offset-proof.txt";
pub const ABI_OFFSET_SMOKE_FILE_CONTENT: &str = "chunk-one|chunk-two\n";
pub const ABI_ARGV_SMOKE_ELF_PATH: &str = "/bin/warexec-argv-smoke.elf";
pub const ABI_ARGV_SMOKE_ELF_EXIT_CODE: i32 = 45;
pub const ABI_ARGV_SMOKE_ARG1: &str = "alpha";
pub const ABI_ARGV_SMOKE_ARG2: &str = "beta";
pub const ABI_EXEC_PARENT_ELF_PATH: &str = "/bin/warexec-exec-parent.elf";
pub const ABI_EXEC_CHILD_ELF_PATH: &str = "/bin/warexec-exec-child.elf";
pub const ABI_EXEC_CHILD_ELF_EXIT_CODE: i32 = 46;
pub const ABI_EXEC_SMOKE_ARG1: &str = "gamma";
pub const ABI_EXEC_SMOKE_ARG2: &str = "delta";
pub const ABI_HEAP_SMOKE_ELF_PATH: &str = "/bin/warexec-heap-smoke.elf";
pub const ABI_HEAP_SMOKE_ELF_EXIT_CODE: i32 = 47;
pub const ABI_FAULT_SMOKE_ELF_PATH: &str = "/bin/warexec-fault-smoke.elf";
pub const ABI_FAULT_SMOKE_ELF_EXIT_CODE: i32 = 48;

const ELF_BASE_VADDR: u64 = 0x0000_0000_0040_0000;
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const CODE_SIZE: usize = 38;
const ABI_READ_BUFFER_SIZE: u8 = 64;
const ABI_SMOKE_FAILURE_EXIT_CODE: i32 = 1;
const ABI_OFFSET_FIRST_CHUNK_LEN: usize = 10;
const ABI_OFFSET_SECOND_CHUNK_LEN: usize = 10;
const ABI_OFFSET_EOF_PROBE_LEN: usize = 1;
const ABI_ARGV_SMOKE_EXPECTED_ARGC: usize = 3;
const ABI_ARGV_SMOKE_ARGC_LINE: &str = "argc=3\n";
const ABI_ARGV_SMOKE_ARGV1_PREFIX: &str = "argv1=";
const ABI_ARGV_SMOKE_ARGV2_PREFIX: &str = "argv2=";
const ABI_ARGV_SMOKE_NEWLINE: &str = "\n";
const ABI_EXEC_PARENT_STDOUT: &str = "exec-parent-start\n";
const ABI_EXEC_CHILD_STDOUT: &str = "exec-child\n";
const ABI_HEAP_START_STDOUT: &str = "heap-proof-start\n";
const ABI_HEAP_BYTES_OK_STDOUT: &str = "heap-bytes-ok\n";
const ABI_HEAP_PREFIX_STDOUT: &str = "heap-string=";
const ABI_HEAP_VALUE: &str = "waros-heap-proof";
const ABI_HEAP_GROWTH_SIZE: u32 = 4096;
const ABI_FAULT_START_STDOUT: &str = "fault-proof-start\n";
const ABI_FAULT_ERR_STDOUT: &str = "fault-err=-14\n";
const ABI_FAULT_BAD_PTR: u32 = 0x7000_0000;
const ABI_FAULT_BAD_WRITE_LEN: u32 = 4;
const ABI_FAULT_EXPECTED_ERR: i8 = -14;

const ET_EXEC: u16 = 2;
const EM_X86_64: u16 = 0x3E;
const EV_CURRENT: u32 = 1;
const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_R: u32 = 4;

/// Build a tiny statically encoded x86_64 ELF that performs:
/// 1. `write(1, "...", len)`
/// 2. `exit(42)`
///
/// This keeps WarOS honest: one real, minimal userspace path with no broadened
/// syscall or compatibility claims.
#[must_use]
pub fn elf_bytes() -> Vec<u8> {
    build_write_exit_smoke_elf()
}

#[must_use]
pub fn abi_read_elf_bytes() -> Vec<u8> {
    build_read_abi_smoke_elf()
}

#[must_use]
pub fn abi_offset_elf_bytes() -> Vec<u8> {
    build_offset_abi_smoke_elf()
}

#[must_use]
pub fn abi_argv_elf_bytes() -> Vec<u8> {
    build_argv_abi_smoke_elf()
}

#[must_use]
pub fn abi_exec_parent_elf_bytes() -> Vec<u8> {
    build_exec_parent_abi_smoke_elf()
}

#[must_use]
pub fn abi_exec_child_elf_bytes() -> Vec<u8> {
    build_exec_child_abi_smoke_elf()
}

#[must_use]
pub fn abi_heap_elf_bytes() -> Vec<u8> {
    build_heap_abi_smoke_elf()
}

#[must_use]
pub fn abi_fault_elf_bytes() -> Vec<u8> {
    build_fault_abi_smoke_elf()
}

fn build_write_exit_smoke_elf() -> Vec<u8> {
    let message = SMOKE_ELF_STDOUT.as_bytes();
    let message_offset = CODE_SIZE;
    let entry_point = ELF_BASE_VADDR + (ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE) as u64;
    let message_vaddr = ELF_BASE_VADDR + (ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE + message_offset) as u64;
    let lea_disp = (message_vaddr - (entry_point + 17)) as u32;
    let message_len = message.len() as u32;

    let mut payload = Vec::with_capacity(CODE_SIZE + message.len());
    // mov eax, 1        ; sys_write
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // mov edi, 1        ; fd = stdout
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    // lea rsi, [rip+msg]
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    payload.extend_from_slice(&lea_disp.to_le_bytes());
    // mov edx, len
    payload.extend_from_slice(&[0xBA]);
    payload.extend_from_slice(&message_len.to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // mov eax, 60       ; sys_exit
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 42
    payload.extend_from_slice(&[0xBF, SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // ud2              ; should never be reached
    payload.extend_from_slice(&[0x0F, 0x0B]);
    payload.extend_from_slice(message);
    build_single_segment_rx_elf(&payload)
}

pub fn run() -> Result<i32, ExecError> {
    run_program(SMOKE_ELF_PATH)
}

pub fn run_abi_read_smoke() -> Result<i32, ExecError> {
    run_program(ABI_READ_SMOKE_ELF_PATH)
}

pub fn run_abi_offset_smoke() -> Result<i32, ExecError> {
    run_program(ABI_OFFSET_SMOKE_ELF_PATH)
}

pub fn run_abi_argv_smoke() -> Result<i32, ExecError> {
    let args = [ABI_ARGV_SMOKE_ELF_PATH, ABI_ARGV_SMOKE_ARG1, ABI_ARGV_SMOKE_ARG2];
    run_program_with_args(ABI_ARGV_SMOKE_ELF_PATH, &args)
}

pub fn run_abi_exec_smoke() -> Result<i32, ExecError> {
    let args = [ABI_EXEC_PARENT_ELF_PATH];
    run_program_with_args(ABI_EXEC_PARENT_ELF_PATH, &args)
}

pub fn run_abi_heap_smoke() -> Result<i32, ExecError> {
    let args = [ABI_HEAP_SMOKE_ELF_PATH];
    run_program_with_args(ABI_HEAP_SMOKE_ELF_PATH, &args)
}

pub fn run_abi_fault_smoke() -> Result<i32, ExecError> {
    let args = [ABI_FAULT_SMOKE_ELF_PATH];
    run_program_with_args(ABI_FAULT_SMOKE_ELF_PATH, &args)
}

fn run_program(path: &str) -> Result<i32, ExecError> {
    let args = [path];
    run_program_with_args(path, &args)
}

fn run_program_with_args(path: &str, args: &[&str]) -> Result<i32, ExecError> {
    let env: Vec<(String, String)> = Vec::new();
    let pid = loader::spawn_process(
        path,
        args,
        &env,
        session::current_uid(),
        super::ensure_shell_process(),
        Priority::Normal,
    )?;
    run_user_process(pid)
}

fn build_read_abi_smoke_elf() -> Vec<u8> {
    let path = ABI_READ_SMOKE_FILE_PATH.as_bytes();
    let expected_read_len = ABI_READ_SMOKE_FILE_CONTENT.len();
    let mut payload = Vec::with_capacity(160);

    // sub rsp, 64       ; reserve a small stack buffer for sys_read
    payload.extend_from_slice(&[0x48, 0x83, 0xEC, ABI_READ_BUFFER_SIZE]);
    // mov eax, 2        ; sys_open
    payload.extend_from_slice(&[0xB8, 0x02, 0x00, 0x00, 0x00]);
    // lea rdi, [rip+path]
    payload.extend_from_slice(&[0x48, 0x8D, 0x3D]);
    let path_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // xor esi, esi      ; flags = 0 (read-only narrow path)
    payload.extend_from_slice(&[0x31, 0xF6]);
    // xor edx, edx      ; mode = 0
    payload.extend_from_slice(&[0x31, 0xD2]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // test eax, eax
    payload.extend_from_slice(&[0x85, 0xC0]);
    // js fail
    payload.extend_from_slice(&[0x78, 0x00]);
    let open_fail_jump = payload.len() - 1;
    // mov ebx, eax      ; preserve fd for read + close
    payload.extend_from_slice(&[0x89, 0xC3]);
    // xor eax, eax      ; sys_read
    payload.extend_from_slice(&[0x31, 0xC0]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(expected_read_len as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // cmp eax, expected_len
    payload.extend_from_slice(&[0x83, 0xF8, expected_read_len as u8]);
    // jne fail
    payload.extend_from_slice(&[0x75, 0x00]);
    let read_fail_jump = payload.len() - 1;
    // mov eax, 1        ; sys_write
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // mov edi, 1        ; stdout
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(expected_read_len as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // mov eax, 3        ; sys_close
    payload.extend_from_slice(&[0xB8, 0x03, 0x00, 0x00, 0x00]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // mov eax, 60       ; sys_exit
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 43
    payload.extend_from_slice(&[0xBF, ABI_READ_SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);

    let fail_offset = payload.len();
    // mov eax, 60       ; sys_exit
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 1
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // ud2              ; should never be reached
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let path_offset = payload.len();
    payload.extend_from_slice(path);
    payload.push(0);

    patch_rel32(&mut payload, path_disp_offset, path_offset);
    patch_rel8(&mut payload, open_fail_jump, fail_offset);
    patch_rel8(&mut payload, read_fail_jump, fail_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_offset_abi_smoke_elf() -> Vec<u8> {
    let path = ABI_OFFSET_SMOKE_FILE_PATH.as_bytes();
    let mut payload = Vec::with_capacity(224);

    // sub rsp, 64       ; reserve a small stack buffer for sys_read
    payload.extend_from_slice(&[0x48, 0x83, 0xEC, ABI_READ_BUFFER_SIZE]);
    // mov eax, 2        ; sys_open
    payload.extend_from_slice(&[0xB8, 0x02, 0x00, 0x00, 0x00]);
    // lea rdi, [rip+path]
    payload.extend_from_slice(&[0x48, 0x8D, 0x3D]);
    let path_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // xor esi, esi      ; flags = 0 (read-only narrow path)
    payload.extend_from_slice(&[0x31, 0xF6]);
    // xor edx, edx      ; mode = 0
    payload.extend_from_slice(&[0x31, 0xD2]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // test eax, eax
    payload.extend_from_slice(&[0x85, 0xC0]);
    // js fail
    payload.extend_from_slice(&[0x0F, 0x88]);
    let open_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // mov ebx, eax      ; preserve fd across reads + close
    payload.extend_from_slice(&[0x89, 0xC3]);

    // First read: "chunk-one|"
    // xor eax, eax      ; sys_read
    payload.extend_from_slice(&[0x31, 0xC0]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(ABI_OFFSET_FIRST_CHUNK_LEN as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // cmp eax, expected_len
    payload.extend_from_slice(&[0x83, 0xF8, ABI_OFFSET_FIRST_CHUNK_LEN as u8]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let first_read_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // mov eax, 1        ; sys_write
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // mov edi, 1        ; stdout
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(ABI_OFFSET_FIRST_CHUNK_LEN as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);

    // Second read: "chunk-two\n"
    // xor eax, eax      ; sys_read
    payload.extend_from_slice(&[0x31, 0xC0]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(ABI_OFFSET_SECOND_CHUNK_LEN as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // cmp eax, expected_len
    payload.extend_from_slice(&[0x83, 0xF8, ABI_OFFSET_SECOND_CHUNK_LEN as u8]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let second_read_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // mov eax, 1        ; sys_write
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // mov edi, 1        ; stdout
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, len
    payload.push(0xBA);
    payload.extend_from_slice(&(ABI_OFFSET_SECOND_CHUNK_LEN as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);

    // EOF probe: read one more byte and require 0
    // xor eax, eax      ; sys_read
    payload.extend_from_slice(&[0x31, 0xC0]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // mov rsi, rsp      ; buffer
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    // mov edx, 1
    payload.push(0xBA);
    payload.extend_from_slice(&(ABI_OFFSET_EOF_PROBE_LEN as u32).to_le_bytes());
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // test eax, eax
    payload.extend_from_slice(&[0x85, 0xC0]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let eof_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // mov eax, 3        ; sys_close
    payload.extend_from_slice(&[0xB8, 0x03, 0x00, 0x00, 0x00]);
    // mov edi, ebx      ; fd
    payload.extend_from_slice(&[0x89, 0xDF]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // test eax, eax
    payload.extend_from_slice(&[0x85, 0xC0]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let close_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // mov eax, 60       ; sys_exit
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 44
    payload.extend_from_slice(&[0xBF, ABI_OFFSET_SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);

    let fail_offset = payload.len();
    // mov eax, 60       ; sys_exit
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 1
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    payload.extend_from_slice(&[0x0F, 0x05]);
    // ud2              ; should never be reached
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let path_offset = payload.len();
    payload.extend_from_slice(path);
    payload.push(0);

    patch_rel32(&mut payload, path_disp_offset, path_offset);
    patch_rel32(&mut payload, open_fail_jump, fail_offset);
    patch_rel32(&mut payload, first_read_fail_jump, fail_offset);
    patch_rel32(&mut payload, second_read_fail_jump, fail_offset);
    patch_rel32(&mut payload, eof_fail_jump, fail_offset);
    patch_rel32(&mut payload, close_fail_jump, fail_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_argv_abi_smoke_elf() -> Vec<u8> {
    build_arg_report_smoke_elf(
        None,
        ABI_ARGV_SMOKE_ARG1,
        ABI_ARGV_SMOKE_ARG2,
        ABI_ARGV_SMOKE_ELF_EXIT_CODE,
    )
}

fn build_exec_parent_abi_smoke_elf() -> Vec<u8> {
    let parent_line = ABI_EXEC_PARENT_STDOUT.as_bytes();
    let child_path = ABI_EXEC_CHILD_ELF_PATH.as_bytes();
    let arg1 = ABI_EXEC_SMOKE_ARG1.as_bytes();
    let arg2 = ABI_EXEC_SMOKE_ARG2.as_bytes();
    let mut payload = Vec::with_capacity(256);

    // sub rsp, 32       ; argv[0..2] + NULL terminator for execve
    payload.extend_from_slice(&[0x48, 0x83, 0xEC, 0x20]);

    // write("exec-parent-start\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let parent_line_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(parent_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // lea r12, [rip+child_path]
    payload.extend_from_slice(&[0x4C, 0x8D, 0x25]);
    let child_path_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // lea r13, [rip+arg1]
    payload.extend_from_slice(&[0x4C, 0x8D, 0x2D]);
    let arg1_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    // lea r14, [rip+arg2]
    payload.extend_from_slice(&[0x4C, 0x8D, 0x35]);
    let arg2_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // argv[0] = child path
    payload.extend_from_slice(&[0x4C, 0x89, 0x24, 0x24]);
    // argv[1] = gamma
    payload.extend_from_slice(&[0x4C, 0x89, 0x6C, 0x24, 0x08]);
    // argv[2] = delta
    payload.extend_from_slice(&[0x4C, 0x89, 0x74, 0x24, 0x10]);
    // argv[3] = NULL
    payload.extend_from_slice(&[0x48, 0xC7, 0x44, 0x24, 0x18, 0x00, 0x00, 0x00, 0x00]);

    // execve(child_path, argv, NULL)
    payload.extend_from_slice(&[0xB8, 0x3B, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x4C, 0x89, 0xE7]);
    payload.extend_from_slice(&[0x48, 0x89, 0xE6]);
    payload.extend_from_slice(&[0x31, 0xD2]);
    payload.extend_from_slice(&[0x0F, 0x05]);

    // Successful exec must not return. If it does, fail hard with exit(1).
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let parent_line_offset = payload.len();
    payload.extend_from_slice(parent_line);
    payload.push(0);
    let child_path_offset = payload.len();
    payload.extend_from_slice(child_path);
    payload.push(0);
    let arg1_offset = payload.len();
    payload.extend_from_slice(arg1);
    payload.push(0);
    let arg2_offset = payload.len();
    payload.extend_from_slice(arg2);
    payload.push(0);

    patch_rel32(&mut payload, parent_line_disp_offset, parent_line_offset);
    patch_rel32(&mut payload, child_path_disp_offset, child_path_offset);
    patch_rel32(&mut payload, arg1_disp_offset, arg1_offset);
    patch_rel32(&mut payload, arg2_disp_offset, arg2_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_exec_child_abi_smoke_elf() -> Vec<u8> {
    build_arg_report_smoke_elf(
        Some(ABI_EXEC_CHILD_STDOUT),
        ABI_EXEC_SMOKE_ARG1,
        ABI_EXEC_SMOKE_ARG2,
        ABI_EXEC_CHILD_ELF_EXIT_CODE,
    )
}

fn build_heap_abi_smoke_elf() -> Vec<u8> {
    let start_line = ABI_HEAP_START_STDOUT.as_bytes();
    let bytes_ok_line = ABI_HEAP_BYTES_OK_STDOUT.as_bytes();
    let prefix = ABI_HEAP_PREFIX_STDOUT.as_bytes();
    let heap_value = ABI_HEAP_VALUE.as_bytes();
    let newline = ABI_ARGV_SMOKE_NEWLINE.as_bytes();
    let mut payload = Vec::with_capacity(320);

    // write("heap-proof-start\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let start_line_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(start_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // current = brk(0)
    payload.extend_from_slice(&[0xB8, 0x0C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x31, 0xFF]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    // test rax, rax
    payload.extend_from_slice(&[0x48, 0x85, 0xC0]);
    // je fail
    payload.extend_from_slice(&[0x0F, 0x84]);
    let brk_query_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // r12 = old_break
    payload.extend_from_slice(&[0x49, 0x89, 0xC4]);
    // rdi = old_break
    payload.extend_from_slice(&[0x48, 0x89, 0xC7]);
    // rdi += 4096
    payload.extend_from_slice(&[0x48, 0x81, 0xC7]);
    payload.extend_from_slice(&ABI_HEAP_GROWTH_SIZE.to_le_bytes());

    // new = brk(old + 4096)
    payload.extend_from_slice(&[0xB8, 0x0C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    // cmp rax, rdi
    payload.extend_from_slice(&[0x48, 0x39, 0xF8]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let brk_grow_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // Copy "waros-heap-proof" into the newly allocated heap at old_break.
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let heap_value_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.extend_from_slice(&[0x4C, 0x89, 0xE7]);
    payload.extend_from_slice(&[0xB9]);
    payload.extend_from_slice(&(heap_value.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0xF3, 0xA4]);

    // write("heap-bytes-ok\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let bytes_ok_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(bytes_ok_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("heap-string=")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let prefix_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(prefix.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write(heap_ptr, heap_value.len())
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x4C, 0x89, 0xE6]);
    payload.push(0xBA);
    payload.extend_from_slice(&(heap_value.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let newline_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(newline.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // exit(47)
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_HEAP_SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);

    let fail_offset = payload.len();
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let start_line_offset = payload.len();
    payload.extend_from_slice(start_line);
    let bytes_ok_line_offset = payload.len();
    payload.extend_from_slice(bytes_ok_line);
    let prefix_offset = payload.len();
    payload.extend_from_slice(prefix);
    let heap_value_offset = payload.len();
    payload.extend_from_slice(heap_value);
    let newline_offset = payload.len();
    payload.extend_from_slice(newline);

    patch_rel32(&mut payload, start_line_disp_offset, start_line_offset);
    patch_rel32(&mut payload, brk_query_fail_jump, fail_offset);
    patch_rel32(&mut payload, brk_grow_fail_jump, fail_offset);
    patch_rel32(&mut payload, heap_value_disp_offset, heap_value_offset);
    patch_rel32(&mut payload, bytes_ok_disp_offset, bytes_ok_line_offset);
    patch_rel32(&mut payload, prefix_disp_offset, prefix_offset);
    patch_rel32(&mut payload, newline_disp_offset, newline_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_fault_abi_smoke_elf() -> Vec<u8> {
    let start_line = ABI_FAULT_START_STDOUT.as_bytes();
    let err_line = ABI_FAULT_ERR_STDOUT.as_bytes();
    let mut payload = Vec::with_capacity(192);

    // write("fault-proof-start\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let start_line_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(start_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write(1, BAD_PTR, 4) must fail with the current narrow EFAULT contract.
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBE]);
    payload.extend_from_slice(&ABI_FAULT_BAD_PTR.to_le_bytes());
    payload.push(0xBA);
    payload.extend_from_slice(&ABI_FAULT_BAD_WRITE_LEN.to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // cmp eax, -14
    payload.extend_from_slice(&[0x83, 0xF8, ABI_FAULT_EXPECTED_ERR as u8]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let fault_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // write("fault-err=-14\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let err_line_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(err_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // exit(48)
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_FAULT_SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);

    let fail_offset = payload.len();
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let start_line_offset = payload.len();
    payload.extend_from_slice(start_line);
    let err_line_offset = payload.len();
    payload.extend_from_slice(err_line);

    patch_rel32(&mut payload, start_line_disp_offset, start_line_offset);
    patch_rel32(&mut payload, fault_fail_jump, fail_offset);
    patch_rel32(&mut payload, err_line_disp_offset, err_line_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_arg_report_smoke_elf(
    header_line: Option<&str>,
    arg1: &str,
    arg2: &str,
    exit_code: i32,
) -> Vec<u8> {
    let header_line = header_line.map(str::as_bytes);
    let argc_line = ABI_ARGV_SMOKE_ARGC_LINE.as_bytes();
    let argv1_prefix = ABI_ARGV_SMOKE_ARGV1_PREFIX.as_bytes();
    let argv2_prefix = ABI_ARGV_SMOKE_ARGV2_PREFIX.as_bytes();
    let newline = ABI_ARGV_SMOKE_NEWLINE.as_bytes();
    let mut payload = Vec::with_capacity(384);

    // mov rax, [rsp]    ; argc from the WarExec entry frame
    payload.extend_from_slice(&[0x48, 0x8B, 0x04, 0x24]);
    // cmp eax, 3
    payload.extend_from_slice(&[0x83, 0xF8, ABI_ARGV_SMOKE_EXPECTED_ARGC as u8]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let argc_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // mov rbx, [rsp+32] ; argv[argc] terminator must be NULL
    payload.extend_from_slice(&[0x48, 0x8B, 0x5C, 0x24, 0x20]);
    // test rbx, rbx
    payload.extend_from_slice(&[0x48, 0x85, 0xDB]);
    // jne fail
    payload.extend_from_slice(&[0x0F, 0x85]);
    let argv_null_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // mov r12, [rsp+16] ; argv[1]
    payload.extend_from_slice(&[0x4C, 0x8B, 0x64, 0x24, 0x10]);
    // test r12, r12
    payload.extend_from_slice(&[0x4D, 0x85, 0xE4]);
    // je fail
    payload.extend_from_slice(&[0x0F, 0x84]);
    let argv1_null_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    // mov r13, [rsp+24] ; argv[2]
    payload.extend_from_slice(&[0x4C, 0x8B, 0x6C, 0x24, 0x18]);
    // test r13, r13
    payload.extend_from_slice(&[0x4D, 0x85, 0xED]);
    // je fail
    payload.extend_from_slice(&[0x0F, 0x84]);
    let argv2_null_fail_jump = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);

    let header_line_disp_offset = if header_line.is_some() {
        payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
        let disp_offset = payload.len();
        payload.extend_from_slice(&[0, 0, 0, 0]);
        payload.push(0xBA);
        payload.extend_from_slice(&(header_line.as_ref().map_or(0, |line| line.len()) as u32).to_le_bytes());
        payload.extend_from_slice(&[0x0F, 0x05]);
        Some(disp_offset)
    } else {
        None
    };

    // write("argc=3\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let argc_line_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(argc_line.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("argv1=")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let argv1_prefix_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(argv1_prefix.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write(argv[1], arg1.len())
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x4C, 0x89, 0xE6]);
    payload.push(0xBA);
    payload.extend_from_slice(&(arg1.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let newline1_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(newline.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("argv2=")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let argv2_prefix_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(argv2_prefix.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write(argv[2], arg2.len())
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x4C, 0x89, 0xEE]);
    payload.push(0xBA);
    payload.extend_from_slice(&(arg2.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // write("\n")
    payload.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x48, 0x8D, 0x35]);
    let newline2_disp_offset = payload.len();
    payload.extend_from_slice(&[0, 0, 0, 0]);
    payload.push(0xBA);
    payload.extend_from_slice(&(newline.len() as u32).to_le_bytes());
    payload.extend_from_slice(&[0x0F, 0x05]);

    // exit(exit_code)
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, exit_code as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);

    let fail_offset = payload.len();
    // exit(1)
    payload.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0xBF, ABI_SMOKE_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    payload.extend_from_slice(&[0x0F, 0x05]);
    payload.extend_from_slice(&[0x0F, 0x0B]);

    let header_line_offset = if let Some(line) = header_line {
        let offset = payload.len();
        payload.extend_from_slice(line);
        Some(offset)
    } else {
        None
    };
    let argc_line_offset = payload.len();
    payload.extend_from_slice(argc_line);
    let argv1_prefix_offset = payload.len();
    payload.extend_from_slice(argv1_prefix);
    let argv2_prefix_offset = payload.len();
    payload.extend_from_slice(argv2_prefix);
    let newline_offset = payload.len();
    payload.extend_from_slice(newline);

    patch_rel32(&mut payload, argc_fail_jump, fail_offset);
    patch_rel32(&mut payload, argv_null_fail_jump, fail_offset);
    patch_rel32(&mut payload, argv1_null_fail_jump, fail_offset);
    patch_rel32(&mut payload, argv2_null_fail_jump, fail_offset);
    if let (Some(disp_offset), Some(line_offset)) = (header_line_disp_offset, header_line_offset) {
        patch_rel32(&mut payload, disp_offset, line_offset);
    }
    patch_rel32(&mut payload, argc_line_disp_offset, argc_line_offset);
    patch_rel32(&mut payload, argv1_prefix_disp_offset, argv1_prefix_offset);
    patch_rel32(&mut payload, newline1_disp_offset, newline_offset);
    patch_rel32(&mut payload, argv2_prefix_disp_offset, argv2_prefix_offset);
    patch_rel32(&mut payload, newline2_disp_offset, newline_offset);

    build_single_segment_rx_elf(&payload)
}

fn build_single_segment_rx_elf(payload: &[u8]) -> Vec<u8> {
    let entry_point = ELF_BASE_VADDR + (ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE) as u64;
    let file_size = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE + payload.len();
    let mut bytes = Vec::with_capacity(file_size);

    bytes.extend_from_slice(b"\x7FELF");
    bytes.push(2); // ELFCLASS64
    bytes.push(1); // ELFDATA2LSB
    bytes.push(1); // EV_CURRENT
    bytes.push(0); // System V ABI
    bytes.push(0); // ABI version
    bytes.extend_from_slice(&[0; 7]);

    bytes.extend_from_slice(&ET_EXEC.to_le_bytes());
    bytes.extend_from_slice(&EM_X86_64.to_le_bytes());
    bytes.extend_from_slice(&EV_CURRENT.to_le_bytes());
    bytes.extend_from_slice(&entry_point.to_le_bytes());
    bytes.extend_from_slice(&(ELF_HEADER_SIZE as u64).to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&(ELF_HEADER_SIZE as u16).to_le_bytes());
    bytes.extend_from_slice(&(PROGRAM_HEADER_SIZE as u16).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());

    bytes.extend_from_slice(&PT_LOAD.to_le_bytes());
    bytes.extend_from_slice(&(PF_R | PF_X).to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&ELF_BASE_VADDR.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&(file_size as u64).to_le_bytes());
    bytes.extend_from_slice(&(file_size as u64).to_le_bytes());
    bytes.extend_from_slice(&0x1000u64.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn patch_rel32(bytes: &mut [u8], disp_offset: usize, target_offset: usize) {
    let displacement = target_offset as isize - (disp_offset + 4) as isize;
    let encoded = (displacement as i32).to_le_bytes();
    bytes[disp_offset..disp_offset + 4].copy_from_slice(&encoded);
}

fn patch_rel8(bytes: &mut [u8], disp_offset: usize, target_offset: usize) {
    let displacement = target_offset as isize - (disp_offset + 1) as isize;
    debug_assert!((-128..=127).contains(&displacement));
    bytes[disp_offset] = displacement as i8 as u8;
}
