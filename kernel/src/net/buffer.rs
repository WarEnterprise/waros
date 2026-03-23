use core::ops::{Deref, DerefMut};
use core::ptr;
use core::slice;

use x86_64::PhysAddr;

use crate::memory;

use super::NetError;

const PAGE_SIZE: usize = 4096;

/// Physically contiguous DMA-capable region backed by the kernel frame allocator.
pub struct DmaRegion {
    physical: PhysAddr,
    len: usize,
    pages: usize,
}

impl DmaRegion {
    pub fn allocate(len: usize) -> Result<Self, NetError> {
        let pages = len.div_ceil(PAGE_SIZE);
        let physical = {
            let mut guard = memory::FRAME_ALLOCATOR.lock();
            let allocator = guard.as_mut().ok_or(NetError::NotInitialized)?;
            allocator
                .allocate_contiguous_frames(pages)
                .ok_or(NetError::OutOfMemory)?
        };

        let region = Self {
            physical,
            len: pages * PAGE_SIZE,
            pages,
        };
        region.zero();
        Ok(region)
    }

    #[must_use]
    pub fn physical(&self) -> PhysAddr {
        self.physical
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn as_ptr(&self) -> *const u8 {
        memory::phys_to_virt(self.physical)
            .expect("physical memory mapping missing")
            .as_ptr()
    }

    #[must_use]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        memory::phys_to_virt(self.physical)
            .expect("physical memory mapping missing")
            .as_mut_ptr()
    }

    #[must_use]
    pub fn slice(&self) -> &[u8] {
        // SAFETY: `DmaRegion` owns a contiguous allocation of `self.len` bytes.
        unsafe { slice::from_raw_parts(self.as_ptr(), self.len) }
    }

    #[must_use]
    pub fn slice_mut(&mut self) -> &mut [u8] {
        // SAFETY: `DmaRegion` owns a unique contiguous allocation of `self.len` bytes.
        unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), self.len) }
    }

    pub fn zero(&self) {
        let Some(virtual_address) = memory::phys_to_virt(self.physical) else {
            return;
        };
        // SAFETY: The direct-mapped virtual address references the region allocated to `self`.
        unsafe {
            ptr::write_bytes(virtual_address.as_mut_ptr::<u8>(), 0, self.len);
        }
    }
}

impl Drop for DmaRegion {
    fn drop(&mut self) {
        let mut guard = memory::FRAME_ALLOCATOR.lock();
        let Some(allocator) = guard.as_mut() else {
            return;
        };
        allocator.free_contiguous_frames(self.physical, self.pages);
    }
}

/// One packet-sized DMA buffer used by RX/TX virtqueues.
pub struct PacketBuffer {
    region: DmaRegion,
}

impl PacketBuffer {
    pub fn new(capacity: usize) -> Result<Self, NetError> {
        Ok(Self {
            region: DmaRegion::allocate(capacity)?,
        })
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.region.len()
    }

    #[must_use]
    pub fn physical(&self) -> PhysAddr {
        self.region.physical()
    }
}

impl Deref for PacketBuffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.region.slice()
    }
}

impl DerefMut for PacketBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.region.slice_mut()
    }
}
