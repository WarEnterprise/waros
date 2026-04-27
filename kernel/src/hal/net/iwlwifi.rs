//! First-layer probe for Intel iwlwifi-class Wi-Fi controllers.
//!
//! This is **not** a working Wi-Fi driver. Real iwlwifi requires a multi-MB
//! signed firmware blob (`iwlwifi-*.ucode`) loaded into the device by the host
//! over the PCI bus, plus a complex command/queue setup, plus a full 802.11
//! MAC/SME/MLME stack on top. WarOS does not yet ship any of those pieces.
//!
//! What this module *does* provide is real, honest hardware introspection:
//! map BAR0, enable bus mastering, read the canonical Intel CSR registers
//! (CSR_HW_REV and CSR_HW_RF_ID), and surface the result. This lets the
//! `wifi probe` shell command tell the user *exactly* which silicon revision
//! they have, and exactly why we cannot drive it yet, instead of returning a
//! generic "unsupported" string.
//!
//! Anything past this layer (firmware load, NVM/EEPROM parse, scan, assoc,
//! WPA, DHCP-over-Wi-Fi) is intentionally left as a known-honest TODO. We do
//! not pretend to support what we cannot.

use alloc::string::String;
use core::ptr::read_volatile;

use x86_64::PhysAddr;

use crate::memory;
use crate::net::pci::{self, PciBar, PciDevice};

use super::firmware::{self, FirmwareRequirement};

/// CSR register offsets (subset). These are stable across all iwlwifi-class
/// chips from 7000-series through Wi-Fi 6E (AX201/AX211/AX210).
const CSR_HW_IF_CONFIG_REG: u64 = 0x000;
const CSR_INT_MASK: u64 = 0x00C;
const CSR_RESET: u64 = 0x020;
const CSR_GP_CNTRL: u64 = 0x024;
const CSR_HW_REV: u64 = 0x028;
const CSR_HW_RF_ID: u64 = 0x09C;
const CSR_GIO_CHICKEN_BITS: u64 = 0x100;

/// Required MMIO mapping size for the CSR window. iwlwifi BAR0 is typically
/// 8 KiB or 16 KiB; we map a single page which is enough for every CSR we
/// touch.
const CSR_MAP_SIZE: usize = 0x1000;

/// Honest probe-only snapshot. Everything is read-only; no firmware is loaded,
/// no queues are armed, no IRQ is hooked.
#[derive(Debug, Clone)]
pub struct IwlwifiProbe {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub revision_id: u8,
    pub bar0_phys: u64,
    pub csr_hw_if_config: u32,
    pub csr_hw_rev: u32,
    pub csr_hw_rf_id: u32,
    pub csr_gp_cntrl: u32,
    pub mmio_alive: bool,
    pub family: &'static str,
    pub firmware_required: &'static str,
    pub firmware: FirmwareRequirement,
    pub driver_state: &'static str,
    pub transport_state: &'static str,
    pub scan_state: &'static str,
    pub connect_state: &'static str,
    pub data_state: &'static str,
    pub next_step: &'static str,
    pub blocker: String,
}

impl IwlwifiProbe {
    /// Decode the iwlwifi DASH/STEP from CSR_HW_REV. The low byte format
    /// across all generations is `[STEP:4][DASH:4]`. Higher bytes encode
    /// silicon family.
    #[must_use]
    pub fn step_dash(&self) -> (u8, u8) {
        let low = (self.csr_hw_rev & 0xFF) as u8;
        let step = (low >> 2) & 0x3;
        let dash = low & 0x3;
        (step, dash)
    }

    /// Decoded RF chip family. The Type field is bits 12:8 of CSR_HW_RF_ID.
    #[must_use]
    pub fn rf_type_label(&self) -> &'static str {
        match (self.csr_hw_rf_id >> 12) & 0xF {
            0x0 => "RF NONE",
            0x1 => "RF JF1",
            0x2 => "RF JF2",
            0x4 => "RF HR1",
            0x5 => "RF HR2",
            0x7 => "RF GF",
            0x9 => "RF GF4",
            0xB => "RF JF",
            _ => "RF Unknown",
        }
    }
}

