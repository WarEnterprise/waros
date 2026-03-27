use alloc::string::String;
use alloc::vec::Vec;

use bitflags::bitflags;

use super::address_space::AddressSpace;
use super::fd_table::FileDescriptorTable;
use crate::security::capabilities::Capabilities;

pub type QuantumRegisterHandle = u32;
pub type CryptoKeyHandle = u32;

#[derive(Debug, Clone)]
pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub uid: u16,
    pub state: ProcessState,
    pub context: CpuContext,
    pub exit_code: Option<i32>,
    pub page_table_phys: u64,
    pub address_space: AddressSpace,
    pub kernel_stack: Vec<u8>,
    pub kernel_stack_top: u64,
    pub fd_table: FileDescriptorTable,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub quantum_registers: Vec<QuantumRegisterHandle>,
    pub crypto_keys: Vec<CryptoKeyHandle>,
    pub priority: Priority,
    pub cpu_ticks: u64,
    pub time_slice: u64,
    pub created_at: u64,
    pub blocked_on: Option<BlockReason>,
    pub syscall_count: u64,
    pub page_fault_count: u64,
    pub memory_pages: usize,
    pub task_id: Option<u64>,
    pub image_kind: ProcessImageKind,
    pub image_path: String,
    pub effective_capabilities: Capabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessImageKind {
    KernelShell,
    ShellCommand,
    ShellScript,
    Elf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Stopped,
    Zombie,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    IoWait(u32),
    Sleep(u64),
    WaitChild(u32),
    QuantumJob(String),
    NetworkRecv,
    PipeRead,
    InputWait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    RealTime = 0,
    System = 1,
    Quantum = 2,
    Interactive = 3,
    Normal = 4,
    Batch = 5,
    Idle = 6,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct CpuContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cs: u64,
    pub ss: u64,
    pub cr3: u64,
    pub fpu_state: [u8; 512],
    pub fpu_valid: bool,
}

impl CpuContext {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0x202,
            cs: 0x23,
            ss: 0x1B,
            cr3: 0,
            fpu_state: [0; 512],
            fpu_valid: false,
        }
    }

    #[must_use]
    pub fn for_user(entry_point: u64, stack_top: u64, page_table: u64) -> Self {
        let mut context = Self::new();
        context.rip = entry_point;
        context.rsp = stack_top;
        context.cr3 = page_table;
        context
    }
}

#[derive(Debug, Clone)]
pub struct MemorySegment {
    pub vaddr: u64,
    pub size: u64,
    pub flags: SegmentFlags,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct SegmentFlags: u8 {
        const READ = 0x01;
        const WRITE = 0x02;
        const EXECUTE = 0x04;
    }
}
