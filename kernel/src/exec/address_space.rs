use alloc::vec::Vec;

use super::process::{MemorySegment, SegmentFlags};

/// Virtual address space layout tracked by WarExec.
#[derive(Debug, Clone)]
pub struct AddressSpace {
    pub segments: Vec<MemorySegment>,
    pub brk: u64,
    pub initial_brk: u64,
    pub heap_limit: u64,
    pub stack_top: u64,
    pub stack_bottom: u64,
    pub mmap_top: u64,
    pub page_table_phys: u64,
}

impl AddressSpace {
    pub const USER_CODE_BASE: u64 = 0x0000_0040_0000;
    pub const USER_HEAP_BASE: u64 = 0x0000_1000_0000;
    pub const USER_STACK_TOP: u64 = 0x0000_7FFF_F000;
    pub const USER_STACK_SIZE: u64 = 0x0000_0020_0000;
    pub const USER_MMAP_TOP: u64 = 0x0000_6000_0000;

    #[must_use]
    pub fn new(page_table_phys: u64) -> Self {
        let stack_top = Self::USER_STACK_TOP;
        let stack_bottom = stack_top - Self::USER_STACK_SIZE;
        Self {
            segments: Vec::new(),
            brk: Self::USER_HEAP_BASE,
            initial_brk: Self::USER_HEAP_BASE,
            heap_limit: Self::USER_MMAP_TOP,
            stack_top,
            stack_bottom,
            mmap_top: Self::USER_MMAP_TOP,
            page_table_phys,
        }
    }

    pub fn register_segment(&mut self, vaddr: u64, size: u64, flags: SegmentFlags) {
        self.segments.push(MemorySegment { vaddr, size, flags });
    }

    pub fn finalize_heap_layout(&mut self) {
        let heap_base = self
            .segments
            .iter()
            .map(|segment| segment.vaddr + segment.size)
            .max()
            .unwrap_or(Self::USER_HEAP_BASE);
        let aligned = (heap_base + 0xFFF) & !0xFFF;
        self.initial_brk = aligned.max(Self::USER_HEAP_BASE);
        self.brk = self.initial_brk;
        self.heap_limit = self.mmap_top.min(self.stack_bottom);
    }
}