/// Probe one Intel WiFi PCI function. Read-only.
pub fn probe(pci_dev: &PciDevice) -> Option<IwlwifiProbe> {
    let bar0 = match pci_dev.bar(0) {
        PciBar::Memory32(base) => u64::from(base),
        PciBar::Memory64(base) => base,
        PciBar::Io(_) | PciBar::Unused => {
            crate::serial_println!(
                "[iwlwifi] {:02X}:{:02X}.{} no memory BAR0; cannot map CSR window",
                pci_dev.bus,
                pci_dev.device,
                pci_dev.function
            );
            return None;
        }
    };

    if bar0 == 0 {
        crate::serial_println!(
            "[iwlwifi] {:02X}:{:02X}.{} BAR0 unprogrammed; BIOS did not assign MMIO",
            pci_dev.bus,
            pci_dev.device,
            pci_dev.function
        );
        return None;
    }

    // Enable I/O + memory + bus mastering before any MMIO read. Many laptops
    // ship with this controller in D3 / disabled state by ACPI; touching CSR
    // before bus mastering is enabled returns 0xFFFF_FFFF.
    pci::enable_bus_mastering(pci_dev);
    // Disable ASPM L0s/L1 — same reason as the Realtek path: known cause of
    // CSR access faults on real notebooks.
    pci::disable_aspm(pci_dev);

    let virt = match memory::map_mmio(PhysAddr::new(bar0), CSR_MAP_SIZE) {
        Ok(v) => v,
        Err(_) => {
            crate::serial_println!(
                "[iwlwifi] {:02X}:{:02X}.{} CSR mapping failed bar0=0x{:08X}",
                pci_dev.bus,
                pci_dev.device,
                pci_dev.function,
                bar0
            );
            return None;
        }
    };

    let base = virt.as_u64();
    let read = |offset: u64| -> u32 {
        unsafe { read_volatile((base + offset) as *const u32) }
    };

    let csr_hw_if_config = read(CSR_HW_IF_CONFIG_REG);
    let csr_hw_rev = read(CSR_HW_REV);
    let csr_hw_rf_id = read(CSR_HW_RF_ID);
    let csr_gp_cntrl = read(CSR_GP_CNTRL);
    let _csr_int_mask = read(CSR_INT_MASK);
    let _csr_reset = read(CSR_RESET);
    let _csr_chicken = read(CSR_GIO_CHICKEN_BITS);

    // Sanity: 0xFFFFFFFF means the device did not ACK the read (off-bus / D3).
    let mmio_alive = csr_hw_rev != 0xFFFF_FFFF && csr_hw_rev != 0;
    let family = identify_family(pci_dev.device_id);
    let firmware_required = firmware_name(pci_dev.device_id);
    let firmware = firmware::require_blob(firmware_required);
    let transport_state = if !mmio_alive {
        "csr-probe-failed; transport-not-initialized"
    } else if firmware.blob_found {
        "csr-probe-ok; firmware-blob-ready; transport-not-initialized"
    } else {
        "csr-probe-ok; firmware-blob-missing; transport-not-initialized"
    };
    let next_step = if !mmio_alive {
        "fix PCI power/MMIO access before firmware upload can start"
    } else if firmware.blob_found {
        "attempt firmware upload into the transport, then bring up FH command queues and NVM parsing"
    } else {
        "place the required firmware blob under /lib/firmware or /firmware, then implement upload and FH transport init"
    };

    let blocker = if !mmio_alive {
        alloc::format!(
            "CSR_HW_REV reads 0x{csr_hw_rev:08X} — controller is not powered or BAR0 routing is broken; ACPI must enable PCIe slot first"
        )
    } else if let Some(path) = firmware.blob_path.as_deref() {
        alloc::format!(
            "Firmware blob '{firmware_required}' is present at {path}, but WarOS still lacks the upload path, FH command queue, NVM parser, and 802.11 MAC."
        )
    } else {
        alloc::format!(
            "iwlwifi requires signed firmware '{firmware_required}' loaded into the device. WarOS can now look for that blob in WarFS, but no matching file was found and upload/NVM/MAC are still unimplemented. Hardware is alive (CSR_HW_REV=0x{csr_hw_rev:08X})."
        )
    };

    crate::serial_println!(
        "[iwlwifi] {:02X}:{:02X}.{} dev={:04X} bar0=0x{:08X} HW_REV=0x{:08X} HW_RF_ID=0x{:08X} GP_CNTRL=0x{:08X} HW_IF_CFG=0x{:08X} alive={}",
        pci_dev.bus,
        pci_dev.device,
        pci_dev.function,
        pci_dev.device_id,
        bar0,
        csr_hw_rev,
        csr_hw_rf_id,
        csr_gp_cntrl,
        csr_hw_if_config,
        mmio_alive
    );

    Some(IwlwifiProbe {
        bus: pci_dev.bus,
        device: pci_dev.device,
        function: pci_dev.function,
        vendor_id: pci_dev.vendor_id,
        device_id: pci_dev.device_id,
        revision_id: pci_dev.revision_id,
        bar0_phys: bar0,
        csr_hw_if_config,
        csr_hw_rev,
        csr_hw_rf_id,
        csr_gp_cntrl,
        mmio_alive,
        family,
        firmware_required,
        firmware,
        driver_state: "probe-foundation",
        transport_state,
        scan_state: "unsupported",
        connect_state: "unsupported",
        data_state: "unsupported",
        next_step,
        blocker,
    })
}

