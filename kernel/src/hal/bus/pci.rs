#![allow(dead_code)]

use alloc::format;
use alloc::string::String;

use super::super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceInfo, DeviceStatus, DriverState,
};
use crate::hal::DEVICES;
use crate::net::pci as legacy_pci;

pub use crate::net::pci::PciDevice;

#[must_use]
pub fn enumerate() -> alloc::vec::Vec<PciDevice> {
    legacy_pci::enumerate_pci()
}

#[must_use]
pub fn class_name(class_code: u8, subclass: u8) -> &'static str {
    legacy_pci::class_name(class_code, subclass)
}

pub fn enable_bus_mastering(device: &PciDevice) {
    legacy_pci::enable_bus_mastering(device);
}

pub fn enumerate_and_register(devices: &[PciDevice]) -> usize {
    let mut registered = 0usize;

    for device in devices {
        DEVICES.lock().register_or_update(
            DeviceInfo {
                name: describe(device),
                category: categorize(device),
                bus: BusLocation::Pci {
                    bus: device.bus,
                    device: device.device,
                    function: device.function,
                },
                vendor_id: device.vendor_id,
                product_id: device.device_id,
                capabilities: DeviceCapabilities::None,
            },
            DriverState::Loaded(String::from("pci-enum")),
            DeviceStatus::Discovered,
        );
        registered += 1;
    }

    registered
}

fn categorize(device: &PciDevice) -> DeviceCategory {
    match (device.class_code, device.subclass) {
        (0x01, _) => DeviceCategory::Storage,
        (0x02, _) => DeviceCategory::Network,
        (0x03, _) => DeviceCategory::Display,
        (0x0C, 0x03) => DeviceCategory::UsbController,
        _ => DeviceCategory::Other,
    }
}

fn describe(device: &PciDevice) -> String {
    format!(
        "{} (PCI {:02X}:{:02X}.{} VID:{:04X} DID:{:04X})",
        class_name(device.class_code, device.subclass),
        device.bus,
        device.device,
        device.function,
        device.vendor_id,
        device.device_id
    )
}
