use x86_64::structures::paging::{FrameAllocator, Mapper, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

use crate::exec::PROCESS_TABLE;
use crate::memory;
use crate::memory::paging::map_page;

use super::ENOSYS;

const PROT_WRITE: u32 = 0x2;
const MAP_ANONYMOUS: u32 = 0x20;

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
        return -22; // EINVAL
    }
    let aligned_len = len.div_ceil(4096) * 4096;

    let base = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return -1;
        };
        let base = process.address_space.mmap_top - aligned_len;
        process.address_space.mmap_top = base;
        base
    };

    // Map the pages into the active page table.
    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return -1,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return -1,
    };

    let mut flags_pt = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    if prot & PROT_WRITE != 0 {
        flags_pt |= PageTableFlags::WRITABLE;
    }

    for page_base in (base..base + aligned_len).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
        let frame = match FrameAllocator::<Size4KiB>::allocate_frame(allocator) {
            Some(f) => f,
            None => return -12, // ENOMEM
        };
        match map_page(&mut mapper, page, frame, flags_pt, allocator) {
            Ok(()) => {
                // SAFETY: page was just mapped.
                unsafe { core::ptr::write_bytes(page_base as *mut u8, 0, 4096); }
            }
            Err(_) => {
                allocator.free_frame(frame.start_address());
                return -1;
            }
        }
    }

    base as i64
}

pub fn sys_munmap(addr: u64, len: u64) -> i64 {
    if addr == 0 || len == 0 {
        return -22;
    }
    let aligned_len = len.div_ceil(4096) * 4096;
    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return -1,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return -1,
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
    if address == 0 {
        let process_table = PROCESS_TABLE.lock();
        return super::current_pid()
            .and_then(|pid| process_table.get(pid).map(|p| p.address_space.brk as i64))
            .unwrap_or(-1);
    }

    let (old_brk, new_brk) = {
        let mut process_table = PROCESS_TABLE.lock();
        let Some(process) = super::current_pid().and_then(|pid| process_table.get_mut(pid)) else {
            return -1;
        };
        let old = process.address_space.brk;
        let new = address.max(process.address_space.initial_brk);
        process.address_space.brk = new;
        (old, new)
    };

    let offset = match memory::physical_memory_offset() {
        Some(o) => o,
        None => return -1,
    };
    // SAFETY: physical_memory_offset is valid for the kernel lifetime.
    let mut mapper = unsafe { memory::paging::init(offset) };
    let mut allocator_guard = memory::FRAME_ALLOCATOR.lock();
    let allocator = match allocator_guard.as_mut() {
        Some(a) => a,
        None => return -1,
    };

    if new_brk > old_brk {
        // Expand: map new pages.
        let start_page = (old_brk + 4095) & !4095;
        let end_page = (new_brk + 4095) & !4095;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE
            | PageTableFlags::NO_EXECUTE;
        for page_base in (start_page..end_page).step_by(4096) {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
            let frame = match FrameAllocator::<Size4KiB>::allocate_frame(allocator) {
                Some(f) => f,
                None => return old_brk as i64,
            };
            match map_page(&mut mapper, page, frame, flags, allocator) {
                Ok(()) => {
                    // SAFETY: page was just mapped.
                    unsafe { core::ptr::write_bytes(page_base as *mut u8, 0, 4096); }
                }
                Err(_) => {
                    allocator.free_frame(frame.start_address());
                }
            }
        }
    } else if new_brk < old_brk {
        // Shrink: unmap freed pages.
        let start_page = (new_brk + 4095) & !4095;
        let end_page = (old_brk + 4095) & !4095;
        for page_base in (start_page..end_page).step_by(4096) {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_base));
            if let Ok((frame, flush)) = mapper.unmap(page) {
                flush.flush();
                allocator.free_frame(frame.start_address());
            }
        }
    }

    new_brk as i64
}
