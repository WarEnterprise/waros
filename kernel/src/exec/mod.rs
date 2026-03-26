use alloc::string::{String, ToString};
use alloc::vec::Vec;

use spin::{Lazy, Mutex};

use crate::auth::session;
use crate::task;

pub mod address_space;
pub mod compat;
pub mod context;
pub mod elf;
pub mod fd_table;
pub mod loader;
pub mod pipe;
pub mod process;
pub mod scheduler;
pub mod signal;
pub mod smoke;
pub mod syscall;
pub mod syscalls;

use address_space::AddressSpace;
use fd_table::FileDescriptorTable;
use process::{CpuContext, Priority, Process, ProcessImageKind, ProcessState};
use scheduler::{Scheduler, DEFAULT_TIME_SLICE};

pub static PROCESS_TABLE: Lazy<Mutex<ProcessTable>> = Lazy::new(|| Mutex::new(ProcessTable::new()));
pub static SCHEDULER: Lazy<Mutex<Scheduler>> = Lazy::new(|| Mutex::new(Scheduler::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecError {
    TooSmall,
    NotElf,
    Not64Bit,
    NotLittleEndian,
    WrongArchitecture,
    NotExecutable,
    InvalidProgramHeader,
    SegmentOverflow,
    NoLoadableSegments,
    MemoryAllocationFailed,
    PageTableError,
    FileNotFound,
    PermissionDenied,
    ProcessTableFull,
    InvalidSyscall,
    ProcessNotFound,
    LoadFailed,
}

pub struct ProcessTable {
    processes: Vec<Process>,
    next_pid: u32,
    shell_pid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub parent_pid: u32,
    pub uid: u16,
    pub name: String,
    pub state: ProcessState,
    pub priority: Priority,
    pub cpu_ticks: u64,
    pub memory_pages: usize,
    pub qubits: usize,
    pub image_kind: ProcessImageKind,
    pub exit_code: Option<i32>,
}

impl ProcessTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            processes: Vec::new(),
            next_pid: 1,
            shell_pid: None,
        }
    }

    pub fn create_process(&mut self, mut process: Process) -> Result<u32, ExecError> {
        if self.processes.len() >= 256 {
            return Err(ExecError::ProcessTableFull);
        }
        let pid = self.next_pid;
        self.next_pid = self.next_pid.saturating_add(1);
        process.pid = pid;
        self.processes.push(process);
        Ok(pid)
    }

    #[must_use]
    pub fn get(&self, pid: u32) -> Option<&Process> {
        self.processes.iter().find(|process| process.pid == pid)
    }

    pub fn get_mut(&mut self, pid: u32) -> Option<&mut Process> {
        self.processes.iter_mut().find(|process| process.pid == pid)
    }

    pub fn remove(&mut self, pid: u32) {
        self.processes.retain(|process| process.pid != pid);
    }

    #[must_use]
    pub fn list_active(&self) -> Vec<&Process> {
        self.processes
            .iter()
            .filter(|process| process.state != ProcessState::Zombie)
            .collect()
    }

    #[must_use]
    pub fn find_zombie_child(&self, parent_pid: u32, target_pid: i32) -> Option<(u32, i32)> {
        self.processes.iter().find(|p| {
            p.parent_pid == parent_pid
                && p.state == ProcessState::Zombie
                && (target_pid == -1 || p.pid as i32 == target_pid)
        }).map(|p| (p.pid, p.exit_code.unwrap_or(0)))
    }
}

pub fn init() {
    loader::save_kernel_cr3();
    *PROCESS_TABLE.lock() = ProcessTable::new();
    let mut scheduler = SCHEDULER.lock();
    *scheduler = Scheduler::new();
    scheduler.enable();
}

pub fn tick() {
    SCHEDULER.lock().timer_tick();
}

