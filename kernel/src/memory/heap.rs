use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

use linked_list_allocator::LockedHeap;
use x86_64::structures::paging::mapper::MapToError;
use x86_64::structures::paging::{OffsetPageTable, Page, PageTableFlags, Size4KiB};
use x86_64::VirtAddr;

use crate::memory::paging;
use crate::memory::physical::BitmapAllocator;

pub const HEAP_START: u64 = 0x_4444_4444_0000;
pub const HEAP_SIZE: u64 = 8 * 1024 * 1024;
pub const LARGE_ALLOCATION_THRESHOLD: usize = 64 * 1024;

#[global_allocator]
static ALLOCATOR: TrackingHeap = TrackingHeap::empty();

static CURRENT_USED_BYTES: AtomicUsize = AtomicUsize::new(0);
static PEAK_USED_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static DEALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static LARGE_ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);
static LARGEST_REQUEST_BYTES: AtomicUsize = AtomicUsize::new(0);
static LAST_LARGE_REQUEST_BYTES: AtomicUsize = AtomicUsize::new(0);

struct TrackingHeap(LockedHeap);

#[derive(Debug, Clone, Copy)]
pub struct HeapStats {
    pub size: usize,
    pub used: usize,
    pub free: usize,
    pub peak_used: usize,
    pub allocations: usize,
    pub deallocations: usize,
    pub large_allocations: usize,
    pub largest_request: usize,
    pub last_large_request: usize,
}

impl TrackingHeap {
    const fn empty() -> Self {
        Self(LockedHeap::empty())
    }
}

unsafe impl GlobalAlloc for TrackingHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.0.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
            record_request(layout.size());
            refresh_usage_snapshot();
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) };
        DEALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        refresh_usage_snapshot();
    }
}

fn record_request(size: usize) {
    update_max(&LARGEST_REQUEST_BYTES, size);
    if size >= LARGE_ALLOCATION_THRESHOLD {
        LARGE_ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
        LAST_LARGE_REQUEST_BYTES.store(size, Ordering::Relaxed);
    }
}

fn refresh_usage_snapshot() {
    let used = ALLOCATOR.0.lock().used();
    CURRENT_USED_BYTES.store(used, Ordering::Relaxed);
    update_max(&PEAK_USED_BYTES, used);
}

fn update_max(target: &AtomicUsize, value: usize) {
    let mut current = target.load(Ordering::Relaxed);
    while value > current {
        match target.compare_exchange(current, value, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(previous) => current = previous,
        }
    }
}

#[must_use]
pub fn stats() -> HeapStats {
    let guard = ALLOCATOR.0.lock();
    HeapStats {
        size: guard.size(),
        used: guard.used(),
        free: guard.free(),
        peak_used: PEAK_USED_BYTES.load(Ordering::Relaxed),
        allocations: ALLOCATION_COUNT.load(Ordering::Relaxed),
        deallocations: DEALLOCATION_COUNT.load(Ordering::Relaxed),
        large_allocations: LARGE_ALLOCATION_COUNT.load(Ordering::Relaxed),
        largest_request: LARGEST_REQUEST_BYTES.load(Ordering::Relaxed),
        last_large_request: LAST_LARGE_REQUEST_BYTES.load(Ordering::Relaxed),
    }
}

pub fn log_usage(label: &str) {
    let stats = stats();
    crate::serial_println!(
        "[HEAP] stage={} used={} free={} peak={} allocs={} frees={} large={} largest={} last_large={}",
        label,
        stats.used,
        stats.free,
        stats.peak_used,
        stats.allocations,
        stats.deallocations,
        stats.large_allocations,
        stats.largest_request,
        stats.last_large_request
    );
}

pub fn log_named_request(label: &str, bytes: usize) {
    let stats = stats();
    crate::serial_println!(
        "[HEAP] request={} bytes={} used={} free={} peak={}",
        label,
        bytes,
        stats.used,
        stats.free,
        stats.peak_used
    );
}

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
            .0
            .lock()
            .init(HEAP_START as *mut u8, HEAP_SIZE as usize);
    }
    refresh_usage_snapshot();

    Ok(())
}
