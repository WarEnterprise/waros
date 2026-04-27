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

pub use xhci::{UsbNetDiagnostics, UsbNetProtocol, UsbNetworkInfo};
pub use xhci::{UsbAttachState, UsbInterfaceStatus, UsbPortVisibility};
pub use xhci::UsbRetiredPortFailure;

use super::device::{
    BusLocation, DeviceCapabilities, DeviceCategory, DeviceId, DeviceInfo, DeviceStatus,
    DriverState, NetworkCapabilities, StorageCapabilities, UsbCapabilities, UsbSpeed,
};

static XHCI_CONTROLLERS: Lazy<Mutex<Vec<xhci::XhciController>>> =
    Lazy::new(|| Mutex::new(Vec::new()));

#[derive(Debug, Clone)]
pub struct UsbDeviceSnapshot {
    pub controller_index: usize,
    pub controller_bus: u8,
    pub controller_device: u8,
    pub controller_function: u8,
    pub port: u8,
    pub connected: bool,
    pub enabled: bool,
    pub addressed: bool,
    pub configured: bool,
    pub speed: UsbSpeed,
    pub slot_id: Option<u8>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub category: DeviceCategory,
    pub driver: &'static str,
    pub name: String,
    pub configuration_count: u8,
    pub active_configuration: Option<u8>,
    pub class_code: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub interface_count: u8,
    pub interfaces: Vec<UsbInterfaceStatus>,
    pub visibility: UsbPortVisibility,
    pub attach_state: UsbAttachState,
    pub attach_reason: &'static str,
    pub enumeration_note: &'static str,
    pub network: Option<UsbNetworkInfo>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbRuntimeControllerSnapshot {
    pub controller_index: usize,
    pub controller_bus: u8,
    pub controller_device: u8,
    pub controller_function: u8,
    pub pending_topology: bool,
    pub last_runtime_event: &'static str,
    pub last_runtime_event_port: Option<u8>,
    pub last_runtime_event_ms: u64,
    pub last_topology_service_result: &'static str,
    pub last_topology_service_ms: u64,
    pub last_inventory_sync_ms: u64,
    pub last_inventory_sync_devices: usize,
    pub cached_ports: usize,
    pub cached_connected_devices: usize,
    pub active_attempt_port: Option<u8>,
    pub progress: xhci::UsbRuntimePortProgress,
    pub last_enumeration_failure_port: Option<u8>,
    pub last_enumeration_failure_reason: &'static str,
    pub stale_failed_ports: [Option<UsbRetiredPortFailure>; 4],
}

#[derive(Debug, Clone)]
pub struct UsbRuntimeSnapshot {
    pub controllers: usize,
    pub pending_topology_controllers: usize,
    pub cached_devices: usize,
    pub controllers_detail: Vec<UsbRuntimeControllerSnapshot>,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbRuntimeCatchUpReport {
    pub controllers: usize,
    pub pending_topology_before: usize,
    pub pending_topology_after: usize,
    pub rescanned_controllers: usize,
    pub cached_devices: usize,
}

pub fn poll() {
    let Some(mut controllers) = XHCI_CONTROLLERS.try_lock() else {
        return;
    };
    for controller in controllers.iter_mut() {
        if controller.poll() {
            sync_controller_inventory(controller);
        }
    }
}

pub fn poll_runtime() {
    let Some(mut controllers) = XHCI_CONTROLLERS.try_lock() else {
        return;
    };
    for controller in controllers.iter_mut() {
        let _ = controller.poll_runtime();
    }
}

/// Service deferred xHCI topology rescans outside the input hot path.
/// Returns how many controllers actually rescanned ports.
pub fn service_pending_topology() -> usize {
    let Some(mut controllers) = XHCI_CONTROLLERS.try_lock() else {
        return 0;
    };
    runtime_catch_up_locked(&mut controllers, false).rescanned_controllers
}

pub fn rescan_topology() -> usize {
    let Some(mut controllers) = XHCI_CONTROLLERS.try_lock() else {
        return 0;
    };
    let mut rescanned = 0usize;
    for controller in controllers.iter_mut() {
        if controller.scan_ports() {
            sync_controller_inventory(controller);
            rescanned = rescanned.saturating_add(1);
        }
    }
    rescanned
}

#[must_use]
pub fn runtime_catch_up() -> UsbRuntimeCatchUpReport {
    let Some(mut controllers) = XHCI_CONTROLLERS.try_lock() else {
        return UsbRuntimeCatchUpReport {
            controllers: 0,
            pending_topology_before: 0,
            pending_topology_after: 0,
            rescanned_controllers: 0,
            cached_devices: 0,
        };
    };
    runtime_catch_up_locked(&mut controllers, true)
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

        if let Some(mut controller) = controller_to_store {
            controller.controller_device_id = Some(controller_id);
            sync_controller_inventory(&mut controller);
            XHCI_CONTROLLERS.lock().push(controller);
        } else {
            sync_detected_ports(controller_id, &detected_ports);
        }

        count += 1;
    }

