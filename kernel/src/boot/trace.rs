use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use x86_64::registers::control::Cr3;

use crate::memory;

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum BootStage {
    Unknown = 0,
    KernelEntry,
    BootHandoff,
    DescriptorTables,
    Memory,
    Acpi,
    Storage,
    Exec,
    Hardware,
    Input,
    Proofs,
    Shell,
}

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum BreadcrumbTag {
    None = 0,
    KernelEntry,
    BootContext,
    PhysMapRegistered,
    ConsoleReady,
    GdtLoaded,
    SyscallReady,
    IdtLoaded,
    PicReady,
    PitReady,
    MemoryMapRegistered,
    PhysicalAllocatorReady,
    PagingMapperReady,
    HeapReady,
    HalReady,
    FsReady,
    AcpiInit,
    AcpiRsdpBootloader,
    AcpiRsdpLegacyScan,
    AcpiRsdt,
    AcpiXsdt,
    AcpiFadt,
    AcpiDsdt,
    DiskReady,
    AuthReady,
    TaskReady,
    ExecReady,
    SecurityReady,
    NetInit,
    UsbProbe,
    InputReady,
    InterruptsEnabled,
    ProofsActive,
    ShellReady,
    PagingActiveCr3,
}

static LAST_STAGE: AtomicU32 = AtomicU32::new(BootStage::Unknown as u32);
static LAST_TAG: AtomicU32 = AtomicU32::new(BreadcrumbTag::None as u32);
static PHYSICAL_MEMORY_OFFSET: AtomicU64 = AtomicU64::new(0);
static PHYSICAL_MEMORY_LIMIT: AtomicU64 = AtomicU64::new(0);
static LAST_RSDP_ADDR: AtomicU64 = AtomicU64::new(0);
static LAST_CR3_PHYS: AtomicU64 = AtomicU64::new(0);
static LAST_DIRECT_MAP_SOURCE: AtomicU32 = AtomicU32::new(BreadcrumbTag::None as u32);
static LAST_DIRECT_MAP_PHYS: AtomicU64 = AtomicU64::new(0);
static LAST_DIRECT_MAP_VIRT: AtomicU64 = AtomicU64::new(0);
static LAST_PAGE_FAULT_ADDR: AtomicU64 = AtomicU64::new(0);

pub fn set_stage(stage: BootStage, tag: BreadcrumbTag) {
    LAST_STAGE.store(stage as u32, Ordering::Relaxed);
    LAST_TAG.store(tag as u32, Ordering::Relaxed);
    record_current_cr3();
}

pub fn record_tag(tag: BreadcrumbTag) {
    LAST_TAG.store(tag as u32, Ordering::Relaxed);
    record_current_cr3();
}

pub fn record_physical_memory_mapping(offset: u64, limit: u64) {
    PHYSICAL_MEMORY_OFFSET.store(offset, Ordering::Relaxed);
    PHYSICAL_MEMORY_LIMIT.store(limit, Ordering::Relaxed);
}

pub fn record_rsdp_addr(address: Option<u64>) {
    LAST_RSDP_ADDR.store(address.unwrap_or(0), Ordering::Relaxed);
}

pub fn record_current_cr3() {
    let (frame, _) = Cr3::read();
    LAST_CR3_PHYS.store(frame.start_address().as_u64(), Ordering::Relaxed);
}

pub fn record_direct_map(source: BreadcrumbTag, physical: u64, virtual_address: u64) {
    LAST_DIRECT_MAP_SOURCE.store(source as u32, Ordering::Relaxed);
    LAST_DIRECT_MAP_PHYS.store(physical, Ordering::Relaxed);
    LAST_DIRECT_MAP_VIRT.store(virtual_address, Ordering::Relaxed);
    record_tag(source);
}

pub fn record_page_fault(address: u64) {
    LAST_PAGE_FAULT_ADDR.store(address, Ordering::Relaxed);
    record_current_cr3();
}

