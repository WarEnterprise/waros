use alloc::string::{String, ToString};
use alloc::vec;

use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::registers::control::{Cr3, Cr3Flags};
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::arch::x86_64::gdt;
use crate::auth::{session, UserRole, USER_DB};
use crate::fs;
use crate::memory;

use super::address_space::AddressSpace;
use super::elf::{parse_elf, ElfInfo, ElfSegment};
use super::fd_table::FileDescriptorTable;
use super::process::{CpuContext, Priority, Process, ProcessImageKind, ProcessState, SegmentFlags};
use super::scheduler::DEFAULT_TIME_SLICE;
use super::{ExecError, PROCESS_TABLE, SCHEDULER};

static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

pub fn save_kernel_cr3() {
    let (frame, _) = Cr3::read();
    KERNEL_CR3.store(frame.start_address().as_u64(), Ordering::Relaxed);
}

pub fn kernel_cr3() -> u64 {
    KERNEL_CR3.load(Ordering::Relaxed)
}

/// Switch the active CR3 to any physical address.
/// # Safety
/// Caller must ensure the target page table has valid kernel mappings in the upper half.
unsafe fn switch_cr3(phys: u64) {
    let frame = PhysFrame::containing_address(PhysAddr::new(phys));
    // SAFETY: Upheld by caller contract.
    unsafe { Cr3::write(frame, Cr3Flags::empty()); }
}

/// Allocate a new PML4 frame, zero it, copy kernel-half entries (indices 256-511)
/// from the currently active page table, and return the new PML4 physical address.
pub fn create_user_page_table_pub() -> Result<u64, ExecError> {
    create_user_page_table()
}

fn create_user_page_table() -> Result<u64, ExecError> {
    let offset = memory::physical_memory_offset().ok_or(ExecError::PageTableError)?;
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = allocator_guard.as_mut().ok_or(ExecError::MemoryAllocationFailed)?;

    // Allocate one 4 KiB frame for the new PML4.
    let new_phys = FrameAllocator::<Size4KiB>::allocate_frame(allocator)
        .ok_or(ExecError::MemoryAllocationFailed)?
        .start_address()
        .as_u64();

    // Zero the new PML4.
    // SAFETY: `new_phys` was just allocated and the phys-offset map covers all RAM.
    unsafe {
        core::ptr::write_bytes((offset + new_phys).as_mut_ptr::<u8>(), 0, 4096);
    }

    // Copy kernel-half entries from the active PML4.
    let (current_frame, _) = Cr3::read();
    let current_phys = current_frame.start_address().as_u64();
    // SAFETY: Both PML4s are within the physical-memory direct map.
    unsafe {
        let src = (offset + current_phys).as_ptr::<u64>();
        let dst = (offset + new_phys).as_mut_ptr::<u64>();
        for i in 256..512usize {
            dst.add(i).write(src.add(i).read());
        }
    }

    Ok(new_phys)
}

/// Public wrapper for map_process_image, used by syscalls/process.rs.
pub fn map_process_image_pub(
    page_table_phys: u64,
    elf: &ElfInfo,
    elf_data: &[u8],
    args: &[&str],
    env: &[(String, String)],
) -> Result<(AddressSpace, CpuContext, usize), ExecError> {
    map_process_image(page_table_phys, elf, elf_data, args, env)
}

