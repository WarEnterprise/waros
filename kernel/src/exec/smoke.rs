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

const ELF_BASE_VADDR: u64 = 0x0000_0000_0040_0000;
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const CODE_SIZE: usize = 38;
const ABI_READ_BUFFER_SIZE: u8 = 64;
const ABI_READ_FAILURE_EXIT_CODE: i32 = 1;

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

fn run_program(path: &str) -> Result<i32, ExecError> {
    let args = [path];
    let env: Vec<(String, String)> = Vec::new();
    let pid = loader::spawn_process(
        path,
        &args,
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
    payload.extend_from_slice(&[0xBF, ABI_READ_FAILURE_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
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
