pub mod e1000;
pub mod firmware;
pub mod iwlwifi;
pub mod nic_select;
pub mod rtl8169;

use alloc::format;
use alloc::vec::Vec;

use crate::hal::DEVICES;
use crate::net;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, NetworkCapabilities,
};

struct DetectedNetworkDevice {
    name: &'static str,
    max_speed_mbps: u32,
    has_wifi: bool,
    has_ethernet: bool,
}

pub fn register_active_network() -> Option<DeviceId> {
    let hardware = net::hardware_status()?;
    let pci = net::pci_devices()
        .into_iter()
        .find(|device| match hardware.driver {
            "virtio-net" => nic_select::is_virtio_net(device),
            "e1000" => nic_select::is_e1000(device),
            "rtl8169" => nic_select::is_rtl8169(device),
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
        DriverState::Loaded(driver.into()),
        if hardware.link_state == net::LinkState::Up {
            DeviceStatus::Active
        } else {
            DeviceStatus::Initialized
        },
    ))
}

pub fn register_detected_nics() -> usize {
    let mut count = 0usize;

    for device in net::pci_devices() {
        let Some(classification) = classify_network_device(
            device.vendor_id,
            device.device_id,
            device.class_code,
            device.subclass,
        ) else {
            continue;
        };
        DEVICES.lock().register_or_update(
            DeviceInfo {
                name: format!(
                    "{} (PCI {:02X}:{:02X}.{})",
                    classification.name, device.bus, device.device, device.function
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
                    max_speed_mbps: classification.max_speed_mbps,
                    has_wifi: classification.has_wifi,
                    has_ethernet: classification.has_ethernet,
                    mac_address: [0; 6],
                    pq_tls_offload: false,
                }),
            },
            DriverState::None,
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
    subclass: u8,
) -> Option<DetectedNetworkDevice> {
    if class_code != 0x02 {
        return None;
    }

    // Wireless controllers: subclass 0x80 = "Other", many Intel WiFi chips use this
    if subclass == 0x80 {
        // Identify known Intel WiFi families for better diagnostics
        if vendor_id == 0x8086 {
            return Some(DetectedNetworkDevice {
                name: classify_intel_wifi(device_id),
                max_speed_mbps: 0,
                has_wifi: true,
                has_ethernet: false,
            });
        }
        return Some(DetectedNetworkDevice {
            name: "Wireless-class PCI Network Controller (detected only)",
            max_speed_mbps: 0,
            has_wifi: true,
            has_ethernet: false,
        });
    }

    if nic_select::is_virtio_net_id(vendor_id, device_id) {
        return Some(DetectedNetworkDevice {
            name: "VirtIO Network",
            max_speed_mbps: 1000,
            has_wifi: false,
            has_ethernet: true,
        });
    }

    if nic_select::is_e1000_id(vendor_id, device_id) {
        return Some(DetectedNetworkDevice {
            name: "Intel E1000/E1000E",
            max_speed_mbps: 1000,
            has_wifi: false,
            has_ethernet: true,
        });
    }

    if nic_select::is_rtl8169_id(vendor_id, device_id) {
        return Some(DetectedNetworkDevice {
            name: "Realtek RTL8169-family",
            max_speed_mbps: 1000,
            has_wifi: false,
            has_ethernet: true,
        });
    }

    if vendor_id == 0x8086 {
        return Some(DetectedNetworkDevice {
            name: "Intel Ethernet Controller (detected only)",
            max_speed_mbps: 0,
            has_wifi: false,
            has_ethernet: true,
        });
    }

    if vendor_id == 0x10EC {
        return Some(DetectedNetworkDevice {
            name: "Realtek Ethernet Controller (detected only)",
            max_speed_mbps: 0,
            has_wifi: false,
            has_ethernet: true,
        });
    }

    Some(DetectedNetworkDevice {
        name: "Ethernet Controller (detected only)",
        max_speed_mbps: 0,
        has_wifi: false,
        has_ethernet: true,
    })
}

/// Walk PCI for every Intel-class wireless controller and run a real
/// hardware probe (BAR0 map + CSR read). Returns one [`IwlwifiProbe`] per
/// detected device. This is read-only and never loads firmware.
#[must_use]
pub fn probe_intel_wifi() -> Vec<iwlwifi::IwlwifiProbe> {
    let mut results = Vec::new();
    for device in net::pci_devices() {
        if device.class_code == 0x02
            && device.subclass == 0x80
            && nic_select::is_intel_wifi(&device)
        {
            if let Some(probe) = iwlwifi::probe(&device) {
                results.push(probe);
            }
        }
    }
    results
}

/// Identify Intel WiFi chip family by PCI device ID for honest, precise diagnostics.
fn classify_intel_wifi(device_id: u16) -> &'static str {
    match device_id {
        // Intel Wi-Fi 6 AX200/AX201 family
        0x2723 => "Intel Wi-Fi 6 AX200 (iwlwifi, requires firmware — no WarOS driver)",
        0xA0F0 | 0x02F0 | 0x06F0 | 0x34F0 => {
            "Intel Wi-Fi 6 AX201 (iwlwifi, requires firmware — no WarOS driver)"
        }
        0x4DF0 | 0x51F0 | 0x54F0 => {
            "Intel Wi-Fi 6E AX211 (iwlwifi, requires firmware — no WarOS driver)"
        }
        // Intel Wi-Fi 6E AX210 family
        0x2725 | 0x2726 => {
            "Intel Wi-Fi 6E AX210 (iwlwifi, requires firmware — no WarOS driver)"
        }
        // Intel Wireless AC 9260/9560 family
        0x2526 => "Intel Wireless-AC 9260 (iwlwifi, requires firmware — no WarOS driver)",
        0x9DF0 | 0xA370 | 0x31DC | 0x30DC => {
            "Intel Wireless-AC 9560 (iwlwifi, requires firmware — no WarOS driver)"
        }
        // Intel Wireless AC 8265/8275 family
        0x24FD | 0x24FB => {
            "Intel Wireless-AC 8265 (iwlwifi, requires firmware — no WarOS driver)"
        }
        // Intel Wireless AC 7265 family
        0x095A | 0x095B => {
            "Intel Dual Band Wireless-AC 7265 (iwlwifi, requires firmware — no WarOS driver)"
        }
        // Catch-all for other Intel wireless
        _ => "Intel Wireless Controller (iwlwifi-class, requires firmware — no WarOS driver)",
    }
}
