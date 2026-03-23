use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use x86_64::PhysAddr;

use crate::net::buffer::DmaRegion;
use crate::net::NetError;

use super::VIRTQ_DESC_F_NEXT;

const VIRTQ_ALIGN: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub struct VirtqueueSnapshot {
    pub size: u16,
    pub avail_idx: u16,
    pub used_idx: u16,
    pub last_used_idx: u16,
}

#[repr(C, align(16))]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtqUsedElem {
    pub id: u32,
    pub len: u32,
}

/// Shared-memory descriptor ring owned jointly by the guest and the virtio device.
pub struct Virtqueue {
    #[allow(dead_code)]
    pub queue_index: u16,
    pub size: u16,
    region: DmaRegion,
    descriptors: *mut VirtqDesc,
    avail_flags: *mut u16,
    avail_idx: *mut u16,
    avail_ring: *mut u16,
    used_flags: *mut u16,
    used_idx: *mut u16,
    used_ring: *mut VirtqUsedElem,
    last_used_idx: u16,
}

// SAFETY: `Virtqueue` points to a uniquely owned DMA region and is always accessed behind
// the global networking mutex in the current single-core kernel execution model.
unsafe impl Send for Virtqueue {}

impl Virtqueue {
    pub fn new(queue_index: u16, size: u16) -> Result<Self, NetError> {
        if size == 0 {
            return Err(NetError::InitializationFailed("virtqueue size is zero"));
        }

        let desc_bytes = usize::from(size) * size_of::<VirtqDesc>();
        let avail_bytes = 6 + (usize::from(size) * size_of::<u16>());
        let used_offset = align_up(desc_bytes + avail_bytes, VIRTQ_ALIGN);
        let used_bytes = 6 + (usize::from(size) * size_of::<VirtqUsedElem>());
        let total_bytes = used_offset + used_bytes;

        let mut region = DmaRegion::allocate(total_bytes)?;
        let base = region.as_mut_ptr();

        // SAFETY: `base` points to a zeroed, page-aligned, physically contiguous DMA region.
        let descriptors = base.cast::<VirtqDesc>();
        // SAFETY: The available ring lives immediately after the descriptor table.
        let avail_flags = unsafe { base.add(desc_bytes).cast::<u16>() };
        // SAFETY: `avail_idx` follows `avail_flags` in the ring header.
        let avail_idx = unsafe { avail_flags.add(1) };
        // SAFETY: `avail_ring` follows the flags and idx fields.
        let avail_ring = unsafe { avail_flags.add(2) };
        // SAFETY: The used ring starts at the spec-mandated aligned offset.
        let used_flags = unsafe { base.add(used_offset).cast::<u16>() };
        // SAFETY: `used_idx` follows `used_flags`.
        let used_idx = unsafe { used_flags.add(1) };
        // SAFETY: `used_ring` follows the flags/idx pair.
        let used_ring = unsafe { used_flags.add(2).cast::<VirtqUsedElem>() };

        Ok(Self {
            queue_index,
            size,
            region,
            descriptors,
            avail_flags,
            avail_idx,
            avail_ring,
            used_flags,
            used_idx,
            used_ring,
            last_used_idx: 0,
        })
    }

    #[must_use]
    pub fn pfn(&self) -> u32 {
        (self.region.physical().as_u64() >> 12) as u32
    }

    pub fn set_descriptor(
        &mut self,
        id: u16,
        address: PhysAddr,
        length: u32,
        flags: u16,
        next: u16,
    ) -> Result<(), NetError> {
        if id >= self.size {
            return Err(NetError::InvalidFrame);
        }

        // SAFETY: `id < self.size`, so the descriptor slot is within the descriptor table.
        unsafe {
            write_volatile(
                self.descriptors.add(usize::from(id)),
                VirtqDesc {
                    addr: address.as_u64(),
                    len: length,
                    flags,
                    next: if flags & VIRTQ_DESC_F_NEXT != 0 { next } else { 0 },
                },
            );
        }
        Ok(())
    }

    pub fn add_available(&mut self, id: u16) -> Result<(), NetError> {
        if id >= self.size {
            return Err(NetError::InvalidFrame);
        }

        let index = self.available_index();
        let slot = usize::from(index % self.size);
        // SAFETY: `slot < self.size`, and the available ring has `self.size` entries.
        unsafe {
            write_volatile(self.avail_ring.add(slot), id);
        }
        fence(Ordering::SeqCst);
        self.set_available_index(index.wrapping_add(1));
        Ok(())
    }

    #[must_use]
    pub fn pop_used(&mut self) -> Option<VirtqUsedElem> {
        let used_index = self.used_index();
        if self.last_used_idx == used_index {
            return None;
        }

        let slot = usize::from(self.last_used_idx % self.size);
        // SAFETY: `slot < self.size`, and `self.last_used_idx != used_index` guarantees a valid used entry.
        let element = unsafe { read_volatile(self.used_ring.add(slot)) };
        self.last_used_idx = self.last_used_idx.wrapping_add(1);
        Some(element)
    }

    #[must_use]
    pub fn snapshot(&self) -> VirtqueueSnapshot {
        VirtqueueSnapshot {
            size: self.size,
            avail_idx: self.available_index(),
            used_idx: self.used_index(),
            last_used_idx: self.last_used_idx,
        }
    }

    fn available_index(&self) -> u16 {
        // SAFETY: `avail_idx` points into the queue's shared DMA memory.
        unsafe { read_volatile(self.avail_idx) }
    }

    fn set_available_index(&mut self, value: u16) {
        // SAFETY: `avail_idx` points into the queue's shared DMA memory.
        unsafe {
            write_volatile(self.avail_idx, value);
        }
    }

    fn used_index(&self) -> u16 {
        // SAFETY: `used_idx` points into the queue's shared DMA memory.
        unsafe { read_volatile(self.used_idx) }
    }
}

impl Drop for Virtqueue {
    fn drop(&mut self) {
        let _ = self.avail_flags;
        let _ = self.used_flags;
    }
}

const fn align_up(value: usize, alignment: usize) -> usize {
    (value + (alignment - 1)) & !(alignment - 1)
}