pub fn ensure_shell_process() -> u32 {
    let uid = session::current_uid();
    let mut process_table = PROCESS_TABLE.lock();
    if let Some(pid) = process_table.shell_pid {
        if process_table.get(pid).is_some_and(|process| process.uid == uid) {
            SCHEDULER.lock().set_current_pid(Some(pid));
            return pid;
        }
    }

    let process = Process {
        pid: 0,
        parent_pid: 0,
        name: String::from("waros-shell"),
        uid,
        state: ProcessState::Running,
        context: CpuContext::new(),
        exit_code: None,
        page_table_phys: 0,
        address_space: AddressSpace::new(0),
        kernel_stack: alloc::vec![0u8; 16 * 1024],
        kernel_stack_top: crate::arch::x86_64::gdt::kernel_stack_top().as_u64(),
        fd_table: FileDescriptorTable::new_with_stdio(),
        cwd: session::resolve_path("."),
        env: Vec::new(),
        quantum_registers: Vec::new(),
        crypto_keys: Vec::new(),
        priority: Priority::Interactive,
        cpu_ticks: 0,
        time_slice: DEFAULT_TIME_SLICE,
        created_at: crate::arch::x86_64::interrupts::tick_count(),
        blocked_on: None,
        syscall_count: 0,
        page_fault_count: 0,
        memory_pages: 0,
        task_id: None,
        image_kind: ProcessImageKind::KernelShell,
        image_path: String::from("shell"),
    };
    let pid = process_table.create_process(process).unwrap_or(1);
    process_table.shell_pid = Some(pid);
    drop(process_table);
    SCHEDULER.lock().set_current_pid(Some(pid));
    pid
}

pub fn mark_running(pid: u32) {
    if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
        process.state = ProcessState::Running;
        process.cpu_ticks = process.cpu_ticks.saturating_add(1);
    }
    SCHEDULER.lock().set_current_pid(Some(pid));
}

pub fn mark_exit(pid: u32, exit_code: i32) {
    if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
        process.state = ProcessState::Zombie;
        process.exit_code = Some(exit_code);
    }
    SCHEDULER.lock().dequeue(pid);
}

pub fn current_pid() -> Option<u32> {
    SCHEDULER.lock().current_pid()
}

pub fn current_uid() -> u16 {
    current_pid()
        .and_then(|pid| PROCESS_TABLE.lock().get(pid).map(|process| process.uid))
        .unwrap_or_else(session::current_uid)
}

pub fn run_user_process(pid: u32) -> Result<i32, ExecError> {
    let shell_pid = ensure_shell_process();
    let process = PROCESS_TABLE
        .lock()
        .get(pid)
        .cloned()
        .ok_or(ExecError::ProcessNotFound)?;

    SCHEDULER.lock().disable();
    mark_running(pid);

    // Switch to the process's isolated page table before entering ring 3.
    let kcr3 = loader::kernel_cr3();
    if process.page_table_phys != 0 && process.page_table_phys != kcr3 {
        use x86_64::registers::control::{Cr3, Cr3Flags};
        use x86_64::structures::paging::PhysFrame;
        use x86_64::PhysAddr;
        let frame = PhysFrame::containing_address(PhysAddr::new(process.page_table_phys));
        // SAFETY: process.page_table_phys preserves the non-user kernel/runtime mappings needed
        // after the CR3 switch.
        unsafe { Cr3::write(frame, Cr3Flags::empty()); }
    }

    let exit_code = unsafe {
        syscall::run_user_process(
            process.context.rip,
            process.context.rsp,
            process.context.rflags,
            process.context.cs,
            process.context.ss,
        )
    } as i32;

    // Restore kernel page table.
    if kcr3 != 0 {
        use x86_64::registers::control::{Cr3, Cr3Flags};
        use x86_64::structures::paging::PhysFrame;
        use x86_64::PhysAddr;
        let frame = PhysFrame::containing_address(PhysAddr::new(kcr3));
        // SAFETY: KERNEL_CR3 was saved at boot from the bootloader-established CR3.
        unsafe { Cr3::write(frame, Cr3Flags::empty()); }
    }

    {
        let mut scheduler = SCHEDULER.lock();
        scheduler.dequeue(pid);
        scheduler.set_current_pid(Some(shell_pid));
        scheduler.enable();
    }

    if let Some(process) = PROCESS_TABLE.lock().get_mut(shell_pid) {
        process.state = ProcessState::Running;
    }

    let teardown_result = loader::teardown_process(pid);
    PROCESS_TABLE.lock().remove(pid);
    teardown_result?;
    Ok(exit_code)
}

