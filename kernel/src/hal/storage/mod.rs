use alloc::format;
use alloc::string::String;

use crate::disk;
use crate::hal::DEVICES;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, StorageCapabilities,
};

pub fn register_active_storage() -> Option<DeviceId> {
    let status = disk::disk_status().ok().flatten()?;
    Some(DEVICES.lock().register_or_update(
        DeviceInfo {
            name: format!(
                "VirtIO Block Device {} MB",
                status.disk_size / (1024 * 1024)
            ),
            category: DeviceCategory::Storage,
            bus: BusLocation::Pci {
                bus: status.bus,
                device: status.device,
                function: status.function,
            },
            vendor_id: 0x1AF4,
            product_id: 0x1001,
            capabilities: DeviceCapabilities::Storage(StorageCapabilities {
                capacity_bytes: status.disk_size,
                sector_size: 512,
                is_removable: false,
                is_readonly: false,
                supports_trim: false,
            }),
        },
        DriverState::Loaded(String::from("virtio-blk")),
        DeviceStatus::Active,
    ))
}