pub fn write_panic_breadcrumbs(writer: &mut impl Write) -> fmt::Result {
    let stage = BootStage::from_raw(LAST_STAGE.load(Ordering::Relaxed));
    let tag = BreadcrumbTag::from_raw(LAST_TAG.load(Ordering::Relaxed));
    let physical_memory_offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::Relaxed);
    let physical_memory_limit = PHYSICAL_MEMORY_LIMIT.load(Ordering::Relaxed);
    let rsdp_addr = LAST_RSDP_ADDR.load(Ordering::Relaxed);
    let cr3_phys = LAST_CR3_PHYS.load(Ordering::Relaxed);
    let last_direct_map_source =
        BreadcrumbTag::from_raw(LAST_DIRECT_MAP_SOURCE.load(Ordering::Relaxed));
    let last_direct_map_phys = LAST_DIRECT_MAP_PHYS.load(Ordering::Relaxed);
    let last_direct_map_virt = LAST_DIRECT_MAP_VIRT.load(Ordering::Relaxed);
    let last_page_fault_addr = LAST_PAGE_FAULT_ADDR.load(Ordering::Relaxed);

    writeln!(
        writer,
        "  Boot stage: {} / {}",
        stage.label(),
        tag.label()
    )?;

    if physical_memory_offset != 0 {
        let limit_end = physical_memory_limit
            .checked_sub(1)
            .and_then(|limit| physical_memory_offset.checked_add(limit))
            .unwrap_or(physical_memory_offset);
        writeln!(
            writer,
            "  Phys map: 0x{physical_memory_offset:016X}..0x{limit_end:016X}"
        )?;
    }

    if rsdp_addr != 0 {
        writeln!(writer, "  RSDP addr: 0x{rsdp_addr:016X}")?;
    }

    if cr3_phys != 0 {
        writeln!(writer, "  CR3 phys: 0x{cr3_phys:016X}")?;
    }

    if last_direct_map_virt != 0 {
        writeln!(
            writer,
            "  Last direct map: {} phys 0x{last_direct_map_phys:016X} -> virt 0x{last_direct_map_virt:016X}",
            last_direct_map_source.label()
        )?;
    }

    if last_page_fault_addr != 0 {
        writeln!(writer, "  Fault CR2: 0x{last_page_fault_addr:016X}")?;
        if let Some(classification) = memory::classify_direct_map_address(last_page_fault_addr) {
            writeln!(
                writer,
                "  Fault class: direct map phys 0x{:016X}",
                classification.physical_address
            )?;
            if let Some(region) = classification.region {
                writeln!(
                    writer,
                    "  Fault phys region: {} [0x{:016X}-0x{:016X})",
                    memory::memory_region_kind_name(region.kind),
                    region.start,
                    region.end
                )?;
            }
        }
    }

    Ok(())
}

pub fn write_serial_panic_breadcrumbs() {
    use crate::serial_println;

    let stage = BootStage::from_raw(LAST_STAGE.load(Ordering::Relaxed));
    let tag = BreadcrumbTag::from_raw(LAST_TAG.load(Ordering::Relaxed));
    serial_println!("[PANIC] boot stage: {} / {}", stage.label(), tag.label());

    let physical_memory_offset = PHYSICAL_MEMORY_OFFSET.load(Ordering::Relaxed);
    let physical_memory_limit = PHYSICAL_MEMORY_LIMIT.load(Ordering::Relaxed);
    if physical_memory_offset != 0 {
        let limit_end = physical_memory_limit
            .checked_sub(1)
            .and_then(|limit| physical_memory_offset.checked_add(limit))
            .unwrap_or(physical_memory_offset);
        serial_println!(
            "[PANIC] phys map: 0x{physical_memory_offset:016X}..0x{limit_end:016X}"
        );
    }

    let rsdp_addr = LAST_RSDP_ADDR.load(Ordering::Relaxed);
    if rsdp_addr != 0 {
        serial_println!("[PANIC] rsdp addr: 0x{rsdp_addr:016X}");
    }

    let cr3_phys = LAST_CR3_PHYS.load(Ordering::Relaxed);
    if cr3_phys != 0 {
        serial_println!("[PANIC] cr3 phys: 0x{cr3_phys:016X}");
    }

    let last_direct_map_virt = LAST_DIRECT_MAP_VIRT.load(Ordering::Relaxed);
    if last_direct_map_virt != 0 {
        let source = BreadcrumbTag::from_raw(LAST_DIRECT_MAP_SOURCE.load(Ordering::Relaxed));
        let last_direct_map_phys = LAST_DIRECT_MAP_PHYS.load(Ordering::Relaxed);
        serial_println!(
            "[PANIC] last direct map: {} phys 0x{last_direct_map_phys:016X} -> virt 0x{last_direct_map_virt:016X}",
            source.label()
        );
    }

    let last_page_fault_addr = LAST_PAGE_FAULT_ADDR.load(Ordering::Relaxed);
    if last_page_fault_addr != 0 {
        serial_println!("[PANIC] fault cr2: 0x{last_page_fault_addr:016X}");
        if let Some(classification) = memory::classify_direct_map_address(last_page_fault_addr) {
            serial_println!(
                "[PANIC] fault class: direct map phys 0x{:016X}",
                classification.physical_address
            );
            if let Some(region) = classification.region {
                serial_println!(
                    "[PANIC] fault phys region: {} [0x{:016X}-0x{:016X})",
                    memory::memory_region_kind_name(region.kind),
                    region.start,
                    region.end
                );
            }
        }
    }
}

impl BootStage {
    fn from_raw(value: u32) -> Self {
        match value {
            x if x == Self::KernelEntry as u32 => Self::KernelEntry,
            x if x == Self::BootHandoff as u32 => Self::BootHandoff,
            x if x == Self::DescriptorTables as u32 => Self::DescriptorTables,
            x if x == Self::Memory as u32 => Self::Memory,
            x if x == Self::Acpi as u32 => Self::Acpi,
            x if x == Self::Storage as u32 => Self::Storage,
            x if x == Self::Exec as u32 => Self::Exec,
            x if x == Self::Hardware as u32 => Self::Hardware,
            x if x == Self::Input as u32 => Self::Input,
            x if x == Self::Proofs as u32 => Self::Proofs,
            x if x == Self::Shell as u32 => Self::Shell,
            _ => Self::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::KernelEntry => "kernel-entry",
            Self::BootHandoff => "boot-handoff",
            Self::DescriptorTables => "descriptor-tables",
            Self::Memory => "memory",
            Self::Acpi => "acpi",
            Self::Storage => "storage",
            Self::Exec => "exec",
            Self::Hardware => "hardware",
            Self::Input => "input",
            Self::Proofs => "proofs",
            Self::Shell => "shell",
        }
    }
}

