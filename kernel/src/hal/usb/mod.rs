pub mod descriptors;
pub mod hid;
pub mod mass_storage;
pub mod xhci;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use spin::{Lazy, Mutex};

use crate::hal::DEVICES;
use crate::net;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceInfo, DeviceStatus, DriverState,
    StorageCapabilities, UsbCapabilities, UsbSpeed,
};

static XHCI_CONTROLLERS: Lazy<Mutex<Vec<xhci::XhciController>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

pub fn poll() {
    let mut controllers = XHCI_CONTROLLERS.lock();
    for controller in controllers.iter_mut() {
        controller.poll();
    }
}

pub fn probe_controllers() -> usize {
    XHCI_CONTROLLERS.lock().clear();
    let mut count = 0usize;

    for device in net::pci_devices() {
        if device.class_code != 0x0C || device.subclass != 0x03 {
            continue;
        }

        let (driver, speed, status) = match device.prog_if {
            0x30 => ("xhci-waros", UsbSpeed::Super, DeviceStatus::Active),
            0x20 => ("ehci-probe", UsbSpeed::High, DeviceStatus::Discovered),
            0x10 => ("ohci-probe", UsbSpeed::Full, DeviceStatus::Discovered),
            0x00 => ("uhci-probe", UsbSpeed::Full, DeviceStatus::Discovered),
            _ => ("usb-probe", UsbSpeed::Full, DeviceStatus::Discovered),
        };

        let mut status = status;
        let mut driver_name = driver;
        let mut detected_ports = alloc::vec::Vec::new();
        let mut controller_to_store = None;
        if device.prog_if == 0x30 {
            match xhci::XhciController::init(device) {
                Ok(controller) => {
                    detected_ports = controller.ports.clone();
                    controller_to_store = Some(controller);
                }
                Err(_) => {
                    driver_name = "xhci-probe";
                    status = DeviceStatus::Discovered;
                }
            }
        }

        let controller_id = DEVICES.lock().register_or_update(
            DeviceInfo {
                name: format!(
                    "USB Controller (PCI {:02X}:{:02X}.{}, prog_if {:02X})",
                    device.bus, device.device, device.function, device.prog_if
                ),
                category: DeviceCategory::UsbController,
                bus: BusLocation::Pci {
                    bus: device.bus,
                    device: device.device,
                    function: device.function,
                },
                vendor_id: device.vendor_id,
                product_id: device.device_id,
                capabilities: DeviceCapabilities::Usb(UsbCapabilities {
                    speed,
                    class: device.class_code,
                    subclass: device.subclass,
                    protocol: device.prog_if,
                    max_packet_size: 0,
                    num_endpoints: 0,
                }),
            },
            DriverState::Loaded(String::from(driver_name)),
            status,
        );

        if let Some(controller) = controller_to_store {
            XHCI_CONTROLLERS.lock().push(controller);
        }

        for port in detected_ports {
            if let Some(kind) = port.hid_kind {
                hid::register_hid_device(
                    controller_id,
                    port.port,
                    port.slot_id.unwrap_or(0),
                    port.vendor_id.unwrap_or(0),
                    port.product_id.unwrap_or(0),
                    &port.name,
                    kind,
                );
                continue;
            }

            let category = port.category;
            let capabilities = match category {
                DeviceCategory::Storage => {
                    let info = port.storage.unwrap_or(mass_storage::UsbMassStorageInfo {
                        capacity_sectors: 0,
                        sector_size: 512,
                    });
                    DeviceCapabilities::Storage(StorageCapabilities {
                        capacity_bytes: info.capacity_bytes(),
                        sector_size: info.sector_size,
                        is_removable: true,
                        is_readonly: false,
                        supports_trim: false,
                    })
                }
                _ => DeviceCapabilities::Usb(UsbCapabilities {
                    speed: port.speed,
                    class: 0,
                    subclass: 0,
                    protocol: 0,
                    max_packet_size: 0,
                    num_endpoints: 0,
                }),
            };

            DEVICES.lock().register_or_update(
                DeviceInfo {
                    name: port.name,
                    category,
                    bus: BusLocation::Usb {
                        controller: controller_id,
                        port: port.port,
                        address: port.slot_id.unwrap_or(0),
                    },
                    vendor_id: port.vendor_id.unwrap_or(0),
                    product_id: port.product_id.unwrap_or(0),
                    capabilities,
                },
                DriverState::Loaded(String::from(port.driver)),
                if port.configured {
                    DeviceStatus::Active
                } else {
                    DeviceStatus::Initialized
                },
            );
        }

        count += 1;
    }

    count
}
