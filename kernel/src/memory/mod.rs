use spin::Mutex;

use bootloader_api::info::MemoryRegions;

use crate::memory::physical::BitmapAllocator;

pub mod heap;
pub mod paging;
pub mod physical;

pub static FRAME_ALLOCATOR: Mutex<Option<BitmapAllocator>> = Mutex::new(None);

/// Snapshot of physical memory allocator state.
#[derive(Debug, Clone, Copy)]
pub struct MemoryStats {
    pub total_frames: usize,
    pub free_frames: usize,
}

/// Initialize the global physical frame allocator from the firmware memory map.
pub fn init(memory_regions: &MemoryRegions) -> Result<(), &'static str> {
    let allocator = BitmapAllocator::init(memory_regions)?;
    *FRAME_ALLOCATOR.lock() = Some(allocator);
    Ok(())
}

/// Return current physical memory statistics.
#[must_use]
pub fn stats() -> MemoryStats {
    let guard = FRAME_ALLOCATOR.lock();
    let allocator = guard.as_ref();
    MemoryStats {
        total_frames: allocator.map_or(0, BitmapAllocator::total_frames),
        free_frames: allocator.map_or(0, BitmapAllocator::free_frames),
    }
}
