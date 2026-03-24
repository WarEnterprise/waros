use core::arch::global_asm;
use core::cell::UnsafeCell;

use x86_64::registers::model_specific::{Efer, EferFlags, KernelGsBase, LStar, SFMask, Star};
use x86_64::registers::rflags::RFlags;
use x86_64::VirtAddr;

use crate::arch::x86_64::gdt;

use super::{syscalls, PROCESS_TABLE, SCHEDULER};

#[repr(C)]
struct SyscallCpuLocal {
    user_rsp: u64,
    kernel_rsp: u64,
}

#[repr(transparent)]
struct SyscallCpuLocalCell(UnsafeCell<SyscallCpuLocal>);

// SAFETY: WarOS currently operates on a single CPU; the structure is only mutated through
// serialized kernel control flow.
unsafe impl Sync for SyscallCpuLocalCell {}

static SYSCALL_CPU_LOCAL: SyscallCpuLocalCell = SyscallCpuLocalCell(UnsafeCell::new(SyscallCpuLocal {
    user_rsp: 0,
    kernel_rsp: 0,
}));

#[unsafe(no_mangle)]
static mut WAROS_USER_RETURN_RSP: u64 = 0;

#[unsafe(no_mangle)]
static mut WAROS_USER_RETURN_RIP: u64 = 0;

#[unsafe(no_mangle)]
static mut WAROS_USER_EXIT_CODE: i64 = -1;

#[unsafe(no_mangle)]
static mut WAROS_USER_RETURN_PENDING: u8 = 0;

global_asm!(
    r#"
    .global waros_syscall_entry
waros_syscall_entry:
    swapgs
    mov gs:[0], rsp
    mov rsp, gs:[8]

    push r15
    push r14
    push r13
    push r12
    push rbp
    push rbx
    push r9
    push r8
    push r10
    push rdx
    push rsi
    push rdi
    push rcx
    push r11

    sub rsp, 8
    push r9
    mov r9, r8
    mov r8, r10
    mov rcx, rdx
    mov rdx, rsi
    mov rsi, rdi
    mov rdi, rax
    call syscall_dispatch
    add rsp, 16

    cmp byte ptr [rip + WAROS_USER_RETURN_PENDING], 0
    je .Lwaros_sysret

    mov byte ptr [rip + WAROS_USER_RETURN_PENDING], 0
    mov rsp, qword ptr [rip + WAROS_USER_RETURN_RSP]
    swapgs
    jmp qword ptr [rip + WAROS_USER_RETURN_RIP]

.Lwaros_sysret:
    pop r11
    pop rcx
    pop rdi
    pop rsi
    pop rdx
    pop r10
    pop r8
    pop r9
    pop rbx
    pop rbp
    pop r12
    pop r13
    pop r14
    pop r15

    mov rsp, gs:[0]
    swapgs
    sysretq

    .global waros_run_user_process
waros_run_user_process:
    push r15
    push r14
    push r13
    push r12
    push rbx
    push rbp

    lea rax, [rip + .Lwaros_user_resume]
    mov qword ptr [rip + WAROS_USER_RETURN_RIP], rax
    mov qword ptr [rip + WAROS_USER_RETURN_RSP], rsp
    mov qword ptr [rip + WAROS_USER_EXIT_CODE], -1
    mov byte ptr [rip + WAROS_USER_RETURN_PENDING], 0

    push r8
    push rsi
    push rdx
    push rcx
    push rdi
    iretq

.Lwaros_user_resume:
    mov rax, qword ptr [rip + WAROS_USER_EXIT_CODE]
    pop rbp
    pop rbx
    pop r12
    pop r13
    pop r14
    pop r15
    ret
    "#
);

unsafe extern "C" {
    fn waros_syscall_entry();
    fn waros_run_user_process(
        entry: u64,
        user_stack: u64,
        user_rflags: u64,
        user_cs: u64,
        user_ss: u64,
    ) -> i64;
}

pub fn init() {
    let selectors = gdt::selectors();
    let _ = Star::write(
        selectors.user_code,
        selectors.user_data,
        selectors.kernel_code,
        selectors.kernel_data,
    );
    LStar::write(VirtAddr::from_ptr(waros_syscall_entry as *const ()));
    SFMask::write(RFlags::INTERRUPT_FLAG);
    KernelGsBase::write(VirtAddr::new(SYSCALL_CPU_LOCAL.0.get() as u64));
    set_kernel_stack_top(gdt::kernel_stack_top().as_u64());
    // SAFETY: We only enable the syscall extension bit and preserve all other EFER flags.
    unsafe {
        Efer::update(|flags| flags.insert(EferFlags::SYSTEM_CALL_EXTENSIONS));
    }
}

pub fn set_kernel_stack_top(stack_top: u64) {
    // SAFETY: The structure is a process-local singleton for the current CPU.
    unsafe {
        (*SYSCALL_CPU_LOCAL.0.get()).kernel_rsp = stack_top;
    }
}

pub fn request_kernel_return(exit_code: i32) {
    // SAFETY: WarOS currently runs a single CPU and only one userspace process can be in the
    // synchronous bootstrap path at a time.
    unsafe {
        WAROS_USER_EXIT_CODE = i64::from(exit_code);
        WAROS_USER_RETURN_PENDING = 1;
    }
}

pub unsafe fn run_user_process(
    entry: u64,
    user_stack: u64,
    user_rflags: u64,
    user_cs: u64,
    user_ss: u64,
) -> i64 {
    // SAFETY: The caller guarantees that the entry point and stack belong to a mapped
    // ring-3 image and that the process kernel stack/TSS are already active.
    unsafe { waros_run_user_process(entry, user_stack, user_rflags, user_cs, user_ss) }
}