pub fn spawn_process(
    path: &str,
    args: &[&str],
    env: &[(String, String)],
    uid: u16,
    parent_pid: u32,
    priority: Priority,
) -> Result<u32, ExecError> {
    let role = role_for_uid(uid);
    let elf_data = fs::FILESYSTEM
        .lock()
        .read_as(path, uid, role)
        .map_err(|error| match error {
            fs::FsError::FileNotFound => ExecError::FileNotFound,
            fs::FsError::PermissionDenied => ExecError::PermissionDenied,
            _ => ExecError::LoadFailed,
        })?
        .to_vec();
    let elf = parse_elf(&elf_data)?;

    // Create an isolated user page table (kernel half pre-populated).
    let new_cr3 = create_user_page_table()?;

    // Temporarily switch to the new page table while mapping the process image.
    // The kernel remains accessible because we copied the upper-half PML4 entries.
    let saved_cr3 = kernel_cr3();
    // SAFETY: `new_cr3` has valid kernel mappings in the upper half.
    unsafe { switch_cr3(new_cr3); }

    let result = build_process(path, args, env, uid, parent_pid, priority, &elf, &elf_data, new_cr3);

    // Always restore the kernel page table.
    // SAFETY: `saved_cr3` is the valid kernel CR3.
    unsafe { switch_cr3(saved_cr3); }

    let process = result?;
    let pid = PROCESS_TABLE.lock().create_process(process)?;
    SCHEDULER.lock().enqueue(pid, priority);
    Ok(pid)
}

#[must_use]
pub fn load_elf_metadata(path: &str, uid: u16) -> Result<(ElfInfo, usize), ExecError> {
    let role = role_for_uid(uid);
    let filesystem = fs::FILESYSTEM.lock();
    let data = filesystem
        .read_as(path, uid, role)
        .map_err(|error| match error {
            fs::FsError::FileNotFound => ExecError::FileNotFound,
            fs::FsError::PermissionDenied => ExecError::PermissionDenied,
            _ => ExecError::LoadFailed,
        })?;
    let elf = parse_elf(data)?;
    Ok((elf, data.len()))
}

pub fn teardown_process(pid: u32) -> Result<(), ExecError> {
    let process = PROCESS_TABLE
        .lock()
        .get(pid)
        .cloned()
        .ok_or(ExecError::ProcessNotFound)?;

    // Switch to the process's page table so active_mapper operates on it.
    // SAFETY: process.page_table_phys has kernel upper-half entries copied in.
    if process.page_table_phys != 0 && process.page_table_phys != kernel_cr3() {
        unsafe { switch_cr3(process.page_table_phys); }
    }

    let mut mapper = active_mapper()?;
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = allocator_guard
        .as_mut()
        .ok_or(ExecError::MemoryAllocationFailed)?;

    for segment in &process.address_space.segments {
        unmap_range(
            &mut mapper,
            allocator,
            segment.vaddr,
            segment.vaddr.saturating_add(segment.size),
        );
    }
    unmap_range(
        &mut mapper,
        allocator,
        process.address_space.stack_bottom,
        process.address_space.stack_top,
    );

    // Restore kernel page table.
    let kcr3 = kernel_cr3();
    if kcr3 != 0 {
        // SAFETY: KERNEL_CR3 was saved from the original kernel CR3 at boot.
        unsafe { switch_cr3(kcr3); }
    }

    // Free the PML4 frame itself (but not intermediate tables — acceptable for now).
    if process.page_table_phys != 0 && process.page_table_phys != kcr3 {
        allocator.free_frame(PhysAddr::new(process.page_table_phys));
    }

    Ok(())
}

fn build_process(
    path: &str,
    args: &[&str],
    env: &[(String, String)],
    uid: u16,
    parent_pid: u32,
    priority: Priority,
    elf: &ElfInfo,
    elf_data: &[u8],
    page_table_phys: u64,
) -> Result<Process, ExecError> {
    let (address_space, context, memory_pages) =
        map_process_image(page_table_phys, elf, elf_data, args, env)?;

    let kernel_stack = vec![0u8; 16 * 1024];
    let kernel_stack_top = kernel_stack.as_ptr() as u64 + kernel_stack.len() as u64;

    Ok(Process {
        pid: 0,
        parent_pid,
        name: file_name(path),
        uid,
        state: ProcessState::Ready,
        context,
        exit_code: None,
        page_table_phys,
        address_space,
        kernel_stack,
        kernel_stack_top,
        fd_table: FileDescriptorTable::new_with_stdio(),
        cwd: session::resolve_path("."),
        env: env.to_vec(),
        quantum_registers: alloc::vec::Vec::new(),
        crypto_keys: alloc::vec::Vec::new(),
        priority,
        cpu_ticks: 0,
        time_slice: DEFAULT_TIME_SLICE,
        created_at: crate::arch::x86_64::interrupts::tick_count(),
        blocked_on: None,
        syscall_count: 0,
        page_fault_count: 0,
        memory_pages,
        task_id: None,
        image_kind: ProcessImageKind::Elf,
        image_path: path.to_string(),
    })
}