pub fn kill_process(pid: u32, exit_code: i32) -> Result<(), ExecError> {
    let task_id = PROCESS_TABLE
        .lock()
        .get(pid)
        .and_then(|process| process.task_id);
    if let Some(task_id) = task_id {
        let _ = task::kill(task_id);
    }

    let mut process_table = PROCESS_TABLE.lock();
    let process = process_table.get_mut(pid).ok_or(ExecError::ProcessNotFound)?;
    process.state = ProcessState::Zombie;
    process.exit_code = Some(exit_code);
    drop(process_table);
    SCHEDULER.lock().dequeue(pid);
    Ok(())
}

#[must_use]
pub fn snapshot() -> Vec<ProcessSnapshot> {
    PROCESS_TABLE
        .lock()
        .processes
        .iter()
        .map(|process| ProcessSnapshot {
            pid: process.pid,
            parent_pid: process.parent_pid,
            uid: process.uid,
            name: process.name.clone(),
            state: process.state,
            priority: process.priority,
            cpu_ticks: process.cpu_ticks,
            memory_pages: process.memory_pages,
            qubits: process.quantum_registers.len(),
            image_kind: process.image_kind,
            exit_code: process.exit_code,
        })
        .collect()
}

#[must_use]
pub fn context_switch_count() -> u64 {
    SCHEDULER.lock().context_switch_count
}

pub fn attach_task(pid: u32, task_id: u64) {
    if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
        process.task_id = Some(task_id);
    }
}

pub fn spawn_shell_command(command_line: &str, priority: Priority) -> Result<u32, ExecError> {
    let uid = session::current_uid();
    let parent_pid = ensure_shell_process();
    let command = command_line.trim().to_string();
    let name = if command_line.len() > 32 {
        &command_line[..32]
    } else {
        command_line
    };

    let process = Process {
        pid: 0,
        parent_pid,
        name: name.trim().to_string(),
        uid,
        state: ProcessState::Ready,
        context: CpuContext::new(),
        exit_code: None,
        page_table_phys: 0,
        address_space: AddressSpace::new(0),
        kernel_stack: alloc::vec![0u8; 16 * 1024],
        kernel_stack_top: crate::arch::x86_64::gdt::kernel_stack_top().as_u64(),
        fd_table: FileDescriptorTable::new_with_stdio(),
        cwd: session::resolve_path("."),
        env: Vec::new(),
        quantum_registers: Vec::new(),
        crypto_keys: Vec::new(),
        priority,
        cpu_ticks: 0,
        time_slice: DEFAULT_TIME_SLICE,
        created_at: crate::arch::x86_64::interrupts::tick_count(),
        blocked_on: None,
        syscall_count: 0,
        page_fault_count: 0,
        memory_pages: 0,
        task_id: None,
        image_kind: ProcessImageKind::ShellCommand,
        image_path: command.clone(),
    };
    let pid = PROCESS_TABLE.lock().create_process(process)?;
    SCHEDULER.lock().enqueue(pid, priority);

    let command_for_task = command.clone();
    let task_id = crate::task::SCHEDULER
        .lock()
        .spawn(name, async move {
            crate::task::yield_once().await;
            crate::exec::mark_running(pid);
            crate::shell::commands::execute_command(&command_for_task);
            crate::exec::mark_exit(pid, 0);
            crate::shell::reprompt();
        })
        .map_err(|_| ExecError::LoadFailed)?;
    attach_task(pid, task_id);
    Ok(pid)
}