    count
}

// ================================================================
// USB Network Interface access (called from net subsystem)
// ================================================================

/// Find the first active USB network device across all controllers.
/// Returns (controller_index, slot_id, info).
pub fn find_usb_nic() -> Option<UsbNetworkInfo> {
    let snapshot = best_usb_network_ready()?;
    let mut info = snapshot.network?;
    info.controller_index = snapshot.controller_index;
    Some(info)
}

#[must_use]
pub fn device_snapshots() -> Vec<UsbDeviceSnapshot> {
    let controllers = XHCI_CONTROLLERS.lock();
    let mut snapshots = Vec::new();
    for (controller_index, controller) in controllers.iter().enumerate() {
        for port in &controller.ports {
            snapshots.push(UsbDeviceSnapshot {
                controller_index,
                controller_bus: controller.pci.bus,
                controller_device: controller.pci.device,
                controller_function: controller.pci.function,
                port: port.port,
                connected: port.connected,
                enabled: port.enabled,
                addressed: port.addressed,
                configured: port.configured,
                speed: port.speed,
                slot_id: port.slot_id,
                vendor_id: port.vendor_id,
                product_id: port.product_id,
                category: port.category,
                driver: port.driver,
                name: port.name.clone(),
                configuration_count: port.configuration_count,
                active_configuration: port.active_configuration,
                class_code: port.class_code,
                subclass: port.subclass,
                protocol: port.protocol,
                interface_count: port.interface_count,
                interfaces: port.interfaces.clone(),
                visibility: port.visibility,
                attach_state: port.attach_state,
                attach_reason: port.attach_reason,
                enumeration_note: port.enumeration_note,
                network: port.network,
            });
        }
    }
    snapshots
}

#[must_use]
pub fn runtime_snapshot() -> UsbRuntimeSnapshot {
    let mut controllers = XHCI_CONTROLLERS.lock();
    let mut detail = Vec::new();
    let mut pending = 0usize;
    let mut cached_devices = 0usize;

    for (controller_index, controller) in controllers.iter_mut().enumerate() {
        controller.prune_stale_failed_ports();
        let status = controller.runtime_status();
        if status.pending_topology {
            pending = pending.saturating_add(1);
        }
        cached_devices = cached_devices.saturating_add(status.cached_connected_devices);
        detail.push(UsbRuntimeControllerSnapshot {
            controller_index,
            controller_bus: controller.pci.bus,
            controller_device: controller.pci.device,
            controller_function: controller.pci.function,
            pending_topology: status.pending_topology,
            last_runtime_event: status.last_runtime_event,
            last_runtime_event_port: status.last_runtime_event_port,
            last_runtime_event_ms: status.last_runtime_event_ms,
            last_topology_service_result: status.last_topology_service_result,
            last_topology_service_ms: status.last_topology_service_ms,
            last_inventory_sync_ms: status.last_inventory_sync_ms,
            last_inventory_sync_devices: status.last_inventory_sync_devices,
            cached_ports: status.cached_ports,
            cached_connected_devices: status.cached_connected_devices,
            active_attempt_port: status.active_attempt_port,
            progress: status.progress,
            last_enumeration_failure_port: status.last_enumeration_failure_port,
            last_enumeration_failure_reason: status.last_enumeration_failure_reason,
            stale_failed_ports: status.stale_failed_ports,
        });
    }

    UsbRuntimeSnapshot {
        controllers: detail.len(),
        pending_topology_controllers: pending,
        cached_devices,
        controllers_detail: detail,
    }
}

#[must_use]
pub fn best_usb_network_candidate() -> Option<UsbDeviceSnapshot> {
    fn rank(snapshot: &UsbDeviceSnapshot) -> (u8, u8, u8, u8, u8, usize, usize, usize) {
        let attach_rank = match snapshot.attach_state {
            UsbAttachState::Ready => 5,
            UsbAttachState::Candidate => 4,
            UsbAttachState::Unsupported => match snapshot.visibility {
                UsbPortVisibility::NetworkCandidate => 3,
                UsbPortVisibility::PhoneMedia | UsbPortVisibility::PhoneAdb => 2,
                UsbPortVisibility::VendorSpecific => 1,
                _ => 0,
            },
            UsbAttachState::NotApplicable => match snapshot.visibility {
                UsbPortVisibility::PhoneMedia | UsbPortVisibility::PhoneAdb => 1,
                _ => 0,
            },
        };
        (
            attach_rank,
            u8::from(snapshot.network.is_some()),
            u8::from(snapshot.configured),
            u8::from(snapshot.connected),
            u8::from(snapshot.slot_id.is_some()),
            snapshot.interfaces.len(),
            usize::from(snapshot.controller_index),
            usize::from(snapshot.port),
        )
    }

    device_snapshots()
        .into_iter()
        .filter(|snapshot| rank(snapshot).0 != 0)
        .max_by_key(|snapshot| rank(snapshot))
}

#[must_use]
pub fn best_usb_network_ready() -> Option<UsbDeviceSnapshot> {
    device_snapshots()
        .into_iter()
        .filter(|snapshot| {
            snapshot.connected
                && snapshot.configured
                && snapshot.attach_state == UsbAttachState::Ready
                && snapshot.network.is_some()
        })
        .max_by_key(|snapshot| {
            (
                u8::from(snapshot.network.is_some()),
                u8::from(snapshot.configured),
                snapshot.interfaces.len(),
                usize::from(snapshot.controller_index),
                usize::from(snapshot.port),
            )
        })
}

/// Send an Ethernet frame via USB NIC.
pub fn usb_net_send(controller_index: usize, slot_id: u8, frame: &[u8]) -> Result<(), &'static str> {
    let mut controllers = XHCI_CONTROLLERS.lock();
    let controller = controllers
        .get_mut(controller_index)
        .ok_or("USB controller not found")?;
    controller.network_send_frame(slot_id, frame)
}

