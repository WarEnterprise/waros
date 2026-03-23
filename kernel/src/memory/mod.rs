use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;
use x86_64::PhysAddr;
use x86_64::VirtAddr;

use bootloader_api::info::MemoryRegions;

use crate::memory::physical::BitmapAllocator;

pub mod heap;
pub mod paging;
pub mod physical;

pub static FRAME_ALLOCATOR: Mutex<Option<BitmapAllocator>> = Mutex::new(None);
static PHYSICAL_MEMORY_OFFSET: AtomicU64 = AtomicU64::new(0);

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

/// Record the virtual mapping used for direct physical-memory access.
pub fn register_physical_memory_mapping(offset: VirtAddr) {
    PHYSICAL_MEMORY_OFFSET.store(offset.as_u64(), Ordering::Relaxed);
}

/// Return the direct physical-memory mapping offset, if available.
#[must_use]
pub fn physical_memory_offset() -> Option<VirtAddr> {
    let offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::Relaxed);
    if offset == 0 {
        None
    } else {
        Some(VirtAddr::new(offset))
    }
}

/// Translate a physical address through the bootloader-provided direct mapping.
#[must_use]
pub fn phys_to_virt(address: PhysAddr) -> Option<VirtAddr> {
    physical_memory_offset().map(|offset| offset + address.as_u64())
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

/// Return whether an address is safe for the shell hex dumper to inspect.
#[must_use]
pub fn is_debug_readable(address: u64) -> bool {
    let physical_memory_offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::Relaxed);
    let heap_range = heap::HEAP_START..(heap::HEAP_START + heap::HEAP_SIZE);
    let physical_range = physical_memory_offset..(physical_memory_offset + (4 * 1024 * 1024 * 1024));

    heap_range.contains(&address) || physical_range.contains(&address)
}