impl BreadcrumbTag {
    fn from_raw(value: u32) -> Self {
        match value {
            x if x == Self::KernelEntry as u32 => Self::KernelEntry,
            x if x == Self::BootContext as u32 => Self::BootContext,
            x if x == Self::PhysMapRegistered as u32 => Self::PhysMapRegistered,
            x if x == Self::ConsoleReady as u32 => Self::ConsoleReady,
            x if x == Self::GdtLoaded as u32 => Self::GdtLoaded,
            x if x == Self::SyscallReady as u32 => Self::SyscallReady,
            x if x == Self::IdtLoaded as u32 => Self::IdtLoaded,
            x if x == Self::PicReady as u32 => Self::PicReady,
            x if x == Self::PitReady as u32 => Self::PitReady,
            x if x == Self::MemoryMapRegistered as u32 => Self::MemoryMapRegistered,
            x if x == Self::PhysicalAllocatorReady as u32 => Self::PhysicalAllocatorReady,
            x if x == Self::PagingMapperReady as u32 => Self::PagingMapperReady,
            x if x == Self::HeapReady as u32 => Self::HeapReady,
            x if x == Self::HalReady as u32 => Self::HalReady,
            x if x == Self::FsReady as u32 => Self::FsReady,
            x if x == Self::AcpiInit as u32 => Self::AcpiInit,
            x if x == Self::AcpiRsdpBootloader as u32 => Self::AcpiRsdpBootloader,
            x if x == Self::AcpiRsdpLegacyScan as u32 => Self::AcpiRsdpLegacyScan,
            x if x == Self::AcpiRsdt as u32 => Self::AcpiRsdt,
            x if x == Self::AcpiXsdt as u32 => Self::AcpiXsdt,
            x if x == Self::AcpiFadt as u32 => Self::AcpiFadt,
            x if x == Self::AcpiDsdt as u32 => Self::AcpiDsdt,
            x if x == Self::DiskReady as u32 => Self::DiskReady,
            x if x == Self::AuthReady as u32 => Self::AuthReady,
            x if x == Self::TaskReady as u32 => Self::TaskReady,
            x if x == Self::ExecReady as u32 => Self::ExecReady,
            x if x == Self::SecurityReady as u32 => Self::SecurityReady,
            x if x == Self::NetInit as u32 => Self::NetInit,
            x if x == Self::UsbProbe as u32 => Self::UsbProbe,
            x if x == Self::InputReady as u32 => Self::InputReady,
            x if x == Self::InterruptsEnabled as u32 => Self::InterruptsEnabled,
            x if x == Self::ProofsActive as u32 => Self::ProofsActive,
            x if x == Self::ShellReady as u32 => Self::ShellReady,
            x if x == Self::PagingActiveCr3 as u32 => Self::PagingActiveCr3,
            _ => Self::None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::KernelEntry => "kernel-entry",
            Self::BootContext => "boot-context",
            Self::PhysMapRegistered => "phys-map-registered",
            Self::ConsoleReady => "console-ready",
            Self::GdtLoaded => "gdt-loaded",
            Self::SyscallReady => "syscall-ready",
            Self::IdtLoaded => "idt-loaded",
            Self::PicReady => "pic-ready",
            Self::PitReady => "pit-ready",
            Self::MemoryMapRegistered => "memory-map-registered",
            Self::PhysicalAllocatorReady => "physical-allocator-ready",
            Self::PagingMapperReady => "paging-mapper-ready",
            Self::HeapReady => "heap-ready",
            Self::HalReady => "hal-ready",
            Self::FsReady => "fs-ready",
            Self::AcpiInit => "acpi-init",
            Self::AcpiRsdpBootloader => "acpi-rsdp-bootloader",
            Self::AcpiRsdpLegacyScan => "acpi-rsdp-legacy-scan",
            Self::AcpiRsdt => "acpi-rsdt",
            Self::AcpiXsdt => "acpi-xsdt",
            Self::AcpiFadt => "acpi-fadt",
            Self::AcpiDsdt => "acpi-dsdt",
            Self::DiskReady => "disk-ready",
            Self::AuthReady => "auth-ready",
            Self::TaskReady => "task-ready",
            Self::ExecReady => "exec-ready",
            Self::SecurityReady => "security-ready",
            Self::NetInit => "net-init",
            Self::UsbProbe => "usb-probe",
            Self::InputReady => "input-ready",
            Self::InterruptsEnabled => "interrupts-enabled",
            Self::ProofsActive => "proofs-active",
            Self::ShellReady => "shell-ready",
            Self::PagingActiveCr3 => "paging-active-cr3",
        }
    }
}