fn identify_family(device_id: u16) -> &'static str {
    match device_id {
        0x2723 => "Intel Wi-Fi 6 AX200 (Cyclone Peak)",
        0xA0F0 | 0x02F0 | 0x06F0 | 0x34F0 => "Intel Wi-Fi 6 AX201 (Tiger Lake / Comet Lake)",
        0x4DF0 | 0x51F0 | 0x54F0 => "Intel Wi-Fi 6E AX211 (Alder Lake / Raptor Lake)",
        0x2725 | 0x2726 => "Intel Wi-Fi 6E AX210 (Typhoon Peak)",
        0x2526 => "Intel Wireless-AC 9260",
        0x9DF0 | 0xA370 | 0x31DC | 0x30DC => "Intel Wireless-AC 9560",
        0x24FD | 0x24FB => "Intel Wireless-AC 8265",
        0x095A | 0x095B => "Intel Dual Band Wireless-AC 7265",
        _ => "Intel Wireless (unrecognised iwlwifi-class)",
    }
}

#[must_use]
pub fn firmware_name(device_id: u16) -> &'static str {
    match device_id {
        0x2723 => "iwlwifi-cc-a0-*.ucode",
        0xA0F0 | 0x02F0 | 0x06F0 | 0x34F0 => "iwlwifi-QuZ-a0-hr-b0-*.ucode",
        0x4DF0 | 0x51F0 | 0x54F0 => "iwlwifi-so-a0-gf-a0-*.ucode",
        0x2725 | 0x2726 => "iwlwifi-ty-a0-gf-a0-*.ucode",
        0x2526 => "iwlwifi-9260-th-b0-jf-b0-*.ucode",
        0x9DF0 | 0xA370 | 0x31DC | 0x30DC => "iwlwifi-9000-pu-b0-jf-b0-*.ucode",
        0x24FD | 0x24FB => "iwlwifi-8265-*.ucode",
        0x095A | 0x095B => "iwlwifi-7265D-*.ucode",
        _ => "iwlwifi-*.ucode",
    }
}