fn map_process_image(
    page_table_phys: u64,
    elf: &ElfInfo,
    elf_data: &[u8],
    args: &[&str],
    env: &[(String, String)],
) -> Result<(AddressSpace, CpuContext, usize), ExecError> {
    let mut mapper = active_mapper()?;
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = allocator_guard
        .as_mut()
        .ok_or(ExecError::MemoryAllocationFailed)?;

    let mut address_space = AddressSpace::new(page_table_phys);
    let mut memory_pages = 0usize;

    for segment in &elf.segments {
        map_segment(&mut mapper, allocator, segment, elf_data)?;
        address_space.register_segment(segment.vaddr, segment.memsz, segment.flags);
        memory_pages = memory_pages.saturating_add(aligned_page_count(segment.memsz));
    }

    map_stack(&mut mapper, allocator, &mut address_space)?;
    memory_pages = memory_pages.saturating_add(aligned_page_count(AddressSpace::USER_STACK_SIZE));
    address_space.finalize_heap_layout();

    let stack_top = setup_user_stack(address_space.stack_top, args, env)?;
    let selectors = gdt::selectors();
    let mut context = CpuContext::for_user(elf.entry_point, stack_top, page_table_phys);
    context.cs = u64::from(selectors.user_code.0);
    context.ss = u64::from(selectors.user_data.0);
    Ok((address_space, context, memory_pages))
}

fn map_segment(
    mapper: &mut OffsetPageTable<'static>,
    allocator: &mut impl FrameAllocator<Size4KiB>,
    segment: &ElfSegment,
    elf_data: &[u8],
) -> Result<(), ExecError> {
    let start = segment.vaddr & !0xFFF;
    let end = (segment.vaddr.saturating_add(segment.memsz).saturating_add(0xFFF)) & !0xFFF;
    for address in (start..end).step_by(4096) {
        let page: Page<Size4KiB> = Page::containing_address(VirtAddr::new(address));
        let frame = FrameAllocator::<Size4KiB>::allocate_frame(allocator)
            .ok_or(ExecError::MemoryAllocationFailed)?;
        let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
        if segment.flags.contains(SegmentFlags::WRITE) {
            flags |= PageTableFlags::WRITABLE;
        }
        if !segment.flags.contains(SegmentFlags::EXECUTE) {
            flags |= PageTableFlags::NO_EXECUTE;
        }
        match memory::paging::map_page(mapper, page, frame, flags, allocator) {
            Ok(()) => {
                // SAFETY: The page was just mapped in the current address space.
                unsafe {
                    core::ptr::write_bytes(address as *mut u8, 0, 4096);
                }
            }
            Err(MapToError::PageAlreadyMapped(_)) => return Err(ExecError::LoadFailed),
            Err(_) => return Err(ExecError::PageTableError),
        }
    }

    let source = &elf_data[segment.offset as usize..(segment.offset + segment.filesz) as usize];
    // SAFETY: Destination pages were mapped above and sized according to the ELF segment.
    unsafe {
        core::ptr::copy_nonoverlapping(source.as_ptr(), segment.vaddr as *mut u8, source.len());
        if segment.memsz > segment.filesz {
            core::ptr::write_bytes(
                (segment.vaddr + segment.filesz) as *mut u8,
                0,
                (segment.memsz - segment.filesz) as usize,
            );
        }
    }
    Ok(())
}

