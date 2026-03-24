use core::cell::UnsafeCell;
use core::ptr::addr_of;

use spin::Lazy;
use x86_64::instructions::segmentation::{Segment, CS, DS, ES, SS};
use x86_64::instructions::tables::load_tss;
use x86_64::structures::gdt::{Descriptor, GlobalDescriptorTable, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;
use x86_64::VirtAddr;

pub const DOUBLE_FAULT_IST_INDEX: u16 = 0;
pub const USER_KERNEL_STACK_INDEX: usize = 0;

const DOUBLE_FAULT_STACK_SIZE: usize = 4096 * 5;
const USER_KERNEL_STACK_SIZE: usize = 4096 * 8;

static mut DOUBLE_FAULT_STACK: [u8; DOUBLE_FAULT_STACK_SIZE] = [0; DOUBLE_FAULT_STACK_SIZE];
static mut USER_KERNEL_STACK: [u8; USER_KERNEL_STACK_SIZE] = [0; USER_KERNEL_STACK_SIZE];

struct TssCell(UnsafeCell<TaskStateSegment>);

// SAFETY: The contained TSS is only mutated through explicit boot/runtime helpers and is used
// as a singleton for the current CPU.
unsafe impl Sync for TssCell {}

static TSS: TssCell = TssCell(UnsafeCell::new(TaskStateSegment::new()));

#[derive(Clone, Copy)]
struct Selectors {
    kernel_code_selector: SegmentSelector,
    kernel_data_selector: SegmentSelector,
    user_data_selector: SegmentSelector,
    user_code_selector: SegmentSelector,
    tss_selector: SegmentSelector,
}

#[derive(Clone, Copy)]
pub struct SelectorSet {
    pub kernel_code: SegmentSelector,
    pub kernel_data: SegmentSelector,
    pub user_data: SegmentSelector,
    pub user_code: SegmentSelector,
    pub tss: SegmentSelector,
}

static GDT: Lazy<(GlobalDescriptorTable, Selectors)> = Lazy::new(|| {
    let tss = tss_mut();
    tss.interrupt_stack_table[usize::from(DOUBLE_FAULT_IST_INDEX)] = double_fault_stack_top();
    tss.privilege_stack_table[USER_KERNEL_STACK_INDEX] = user_kernel_stack_top();

    let mut gdt = GlobalDescriptorTable::new();
    let kernel_code_selector = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data_selector = gdt.append(Descriptor::kernel_data_segment());
    let user_data_selector = gdt.append(Descriptor::user_data_segment());
    let user_code_selector = gdt.append(Descriptor::user_code_segment());
    let tss_selector = gdt.append(Descriptor::tss_segment(tss_ref()));

    (
        gdt,
        Selectors {
            kernel_code_selector,
            kernel_data_selector,
            user_data_selector,
            user_code_selector,
            tss_selector,
        },
    )
});

/// Load the global descriptor table and task state segment for long mode plus ring-3 transitions.
pub fn init() {
    let (gdt, selectors) = &*GDT;
    gdt.load();

    // SAFETY: The selectors point into the loaded static GDT and remain valid for the
    // lifetime of the kernel.
    unsafe {
        CS::set_reg(selectors.kernel_code_selector);
        DS::set_reg(selectors.kernel_data_selector);
        ES::set_reg(selectors.kernel_data_selector);
        SS::set_reg(selectors.kernel_data_selector);
        load_tss(selectors.tss_selector);
    }
}

#[must_use]
pub fn selectors() -> SelectorSet {
    let (_, selectors) = &*GDT;
    SelectorSet {
        kernel_code: selectors.kernel_code_selector,
        kernel_data: selectors.kernel_data_selector,
        user_data: selectors.user_data_selector,
        user_code: selectors.user_code_selector,
        tss: selectors.tss_selector,
    }
}

#[must_use]
pub fn kernel_stack_top() -> VirtAddr {
    tss_ref().privilege_stack_table[USER_KERNEL_STACK_INDEX]
}

pub fn set_kernel_stack_top(stack_top: VirtAddr) {
    tss_mut().privilege_stack_table[USER_KERNEL_STACK_INDEX] = stack_top;
}

fn tss_ref() -> &'static TaskStateSegment {
    // SAFETY: `TSS` is a static singleton and only exposes shared access here.
    unsafe { &*TSS.0.get() }
}

fn tss_mut() -> &'static mut TaskStateSegment {
    // SAFETY: WarOS currently runs on a single CPU. Mutations happen through serialized kernel
    // control flow during init or process transitions.
    unsafe { &mut *TSS.0.get() }
}

fn double_fault_stack_top() -> VirtAddr {
    let stack_start = VirtAddr::from_ptr(addr_of!(DOUBLE_FAULT_STACK));
    stack_start + DOUBLE_FAULT_STACK_SIZE as u64
}

fn user_kernel_stack_top() -> VirtAddr {
    let stack_start = VirtAddr::from_ptr(addr_of!(USER_KERNEL_STACK));
    stack_start + USER_KERNEL_STACK_SIZE as u64
}
