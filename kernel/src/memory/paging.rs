use x86_64::registers::control::Cr3;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::VirtAddr;

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

unsafe fn active_level_4_table(physical_memory_offset: VirtAddr) -> &'static mut PageTable {
    let (level_4_frame, _) = Cr3::read();
    let physical_address = level_4_frame.start_address();
    let virtual_address = physical_memory_offset + physical_address.as_u64();
    let page_table_ptr: *mut PageTable = virtual_address.as_mut_ptr();

    // SAFETY: The bootloader created a complete physical memory mapping at the supplied offset,
    // so translating the active CR3 frame through that offset yields a valid mutable pointer to
    // the active level-4 page table for the entire kernel lifetime.
    unsafe { &mut *page_table_ptr }
}
