use core::slice;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use spin::Mutex;
use x86_64::PhysAddr;
use x86_64::VirtAddr;

use bootloader_api::info::{MemoryRegion, MemoryRegionKind, MemoryRegions};

use crate::memory::physical::BitmapAllocator;

pub mod heap;
pub mod paging;
pub mod physical;

pub static FRAME_ALLOCATOR: Mutex<Option<BitmapAllocator>> = Mutex::new(None);
static PHYSICAL_MEMORY_OFFSET: AtomicU64 = AtomicU64::new(0);
static MAX_PHYSICAL_ADDRESS: AtomicU64 = AtomicU64::new(0);
static BOOT_MEMORY_REGION_PTR: AtomicUsize = AtomicUsize::new(0);
static BOOT_MEMORY_REGION_LEN: AtomicUsize = AtomicUsize::new(0);

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

/// Record the firmware memory map for later fault classification.
pub fn register_boot_memory_regions(memory_regions: &MemoryRegions) {
    let regions: &[MemoryRegion] = memory_regions;
    BOOT_MEMORY_REGION_PTR.store(regions.as_ptr() as usize, Ordering::Relaxed);
    BOOT_MEMORY_REGION_LEN.store(regions.len(), Ordering::Relaxed);
    let max_physical = regions.iter().map(|region| region.end).max().unwrap_or(0);
    MAX_PHYSICAL_ADDRESS.store(max_physical, Ordering::Relaxed);
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

/// Return the top physical address reported by the bootloader memory map.
#[must_use]
pub fn max_physical_address() -> u64 {
    MAX_PHYSICAL_ADDRESS.load(Ordering::Relaxed)
}

/// Summary of a direct-map virtual address.
#[derive(Debug, Clone, Copy)]
pub struct DirectMapAddressInfo {
    pub physical_address: u64,
    pub region: Option<MemoryRegion>,
}

/// Classify a physical address against the bootloader memory map.
#[must_use]
pub fn classify_physical_address(address: u64) -> Option<MemoryRegion> {
    boot_memory_regions()?
        .iter()
        .copied()
        .find(|region| address >= region.start && address < region.end)
}

/// Determine whether a virtual address lies inside the bootloader direct physical-memory map.
#[must_use]
pub fn classify_direct_map_address(address: u64) -> Option<DirectMapAddressInfo> {
    let offset = physical_memory_offset()?.as_u64();
    let max_physical = max_physical_address();
    if max_physical == 0 || address < offset {
        return None;
    }

    let physical_address = address.checked_sub(offset)?;
    if physical_address >= max_physical {
        return None;
    }

    Some(DirectMapAddressInfo {
        physical_address,
        region: classify_physical_address(physical_address),
    })
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
    let max_physical = MAX_PHYSICAL_ADDRESS.load(Ordering::Relaxed);
    let heap_range = heap::HEAP_START..(heap::HEAP_START + heap::HEAP_SIZE);
    let physical_range = physical_memory_offset..physical_memory_offset.saturating_add(max_physical);

    heap_range.contains(&address) || physical_range.contains(&address)
}

#[must_use]
pub fn memory_region_kind_name(kind: MemoryRegionKind) -> &'static str {
    match kind {
        MemoryRegionKind::Usable => "usable",
        MemoryRegionKind::Bootloader => "bootloader",
        MemoryRegionKind::UnknownUefi(_) => "unknown-uefi",
        MemoryRegionKind::UnknownBios(_) => "unknown-bios",
        _ => "unknown",
    }
}

fn boot_memory_regions() -> Option<&'static [MemoryRegion]> {
    let ptr = BOOT_MEMORY_REGION_PTR.load(Ordering::Relaxed);
    let len = BOOT_MEMORY_REGION_LEN.load(Ordering::Relaxed);
    if ptr == 0 || len == 0 {
        return None;
    }

    // SAFETY: The bootloader-owned memory map is valid for the kernel lifetime. The pointer and
    // length are captured once during early boot and then treated as immutable metadata.
    Some(unsafe { slice::from_raw_parts(ptr as *const MemoryRegion, len) })
}
