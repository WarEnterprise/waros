use crate::net::pci::PciDevice;

const INTEL_E1000_IDS: &[u16] = &[0x100E, 0x10D3, 0x153A, 0x15B8, 0x15BD, 0x15BE];

#[must_use]
pub fn is_virtio_net(device: &PciDevice) -> bool {
    device.vendor_id == 0x1AF4
        && matches!(device.device_id, 0x1000..=0x103F | 0x1041)
        && device.class_code == 0x02
}

#[must_use]
pub fn is_e1000(device: &PciDevice) -> bool {
    device.vendor_id == 0x8086
        && INTEL_E1000_IDS.contains(&device.device_id)
        && device.class_code == 0x02
}
