#![allow(dead_code)]

use alloc::string::String;

use spin::{Lazy, Mutex};
use x86_64::instructions::port::Port;
use x86_64::PhysAddr;

use crate::boot::trace::{self, BootStage, BreadcrumbTag};
use crate::hal::DEVICES;
use crate::memory;
use crate::serial_println;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState,
};

mod tables;

pub use tables::{FadtInfo, Rsdp};

pub static ACPI: Lazy<Mutex<Option<AcpiSubsystem>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiError {
    RsdpNotFound,
    InvalidTable,
    Unsupported,
}

pub struct AcpiSubsystem {
    pub fadt: Option<FadtInfo>,
    pub pm1a_cnt_blk: u16,
    pub slp_typ_s5: u16,
    pub hal_id: DeviceId,
}

impl AcpiSubsystem {
    pub fn init(bootloader_rsdp: Option<u64>) -> Result<Self, AcpiError> {
        trace::set_stage(BootStage::Acpi, BreadcrumbTag::AcpiInit);
        let rsdp = find_rsdp(bootloader_rsdp)?;
        let fadt = parse_fadt(&rsdp)?;
        let slp_typ_s5 = fadt
            .as_ref()
            .and_then(|info| find_s5_slp_typ(info.dsdt_address))
            .unwrap_or(0);

        let subsystem = Self {
            pm1a_cnt_blk: fadt.as_ref().map_or(0, |info| info.pm1a_cnt_blk),
            fadt,
            slp_typ_s5,
            hal_id: DEVICES.lock().register_or_update(
                DeviceInfo {
                    name: String::from("ACPI Power Management"),
                    category: DeviceCategory::PowerManagement,
                    bus: BusLocation::Platform,
                    vendor_id: 0,
                    product_id: 0,
                    capabilities: DeviceCapabilities::None,
                },
                DriverState::Loaded(String::from("acpi-waros")),
                DeviceStatus::Active,
            ),
        };

        Ok(subsystem)
    }

    pub fn shutdown(&self) -> ! {
        serial_println!("[ACPI] shutdown requested");

        if self.pm1a_cnt_blk != 0 && self.slp_typ_s5 != 0 {
            let value = (self.slp_typ_s5 << 10) | (1 << 13);
            unsafe {
                Port::<u16>::new(self.pm1a_cnt_blk).write(value);
            }
        }

        unsafe {
            Port::<u16>::new(0x604).write(0x2000);
        }

        crate::arch::x86_64::hlt_loop()
    }

    pub fn reboot(&self) -> ! {
        serial_println!("[ACPI] reboot requested");

        if let Some(fadt) = self.fadt {
            if fadt.reset_reg_address != 0 {
                let reset_port = fadt.reset_reg_address as u16;
                unsafe {
                    Port::<u8>::new(reset_port).write(fadt.reset_value);
                }
            }
        }

        unsafe {
            Port::<u8>::new(0x64).write(0xFE);
        }

        crate::arch::x86_64::hlt_loop()
    }
}

pub fn init_global(bootloader_rsdp: Option<u64>) -> Result<DeviceId, AcpiError> {
    let acpi = AcpiSubsystem::init(bootloader_rsdp)?;
    let id = acpi.hal_id;
    *ACPI.lock() = Some(acpi);
    Ok(id)
}

#[must_use]
pub fn is_available() -> bool {
    ACPI.lock().is_some()
}

pub fn shutdown() -> ! {
    if let Some(acpi) = ACPI.lock().as_ref() {
        acpi.shutdown();
    }

    unsafe {
        Port::<u16>::new(0x604).write(0x2000);
    }
    crate::arch::x86_64::hlt_loop()
}

pub fn reboot() -> ! {
    if let Some(acpi) = ACPI.lock().as_ref() {
        acpi.reboot();
    }

    unsafe {
        Port::<u8>::new(0x64).write(0xFE);
    }
    crate::arch::x86_64::hlt_loop()
}

