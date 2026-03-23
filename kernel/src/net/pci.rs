use alloc::vec::Vec;

use crate::arch::x86_64::port;

/// Standard PCI configuration-space snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision_id: u8,
    pub header_type: u8,
    pub bars: [u32; 6],
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PciBar {
    Unused,
    Io(u16),
    Memory32(u32),
    Memory64(u64),
}

/// Build a configuration-space address for the legacy 0xCF8/0xCFC ports.
#[must_use]
pub fn pci_config_address(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1u32 << 31)
        | (u32::from(bus) << 16)
        | (u32::from(device) << 11)
        | (u32::from(function) << 8)
        | (u32::from(offset) & 0xFC)
}

/// Read one 32-bit register from PCI configuration space.
#[must_use]
pub fn pci_config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    port::outl(0xCF8, pci_config_address(bus, device, function, offset));
    port::inl(0xCFC)
}

/// Write one 32-bit register to PCI configuration space.
pub fn pci_config_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    port::outl(0xCF8, pci_config_address(bus, device, function, offset));
    port::outl(0xCFC, value);
}

/// Scan every bus/device/function visible to the kernel.
#[must_use]
pub fn enumerate_pci() -> Vec<PciDevice> {
    let mut devices = Vec::new();

    for bus in 0..=u8::MAX {
        for device in 0..32u8 {
            let vendor = pci_config_read32(bus, device, 0, 0x00) as u16;
            if vendor == 0xFFFF {
                continue;
            }

            let header_type = ((pci_config_read32(bus, device, 0, 0x0C) >> 16) & 0xFF) as u8;
            let functions = if header_type & 0x80 != 0 { 8 } else { 1 };
            for function in 0..functions {
                let vendor_device = pci_config_read32(bus, device, function, 0x00);
                let vendor_id = (vendor_device & 0xFFFF) as u16;
                if vendor_id == 0xFFFF {
                    continue;
                }

                let device_id = (vendor_device >> 16) as u16;
                let class_reg = pci_config_read32(bus, device, function, 0x08);
                let class_code = (class_reg >> 24) as u8;
                let subclass = (class_reg >> 16) as u8;
                let prog_if = (class_reg >> 8) as u8;
                let revision_id = class_reg as u8;
                let header_type =
                    ((pci_config_read32(bus, device, function, 0x0C) >> 16) & 0xFF) as u8;

                let mut bars = [0u32; 6];
                for (index, bar) in bars.iter_mut().enumerate() {
                    *bar = pci_config_read32(bus, device, function, 0x10 + (index as u8 * 4));
                }

                let interrupt_reg = pci_config_read32(bus, device, function, 0x3C);
                devices.push(PciDevice {
                    bus,
                    device,
                    function,
                    vendor_id,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                    revision_id,
                    header_type,
                    bars,
                    interrupt_line: interrupt_reg as u8,
                    interrupt_pin: (interrupt_reg >> 8) as u8,
                });
            }
        }
    }

    devices
}

/// Return the first detected virtio-net PCI function, if any.
#[must_use]
#[allow(dead_code)]
pub fn find_virtio_net() -> Option<PciDevice> {
    enumerate_pci()
        .into_iter()
        .find(|device| is_virtio_net(device))
}

/// Search a previously captured PCI inventory for virtio-net.
#[must_use]
pub fn find_virtio_net_in(devices: &[PciDevice]) -> Option<PciDevice> {
    devices.iter().copied().find(is_virtio_net)
}

/// Enable I/O decoding, memory decoding, and bus mastering for a PCI function.
pub fn enable_bus_mastering(device: &PciDevice) {
    let mut command_status = pci_config_read32(device.bus, device.device, device.function, 0x04);
    command_status |= 0b111;
    pci_config_write32(
        device.bus,
        device.device,
        device.function,
        0x04,
        command_status,
    );
}

impl PciDevice {
    #[must_use]
    pub fn bar(&self, index: usize) -> PciBar {
        if index >= self.bars.len() {
            return PciBar::Unused;
        }

        let raw = self.bars[index];
        if raw == 0 {
            return PciBar::Unused;
        }

        if raw & 0x1 == 0x1 {
            return PciBar::Io((raw & !0x3) as u16);
        }

        let memory_type = (raw >> 1) & 0x3;
        if memory_type == 0x2 && index + 1 < self.bars.len() {
            let high = u64::from(self.bars[index + 1]);
            PciBar::Memory64((u64::from(raw) & !0xF) | (high << 32))
        } else {
            PciBar::Memory32(raw & !0xF)
        }
    }
}

#[must_use]
pub fn class_name(class_code: u8, subclass: u8) -> &'static str {
    match (class_code, subclass) {
        (0x01, 0x00) => "Mass storage controller",
        (0x01, 0x06) => "SATA controller",
        (0x02, 0x00) => "Ethernet controller",
        (0x03, 0x00) => "VGA controller",
        (0x06, 0x00) => "Host bridge",
        (0x06, 0x01) => "ISA bridge",
        (0x06, 0x04) => "PCI bridge",
        (0x0C, 0x03) => "USB controller",
        _ => "Unknown",
    }
}

fn is_virtio_net(device: &PciDevice) -> bool {
    device.vendor_id == 0x1AF4
        && matches!(device.device_id, 0x1000..=0x103F | 0x1041)
        && device.class_code == 0x02
}