fn map_stack(
    mapper: &mut OffsetPageTable<'static>,
    allocator: &mut impl FrameAllocator<Size4KiB>,
    address_space: &mut AddressSpace,
) -> Result<(), ExecError> {
    let stack_top = AddressSpace::USER_STACK_TOP;
    let stack_bottom = stack_top - AddressSpace::USER_STACK_SIZE;
    for address in (stack_bottom..stack_top).step_by(4096) {
        let page = Page::containing_address(VirtAddr::new(address));
        let frame = FrameAllocator::<Size4KiB>::allocate_frame(allocator)
            .ok_or(ExecError::MemoryAllocationFailed)?;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::NO_EXECUTE;
        memory::paging::map_page(mapper, page, frame, flags, allocator)
            .map_err(|_| ExecError::PageTableError)?;
        // SAFETY: The stack page was just mapped in the current address space.
        unsafe {
            core::ptr::write_bytes(address as *mut u8, 0, 4096);
        }
    }
    address_space.stack_top = stack_top;
    address_space.stack_bottom = stack_bottom;
    Ok(())
}

fn setup_user_stack(
    stack_top: u64,
    args: &[&str],
    env: &[(String, String)],
) -> Result<u64, ExecError> {
    let mut sp = stack_top;

    let mut env_ptrs = alloc::vec::Vec::new();
    for (key, value) in env.iter().rev() {
        let text = alloc::format!("{key}={value}");
        sp = sp.saturating_sub((text.len() + 1) as u64);
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr(), sp as *mut u8, text.len());
            (sp as *mut u8).add(text.len()).write(0);
        }
        env_ptrs.push(sp);
    }
    env_ptrs.reverse();

    let mut arg_ptrs = alloc::vec::Vec::new();
    for arg in args.iter().rev() {
        sp = sp.saturating_sub((arg.len() + 1) as u64);
        unsafe {
            core::ptr::copy_nonoverlapping(arg.as_ptr(), sp as *mut u8, arg.len());
            (sp as *mut u8).add(arg.len()).write(0);
        }
        arg_ptrs.push(sp);
    }
    arg_ptrs.reverse();

    sp &= !0xF;

    sp = sp.saturating_sub(16);
    unsafe {
        (sp as *mut u64).write(0);
        ((sp + 8) as *mut u64).write(0);
    }

    sp = sp.saturating_sub(8);
    unsafe {
        (sp as *mut u64).write(0);
    }
    for pointer in env_ptrs.iter().rev() {
        sp = sp.saturating_sub(8);
        unsafe {
            (sp as *mut u64).write(*pointer);
        }
    }

    sp = sp.saturating_sub(8);
    unsafe {
        (sp as *mut u64).write(0);
    }
    for pointer in arg_ptrs.iter().rev() {
        sp = sp.saturating_sub(8);
        unsafe {
            (sp as *mut u64).write(*pointer);
        }
    }

    sp = sp.saturating_sub(8);
    unsafe {
        (sp as *mut u64).write(args.len() as u64);
    }
    Ok(sp)
}

fn active_mapper() -> Result<OffsetPageTable<'static>, ExecError> {
    let offset = memory::physical_memory_offset().ok_or(ExecError::PageTableError)?;
    // SAFETY: The bootloader direct map remains valid throughout the kernel lifetime.
    Ok(unsafe { memory::paging::init(offset) })
}

fn unmap_range(
    mapper: &mut OffsetPageTable<'static>,
    allocator: &mut memory::physical::BitmapAllocator,
    start: u64,
    end: u64,
) {
    let aligned_start = start & !0xFFF;
    let aligned_end = (end.saturating_add(0xFFF)) & !0xFFF;
    for address in (aligned_start..aligned_end).step_by(4096) {
        let page: Page<Size4KiB> = Page::containing_address(VirtAddr::new(address));
        if let Ok((frame, flush)) = mapper.unmap(page) {
            flush.flush();
            allocator.free_frame(frame.start_address());
        }
    }
}

fn aligned_page_count(size: u64) -> usize {
    size.div_ceil(4096) as usize
}

fn role_for_uid(uid: u16) -> UserRole {
    USER_DB
        .lock()
        .find_by_uid(uid)
        .map(|user| user.role)
        .unwrap_or(UserRole::Admin)
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}
