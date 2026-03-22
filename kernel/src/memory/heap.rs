use linked_list_allocator::LockedHeap;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{OffsetPageTable, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

use crate::memory::paging;
use crate::memory::physical::BitmapAllocator;

pub const HEAP_START: u64 = 0x_4444_4444_0000;
pub const HEAP_SIZE: u64 = 1024 * 1024;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

/// Map and initialize the kernel heap.
pub fn init_heap(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut BitmapAllocator,
) -> Result<(), MapToError<Size4KiB>> {
    let page_range = {
        let heap_start = VirtAddr::new(HEAP_START);
        let heap_end = heap_start + HEAP_SIZE - 1;
        let start_page = Page::containing_address(heap_start);
        let end_page = Page::containing_address(heap_end);
        Page::range_inclusive(start_page, end_page)
    };

    for page in page_range {
        let Some(frame) =
            x86_64::structures::paging::FrameAllocator::<Size4KiB>::allocate_frame(frame_allocator)
        else {
            return Err(MapToError::FrameAllocationFailed);
        };
        paging::map_page(
            mapper,
            page,
            frame,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            frame_allocator,
        )?;
    }

    // SAFETY: The heap virtual range was just mapped as writable and remains exclusively owned
    // by the global allocator for the rest of the kernel lifetime.
    unsafe {
        ALLOCATOR
            .lock()
            .init(HEAP_START as *mut u8, HEAP_SIZE as usize);
    }

    Ok(())
}
