use alloc::string::String;
use alloc::vec::Vec;

use crate::auth::session;

use super::{loader, process::Priority, run_user_process, ExecError};

pub const SMOKE_ELF_PATH: &str = "/bin/warexec-smoke.elf";
pub const SMOKE_ELF_EXIT_CODE: i32 = 42;
pub const SMOKE_ELF_STDOUT: &str = "warexec smoke user program\n";

const ELF_BASE_VADDR: u64 = 0x0000_0000_0040_0000;
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const CODE_SIZE: usize = 38;

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
    let message = SMOKE_ELF_STDOUT.as_bytes();
    let code_offset = ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE;
    let entry_point = ELF_BASE_VADDR + code_offset as u64;
    let message_offset = code_offset + CODE_SIZE;
    let message_vaddr = ELF_BASE_VADDR + message_offset as u64;
    let lea_disp = (message_vaddr - (entry_point + 17)) as u32;
    let message_len = message.len() as u32;
    let file_size = message_offset + message.len();

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

    // mov eax, 1        ; sys_write
    bytes.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // mov edi, 1        ; fd = stdout
    bytes.extend_from_slice(&[0xBF, 0x01, 0x00, 0x00, 0x00]);
    // lea rsi, [rip+msg]
    bytes.extend_from_slice(&[0x48, 0x8D, 0x35]);
    bytes.extend_from_slice(&lea_disp.to_le_bytes());
    // mov edx, len
    bytes.extend_from_slice(&[0xBA]);
    bytes.extend_from_slice(&message_len.to_le_bytes());
    // syscall
    bytes.extend_from_slice(&[0x0F, 0x05]);
    // mov eax, 60       ; sys_exit
    bytes.extend_from_slice(&[0xB8, 0x3C, 0x00, 0x00, 0x00]);
    // mov edi, 42
    bytes.extend_from_slice(&[0xBF, SMOKE_ELF_EXIT_CODE as u8, 0x00, 0x00, 0x00]);
    // syscall
    bytes.extend_from_slice(&[0x0F, 0x05]);
    // ud2              ; should never be reached
    bytes.extend_from_slice(&[0x0F, 0x0B]);

    bytes.extend_from_slice(message);
    bytes
}

pub fn run() -> Result<i32, ExecError> {
    let args = [SMOKE_ELF_PATH];
    let env: Vec<(String, String)> = Vec::new();
    let pid = loader::spawn_process(
        SMOKE_ELF_PATH,
        &args,
        &env,
        session::current_uid(),
        super::ensure_shell_process(),
        Priority::Normal,
    )?;
    run_user_process(pid)
}