fn find_rsdp(bootloader_rsdp: Option<u64>) -> Result<Rsdp, AcpiError> {
    if let Some(address) = bootloader_rsdp {
        trace::set_stage(BootStage::Acpi, BreadcrumbTag::AcpiRsdpBootloader);
        serial_println!("[ACPI] RSDP source=bootloader addr=0x{address:X}");
        return parse_rsdp_at(address, BreadcrumbTag::AcpiRsdpBootloader, "bootloader");
    }

    trace::set_stage(BootStage::Acpi, BreadcrumbTag::AcpiRsdpLegacyScan);
    let legacy_start_phys = 0xE0000u64;
    let start =
        acpi_direct_map(BreadcrumbTag::AcpiRsdpLegacyScan, legacy_start_phys).ok_or(AcpiError::RsdpNotFound)?;
    serial_println!(
        "[ACPI] RSDP source=legacy-scan range=0x{legacy_start_phys:X}-0x{:X}",
        legacy_start_phys + 0x20000
    );
    let bytes = unsafe { core::slice::from_raw_parts(start as *const u8, 0x20000) };

    for offset in (0..bytes.len().saturating_sub(36)).step_by(16) {
        if &bytes[offset..offset + 8] != b"RSD PTR " {
            continue;
        }
        let physical_address = legacy_start_phys + offset as u64;
        if let Ok(rsdp) =
            parse_rsdp_at(physical_address, BreadcrumbTag::AcpiRsdpLegacyScan, "legacy-scan")
        {
            return Ok(rsdp);
        }
    }

    Err(AcpiError::RsdpNotFound)
}

fn parse_fadt(rsdp: &Rsdp) -> Result<Option<FadtInfo>, AcpiError> {
    if rsdp.revision >= 2 && rsdp.xsdt_address != 0 {
        parse_sdt_entries(rsdp.xsdt_address, true)
    } else if rsdp.rsdt_address != 0 {
        parse_sdt_entries(u64::from(rsdp.rsdt_address), false)
    } else {
        Ok(None)
    }
}

fn parse_sdt_entries(address: u64, xsdt: bool) -> Result<Option<FadtInfo>, AcpiError> {
    let (root_tag, root_label) = if xsdt {
        (BreadcrumbTag::AcpiXsdt, "XSDT")
    } else {
        (BreadcrumbTag::AcpiRsdt, "RSDT")
    };
    let table = map_table(address, root_tag, root_label)?;
    let entry_size = if xsdt { 8 } else { 4 };
    if table.len() < 36 {
        return Err(AcpiError::InvalidTable);
    }
    let length = u32::from_le_bytes([table[4], table[5], table[6], table[7]]) as usize;
    if length > table.len() || length < 36 {
        return Err(AcpiError::InvalidTable);
    }

    let entries = &table[36..length];
    for chunk in entries.chunks_exact(entry_size) {
        let child_address = if xsdt {
            u64::from_le_bytes(chunk.try_into().map_err(|_| AcpiError::InvalidTable)?)
        } else {
            u32::from_le_bytes(chunk.try_into().map_err(|_| AcpiError::InvalidTable)?) as u64
        };

        let child = map_table(child_address, BreadcrumbTag::AcpiFadt, "SDT child")?;
        if child.len() < 116 || &child[0..4] != b"FACP" {
            continue;
        }

        let dsdt_address =
            u32::from_le_bytes([child[40], child[41], child[42], child[43]]) as u64;
        let pm1a_cnt_blk =
            u32::from_le_bytes([child[64], child[65], child[66], child[67]]) as u16;
        let reset_reg_address = if child.len() >= 129 {
            u64::from_le_bytes([
                child[116], child[117], child[118], child[119], child[120], child[121],
                child[122], child[123],
            ])
        } else {
            0
        };
        let reset_value = if child.len() >= 129 { child[128] } else { 0 };
        serial_println!(
            "[ACPI] FADT phys=0x{child_address:X} dsdt=0x{dsdt_address:X} pm1a=0x{pm1a_cnt_blk:04X} reset=0x{reset_reg_address:X}"
        );

        return Ok(Some(FadtInfo {
            pm1a_cnt_blk,
            reset_reg_address,
            reset_value,
            dsdt_address,
        }));
    }

    Ok(None)
}

