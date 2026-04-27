use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::x86_64::__cpuid;

use spin::{Lazy, Mutex};

pub mod acpi;
pub mod bus;
pub mod device;
pub mod display;
pub mod input;
pub mod net;
pub mod storage;
pub mod traits;
pub mod usb;

pub use device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, HardwareDevice, QuantumCapabilities,
};

pub static DEVICES: Lazy<Mutex<DeviceRegistry>> = Lazy::new(|| Mutex::new(DeviceRegistry::new()));

pub struct DeviceRegistry {
    devices: Vec<HardwareDevice>,
    next_id: u32,
}

impl DeviceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            devices: Vec::new(),
            next_id: 1,
        }
    }

    pub fn reset(&mut self) {
        self.devices.clear();
        self.next_id = 1;
    }

    pub fn register_or_update(
        &mut self,
        info: DeviceInfo,
        driver: DriverState,
        status: DeviceStatus,
    ) -> DeviceId {
        if let Some(device) = self
            .devices
            .iter_mut()
            .find(|device| same_identity(&device.info, &info))
        {
            device.info = info;
            device.driver = driver;
            device.status = status;
            return device.id;
        }

        let id = DeviceId(self.next_id);
        self.next_id += 1;
        self.devices.push(HardwareDevice {
            id,
            info,
            driver,
            status,
        });
        id
    }

    pub fn update_capabilities(&mut self, id: DeviceId, capabilities: DeviceCapabilities) {
        if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
            device.info.capabilities = capabilities;
        }
    }

    pub fn update_info(&mut self, id: DeviceId, info: DeviceInfo) {
        if let Some(device) = self.devices.iter_mut().find(|device| device.id == id) {
            device.info = info;
        }
    }

    pub fn mark_usb_children_removed(&mut self, controller: DeviceId) {
        for device in self.devices.iter_mut() {
            if matches!(
                device.info.bus,
                BusLocation::Usb {
                    controller: parent,
                    ..
                } if parent == controller
            ) {
                device.status = DeviceStatus::Removed;
            }
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> Vec<HardwareDevice> {
        let mut devices = self.devices.clone();
        devices.sort_by_key(|device| device.id);
        devices
    }
}

pub fn init_registry() {
    DEVICES.lock().reset();
}

pub fn register_core_devices(framebuffer_width: u32, framebuffer_height: u32) {
    let mut registry = DEVICES.lock();
    let processor_name = cpu_device_name();
    registry.register_or_update(
        DeviceInfo {
            name: processor_name,
            category: DeviceCategory::Processor,
            bus: BusLocation::Platform,
            vendor_id: 0,
            product_id: 0,
            capabilities: DeviceCapabilities::None,
        },
        DriverState::Loaded(String::from("cpu-core")),
        DeviceStatus::Active,
    );
    registry.register_or_update(
        DeviceInfo {
            name: alloc::format!(
                "System Memory ({} MiB visible)",
                (crate::memory::stats().total_frames * 4) / 1024
            ),
            category: DeviceCategory::Memory,
            bus: BusLocation::Platform,
            vendor_id: 0,
            product_id: 0,
            capabilities: DeviceCapabilities::None,
        },
        DriverState::Loaded(String::from("memory-core")),
        DeviceStatus::Active,
    );
    registry.register_or_update(
        DeviceInfo {
            name: alloc::format!(
                "Quantum Simulator ({}x{} console session)",
                framebuffer_width,
                framebuffer_height
            ),
            category: DeviceCategory::QuantumProcessor,
            bus: BusLocation::Virtual,
            vendor_id: 0,
            product_id: 0,
            capabilities: DeviceCapabilities::Quantum(QuantumCapabilities {
                num_qubits: crate::quantum::state::MAX_KERNEL_QUBITS,
                native_gates: vec![
                    String::from("h"),
                    String::from("x"),
                    String::from("y"),
                    String::from("z"),
                    String::from("s"),
                    String::from("t"),
                    String::from("cx"),
                    String::from("cz"),
                    String::from("swap"),
                    String::from("rx"),
                    String::from("ry"),
                    String::from("rz"),
                    String::from("ccx"),
                ],
                connectivity: Vec::new(),
                is_simulator: true,
                coherence_time_us: None,
            }),
        },
        DriverState::Loaded(String::from("sim-statevec")),
        DeviceStatus::Active,
    );
}

#[must_use]
pub fn devices() -> Vec<HardwareDevice> {
    DEVICES.lock().snapshot()
}

fn same_identity(left: &DeviceInfo, right: &DeviceInfo) -> bool {
    left.bus == right.bus
        && left.vendor_id == right.vendor_id
        && left.product_id == right.product_id
        && left.category == right.category
}

fn cpu_device_name() -> String {
    let vendor_leaf = __cpuid(0);
    let vendor_bytes = vendor_string_bytes(vendor_leaf.ebx, vendor_leaf.edx, vendor_leaf.ecx);
    if let Some(brand) = cpu_brand_string() {
        alloc::format!("Bootstrap Processor ({})", brand)
    } else {
        let vendor = core::str::from_utf8(&vendor_bytes).unwrap_or("Unknown CPU");
        alloc::format!("Bootstrap Processor ({})", vendor)
    }
}

fn cpu_brand_string() -> Option<String> {
    let max_extended_leaf = __cpuid(0x8000_0000).eax;
    if max_extended_leaf < 0x8000_0004 {
        return None;
    }

    let mut bytes = [0u8; 48];
    for (index, leaf) in (0x8000_0002..=0x8000_0004).enumerate() {
        let result = __cpuid(leaf);
        let offset = index * 16;
        bytes[offset..offset + 4].copy_from_slice(&result.eax.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&result.ebx.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&result.ecx.to_le_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&result.edx.to_le_bytes());
    }

    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let brand = core::str::from_utf8(&bytes[..end]).ok()?.trim();
    if brand.is_empty() {
        None
    } else {
        Some(String::from(brand))
    }
}

fn vendor_string_bytes(ebx: u32, edx: u32, ecx: u32) -> [u8; 12] {
    let ebx = ebx.to_le_bytes();
    let edx = edx.to_le_bytes();
    let ecx = ecx.to_le_bytes();
    [
        ebx[0], ebx[1], ebx[2], ebx[3], edx[0], edx[1], edx[2], edx[3], ecx[0], ecx[1], ecx[2],
        ecx[3],
    ]
}
