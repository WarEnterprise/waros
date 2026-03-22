use core::cell::UnsafeCell;

use bootloader_api::info::{MemoryRegionKind, MemoryRegions};
use x86_64::structures::paging::{FrameAllocator, PhysFrame, Size4KiB};
use x86_64::PhysAddr;

const FRAME_SIZE: u64 = 4096;
const MAX_TRACKED_PHYSICAL_MEMORY: u64 = 4 * 1024 * 1024 * 1024;
const MAX_TRACKED_FRAMES: usize = (MAX_TRACKED_PHYSICAL_MEMORY / FRAME_SIZE) as usize;
const BITMAP_BYTES: usize = MAX_TRACKED_FRAMES / 8;

struct FrameBitmap(UnsafeCell<[u8; BITMAP_BYTES]>);

// SAFETY: Early boot performs single-threaded allocator initialization and all later
// mutable access happens through the global frame allocator mutex.
unsafe impl Sync for FrameBitmap {}

static FRAME_BITMAP: FrameBitmap = FrameBitmap(UnsafeCell::new([0xFF; BITMAP_BYTES]));

/// Bitmap-based physical frame allocator for 4 KiB frames.
pub struct BitmapAllocator {
    bitmap: &'static mut [u8; BITMAP_BYTES],
    total_frames: usize,
    free_frames: usize,
    scan_hint: usize,
    max_frame_index: usize,
}

impl BitmapAllocator {
    /// Initialize the allocator from a bootloader memory map.
    pub fn init(memory_regions: &MemoryRegions) -> Result<Self, &'static str> {
        let bitmap = unsafe {
            // SAFETY: The bitmap storage is initialized at most once during early boot before
            // concurrent access becomes possible. The returned mutable reference is then owned by
            // the singleton allocator guarded by the global frame allocator mutex.
            &mut *FRAME_BITMAP.0.get()
        };
        bitmap.fill(0xFF);

        let mut allocator = Self {
            bitmap,
            total_frames: 0,
            free_frames: 0,
            scan_hint: 1,
            max_frame_index: 1,
        };

        for region in memory_regions.iter() {
            if region.kind != MemoryRegionKind::Usable {
                continue;
            }

            let mut start = region.start;
            if start == 0 {
                start = FRAME_SIZE;
            }
            let end = region.end.min(MAX_TRACKED_PHYSICAL_MEMORY);

            let start_frame = usize::try_from(start / FRAME_SIZE).map_err(|_| "frame index overflow")?;
            let end_frame = usize::try_from(end / FRAME_SIZE).map_err(|_| "frame index overflow")?;

            for frame_index in start_frame..end_frame {
                if allocator.is_allocated(frame_index) {
                    allocator.set_allocated(frame_index, false);
                    allocator.total_frames += 1;
                    allocator.free_frames += 1;
                }
            }

            allocator.max_frame_index = allocator.max_frame_index.max(end_frame);
        }

        Ok(allocator)
    }

    /// Allocate one 4 KiB physical frame.
    pub fn allocate_frame(&mut self) -> Option<PhysAddr> {
        if self.free_frames == 0 {
            return None;
        }

        let max = self.max_frame_index.min(MAX_TRACKED_FRAMES);
        for offset in 0..max {
            let frame_index = (self.scan_hint + offset) % max;
            if frame_index == 0 || self.is_allocated(frame_index) {
                continue;
            }

            self.set_allocated(frame_index, true);
            self.free_frames -= 1;
            self.scan_hint = frame_index + 1;
            return Some(PhysAddr::new((frame_index as u64) * FRAME_SIZE));
        }

        None
    }

    /// Free a previously allocated physical frame.
    #[allow(dead_code)]
    pub fn free_frame(&mut self, address: PhysAddr) {
        let Ok(frame_index) = usize::try_from(address.as_u64() / FRAME_SIZE) else {
            return;
        };
        if frame_index == 0 || frame_index >= self.max_frame_index {
            return;
        }
        if self.is_allocated(frame_index) {
            self.set_allocated(frame_index, false);
            self.free_frames += 1;
        }
    }

    /// Total number of allocator-managed frames.
    #[must_use]
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }

    /// Current number of free frames.
    #[must_use]
    pub fn free_frames(&self) -> usize {
        self.free_frames
    }

    fn is_allocated(&self, frame_index: usize) -> bool {
        let byte = frame_index / 8;
        let bit = frame_index % 8;
        (self.bitmap[byte] & (1u8 << bit)) != 0
    }

    fn set_allocated(&mut self, frame_index: usize, allocated: bool) {
        let byte = frame_index / 8;
        let bit = frame_index % 8;
        if allocated {
            self.bitmap[byte] |= 1u8 << bit;
        } else {
            self.bitmap[byte] &= !(1u8 << bit);
        }
    }
}

unsafe impl FrameAllocator<Size4KiB> for BitmapAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        BitmapAllocator::allocate_frame(self)
            .map(PhysFrame::<Size4KiB>::containing_address)
    }
}
