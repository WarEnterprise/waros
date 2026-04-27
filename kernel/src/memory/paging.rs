use core::sync::atomic::{AtomicU64, Ordering};

use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

use crate::boot::trace::{self, BreadcrumbTag};

const PAGE_SIZE: u64 = 4096;
const MMIO_VIRT_START: u64 = 0x_5555_0000_0000;
const MMIO_VIRT_END: u64 = 0x_5555_1000_0000;

static NEXT_MMIO_VIRT: AtomicU64 = AtomicU64::new(MMIO_VIRT_START);

/// Create an `OffsetPageTable` from the active level-4 table and physical memory mapping.
pub unsafe fn init(physical_memory_offset: VirtAddr) -> OffsetPageTable<'static> {
    let level_4_table = unsafe { active_level_4_table(physical_memory_offset) };
    // SAFETY: `level_4_table` is the active level-4 page table and
    // `physical_memory_offset` refers to the valid physical-memory mapping set up by the bootloader.
    unsafe { OffsetPageTable::new(level_4_table, physical_memory_offset) }
}

/// Map a single virtual page to a physical frame with the requested flags.
pub fn map_page(
    mapper: &mut OffsetPageTable<'static>,
    page: Page<Size4KiB>,
    frame: PhysFrame<Size4KiB>,
    flags: PageTableFlags,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
) -> Result<(), MapToError<Size4KiB>> {
    // SAFETY: `page` and `frame` were chosen by the caller, and the allocator supplies any
    // intermediate paging frames required by the mapper. The caller is responsible for ensuring
    // that the mapping does not alias an existing incompatible mapping.
    let flush = unsafe { mapper.map_to(page, frame, flags, frame_allocator)? };
    flush.flush();
    Ok(())
}

/// Map a physical MMIO register window into a dedicated kernel virtual range.
pub fn map_mmio_region(physical_start: PhysAddr, size: usize) -> Result<VirtAddr, &'static str> {
    if size == 0 {
        return Err("MMIO mapping length must be non-zero");
    }

    let page_offset = physical_start.as_u64() & (PAGE_SIZE - 1);
    let physical_base = physical_start.as_u64() & !(PAGE_SIZE - 1);
    let mapping_len = align_up(
        page_offset
            .checked_add(size as u64)
            .ok_or("MMIO mapping length overflow")?,
    );
    let virtual_base = reserve_mmio_virtual_range(mapping_len)?;

    let physical_memory_offset =
        crate::memory::physical_memory_offset().ok_or("physical memory mapping missing")?;
    // SAFETY: The bootloader physical-memory offset remains valid for the kernel lifetime.
    let mut mapper = unsafe { init(physical_memory_offset) };
    let mut allocator_guard = crate::memory::FRAME_ALLOCATOR.lock();
    let allocator = allocator_guard
        .as_mut()
        .ok_or("frame allocator unavailable for MMIO mapping")?;
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_CACHE
        | PageTableFlags::NO_EXECUTE;

    let page_count = (mapping_len / PAGE_SIZE) as usize;
    let mut mapped_pages = 0usize;
    for index in 0..page_count {
        let page_offset_bytes = (index as u64) * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(virtual_base + page_offset_bytes);
        let frame = PhysFrame::containing_address(PhysAddr::new(physical_base + page_offset_bytes));
        if map_page(&mut mapper, page, frame, flags, allocator).is_err() {
            rollback_mmio_mapping(&mut mapper, virtual_base, mapped_pages);
            return Err("MMIO page-table mapping failed");
        }
        mapped_pages += 1;
    }

    Ok(virtual_base + page_offset)
}

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_frame, _) = Cr3::read();
    let physical_address = level_4_frame.start_address();
    let virtual_address = physical_memory_offset + physical_address.as_u64();
    trace::record_current_cr3();
    trace::record_direct_map(
        BreadcrumbTag::PagingActiveCr3,
        physical_address.as_u64(),
        virtual_address.as_u64(),
    );
    let page_table_ptr: *mut PageTable = virtual_address.as_mut_ptr();

    // SAFETY: The bootloader created a complete physical memory mapping at the supplied offset,
    // so translating the active CR3 frame through that offset yields a valid mutable pointer to
    // the active level-4 page table for the entire kernel lifetime.
    unsafe { &mut *page_table_ptr }
}

fn reserve_mmio_virtual_range(length: u64) -> Result<VirtAddr, &'static str> {
    let start = NEXT_MMIO_VIRT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            let next = current.checked_add(length)?;
            if next > MMIO_VIRT_END {
                None
            } else {
                Some(next)
            }
        })
        .map_err(|_| "MMIO virtual address space exhausted")?;
    Ok(VirtAddr::new(start))
}

fn rollback_mmio_mapping(
    mapper: &mut OffsetPageTable<'static>,
    virtual_base: VirtAddr,
    mapped_pages: usize,
) {
    for index in 0..mapped_pages {
        let page = Page::<Size4KiB>::containing_address(virtual_base + (index as u64) * PAGE_SIZE);
        if let Ok((_frame, flush)) = mapper.unmap(page) {
            flush.flush();
        }
    }
}

fn align_up(value: u64) -> u64 {
    value.div_ceil(PAGE_SIZE) * PAGE_SIZE
}
