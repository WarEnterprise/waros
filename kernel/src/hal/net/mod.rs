pub mod e1000;
pub mod nic_select;

use alloc::format;
use alloc::string::String;

use crate::hal::DEVICES;
use crate::net;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, NetworkCapabilities,
};

pub fn register_active_network() -> Option<DeviceId> {
    let hardware = net::hardware_status()?;
    let pci = net::pci_devices()
        .into_iter()
        .find(|device| match hardware.driver {
            "virtio-net" => nic_select::is_virtio_net(device),
            "e1000" => nic_select::is_e1000(device),
            _ => false,
        });

    let (bus, vendor_id, product_id, driver) = if let Some(device) = pci {
        (
            BusLocation::Pci {
                bus: device.bus,
                device: device.device,
                function: device.function,
            },
            device.vendor_id,
            device.device_id,
            hardware.driver,
        )
    } else {
        (BusLocation::Virtual, 0, 0, hardware.driver)
    };

    Some(DEVICES.lock().register_or_update(
        DeviceInfo {
            name: format!("{} {}", hardware.name, net::format_mac(&hardware.mac)),
            category: DeviceCategory::Network,
            bus,
            vendor_id,
            product_id,
            capabilities: DeviceCapabilities::Network(NetworkCapabilities {
                max_speed_mbps: hardware.link_speed_mbps,
                has_wifi: false,
                has_ethernet: true,
                mac_address: hardware.mac,
                pq_tls_offload: false,
            }),
        },
        DriverState::Loaded(String::from(driver)),
        DeviceStatus::Active,
    ))
}

pub fn register_detected_nics() -> usize {
    let mut count = 0usize;

    for device in net::pci_devices() {
        let Some((name, max_speed, driver)) =
            classify_network_device(device.vendor_id, device.device_id, device.class_code)
        else {
            continue;
        };
        DEVICES.lock().register_or_update(
            DeviceInfo {
                name: format!(
                    "{} (PCI {:02X}:{:02X}.{})",
                    name, device.bus, device.device, device.function
                ),
                category: DeviceCategory::Network,
                bus: BusLocation::Pci {
                    bus: device.bus,
                    device: device.device,
                    function: device.function,
                },
                vendor_id: device.vendor_id,
                product_id: device.device_id,
                capabilities: DeviceCapabilities::Network(NetworkCapabilities {
                    max_speed_mbps: max_speed,
                    has_wifi: false,
                    has_ethernet: true,
                    mac_address: [0; 6],
                    pq_tls_offload: false,
                }),
            },
            DriverState::Loaded(String::from(driver)),
            DeviceStatus::Discovered,
        );
        count += 1;
    }

    count
}

fn classify_network_device(
    vendor_id: u16,
    device_id: u16,
    class_code: u8,
) -> Option<(&'static str, u32, &'static str)> {
    if class_code != 0x02 {
        return None;
    }

    if vendor_id == 0x1AF4 && matches!(device_id, 0x1000..=0x103F | 0x1041) {
        return Some(("VirtIO Network", 1000, "virtio-net"));
    }

    if vendor_id == 0x8086 {
        return Some(("Intel E1000/E1000E", 1000, "e1000"));
    }

    if vendor_id == 0x10EC {
        return Some(("Realtek Ethernet", 1000, "rtl8169"));
    }

    Some(("Ethernet Controller", 1000, "generic-net"))
}