/// Receive an Ethernet frame via USB NIC (non-blocking).
pub fn usb_net_recv(controller_index: usize, slot_id: u8) -> Option<Vec<u8>> {
    let mut controllers = XHCI_CONTROLLERS.lock();
    let controller = controllers.get_mut(controller_index)?;
    controller.network_recv_frame(slot_id)
}

/// Get USB NIC diagnostics.
pub fn usb_net_diagnostics(controller_index: usize, slot_id: u8) -> Option<UsbNetDiagnostics> {
    let controllers = XHCI_CONTROLLERS.lock();
    let controller = controllers.get(controller_index)?;
    controller.network_diagnostics(slot_id)
}

/// Number of xHCI controllers currently tracked (for diagnostic display).
#[must_use]
pub fn controller_count() -> usize {
    XHCI_CONTROLLERS.lock().len()
}

/// Count of armed HID keyboard endpoints across all controllers.
#[must_use]
pub fn hid_keyboard_count() -> (usize, usize) {
    let controllers = XHCI_CONTROLLERS.lock();
    let mut total = 0usize;
    let mut armed = 0usize;
    for controller in controllers.iter() {
        for slot in &controller.slots {
            if let Some(slot) = slot {
                if let Some(ref hid) = slot.hid {
                    if matches!(hid.hid_kind, hid::HidKind::Keyboard) {
                        total += 1;
                        if hid.in_flight_trb.is_some() {
                            armed += 1;
                        }
                    }
                }
            }
        }
    }
    (total, armed)
}

pub fn full_diagnostics() -> Vec<String> {
    let controllers = XHCI_CONTROLLERS.lock();
    if controllers.is_empty() {
        return alloc::vec![String::from("no xHCI controllers tracked")];
    }

    let mut lines = Vec::new();
    for (index, controller) in controllers.iter().enumerate() {
        lines.push(format!("controller[{index}]"));
        lines.extend(controller.diagnostic_report());
    }
    lines
}

fn runtime_catch_up_locked(
    controllers: &mut [xhci::XhciController],
    force_due: bool,
) -> UsbRuntimeCatchUpReport {
    let pending_topology_before = controllers
        .iter()
        .filter(|controller| controller.runtime_status().pending_topology)
        .count();
    let mut rescanned_controllers = 0usize;
    for controller in controllers.iter_mut() {
        if controller.service_runtime_topology(force_due, xhci::runtime_catch_up_budget_ms()) {
            sync_controller_inventory(controller);
            rescanned_controllers = rescanned_controllers.saturating_add(1);
        }
    }
    let pending_topology_after = controllers
        .iter()
        .filter(|controller| controller.runtime_status().pending_topology)
        .count();
    let cached_devices = controllers
        .iter()
        .map(|controller| controller.runtime_status().cached_connected_devices)
        .sum();

    UsbRuntimeCatchUpReport {
        controllers: controllers.len(),
        pending_topology_before,
        pending_topology_after,
        rescanned_controllers,
        cached_devices,
    }
}

fn sync_controller_inventory(controller: &mut xhci::XhciController) {
    let Some(controller_id) = controller.controller_device_id else {
        return;
    };
    DEVICES.lock().mark_usb_children_removed(controller_id);
    sync_detected_ports(controller_id, &controller.ports);
    controller.note_inventory_sync();
}

fn sync_detected_ports(controller_id: DeviceId, detected_ports: &[xhci::UsbPortStatus]) {
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
            DeviceCategory::Network => {
                let mac_address = port.network.map(|info| info.mac).unwrap_or([0; 6]);
                DeviceCapabilities::Network(NetworkCapabilities {
                    max_speed_mbps: match port.speed {
                        UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2 => 1000,
                        UsbSpeed::High => 480,
                        UsbSpeed::Full => 12,
                        UsbSpeed::Low => 1,
                    },
                    has_wifi: false,
                    has_ethernet: true,
                    mac_address,
                    pq_tls_offload: false,
                })
            }
            _ => DeviceCapabilities::Usb(UsbCapabilities {
                speed: port.speed,
                class: port.class_code,
                subclass: port.subclass,
                protocol: port.protocol,
                max_packet_size: 0,
                num_endpoints: port.interface_count,
            }),
        };

        DEVICES.lock().register_or_update(
            DeviceInfo {
                name: port.name.clone(),
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
            if !port.connected {
                DeviceStatus::Removed
            } else if port.configured {
                DeviceStatus::Active
            } else {
                DeviceStatus::Initialized
            },
        );
    }
}