fn find_s5_slp_typ(dsdt_address: u64) -> Option<u16> {
    if dsdt_address == 0 {
        return None;
    }

    let dsdt = map_table(dsdt_address, BreadcrumbTag::AcpiDsdt, "DSDT").ok()?;
    if dsdt.len() < 40 {
        return None;
    }

    let length = u32::from_le_bytes([dsdt[4], dsdt[5], dsdt[6], dsdt[7]]) as usize;
    if length > dsdt.len() || length <= 36 {
        return None;
    }

    let aml = &dsdt[36..length];
    let pattern = b"_S5_";
    let index = aml.windows(pattern.len()).position(|window| window == pattern)?;
    let tail = &aml[index + pattern.len()..];

    for window in tail.windows(8).take(16) {
        if window.len() < 4 {
            continue;
        }
        if window[0] == 0x12 && (window[1] == 0x04 || window[1] == 0x06) && window[2] == 0x0A {
            return Some((window[3] as u16) & 0x7);
        }
    }

    None
}

fn map_table(
    address: u64,
    tag: BreadcrumbTag,
    label: &'static str,
) -> Result<&'static [u8], AcpiError> {
    trace::set_stage(BootStage::Acpi, tag);
    let virt = acpi_direct_map(tag, address).ok_or(AcpiError::InvalidTable)?;
    let header = unsafe { core::slice::from_raw_parts(virt as *const u8, 36) };
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    if length < 36 {
        return Err(AcpiError::InvalidTable);
    }
    serial_println!(
        "[ACPI] map {label} phys=0x{address:X} virt=0x{virt:X} len=0x{length:X} sig={}{}{}{}",
        char::from(header[0]),
        char::from(header[1]),
        char::from(header[2]),
        char::from(header[3]),
    );
    Ok(unsafe { core::slice::from_raw_parts(virt as *const u8, length) })
}

fn parse_rsdp_at(
    physical_address: u64,
    source_tag: BreadcrumbTag,
    source_label: &'static str,
) -> Result<Rsdp, AcpiError> {
    trace::set_stage(BootStage::Acpi, source_tag);
    let virtual_address = acpi_direct_map(source_tag, physical_address).ok_or(AcpiError::InvalidTable)?;
    let header = unsafe { core::slice::from_raw_parts(virtual_address as *const u8, 36) };
    if &header[0..8] != b"RSD PTR " {
        return Err(AcpiError::InvalidTable);
    }
    if !checksum_valid(&header[..20]) {
        return Err(AcpiError::InvalidTable);
    }

    let revision = header[15];
    let rsdt_address = u32::from_le_bytes([header[16], header[17], header[18], header[19]]);
    let xsdt_address = if revision >= 2 {
        let length = u32::from_le_bytes([header[20], header[21], header[22], header[23]]) as usize;
        if !(36..=4096).contains(&length) {
            return Err(AcpiError::InvalidTable);
        }
        let rsdp = unsafe { core::slice::from_raw_parts(virtual_address as *const u8, length) };
        if !checksum_valid(rsdp) {
            return Err(AcpiError::InvalidTable);
        }
        u64::from_le_bytes([
            header[24], header[25], header[26], header[27], header[28], header[29], header[30],
            header[31],
        ])
    } else {
        0
    };

    trace::record_rsdp_addr(Some(physical_address));
    serial_println!(
        "[ACPI] RSDP source={source_label} phys=0x{physical_address:X} rev={} rsdt=0x{rsdt_address:X} xsdt=0x{xsdt_address:X}",
        revision
    );

    Ok(Rsdp {
        revision,
        rsdt_address,
        xsdt_address,
    })
}

fn checksum_valid(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |acc, byte| acc.wrapping_add(*byte)) == 0
}

fn acpi_direct_map(tag: BreadcrumbTag, physical_address: u64) -> Option<u64> {
    let virtual_address = memory::phys_to_virt(PhysAddr::new(physical_address))?;
    trace::record_direct_map(tag, physical_address, virtual_address.as_u64());
    Some(virtual_address.as_u64())
}
