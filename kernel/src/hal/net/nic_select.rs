use crate::net::pci::PciDevice;

const INTEL_E1000_IDS: &[u16] = &[0x100E, 0x10D3, 0x153A, 0x15B8, 0x15BD, 0x15BE];
const REALTEK_RTL8169_FAMILY_IDS: &[u16] = &[0x8161, 0x8168, 0x8169];

/// Known Intel WiFi device IDs (for detection/classification only, not driver support).
const INTEL_WIFI_IDS: &[u16] = &[
    0x2723, // AX200
    0xA0F0, 0x02F0, 0x06F0, 0x34F0, // AX201
    0x4DF0, 0x51F0, 0x54F0, // AX211
    0x2725, 0x2726, // AX210
    0x2526, // AC 9260
    0x9DF0, 0xA370, 0x31DC, 0x30DC, // AC 9560
    0x24FD, 0x24FB, // AC 8265
    0x095A, 0x095B, // AC 7265
];

#[must_use]
pub fn is_virtio_net_id(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == 0x1AF4 && matches!(device_id, 0x1000..=0x103F | 0x1041)
}

#[must_use]
pub fn is_e1000_id(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == 0x8086 && INTEL_E1000_IDS.contains(&device_id)
}

#[must_use]
pub fn is_rtl8169_id(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == 0x10EC && REALTEK_RTL8169_FAMILY_IDS.contains(&device_id)
}

#[must_use]
pub fn supported_wired_driver(vendor_id: u16, device_id: u16) -> Option<&'static str> {
    if is_virtio_net_id(vendor_id, device_id) {
        Some("virtio-net")
    } else if is_e1000_id(vendor_id, device_id) {
        Some("e1000")
    } else if is_rtl8169_id(vendor_id, device_id) {
        Some("rtl8169")
    } else {
        None
    }
}

#[must_use]
pub fn is_virtio_net(device: &PciDevice) -> bool {
    is_virtio_net_id(device.vendor_id, device.device_id) && device.class_code == 0x02
}

#[must_use]
pub fn is_e1000(device: &PciDevice) -> bool {
    is_e1000_id(device.vendor_id, device.device_id) && device.class_code == 0x02
}

#[must_use]
pub fn is_rtl8169(device: &PciDevice) -> bool {
    is_rtl8169_id(device.vendor_id, device.device_id) && device.class_code == 0x02
}

/// Check if this is a known Intel WiFi adapter (detection only, no driver support).
#[must_use]
pub fn is_intel_wifi(device: &PciDevice) -> bool {
    device.vendor_id == 0x8086
        && device.class_code == 0x02
        && device.subclass == 0x80
        && INTEL_WIFI_IDS.contains(&device.device_id)
}

/// Check by IDs alone (no class check).
#[must_use]
pub fn is_intel_wifi_id(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == 0x8086 && INTEL_WIFI_IDS.contains(&device_id)
}
