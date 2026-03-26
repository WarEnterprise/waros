use x86_64::structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

use crate::exec::PROCESS_TABLE;
use crate::memory;
use crate::memory::paging::map_page;

use super::{EINVAL, ENOMEM, ENOSYS, EPERM};

const PROT_WRITE: u32 = 0x2;
const MAP_ANONYMOUS: u32 = 0x20;
const PAGE_SIZE: u64 = 4096;

pub fn sys_mmap(
    _addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    _fd: u32,
    _offset: i64,
) -> i64 {
    // Only support anonymous private mappings for now.
    if flags & MAP_ANONYMOUS == 0 {
        return ENOSYS;
    }
    if len == 0 {
        return EINVAL;
    }
    let aligned_len = len.div_ceil(4096) * 4096;

    let base = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return EPERM;
        };
        let base = match process.address_space.mmap_top.checked_sub(aligned_len) {
            Some(base) => base,
            None => return ENOMEM,
        };
        if base < process.address_space.brk {
            return ENOMEM; // narrow mmap path cannot overlap the WarExec heap.
        }
        process.address_space.mmap_top = base;
        process.address_space.heap_limit = base;
        base
    };

    // Map the pages into the active page table.
    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return EPERM,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return EPERM,
    };

    let mut flags_pt = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    if prot & PROT_WRITE != 0 {
        flags_pt |= PageTableFlags::WRITABLE;
    }

    for page_base in (base..base + aligned_len).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
        let frame = match FrameAllocator::<Size4KiB>::allocate_frame(allocator) {
            Some(f) => f,
            None => return ENOMEM,
        };
        match map_page(&mut mapper, page, frame, flags_pt, allocator) {
            Ok(()) => {
                // SAFETY: page was just mapped.
                unsafe { core::ptr::write_bytes(page_base as *mut u8, 0, 4096); }
            }
            Err(_) => {
                allocator.free_frame(frame.start_address());
                return EPERM;
            }
        }
    }

    base as i64
}

pub fn sys_munmap(addr: u64, len: u64) -> i64 {
    if addr == 0 || len == 0 {
        return EINVAL;
    }
    let aligned_len = len.div_ceil(4096) * 4096;
    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return EPERM,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return EPERM,
    };

    for page_base in (addr..addr + aligned_len).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
        if let Ok((frame, flush)) = mapper.unmap(page) {
            flush.flush();
            allocator.free_frame(frame.start_address());
        }
    }
    0
}

pub fn sys_brk(address: u64) -> i64 {
    let (initial_brk, old_brk, heap_limit) = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return EPERM;
        };
        (
            process.address_space.initial_brk,
            process.address_space.brk,
            process.address_space.heap_limit.min(process.address_space.stack_bottom),
        )
    };

    if address == 0 {
        return old_brk as i64;
    }

    // WarExec currently exposes one narrow heap-growth ABI only:
    // - `brk(0)` queries the current break
    // - `brk(new_end)` grows monotonically if `new_end` stays inside the reserved heap window
    // - shrinking is intentionally unsupported for now and returns the current break unchanged
    if address < old_brk || address < initial_brk || address > heap_limit {
        return old_brk as i64;
    }
    if address == old_brk {
        return old_brk as i64;
    }

    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return EPERM,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return EPERM,
    };

    let start_page = align_up(old_brk);
    let end_page = align_up(address);
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;
    let mut mapped_end = start_page;

    for page_base in (start_page..end_page).step_by(PAGE_SIZE as usize) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
        let frame = match FrameAllocator::<Size4KiB>::allocate_frame(allocator) {
            Some(frame) => frame,
            None => {
                rollback_heap_growth(&mut mapper, allocator, start_page, mapped_end);
                return old_brk as i64;
            }
        };
        match map_page(&mut mapper, page, frame, flags, allocator) {
            Ok(()) => {
                // SAFETY: page was just mapped into the current process page table.
                unsafe { core::ptr::write_bytes(page_base as *mut u8, 0, PAGE_SIZE as usize) };
                mapped_end = page_base + PAGE_SIZE;
            }
            Err(_) => {
                allocator.free_frame(frame.start_address());
                rollback_heap_growth(&mut mapper, allocator, start_page, mapped_end);
                return old_brk as i64;
            }
        }
    }

    let mut process_table = PROCESS_TABLE.lock();
    let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
        return old_brk as i64;
    };
    process.address_space.brk = address;
    address as i64
}

fn align_up(value: u64) -> u64 {
    value.div_ceil(PAGE_SIZE) * PAGE_SIZE
}

fn rollback_heap_growth(
    mapper: &mut x86_64::structures::paging::OffsetPageTable<'static>,
    allocator: &mut memory::physical::BitmapAllocator,
    start: u64,
    end: u64,
) {
    for page_base in (start..end).step_by(PAGE_SIZE as usize) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
        if let Ok((frame, flush)) = mapper.unmap(page) {
            flush.flush();
            allocator.free_frame(frame.start_address());
        }
    }
}