#[unsafe(no_mangle)]
pub extern "C" fn syscall_dispatch(
    syscall_num: u64,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> i64 {
    if let Some(pid) = SCHEDULER.lock().current_pid() {
        if let Some(process) = PROCESS_TABLE.lock().get_mut(pid) {
            process.syscall_count = process.syscall_count.saturating_add(1);
        }
    }

    match syscall_num {
        0 => syscalls::file::sys_read(arg1 as u32, arg2 as *mut u8, arg3 as usize),
        1 => syscalls::file::sys_write(arg1 as u32, arg2 as *const u8, arg3 as usize),
        2 => syscalls::file::sys_open(arg1 as *const u8, arg2 as u32, arg3 as u32),
        3 => syscalls::file::sys_close(arg1 as u32),
        4 => syscalls::file::sys_stat(arg1 as *const u8, arg2 as *mut u8),
        8 => syscalls::file::sys_seek(arg1 as u32, arg2 as i64, arg3 as u32),
        9 => syscalls::memory::sys_mmap(arg1, arg2, arg3 as u32, arg4 as u32, arg5 as u32, arg6 as i64),
        11 => syscalls::memory::sys_munmap(arg1, arg2),
        12 => syscalls::memory::sys_brk(arg1),
        20 => syscalls::process::sys_getpid(),
        39 => syscalls::process::sys_getppid(),
        57 => syscalls::process::sys_fork(),
        59 => syscalls::process::sys_execve(arg1 as *const u8, arg2 as *const *const u8, arg3 as *const *const u8),
        60 => syscalls::process::sys_exit(arg1 as i32),
        61 => syscalls::process::sys_wait4(arg1 as i32, arg2 as *mut i32, arg3 as u32),
        79 => syscalls::file::sys_getcwd(arg1 as *mut u8, arg2 as usize),
        80 => syscalls::file::sys_chdir(arg1 as *const u8),
        102 => syscalls::process::sys_getuid(),
        200 => syscalls::network::sys_socket(arg1 as u32, arg2 as u32, arg3 as u32),
        201 => syscalls::network::sys_connect(arg1 as u32, arg2 as *const u8, arg3 as u32),
        202 => syscalls::network::sys_send(arg1 as u32, arg2 as *const u8, arg3 as usize),
        203 => syscalls::network::sys_recv(arg1 as u32, arg2 as *mut u8, arg3 as usize),
        204 => syscalls::network::sys_bind(arg1 as u32, arg2 as *const u8, arg3 as u32),
        205 => syscalls::network::sys_listen(arg1 as u32, arg2 as u32),
        206 => syscalls::network::sys_accept(arg1 as u32, arg2 as *mut u8, arg3 as *mut u32),
        210 => syscalls::network::sys_dns_resolve(arg1 as *const u8, arg2 as *mut u8),
        211 => syscalls::network::sys_https_get(arg1 as *const u8, arg2 as *mut u8, arg3 as usize),
        228 => syscalls::time::sys_clock_gettime(arg1 as u32, arg2 as *mut u8),
        230 => syscalls::time::sys_nanosleep(arg1 as *const u8, arg2 as *mut u8),
        300 => syscalls::quantum::sys_qalloc(arg1 as u32),
        301 => syscalls::quantum::sys_qfree(arg1 as u32),
        302 => syscalls::quantum::sys_qgate(arg1 as u32, arg2 as u32, arg3, arg4, arg5),
        303 => syscalls::quantum::sys_qmeasure(arg1 as u32, arg2 as u32, arg3 as *mut u8),
        304 => syscalls::quantum::sys_qstate(arg1 as u32, arg2 as *mut u8, arg3 as usize),
        305 => syscalls::quantum::sys_qcircuit(arg1 as *const u8),
        310 => syscalls::quantum::sys_ibm_submit(arg1 as *const u8, arg2 as u32, arg3 as *mut u8),
        311 => syscalls::quantum::sys_ibm_status(arg1 as *const u8, arg2 as *mut u8),
        320 => syscalls::quantum::sys_qkd_bb84(arg1 as u32, arg2 as *mut u8),
        400 => syscalls::crypto::sys_kem_keygen(arg1 as *mut u8, arg2 as *mut u8),
        401 => syscalls::crypto::sys_kem_encapsulate(arg1 as *const u8, arg2 as *mut u8, arg3 as *mut u8),
        402 => syscalls::crypto::sys_kem_decapsulate(arg1 as *const u8, arg2 as *const u8, arg3 as *mut u8),
        410 => syscalls::crypto::sys_sign(arg1 as *const u8, arg2 as *const u8, arg3 as usize, arg4 as *mut u8),
        411 => syscalls::crypto::sys_verify(arg1 as *const u8, arg2 as *const u8, arg3 as usize, arg4 as *const u8),
        420 => syscalls::crypto::sys_sha3_256(arg1 as *const u8, arg2 as usize, arg3 as *mut u8),
        421 => syscalls::crypto::sys_random_bytes(arg1 as *mut u8, arg2 as usize),
        500 => syscalls::ai::sys_ai_load_model(arg1 as *const u8),
        501 => syscalls::ai::sys_ai_inference(arg1, arg2 as *const u8, arg3 as *mut u8),
        600 => syscalls::io::sys_ioctl(arg1 as u32, arg2, arg3),
        601 => syscalls::io::sys_lsdev(arg1 as *mut u8, arg2 as usize),
        _ => syscalls::ENOSYS,
    }
}
