use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

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
    registry.register_or_update(
        DeviceInfo {
            name: String::from("Bootstrap Processor"),
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
                framebuffer_width, framebuffer_height
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
