use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use smoltcp::iface::{Config as InterfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::socket::dhcpv4;
use smoltcp::time::{Duration, Instant};
use smoltcp::wire::{
    DhcpMessageType, DhcpPacket, DhcpRepr, EthernetAddress,
    EthernetFrame as SmolEthernetFrame, EthernetProtocol, HardwareAddress, IpCidr, IpProtocol,
    Ipv4Packet, UdpPacket, DHCP_CLIENT_PORT, DHCP_SERVER_PORT,
};
use spin::{Lazy, Mutex};

use crate::arch::x86_64::{interrupts, pit};
use crate::hal::net::e1000::{E1000Diagnostics, E1000};
use crate::hal::net::nic_select;
use crate::hal::net::rtl8169::{Rtl8169, Rtl8169Diagnostics};
use crate::hal::usb::{
    self as hal_usb, UsbAttachState, UsbDeviceSnapshot, UsbNetDiagnostics, UsbNetProtocol,
    UsbNetworkInfo,
};

const AUTO_ATTACH_USB_NIC_DURING_BOOT: bool = false;
pub const DEFAULT_DHCP_TIMEOUT_MS: u64 = 15_000;
const BOOT_DHCP_TIMEOUT_MS: u64 = 5_000;
const DHCP_DISCOVER_RETRY_MS: u64 = 2_000;
const DHCP_REQUEST_RETRY_MS: u64 = 2_000;
const DHCP_REQUEST_RETRIES: u16 = 4;
const DHCP_POLL_BURST: usize = 4;
const USB_ATTACH_LEASE_WARMUP_MS: u64 = 1_500;
const USB_ATTACH_AUTO_DHCP_TIMEOUT_MS: u64 = 10_000;
const AUTO_DHCP_RUNTIME_SLICE_MS: u64 = 100;
const READ_ONLY_MAINTENANCE_TIMEOUT_MS: u64 = 750;
const READ_ONLY_MAINTENANCE_COOLDOWN_MS: u64 = 250;

pub mod arp;
pub mod buffer;
pub mod dhcp;
pub mod dns;
pub mod ethernet;
pub mod http;
pub mod ibm;
pub mod icmp;
pub mod ipv4;
pub mod pci;
pub mod serial;
pub mod tcp;
pub mod tls;
pub mod udp;
pub mod virtio;

pub use dhcp::DhcpConfig;
pub use pci::PciDevice;
pub use serial::{Message, MessageType};
pub use virtio::net::VirtioNetDiagnostics;

/// Errors surfaced by the in-kernel networking stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    PayloadTooLarge,
    InvalidFrame,
    NotInitialized,
    FrameTooShort,
    UnsupportedDevice(&'static str),
    InitializationFailed(&'static str),
    OutOfMemory,
    QueueFull,
    NoHardware,
    ProtocolError(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge => formatter.write_str("payload exceeds supported frame size"),
            Self::InvalidFrame => formatter.write_str("invalid network frame"),
            Self::NotInitialized => formatter.write_str("network subsystem not initialized"),
            Self::FrameTooShort => formatter.write_str("frame is shorter than the protocol header"),
            Self::UnsupportedDevice(reason) => {
                write!(formatter, "unsupported network device: {reason}")
            }
            Self::InitializationFailed(reason) => {
                write!(formatter, "network initialization failed: {reason}")
            }
            Self::OutOfMemory => formatter.write_str("network DMA allocation failed"),
            Self::QueueFull => {
                formatter.write_str("network transmit queue has no free descriptors")
            }
            Self::NoHardware => formatter.write_str("no hardware network interface is available"),
            Self::ProtocolError(message) => formatter.write_str(message),
        }
    }
}

/// Snapshot of the kernel's networking initialization.
#[derive(Debug, Clone)]
pub struct NetInitReport {
    pub pci_devices: usize,
    pub serial_status: &'static str,
    pub hardware: Option<NetworkDeviceInfo>,
    pub network_config: Option<DhcpConfig>,
}

#[derive(Debug, Clone, Copy)]
pub enum NetworkTransport {
    Io(u16),
    Mmio(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Up,
    Down,
    Unknown,
}

/// Read-only summary of the active network device.
#[derive(Debug, Clone)]
pub struct NetworkDeviceInfo {
    pub name: &'static str,
    pub driver: &'static str,
    pub mac: [u8; 6],
    pub transport: NetworkTransport,
    pub rx_queue_size: u16,
    pub tx_queue_size: u16,
    pub interrupt_line: u8,
    pub pending_frames: usize,
    pub rx_frames: u64,
    pub tx_frames: u64,
    pub link_speed_mbps: u32,
    pub link_state: LinkState,
    pub full_duplex: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum NetworkDiagnostics {
    Virtio(VirtioNetDiagnostics),
    E1000(E1000Diagnostics),
    Rtl8169(Rtl8169Diagnostics),
    UsbNet(UsbNetDiagnostics),
}

#[derive(Debug, Clone, Copy)]
pub struct DhcpAttemptReport {
    pub state: &'static str,
    pub timeout_ms: u64,
    pub started_ms: u64,
    pub finished_ms: u64,
    pub polls: u32,
    pub interface: &'static str,
    pub discover_sent: u32,
    pub offer_received: u32,
    pub request_sent: u32,
    pub ack_received: u32,
    pub nak_received: u32,
    pub last_event: &'static str,
    pub tx_frames_delta: u64,
    pub rx_frames_delta: u64,
    pub rx_eth_frames: u32,
    pub rx_ipv4_frames: u32,
    pub rx_udp_frames: u32,
    pub rx_udp_67_68: u32,
    pub rx_dhcp_frames: u32,
    pub rx_dhcp_parse_errors: u32,
    pub dhcp_drop_reason: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DhcpOwner {
    None,
    AutoPump,
    ManualCmd,
}

#[derive(Debug, Clone, Copy)]
pub struct NetworkMaintenanceReport {
    pub last_poll_ms: u64,
    pub ms_since_poll: u64,
    pub last_dhcp_event_ms: u64,
    pub ms_since_dhcp_event: u64,
    pub canonical_lease_state: &'static str,
    pub dhcp_socket_ready: bool,
    pub iface_ready: bool,
    pub active_interface: &'static str,
    pub active_driver: &'static str,
    pub active_is_usb: bool,
    pub lease_view: &'static str,
    pub dhcp_last_wire_event: &'static str,
    pub dhcp_owner: &'static str,
    pub dhcp_owner_trigger: &'static str,
    pub dhcp_generation: u64,
    pub dhcp_owner_started_ms: u64,
    pub dhcp_owner_deadline_ms: u64,
    pub dhcp_owner_elapsed_ms: u64,
    pub dhcp_owner_remaining_ms: u64,
    pub dhcp_auto_pump_active: bool,
    pub dhcp_manual_active: bool,
    pub dhcp_auto_pump_trigger: &'static str,
    pub dhcp_auto_pump_state: &'static str,
    pub dhcp_auto_pump_waiting_for: &'static str,
    pub dhcp_auto_pump_started_ms: u64,
    pub dhcp_auto_pump_deadline_ms: u64,
    pub dhcp_auto_pump_last_service_ms: u64,
    pub dhcp_auto_pump_polls: u32,
    pub dhcp_auto_pump_elapsed_ms: u64,
    pub dhcp_auto_pump_remaining_ms: u64,
    pub dhcp_manual_preempted_auto: bool,
    pub dhcp_auto_timed_out_before_manual: bool,
    pub dhcp_last_auto_state: &'static str,
    pub dhcp_stale_worker_ignored: bool,
    pub dhcp_stale_worker_ignored_count: u32,
    pub read_only_trigger: &'static str,
    pub read_only_state: &'static str,
    pub read_only_started_ms: u64,
    pub read_only_finished_ms: u64,
    pub read_only_polls: u32,
}

impl DhcpAttemptReport {
    const fn idle() -> Self {
        Self {
            state: "idle",
            timeout_ms: 0,
            started_ms: 0,
            finished_ms: 0,
            polls: 0,
            interface: "offline",
            discover_sent: 0,
            offer_received: 0,
            request_sent: 0,
            ack_received: 0,
            nak_received: 0,
            last_event: "none",
            tx_frames_delta: 0,
            rx_frames_delta: 0,
            rx_eth_frames: 0,
            rx_ipv4_frames: 0,
            rx_udp_frames: 0,
            rx_udp_67_68: 0,
            rx_dhcp_frames: 0,
            rx_dhcp_parse_errors: 0,
            dhcp_drop_reason: "none",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ReadOnlyMaintenanceReport {
    trigger: &'static str,
    state: &'static str,
    started_ms: u64,
    finished_ms: u64,
    polls: u32,
}

impl ReadOnlyMaintenanceReport {
    const fn idle() -> Self {
        Self {
            trigger: "none",
            state: "idle",
            started_ms: 0,
            finished_ms: 0,
            polls: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DhcpAutoPump {
    active: bool,
    generation: u64,
    trigger: &'static str,
    state: &'static str,
    waiting_for: &'static str,
    started_ms: u64,
    deadline_ms: u64,
    finished_ms: u64,
    last_service_ms: u64,
    polls: u32,
    timeout_ms: u64,
    interface: &'static str,
    tx_start: u64,
    rx_start: u64,
}

impl DhcpAutoPump {
    const fn idle() -> Self {
        Self {
            active: false,
            generation: 0,
            trigger: "none",
            state: "idle",
            waiting_for: "none",
            started_ms: 0,
            deadline_ms: 0,
            finished_ms: 0,
            last_service_ms: 0,
            polls: 0,
            timeout_ms: 0,
            interface: "offline",
            tx_start: 0,
            rx_start: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DhcpWireTrace {
    discover_sent: u32,
    offer_received: u32,
    request_sent: u32,
    ack_received: u32,
    nak_received: u32,
    last_event: &'static str,
    rx_eth_frames: u32,
    rx_ipv4_frames: u32,
    rx_udp_frames: u32,
    rx_udp_67_68: u32,
    rx_dhcp_frames: u32,
    rx_dhcp_parse_errors: u32,
    dhcp_drop_reason: &'static str,
}

impl DhcpWireTrace {
    const fn new() -> Self {
        Self {
            discover_sent: 0,
            offer_received: 0,
            request_sent: 0,
            ack_received: 0,
            nak_received: 0,
            last_event: "none",
            rx_eth_frames: 0,
            rx_ipv4_frames: 0,
            rx_udp_frames: 0,
            rx_udp_67_68: 0,
            rx_dhcp_frames: 0,
            rx_dhcp_parse_errors: 0,
            dhcp_drop_reason: "none",
        }
    }

    fn timeout_state(self) -> &'static str {
        if self.ack_received > 0 {
            "timeout-after-ack"
        } else if self.request_sent > 0 {
            "timeout-no-ack"
        } else if self.offer_received > 0 {
            "timeout-offer-no-request"
        } else if self.discover_sent > 0 {
            "timeout-no-offer"
        } else {
            "timeout-no-discover"
        }
    }
}

static DHCP_WIRE_TRACE: Lazy<Mutex<DhcpWireTrace>> = Lazy::new(|| Mutex::new(DhcpWireTrace::new()));

#[derive(Debug, Clone)]
pub struct UsbTetheringSnapshot {
    pub detected: Option<UsbNetworkInfo>,
    pub active: Option<UsbNetworkInfo>,
    pub candidate: Option<UsbDeviceSnapshot>,
    pub active_matches_detected: bool,
    pub last_event: &'static str,
}

fn format_usb_attach_failure(candidate: &UsbDeviceSnapshot) -> String {
    let vendor_id = candidate.vendor_id.unwrap_or(0);
    let product_id = candidate.product_id.unwrap_or(0);
    let slot = candidate.slot_id.unwrap_or(0);
    format!(
        "USB device visible on ctrl={} port={} slot={} ({:04X}:{:04X}) but not attachable: {}",
        candidate.controller_index,
        candidate.port,
        slot,
        vendor_id,
        product_id,
        candidate.attach_reason
    )
}

/// USB NIC backed by xHCI bulk transfers.
struct UsbNic {
    controller_index: usize,
    slot_id: u8,
    protocol: UsbNetProtocol,
    mac: [u8; 6],
    max_frame_size: u16,
}

impl UsbNic {
    fn info(&self) -> NetworkDeviceInfo {
        let diag = hal_usb::usb_net_diagnostics(self.controller_index, self.slot_id);
        NetworkDeviceInfo {
            name: match self.protocol {
                UsbNetProtocol::Rndis => "USB Network (RNDIS)",
                UsbNetProtocol::CdcEcm => "USB Network (CDC-ECM)",
                UsbNetProtocol::CdcNcm => "USB Network (CDC-NCM)",
            },
            driver: "usb-net",
            mac: self.mac,
            transport: NetworkTransport::Io(0),
            rx_queue_size: 64,
            tx_queue_size: 64,
            interrupt_line: 0,
            pending_frames: diag.map_or(0, |d| d.rx_queue_depth),
            rx_frames: diag.map_or(0, |d| d.rx_frames),
            tx_frames: diag.map_or(0, |d| d.tx_frames),
            link_speed_mbps: 100, // USB tethering typically 100Mbps effective
            link_state: LinkState::Up, // USB NIC is up if configured
            full_duplex: true,
        }
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        hal_usb::usb_net_send(self.controller_index, self.slot_id, frame)
            .map_err(|e| NetError::ProtocolError(alloc::string::String::from(e)))
    }

    fn recv_frame(&mut self) -> Option<Vec<u8>> {
        hal_usb::usb_net_recv(self.controller_index, self.slot_id)
    }

    fn diagnostics(&self) -> UsbNetDiagnostics {
        hal_usb::usb_net_diagnostics(self.controller_index, self.slot_id)
            .unwrap_or(UsbNetDiagnostics {
                protocol: self.protocol,
                mac: self.mac,
                rx_raw_events: 0,
                rx_frames: 0,
                tx_frames: 0,
                tx_errors: 0,
                rx_errors: 0,
                rx_extract_errors: 0,
                rx_short_frames: 0,
                rx_last_raw_len: 0,
                rx_last_frame_len: 0,
                rx_last_drop: "diagnostics-unavailable",
                rx_queue_depth: 0,
                rx_armed: false,
            })
    }
}

enum ActiveNic {
    Virtio(virtio::net::VirtioNet),
    E1000(E1000),
    Rtl8169(Rtl8169),
    UsbNet(UsbNic),
}

impl ActiveNic {
    fn info(&self) -> NetworkDeviceInfo {
        match self {
            Self::Virtio(device) => device.info(),
            Self::E1000(device) => device.info(),
            Self::Rtl8169(device) => device.info(),
            Self::UsbNet(device) => device.info(),
        }
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        match self {
            Self::Virtio(device) => device.send_frame(frame),
            Self::E1000(device) => device.send_frame(frame),
            Self::Rtl8169(device) => device.send_frame(frame),
            Self::UsbNet(device) => device.send_frame(frame),
        }
    }

    fn recv_frame(&mut self) -> Option<Vec<u8>> {
        match self {
            Self::Virtio(device) => device.recv_frame(),
            Self::E1000(device) => device.recv_frame(),
            Self::Rtl8169(device) => device.recv_frame(),
            Self::UsbNet(device) => device.recv_frame(),
        }
    }

    fn diagnostics(&self) -> NetworkDiagnostics {
        match self {
            Self::Virtio(device) => NetworkDiagnostics::Virtio(device.diagnostics()),
            Self::E1000(device) => NetworkDiagnostics::E1000(device.diagnostics()),
            Self::Rtl8169(device) => NetworkDiagnostics::Rtl8169(device.diagnostics()),
            Self::UsbNet(device) => NetworkDiagnostics::UsbNet(device.diagnostics()),
        }
    }

    fn poll(&mut self) {
        match self {
            Self::Virtio(_) | Self::E1000(_) | Self::UsbNet(_) => {}
            Self::Rtl8169(device) => device.poll(),
        }
    }
}

enum DhcpEventSnapshot {
    Configured(DhcpConfig),
    Deconfigured,
}

pub static NET: Lazy<Mutex<NetworkSubsystem>> =
    Lazy::new(|| Mutex::new(NetworkSubsystem::new(serial::COM2_PORT)));

/// Combined legacy serial transport plus the new PCI/virtio/smoltcp path.
pub struct NetworkSubsystem {
    serial: serial::NetInterface,
    pci_devices: Vec<PciDevice>,
    hardware: Option<ActiveNic>,
    iface: Option<Interface>,
    sockets: SocketSet<'static>,
    dhcp_handle: Option<SocketHandle>,
    network_config: Option<DhcpConfig>,
    dns_resolver: dns::DnsResolver,
    arp_cache: arp::ArpCache,
    next_local_port: u16,
    dhcp_last_attempt: DhcpAttemptReport,
    dhcp_owner: DhcpOwner,
    dhcp_generation: u64,
    dhcp_owner_started_ms: u64,
    dhcp_owner_deadline_ms: u64,
    dhcp_owner_trigger: &'static str,
    dhcp_auto_pump: DhcpAutoPump,
    dhcp_manual_preempted_auto: bool,
    dhcp_auto_timed_out_before_manual: bool,
    dhcp_last_auto_state: &'static str,
    dhcp_stale_worker_ignored: bool,
    dhcp_stale_worker_ignored_count: u32,
    last_network_poll_ms: u64,
    last_dhcp_event_ms: u64,
    read_only_maintenance: ReadOnlyMaintenanceReport,
    usb_last_event: &'static str,
}

impl NetworkSubsystem {
    #[must_use]
    pub fn new(serial_port: u16) -> Self {
        Self {
            serial: serial::NetInterface::new(serial_port),
            pci_devices: Vec::new(),
            hardware: None,
            iface: None,
            sockets: SocketSet::new(vec![]),
            dhcp_handle: None,
            network_config: None,
            dns_resolver: dns::DnsResolver::new(),
            arp_cache: arp::ArpCache::new(),
            next_local_port: 49_152,
            dhcp_last_attempt: DhcpAttemptReport::idle(),
            dhcp_owner: DhcpOwner::None,
            dhcp_generation: 0,
            dhcp_owner_started_ms: 0,
            dhcp_owner_deadline_ms: 0,
            dhcp_owner_trigger: "none",
            dhcp_auto_pump: DhcpAutoPump::idle(),
            dhcp_manual_preempted_auto: false,
            dhcp_auto_timed_out_before_manual: false,
            dhcp_last_auto_state: "none",
            dhcp_stale_worker_ignored: false,
            dhcp_stale_worker_ignored_count: 0,
            last_network_poll_ms: 0,
            last_dhcp_event_ms: 0,
            read_only_maintenance: ReadOnlyMaintenanceReport::idle(),
            usb_last_event: "idle",
        }
    }

    pub fn init(&mut self) -> Result<NetInitReport, NetError> {
        self.serial.init();
        self.pci_devices = pci::enumerate_pci();

        if self.hardware.is_none() {
            if let Some(device) = self
                .pci_devices
                .iter()
                .copied()
                .find(nic_select::is_rtl8169)
            {
                if let Ok(driver) = Rtl8169::init(device) {
                    self.hardware = Some(ActiveNic::Rtl8169(driver));
                }
            }
        }

        if self.hardware.is_none() {
            if let Some(device) = self.pci_devices.iter().copied().find(nic_select::is_e1000) {
                if let Ok(driver) = E1000::init(device) {
                    self.hardware = Some(ActiveNic::E1000(driver));
                }
            }
        }

        if self.hardware.is_none() {
            if let Some(device) = self
                .pci_devices
                .iter()
                .copied()
                .find(nic_select::is_virtio_net)
            {
                if let Ok(driver) = virtio::net::VirtioNet::init(device) {
                    self.hardware = Some(ActiveNic::Virtio(driver));
                }
            }
        }

        // Keep USB NIC activation explicit by default so input/auth boot paths
        // stay decoupled from tethering/network bring-up on real hardware.
        if self.hardware.is_none() && AUTO_ATTACH_USB_NIC_DURING_BOOT {
            self.probe_usb_nic();
        }

        if self.hardware.is_some() {
            self.reset_runtime_state();
            self.prepare_stack()?;
            let _ = self.acquire_dhcp(BOOT_DHCP_TIMEOUT_MS);
        }

        Ok(self.report())
    }

    fn probe_usb_nic(&mut self) {
        if let Some(info) = hal_usb::find_usb_nic() {
            crate::serial_println!(
                "[net] USB NIC found: {:?} mac={} ctrl={} slot={}",
                info.protocol,
                format_mac(&info.mac),
                info.controller_index,
                info.slot_id
            );
            self.hardware = Some(ActiveNic::UsbNet(UsbNic {
                controller_index: info.controller_index,
                slot_id: info.slot_id,
                protocol: info.protocol,
                mac: info.mac,
                max_frame_size: info.max_frame_size,
            }));
            self.usb_last_event = "detected";
        }
    }

    /// True if a USB NIC (CDC-ECM/NCM/RNDIS) is currently visible from the
    /// xHCI driver and could be promoted to the active interface.
    #[must_use]
    pub fn usb_nic_available(&self) -> bool {
        hal_usb::find_usb_nic().is_some()
    }

    /// True if the active hardware is a USB NIC.
    #[must_use]
    pub fn active_is_usb(&self) -> bool {
        matches!(&self.hardware, Some(ActiveNic::UsbNet(_)))
    }

    fn active_usb_info(&self) -> Option<UsbNetworkInfo> {
        match self.hardware.as_ref() {
            Some(ActiveNic::UsbNet(nic)) => Some(UsbNetworkInfo {
                protocol: nic.protocol,
                mac: nic.mac,
                max_frame_size: nic.max_frame_size,
                controller_index: nic.controller_index,
                slot_id: nic.slot_id,
            }),
            _ => None,
        }
    }

    fn sync_active_usb_nic(&mut self) -> bool {
        let Some(active) = self.active_usb_info() else {
            return false;
        };

        let Some(detected) = hal_usb::find_usb_nic() else {
            self.reset_runtime_state();
            self.hardware = None;
            self.usb_last_event = "detached";
            return true;
        };

        if detected.controller_index == active.controller_index && detected.slot_id == active.slot_id {
            self.usb_last_event = "active";
            return false;
        }

        self.reset_runtime_state();
        self.hardware = None;
        self.usb_last_event = "detected-other";
        true
    }

    fn usb_tethering_snapshot(&mut self) -> UsbTetheringSnapshot {
        let _ = self.sync_active_usb_nic();
        let detected = hal_usb::find_usb_nic();
        let candidate = hal_usb::best_usb_network_candidate();
        let active = self.active_usb_info();
        let runtime = hal_usb::runtime_snapshot();
        let active_matches_detected = match (active, detected) {
            (Some(active), Some(detected)) => {
                active.controller_index == detected.controller_index && active.slot_id == detected.slot_id
            }
            _ => false,
        };
        let last_event = if matches!(self.usb_last_event, "idle" | "snapshot-empty") {
            if runtime.pending_topology_controllers != 0 {
                "topology-pending"
            } else if runtime.cached_devices != 0 {
                "enumerated"
            } else {
                runtime
                    .controllers_detail
                    .iter()
                    .max_by_key(|controller| controller.last_runtime_event_ms)
                    .map(|controller| controller.last_runtime_event)
                    .unwrap_or(self.usb_last_event)
            }
        } else {
            self.usb_last_event
        };
        UsbTetheringSnapshot {
            detected,
            active,
            candidate,
            active_matches_detected,
            last_event,
        }
    }

    /// Manually attach USB NIC, replacing current hardware.
    /// Used by shell `net usb` command.
    pub fn attach_usb_nic(&mut self) -> Result<(), NetError> {
        let info = match hal_usb::find_usb_nic() {
            Some(info) => info,
            None => {
                self.usb_last_event = "runtime-catch-up";
                let catch_up = hal_usb::runtime_catch_up();
                match hal_usb::find_usb_nic() {
                    Some(info) => info,
                    None => {
                        if let Some(candidate) = hal_usb::best_usb_network_candidate() {
                            self.usb_last_event = match candidate.attach_state {
                                UsbAttachState::Ready => "candidate-ready",
                                UsbAttachState::Candidate => "candidate-partial",
                                UsbAttachState::Unsupported => "candidate-unsupported",
                                UsbAttachState::NotApplicable => "snapshot-empty",
                            };
                            return Err(NetError::ProtocolError(format_usb_attach_failure(
                                &candidate,
                            )));
                        }
                        self.usb_last_event = if catch_up.pending_topology_after != 0 {
                            "topology-pending"
                        } else {
                            "snapshot-empty"
                        };
                        return Err(NetError::NoHardware);
                    }
                }
            }
        };
        self.usb_last_event = "detected";
        if matches!(
            self.hardware.as_ref(),
            Some(ActiveNic::UsbNet(current))
                if current.controller_index == info.controller_index
                    && current.slot_id == info.slot_id
        ) {
            if self.iface.is_none() || self.dhcp_handle.is_none() {
                self.reset_runtime_state();
                self.prepare_stack()?;
            }
            self.begin_dhcp_auto_pump("usb-attach", USB_ATTACH_AUTO_DHCP_TIMEOUT_MS);
            let _ = self.service_read_only_maintenance("usb-attach", USB_ATTACH_LEASE_WARMUP_MS);
            self.usb_last_event = "attach-active";
            return Ok(());
        }
        crate::serial_println!(
            "[net] USB NIC attach: {:?} mac={} ctrl={} slot={}",
            info.protocol,
            format_mac(&info.mac),
            info.controller_index,
            info.slot_id
        );
        self.reset_runtime_state();
        self.hardware = Some(ActiveNic::UsbNet(UsbNic {
            controller_index: info.controller_index,
            slot_id: info.slot_id,
            protocol: info.protocol,
            mac: info.mac,
            max_frame_size: info.max_frame_size,
        }));
        self.prepare_stack()?;
        self.begin_dhcp_auto_pump("usb-attach", USB_ATTACH_AUTO_DHCP_TIMEOUT_MS);
        let _ = self.service_read_only_maintenance("usb-attach", USB_ATTACH_LEASE_WARMUP_MS);
        self.usb_last_event = "attach-active";
        Ok(())
    }

    #[must_use]
    pub fn report(&self) -> NetInitReport {
        NetInitReport {
            pci_devices: self.pci_devices.len(),
            serial_status: self.serial.status(),
            hardware: self.hardware.as_ref().map(ActiveNic::info),
            network_config: self.network_config,
        }
    }

    #[must_use]
    pub fn status(&mut self) -> String {
        let _ = self.sync_active_usb_nic();
        if self.network_config.is_some() {
            let _ = self.poll_network();
        } else {
            let _ = self.service_read_only_maintenance(
                "status",
                READ_ONLY_MAINTENANCE_TIMEOUT_MS,
            );
        }
        let canonical_lease_state = self.canonical_ipv4_lease_state();
        let ip = self
            .network_config
            .map(|config| {
                format!(
                    "{} gw {}",
                    config.cidr_string(),
                    config.gateway.unwrap_or(ipv4::Ipv4Addr::ZERO)
                )
            })
            .unwrap_or_else(|| match canonical_lease_state {
                "offline" => String::from("unconfigured"),
                state => String::from(state),
            });

        match self.hardware.as_ref() {
            Some(device) => {
                let info = device.info();
                let location = match info.transport {
                    NetworkTransport::Io(base) => format!("I/O 0x{base:04X}"),
                    NetworkTransport::Mmio(base) => format!("MMIO 0x{base:08X}"),
                };
                let link = match info.link_state {
                    LinkState::Up => format!(
                        "up {} Mbps {} duplex",
                        info.link_speed_mbps,
                        if info.full_duplex { "full" } else { "half" }
                    ),
                    LinkState::Down => String::from("down"),
                    LinkState::Unknown => String::from("unknown"),
                };
                format!(
                    "serial-link={} | {}={} @ {} | link={} | ipv4={}",
                    self.serial.status(),
                    info.driver,
                    format_mac(&info.mac),
                    location,
                    link,
                    ip
                )
            }
            None => format!("serial-link={} | nic=offline", self.serial.status()),
        }
    }

    pub(crate) fn service_runtime_poll(&mut self) -> usize {
        if self.network_config.is_none() && self.dhcp_auto_pump.active {
            return self.drive_dhcp_auto_pump(AUTO_DHCP_RUNTIME_SLICE_MS, true);
        }
        let events = self.poll_network();
        if self.network_config.is_some() && self.dhcp_auto_pump.active {
            self.finish_dhcp_auto_pump("lease-acquired", self.now_ms());
        }
        events
    }

    pub(crate) fn now_ms(&self) -> u64 {
        pit::elapsed_millis(interrupts::tick_count())
    }

    pub(crate) fn allocate_local_port(&mut self) -> u16 {
        let port = self.next_local_port;
        self.next_local_port = if self.next_local_port >= 65_535 {
            49_152
        } else {
            self.next_local_port + 1
        };
        port
    }

    pub(crate) fn resolve_host(&mut self, host: &str) -> Result<ipv4::Ipv4Addr, NetError> {
        if let Some(ip) = ipv4::Ipv4Addr::parse(host) {
            return Ok(ip);
        }

        let mut resolver = core::mem::take(&mut self.dns_resolver);
        let result = resolver.resolve(self, host, 5_000);
        self.dns_resolver = resolver;
        result
    }

    pub(crate) fn poll_network(&mut self) -> usize {
        let _ = self.sync_active_usb_nic();
        let timestamp_ms = self.now_ms();
        self.last_network_poll_ms = timestamp_ms;
        let timestamp = Instant::from_millis(timestamp_ms as i64);
        let dhcp_handle = self.dhcp_handle;
        let _ = interrupts::take_pending_network_irqs();

        let (dhcp_event, link_up) = {
            let (hardware, iface, sockets, arp_cache) = (
                &mut self.hardware,
                &mut self.iface,
                &mut self.sockets,
                &mut self.arp_cache,
            );
            let Some(nic) = hardware.as_mut() else {
                return 0;
            };
            let Some(iface) = iface.as_mut() else {
                return 0;
            };
            nic.poll();
            let link_up = nic.info().link_state != LinkState::Down;

            let mut device = KernelDevice {
                nic,
                arp_cache,
                now_ms: timestamp_ms,
            };
            let _ = iface.poll(timestamp, &mut device, sockets);
            (
                dhcp_handle.and_then(
                    |handle| match sockets.get_mut::<dhcpv4::Socket>(handle).poll() {
                    Some(dhcpv4::Event::Configured(config)) => {
                        Some(DhcpEventSnapshot::Configured(DhcpConfig::from(&config)))
                    }
                    Some(dhcpv4::Event::Deconfigured) => Some(DhcpEventSnapshot::Deconfigured),
                    None => None,
                },
                ),
                link_up,
            )
        };

        if let Some(event) = dhcp_event {
            self.apply_dhcp_event(event);
            1
        } else if !link_up && self.network_config.is_some() {
            self.apply_dhcp_event(DhcpEventSnapshot::Deconfigured);
            1
        } else {
            0
        }
    }

    fn prepare_stack(&mut self) -> Result<(), NetError> {
        let mac = self
            .hardware
            .as_ref()
            .ok_or(NetError::NoHardware)?
            .info()
            .mac;

        let mut config = InterfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        config.random_seed = 0x5741_524F_5300_0001;

        let timestamp = Instant::from_millis(self.now_ms() as i64);
        let mut device = self.device()?;
        let mut iface = Interface::new(config, &mut device, timestamp);
        iface.update_ip_addrs(|addrs| {
            if addrs.iter().next().is_none() {
                addrs
                    .push(IpCidr::Ipv4(smoltcp::wire::Ipv4Cidr::new(
                        smoltcp::wire::Ipv4Address::UNSPECIFIED,
                        0,
                    )))
                    .unwrap();
            }
        });

        self.iface = Some(iface);
        let mut dhcp_socket = dhcpv4::Socket::new();
        dhcp_socket.set_retry_config(dhcp_retry_config());
        self.dhcp_handle = Some(self.sockets.add(dhcp_socket));
        Ok(())
    }

    fn reset_runtime_state(&mut self) {
        self.clear_cached_network_state();
        self.network_config = None;
        self.iface = None;
        self.sockets = SocketSet::new(vec![]);
        self.dhcp_handle = None;
        self.dhcp_last_attempt = DhcpAttemptReport::idle();
        self.dhcp_owner = DhcpOwner::None;
        self.dhcp_owner_started_ms = 0;
        self.dhcp_owner_deadline_ms = 0;
        self.dhcp_owner_trigger = "none";
        self.dhcp_auto_pump = DhcpAutoPump::idle();
        self.dhcp_manual_preempted_auto = false;
        self.dhcp_auto_timed_out_before_manual = false;
        self.dhcp_last_auto_state = "none";
        self.dhcp_stale_worker_ignored = false;
        self.dhcp_stale_worker_ignored_count = 0;
        self.last_dhcp_event_ms = 0;
        self.read_only_maintenance = ReadOnlyMaintenanceReport::idle();
        reset_dhcp_wire_trace();
    }

    fn acquire_dhcp(&mut self, timeout_ms: u64) -> Result<Option<DhcpConfig>, NetError> {
        let start_ms = self.now_ms();
        let generation = self.claim_manual_dhcp_owner("net-dhcp", timeout_ms);
        let mut polls = 0u32;
        let interface = self
            .hardware
            .as_ref()
            .map(ActiveNic::info)
            .map_or("offline", |info| info.name);
        let (tx_start, rx_start) = self.nic_frame_counters();
        if let Err(error) = self.require_active_link("DHCP") {
            self.dhcp_last_attempt = self.capture_dhcp_attempt(
                "link-down",
                timeout_ms,
                start_ms,
                self.now_ms(),
                polls,
                interface,
                tx_start,
                rx_start,
            );
            let _ = self.clear_dhcp_owner(DhcpOwner::ManualCmd, generation);
            return Err(error);
        }
        if self.iface.is_none() {
            self.dhcp_last_attempt = self.capture_dhcp_attempt(
                "iface-not-ready",
                timeout_ms,
                start_ms,
                self.now_ms(),
                polls,
                interface,
                tx_start,
                rx_start,
            );
            let _ = self.clear_dhcp_owner(DhcpOwner::ManualCmd, generation);
            return Err(NetError::InitializationFailed(
                "network interface not ready",
            ));
        }
        if self.network_config.is_some() {
            self.dhcp_last_attempt = self.capture_dhcp_attempt(
                "lease-present",
                timeout_ms,
                start_ms,
                self.now_ms(),
                polls,
                interface,
                tx_start,
                rx_start,
            );
            let _ = self.clear_dhcp_owner(DhcpOwner::ManualCmd, generation);
            return Ok(self.network_config);
        }
        self.reset_dhcp_socket();
        self.dhcp_last_attempt = self.capture_dhcp_attempt(
            "in-progress",
            timeout_ms,
            start_ms,
            0,
            0,
            interface,
            tx_start,
            rx_start,
        );
        let deadline = self.now_ms().saturating_add(timeout_ms);
        while self.now_ms() < deadline {
            if self.dhcp_owner != DhcpOwner::ManualCmd || self.dhcp_generation != generation {
                self.note_stale_dhcp_worker();
                self.dhcp_last_attempt = self.capture_dhcp_attempt(
                    "stale-owner",
                    timeout_ms,
                    start_ms,
                    self.now_ms(),
                    polls,
                    interface,
                    tx_start,
                    rx_start,
                );
                return Ok(self.network_config);
            }
            for _ in 0..DHCP_POLL_BURST {
                self.poll_network();
                polls = polls.saturating_add(1);
                if self.network_config.is_some() {
                    self.dhcp_last_attempt = self.capture_dhcp_attempt(
                        "lease-acquired",
                        timeout_ms,
                        start_ms,
                        self.now_ms(),
                        polls,
                        interface,
                        tx_start,
                        rx_start,
                    );
                    let _ = self.clear_dhcp_owner(DhcpOwner::ManualCmd, generation);
                    return Ok(self.network_config);
                }
            }
            wait_for_runtime_progress();
        }
        let timeout_state = dhcp_wire_trace().timeout_state();
        self.dhcp_last_attempt = self.capture_dhcp_attempt(
            timeout_state,
            timeout_ms,
            start_ms,
            self.now_ms(),
            polls,
            interface,
            tx_start,
            rx_start,
        );
        let _ = self.clear_dhcp_owner(DhcpOwner::ManualCmd, generation);
        Ok(None)
    }

    fn apply_dhcp_event(&mut self, event: DhcpEventSnapshot) {
        self.last_dhcp_event_ms = self.now_ms();
        match event {
            DhcpEventSnapshot::Configured(converted) => {
                self.clear_cached_network_state();
                if let Some(iface) = self.iface.as_mut() {
                    iface.update_ip_addrs(|addrs| {
                        let cidr = ipv4::ip_cidr(converted.ip, converted.prefix_len);
                        if let Some(dest) = addrs.iter_mut().next() {
                            *dest = cidr;
                        } else {
                            addrs.push(cidr).unwrap();
                        }
                    });

                    if let Some(gateway) = converted.gateway {
                        let _ = iface
                            .routes_mut()
                            .add_default_ipv4_route(gateway.as_smoltcp());
                    } else {
                        iface.routes_mut().remove_default_ipv4_route();
                    }
                }
                self.network_config = Some(converted);
            }
            DhcpEventSnapshot::Deconfigured => {
                self.clear_cached_network_state();
                if let Some(iface) = self.iface.as_mut() {
                    iface.update_ip_addrs(|addrs| {
                        let cidr = ipv4::ip_cidr(ipv4::Ipv4Addr::ZERO, 0);
                        if let Some(dest) = addrs.iter_mut().next() {
                            *dest = cidr;
                        } else {
                            addrs.push(cidr).unwrap();
                        }
                    });
                    iface.routes_mut().remove_default_ipv4_route();
                }
                self.network_config = None;
            }
        }
    }

    fn record_read_only_maintenance(
        &mut self,
        trigger: &'static str,
        state: &'static str,
        started_ms: u64,
        finished_ms: u64,
        polls: u32,
    ) {
        self.read_only_maintenance = ReadOnlyMaintenanceReport {
            trigger,
            state,
            started_ms,
            finished_ms,
            polls,
        };
    }

    fn dhcp_owner_label(owner: DhcpOwner) -> &'static str {
        match owner {
            DhcpOwner::None => "none",
            DhcpOwner::AutoPump => "auto",
            DhcpOwner::ManualCmd => "manual",
        }
    }

    fn note_stale_dhcp_worker(&mut self) {
        self.dhcp_stale_worker_ignored = true;
        self.dhcp_stale_worker_ignored_count =
            self.dhcp_stale_worker_ignored_count.saturating_add(1);
    }

    fn set_dhcp_owner(
        &mut self,
        owner: DhcpOwner,
        trigger: &'static str,
        timeout_ms: u64,
        started_ms: u64,
    ) -> u64 {
        let generation = self.dhcp_generation.saturating_add(1);
        self.dhcp_generation = generation;
        self.dhcp_owner = owner;
        self.dhcp_owner_trigger = trigger;
        self.dhcp_owner_started_ms = started_ms;
        self.dhcp_owner_deadline_ms = if owner == DhcpOwner::None {
            0
        } else {
            started_ms.saturating_add(timeout_ms)
        };
        self.dhcp_stale_worker_ignored = false;
        generation
    }

    fn clear_dhcp_owner(&mut self, owner: DhcpOwner, generation: u64) -> bool {
        if self.dhcp_owner == owner && self.dhcp_generation == generation {
            self.dhcp_owner = DhcpOwner::None;
            self.dhcp_owner_started_ms = 0;
            self.dhcp_owner_deadline_ms = 0;
            self.dhcp_owner_trigger = "none";
            true
        } else {
            false
        }
    }

    fn retire_dhcp_auto_pump(
        &mut self,
        state: &'static str,
        finished_ms: u64,
        update_last_attempt: bool,
    ) {
        if self.dhcp_auto_pump.started_ms != 0 && update_last_attempt {
            self.dhcp_last_attempt = self.capture_dhcp_attempt(
                state,
                self.dhcp_auto_pump.timeout_ms,
                self.dhcp_auto_pump.started_ms,
                finished_ms,
                self.dhcp_auto_pump.polls,
                self.dhcp_auto_pump.interface,
                self.dhcp_auto_pump.tx_start,
                self.dhcp_auto_pump.rx_start,
            );
        }

        if state != "idle" {
            self.dhcp_last_auto_state = state;
        }
        let generation = self.dhcp_auto_pump.generation;
        self.dhcp_auto_pump.active = false;
        self.dhcp_auto_pump.state = state;
        self.dhcp_auto_pump.waiting_for = match state {
            "lease-acquired" => "lease",
            "offline" => "offline",
            state if state.starts_with("timeout-") => "timeout",
            state if state.starts_with("preempted-") => "none",
            _ => self.dhcp_waiting_for(),
        };
        self.dhcp_auto_pump.finished_ms = finished_ms;
        self.dhcp_auto_pump.last_service_ms = finished_ms;
        let _ = self.clear_dhcp_owner(DhcpOwner::AutoPump, generation);
    }

    fn claim_manual_dhcp_owner(&mut self, trigger: &'static str, timeout_ms: u64) -> u64 {
        let now = self.now_ms();
        let auto_active = self.dhcp_owner == DhcpOwner::AutoPump && self.dhcp_auto_pump.active;
        self.dhcp_manual_preempted_auto = auto_active;
        self.dhcp_auto_timed_out_before_manual =
            self.dhcp_last_auto_state.starts_with("timeout-")
                || self.dhcp_auto_pump.state.starts_with("timeout-");
        if auto_active {
            self.retire_dhcp_auto_pump("preempted-manual", now, false);
        } else {
            self.dhcp_auto_pump = DhcpAutoPump::idle();
        }
        self.record_read_only_maintenance(trigger, "manual-owner", now, now, 0);
        self.set_dhcp_owner(DhcpOwner::ManualCmd, trigger, timeout_ms, now)
    }

    fn dhcp_waiting_for(&self) -> &'static str {
        if self.network_config.is_some() {
            return "lease";
        }

        let trace = dhcp_wire_trace();
        if trace.request_sent > trace.ack_received
            || matches!(trace.last_event, "offer-received" | "request-sent")
        {
            "ack"
        } else if trace.discover_sent > 0
            || matches!(trace.last_event, "discover-sent" | "nak-received")
        {
            "offer"
        } else {
            "discover"
        }
    }

    fn begin_dhcp_auto_pump(&mut self, trigger: &'static str, timeout_ms: u64) {
        if self.network_config.is_some() {
            self.dhcp_auto_pump = DhcpAutoPump::idle();
            return;
        }

        let Some(link) = self.hardware.as_ref().map(ActiveNic::info) else {
            self.dhcp_auto_pump = DhcpAutoPump::idle();
            return;
        };
        if link.link_state == LinkState::Down || self.iface.is_none() || self.dhcp_handle.is_none() {
            return;
        }
        if self.dhcp_owner == DhcpOwner::ManualCmd {
            return;
        }

        let now = self.now_ms();
        if self.dhcp_auto_pump.active
            && self.dhcp_owner == DhcpOwner::AutoPump
            && self.dhcp_auto_pump.generation == self.dhcp_generation
            && self.dhcp_auto_pump.interface == link.name
            && now < self.dhcp_auto_pump.deadline_ms
        {
            self.dhcp_auto_pump.waiting_for = self.dhcp_waiting_for();
            self.dhcp_auto_pump.last_service_ms = now;
            return;
        }

        self.dhcp_manual_preempted_auto = false;
        self.dhcp_auto_timed_out_before_manual = false;
        let generation = self.set_dhcp_owner(DhcpOwner::AutoPump, trigger, timeout_ms, now);
        self.dhcp_last_auto_state = "lease-pending";
        self.reset_dhcp_socket();
        let (tx_start, rx_start) = self.nic_frame_counters();
        self.dhcp_auto_pump = DhcpAutoPump {
            active: true,
            generation,
            trigger,
            state: "lease-pending",
            waiting_for: self.dhcp_waiting_for(),
            started_ms: now,
            deadline_ms: now.saturating_add(timeout_ms),
            finished_ms: 0,
            last_service_ms: now,
            polls: 0,
            timeout_ms,
            interface: link.name,
            tx_start,
            rx_start,
        };
        self.dhcp_last_attempt = self.capture_dhcp_attempt(
            "in-progress",
            timeout_ms,
            now,
            now,
            0,
            link.name,
            tx_start,
            rx_start,
        );
    }

    fn update_dhcp_auto_pump_snapshot(&mut self, state: &'static str, now: u64) {
        if self.dhcp_auto_pump.started_ms == 0 {
            return;
        }

        self.dhcp_auto_pump.state = state;
        self.dhcp_auto_pump.waiting_for = self.dhcp_waiting_for();
        self.dhcp_auto_pump.last_service_ms = now;
        self.dhcp_last_attempt = self.capture_dhcp_attempt(
            state,
            self.dhcp_auto_pump.timeout_ms,
            self.dhcp_auto_pump.started_ms,
            now,
            self.dhcp_auto_pump.polls,
            self.dhcp_auto_pump.interface,
            self.dhcp_auto_pump.tx_start,
            self.dhcp_auto_pump.rx_start,
        );
    }

    fn finish_dhcp_auto_pump(&mut self, state: &'static str, finished_ms: u64) {
        self.retire_dhcp_auto_pump(state, finished_ms, true);
    }

    fn drive_dhcp_auto_pump(&mut self, budget_ms: u64, allow_wait: bool) -> usize {
        if !self.dhcp_auto_pump.active {
            return 0;
        }
        if self.dhcp_owner != DhcpOwner::AutoPump
            || self.dhcp_auto_pump.generation != self.dhcp_generation
        {
            self.note_stale_dhcp_worker();
            self.retire_dhcp_auto_pump("stale-ignored", self.now_ms(), false);
            return 0;
        }
        if self.network_config.is_some() {
            let finished_ms = self.now_ms();
            self.finish_dhcp_auto_pump("lease-acquired", finished_ms);
            return 0;
        }

        let Some(link) = self.hardware.as_ref().map(ActiveNic::info) else {
            let finished_ms = self.now_ms();
            self.finish_dhcp_auto_pump("offline", finished_ms);
            return 0;
        };
        if link.link_state == LinkState::Down || self.iface.is_none() || self.dhcp_handle.is_none() {
            let finished_ms = self.now_ms();
            self.finish_dhcp_auto_pump("offline", finished_ms);
            return 0;
        }

        let started_ms = self.now_ms();
        if started_ms >= self.dhcp_auto_pump.deadline_ms {
            let timeout_state = dhcp_wire_trace().timeout_state();
            self.finish_dhcp_auto_pump(timeout_state, started_ms);
            return 0;
        }

        let slice_deadline = if budget_ms == 0 {
            started_ms
        } else {
            started_ms
                .saturating_add(budget_ms)
                .min(self.dhcp_auto_pump.deadline_ms)
        };
        let mut events = 0usize;

        loop {
            events = events.saturating_add(self.poll_network());
            self.dhcp_auto_pump.polls = self.dhcp_auto_pump.polls.saturating_add(1);
            let now = self.now_ms();
            self.dhcp_auto_pump.last_service_ms = now;

            if self.network_config.is_some() {
                self.finish_dhcp_auto_pump("lease-acquired", now);
                return events;
            }
            if now >= self.dhcp_auto_pump.deadline_ms {
                let timeout_state = dhcp_wire_trace().timeout_state();
                self.finish_dhcp_auto_pump(timeout_state, now);
                return events;
            }
            if budget_ms == 0 || now >= slice_deadline || !allow_wait {
                break;
            }
            wait_for_runtime_progress();
        }

        self.update_dhcp_auto_pump_snapshot("lease-pending", self.now_ms());
        events
    }

    fn canonical_ipv4_lease_state(&self) -> &'static str {
        if self.network_config.is_some() {
            return "lease-acquired";
        }

        let Some(link) = self.hardware.as_ref().map(ActiveNic::info) else {
            return "offline";
        };
        if link.link_state == LinkState::Down || self.iface.is_none() || self.dhcp_handle.is_none() {
            return "offline";
        }

        if self.dhcp_last_attempt.state.starts_with("timeout-") {
            return self.dhcp_last_attempt.state;
        }
        if self.dhcp_auto_pump.state.starts_with("timeout-") {
            return self.dhcp_auto_pump.state;
        }

        if self.dhcp_owner != DhcpOwner::None
            || self.dhcp_auto_pump.active
            || matches!(self.dhcp_auto_pump.state, "lease-pending")
            || matches!(self.dhcp_last_attempt.state, "in-progress" | "lease-pending")
            || matches!(self.read_only_maintenance.state, "lease-pending")
        {
            return "lease-pending";
        }

        match dhcp_wire_trace().last_event {
            "discover-sent" | "offer-received" | "request-sent" | "ack-received" => {
                "lease-pending"
            }
            _ => "offline",
        }
    }

    fn canonical_lease_view(&self, canonical_state: &'static str) -> &'static str {
        match canonical_state {
            "lease-acquired" => "active-interface",
            "lease-pending" => "pending-dhcp",
            state if state.starts_with("timeout-") => "timeout",
            _ => "none",
        }
    }

    fn service_read_only_maintenance(
        &mut self,
        trigger: &'static str,
        timeout_ms: u64,
    ) -> Option<DhcpConfig> {
        let started_ms = self.now_ms();
        let interface = self
            .hardware
            .as_ref()
            .map(ActiveNic::info)
            .map_or("offline", |info| info.name);
        let (tx_start, rx_start) = self.nic_frame_counters();
        if self.network_config.is_some() {
            self.record_read_only_maintenance(
                trigger,
                "skipped-lease-active",
                started_ms,
                started_ms,
                0,
            );
            return self.network_config;
        }
        let Some(link) = self.hardware.as_ref().map(ActiveNic::info) else {
            self.record_read_only_maintenance(
                trigger,
                "skipped-no-hardware",
                started_ms,
                started_ms,
                0,
            );
            return None;
        };
        if link.link_state == LinkState::Down {
            self.record_read_only_maintenance(
                trigger,
                "skipped-link-down",
                started_ms,
                started_ms,
                0,
            );
            return None;
        }
        if self.iface.is_none() {
            self.record_read_only_maintenance(
                trigger,
                "skipped-iface-missing",
                started_ms,
                started_ms,
                0,
            );
            return None;
        }
        if self.dhcp_handle.is_none() {
            self.record_read_only_maintenance(
                trigger,
                "skipped-dhcp-socket-missing",
                started_ms,
                started_ms,
                0,
            );
            return None;
        }
        if self.dhcp_owner == DhcpOwner::ManualCmd {
            self.record_read_only_maintenance(
                trigger,
                "observer-manual-owner",
                started_ms,
                started_ms,
                0,
            );
            return None;
        }
        let canonical_state_before = self.canonical_ipv4_lease_state();
        let use_auto_pump = matches!(self.hardware.as_ref(), Some(ActiveNic::UsbNet(_)))
            && (self.dhcp_auto_pump.active
                || self.dhcp_owner == DhcpOwner::AutoPump
                || trigger == "usb-attach"
                || canonical_state_before == "lease-pending");
        if use_auto_pump {
            if !self.dhcp_auto_pump.active {
                self.begin_dhcp_auto_pump(trigger, USB_ATTACH_AUTO_DHCP_TIMEOUT_MS);
            }
            let polls_before = self.dhcp_auto_pump.polls;
            let _ = self.drive_dhcp_auto_pump(timeout_ms, true);
            let finished_ms = self.now_ms();
            let polls = self.dhcp_auto_pump.polls.saturating_sub(polls_before);
            let state = self.canonical_ipv4_lease_state();
            self.record_read_only_maintenance(trigger, state, started_ms, finished_ms, polls);
            return self.network_config;
        }
        if started_ms.saturating_sub(self.last_network_poll_ms)
            < READ_ONLY_MAINTENANCE_COOLDOWN_MS
            && canonical_state_before != "lease-pending"
        {
            self.record_read_only_maintenance(
                trigger,
                "skipped-cooldown",
                started_ms,
                started_ms,
                0,
            );
            return None;
        }

        let deadline = started_ms.saturating_add(timeout_ms);
        let mut polls = 0u32;
        loop {
            let _ = self.poll_network();
            polls = polls.saturating_add(1);
            if self.network_config.is_some() {
                let finished_ms = self.now_ms();
                self.record_read_only_maintenance(
                    trigger,
                    "lease-acquired",
                    started_ms,
                    finished_ms,
                    polls,
                );
                self.dhcp_last_attempt = self.capture_dhcp_attempt(
                    "lease-acquired",
                    timeout_ms,
                    started_ms,
                    finished_ms,
                    polls,
                    interface,
                    tx_start,
                    rx_start,
                );
                return self.network_config;
            }
            if self.now_ms() >= deadline {
                break;
            }
            wait_for_runtime_progress();
        }

        let state = self.canonical_ipv4_lease_state();
        let finished_ms = self.now_ms();
        self.record_read_only_maintenance(trigger, state, started_ms, finished_ms, polls);
        if polls != 0 {
            self.dhcp_last_attempt = self.capture_dhcp_attempt(
                state,
                timeout_ms,
                started_ms,
                finished_ms,
                polls,
                interface,
                tx_start,
                rx_start,
            );
        }
        None
    }

    fn clear_cached_network_state(&mut self) {
        self.dns_resolver.clear();
        self.arp_cache.clear();
    }

    fn reset_dhcp_socket(&mut self) {
        reset_dhcp_wire_trace();
        if let Some(handle) = self.dhcp_handle {
            let socket = self.sockets.get_mut::<dhcpv4::Socket>(handle);
            socket.set_retry_config(dhcp_retry_config());
            socket.reset();
        }
    }

    fn nic_frame_counters(&self) -> (u64, u64) {
        let Some(nic) = self.hardware.as_ref() else {
            return (0, 0);
        };
        match nic.diagnostics() {
            NetworkDiagnostics::Virtio(diag) => (diag.tx_frames, diag.rx_frames),
            NetworkDiagnostics::E1000(diag) => (diag.tx_frames, diag.rx_frames),
            NetworkDiagnostics::Rtl8169(diag) => (diag.tx_frames, diag.rx_frames),
            NetworkDiagnostics::UsbNet(diag) => (diag.tx_frames, diag.rx_frames),
        }
    }

    fn capture_dhcp_attempt(
        &self,
        state: &'static str,
        timeout_ms: u64,
        started_ms: u64,
        finished_ms: u64,
        polls: u32,
        interface: &'static str,
        tx_start: u64,
        rx_start: u64,
    ) -> DhcpAttemptReport {
        let trace = dhcp_wire_trace();
        let (tx_now, rx_now) = self.nic_frame_counters();
        DhcpAttemptReport {
            state,
            timeout_ms,
            started_ms,
            finished_ms,
            polls,
            interface,
            discover_sent: trace.discover_sent,
            offer_received: trace.offer_received,
            request_sent: trace.request_sent,
            ack_received: trace.ack_received,
            nak_received: trace.nak_received,
            last_event: trace.last_event,
            tx_frames_delta: tx_now.saturating_sub(tx_start),
            rx_frames_delta: rx_now.saturating_sub(rx_start),
            rx_eth_frames: trace.rx_eth_frames,
            rx_ipv4_frames: trace.rx_ipv4_frames,
            rx_udp_frames: trace.rx_udp_frames,
            rx_udp_67_68: trace.rx_udp_67_68,
            rx_dhcp_frames: trace.rx_dhcp_frames,
            rx_dhcp_parse_errors: trace.rx_dhcp_parse_errors,
            dhcp_drop_reason: trace.dhcp_drop_reason,
        }
    }

    pub(crate) fn require_active_link(&self, purpose: &str) -> Result<NetworkDeviceInfo, NetError> {
        let info = self
            .hardware
            .as_ref()
            .map(ActiveNic::info)
            .ok_or(NetError::NoHardware)?;
        if info.link_state == LinkState::Down {
            return Err(NetError::ProtocolError(format!(
                "{purpose}: wired link is down"
            )));
        }
        Ok(info)
    }

    pub(crate) fn require_configured_ipv4(
        &self,
        purpose: &str,
    ) -> Result<(NetworkDeviceInfo, DhcpConfig), NetError> {
        let info = self.require_active_link(purpose)?;
        let config = self.network_config.ok_or_else(|| {
            NetError::ProtocolError(format!(
                "{purpose}: IPv4 is not configured; run 'net dhcp' on a supported wired NIC first"
            ))
        })?;
        Ok((info, config))
    }

    pub(crate) fn require_route_to(
        &self,
        target: ipv4::Ipv4Addr,
        purpose: &str,
    ) -> Result<(NetworkDeviceInfo, DhcpConfig), NetError> {
        let (info, config) = self.require_configured_ipv4(purpose)?;
        if config.can_reach(target) {
            Ok((info, config))
        } else {
            Err(NetError::ProtocolError(format!(
                "{purpose}: no route to {target}; DHCP did not provide a reachable path"
            )))
        }
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.hardware
            .as_mut()
            .ok_or(NetError::NoHardware)?
            .send_frame(frame)
    }

    pub fn recv_frame(&mut self) -> Option<Vec<u8>> {
        self.hardware.as_mut().and_then(ActiveNic::recv_frame)
    }

    pub fn hardware_diagnostics(&self) -> Option<NetworkDiagnostics> {
        self.hardware.as_ref().map(ActiveNic::diagnostics)
    }

    #[must_use]
    pub fn maintenance_report(&self) -> NetworkMaintenanceReport {
        let now = self.now_ms();
        let canonical_lease_state = self.canonical_ipv4_lease_state();
        let dhcp_owner_elapsed_ms = if self.dhcp_owner == DhcpOwner::None
            || self.dhcp_owner_started_ms == 0
        {
            0
        } else {
            now.saturating_sub(self.dhcp_owner_started_ms)
        };
        let dhcp_owner_remaining_ms = if self.dhcp_owner == DhcpOwner::None {
            0
        } else {
            self.dhcp_owner_deadline_ms.saturating_sub(now)
        };
        let dhcp_auto_pump_elapsed_ms = if self.dhcp_auto_pump.started_ms == 0 {
            0
        } else {
            now.saturating_sub(self.dhcp_auto_pump.started_ms)
        };
        let dhcp_auto_pump_remaining_ms = if self.dhcp_auto_pump.active {
            self.dhcp_auto_pump.deadline_ms.saturating_sub(now)
        } else {
            0
        };
        let (active_interface, active_driver, active_is_usb) = match self.hardware.as_ref() {
            Some(ActiveNic::UsbNet(device)) => (device.info().name, device.info().driver, true),
            Some(device) => {
                let info = device.info();
                (info.name, info.driver, false)
            }
            None => ("offline", "none", false),
        };
        NetworkMaintenanceReport {
            last_poll_ms: self.last_network_poll_ms,
            ms_since_poll: now.saturating_sub(self.last_network_poll_ms),
            last_dhcp_event_ms: self.last_dhcp_event_ms,
            ms_since_dhcp_event: if self.last_dhcp_event_ms == 0 {
                0
            } else {
                now.saturating_sub(self.last_dhcp_event_ms)
            },
            canonical_lease_state,
            dhcp_socket_ready: self.dhcp_handle.is_some(),
            iface_ready: self.iface.is_some(),
            active_interface,
            active_driver,
            active_is_usb,
            lease_view: self.canonical_lease_view(canonical_lease_state),
            dhcp_last_wire_event: dhcp_wire_trace().last_event,
            dhcp_owner: Self::dhcp_owner_label(self.dhcp_owner),
            dhcp_owner_trigger: self.dhcp_owner_trigger,
            dhcp_generation: self.dhcp_generation,
            dhcp_owner_started_ms: self.dhcp_owner_started_ms,
            dhcp_owner_deadline_ms: self.dhcp_owner_deadline_ms,
            dhcp_owner_elapsed_ms,
            dhcp_owner_remaining_ms,
            dhcp_auto_pump_active: self.dhcp_owner == DhcpOwner::AutoPump && self.dhcp_auto_pump.active,
            dhcp_manual_active: self.dhcp_owner == DhcpOwner::ManualCmd,
            dhcp_auto_pump_trigger: self.dhcp_auto_pump.trigger,
            dhcp_auto_pump_state: self.dhcp_auto_pump.state,
            dhcp_auto_pump_waiting_for: self.dhcp_auto_pump.waiting_for,
            dhcp_auto_pump_started_ms: self.dhcp_auto_pump.started_ms,
            dhcp_auto_pump_deadline_ms: self.dhcp_auto_pump.deadline_ms,
            dhcp_auto_pump_last_service_ms: self.dhcp_auto_pump.last_service_ms,
            dhcp_auto_pump_polls: self.dhcp_auto_pump.polls,
            dhcp_auto_pump_elapsed_ms,
            dhcp_auto_pump_remaining_ms,
            dhcp_manual_preempted_auto: self.dhcp_manual_preempted_auto,
            dhcp_auto_timed_out_before_manual: self.dhcp_auto_timed_out_before_manual,
            dhcp_last_auto_state: self.dhcp_last_auto_state,
            dhcp_stale_worker_ignored: self.dhcp_stale_worker_ignored,
            dhcp_stale_worker_ignored_count: self.dhcp_stale_worker_ignored_count,
            read_only_trigger: self.read_only_maintenance.trigger,
            read_only_state: self.read_only_maintenance.state,
            read_only_started_ms: self.read_only_maintenance.started_ms,
            read_only_finished_ms: self.read_only_maintenance.finished_ms,
            read_only_polls: self.read_only_maintenance.polls,
        }
    }

    #[must_use]
    pub fn dhcp_attempt(&self) -> DhcpAttemptReport {
        self.dhcp_last_attempt
    }

    pub fn send_arp_probe(&mut self, target: ipv4::Ipv4Addr) -> Result<(), NetError> {
        let nic = self.hardware.as_mut().ok_or(NetError::NoHardware)?;
        let sender_mac = nic.info().mac;
        let sender_ip = self
            .network_config
            .map(|config| config.ip)
            .unwrap_or(ipv4::Ipv4Addr::ZERO);
        let frame = arp::build_request_frame(sender_mac, sender_ip, target);
        nic.send_frame(&frame)
    }

    fn device(&mut self) -> Result<KernelDevice<'_>, NetError> {
        let now_ms = self.now_ms();
        let nic = self.hardware.as_mut().ok_or(NetError::NoHardware)?;
        Ok(KernelDevice {
            nic,
            arp_cache: &mut self.arp_cache,
            now_ms,
        })
    }
}

struct KernelDevice<'a> {
    nic: &'a mut ActiveNic,
    arp_cache: &'a mut arp::ArpCache,
    now_ms: u64,
}

impl<'a> smoltcp::phy::Device for KernelDevice<'a> {
    type RxToken<'b>
        = KernelRxToken
    where
        Self: 'b;
    type TxToken<'b>
        = KernelTxToken<'b>
    where
        Self: 'b;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let frame = self.nic.recv_frame()?;
        observe_dhcp_frame(&frame, false);
        self.arp_cache.observe_frame(&frame, self.now_ms);
        Some((
            KernelRxToken { buffer: frame },
            KernelTxToken { nic: self.nic },
        ))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(KernelTxToken { nic: self.nic })
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut capabilities = smoltcp::phy::DeviceCapabilities::default();
        capabilities.medium = smoltcp::phy::Medium::Ethernet;
        capabilities.max_transmission_unit = 1500;
        capabilities.max_burst_size = Some(1);
        capabilities
    }
}

struct KernelRxToken {
    buffer: Vec<u8>,
}

impl smoltcp::phy::RxToken for KernelRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = self.buffer;
        f(&mut buffer)
    }
}

struct KernelTxToken<'a> {
    nic: &'a mut ActiveNic,
}

impl<'a> smoltcp::phy::TxToken for KernelTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0u8; len];
        let result = f(&mut frame);
        if let Err(error) = self.nic.send_frame(&frame) {
            crate::serial_println!("[NET] TX submit failed: {}", error);
        } else {
            observe_dhcp_frame(&frame, true);
        }
        result
    }
}

/// Initialize both the legacy serial link and the kernel networking stack.
pub fn init() -> Result<NetInitReport, NetError> {
    NET.lock().init()
}

/// Poll the hardware/network stack.
pub fn poll() -> usize {
    let mut net = NET.lock();
    let had_config = net.network_config.is_some();
    let events = net.service_runtime_poll();
    let has_config = net.network_config.is_some();
    drop(net);
    if events > 0 || (!had_config && has_config) {
        let _ = crate::hal::net::register_active_network();
    }
    events
}

/// Snapshot every enumerated PCI device.
#[must_use]
pub fn pci_devices() -> Vec<PciDevice> {
    NET.lock().pci_devices.clone()
}

/// Return the current DHCP-derived network configuration, if any.
#[must_use]
pub fn network_config() -> Option<DhcpConfig> {
    let mut net = NET.lock();
    let had_config = net.network_config.is_some();
    if had_config {
        let _ = net.poll_network();
    } else {
        let _ = net.service_read_only_maintenance(
            "network-config",
            READ_ONLY_MAINTENANCE_TIMEOUT_MS,
        );
    }
    let config = net.network_config;
    drop(net);
    if !had_config && config.is_some() {
        let _ = crate::hal::net::register_active_network();
    }
    config
}

/// Human-readable combined interface status.
#[must_use]
pub fn status() -> String {
    let mut net = NET.lock();
    let had_config = net.network_config.is_some();
    let summary = net.status();
    let has_config = net.network_config.is_some();
    drop(net);
    if !had_config && has_config {
        let _ = crate::hal::net::register_active_network();
    }
    summary
}

/// Return current active network device details if initialization succeeded.
#[must_use]
pub fn hardware_status() -> Option<NetworkDeviceInfo> {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    net.hardware.as_ref().map(ActiveNic::info)
}

/// Return low-level network driver counters for debugging packet flow.
#[must_use]
pub fn hardware_diagnostics() -> Option<NetworkDiagnostics> {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    net.hardware_diagnostics()
}

#[must_use]
pub fn dhcp_attempt() -> DhcpAttemptReport {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    let had_config = net.network_config.is_some();
    if net.network_config.is_none() && net.dhcp_auto_pump.active {
        let _ = net.drive_dhcp_auto_pump(AUTO_DHCP_RUNTIME_SLICE_MS, true);
    }
    let report = net.dhcp_attempt();
    let has_config = net.network_config.is_some();
    drop(net);
    if !had_config && has_config {
        let _ = crate::hal::net::register_active_network();
    }
    report
}

#[must_use]
pub fn maintenance_report() -> NetworkMaintenanceReport {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    let had_config = net.network_config.is_some();
    if net.network_config.is_none() && net.dhcp_auto_pump.active {
        let _ = net.drive_dhcp_auto_pump(AUTO_DHCP_RUNTIME_SLICE_MS, true);
    }
    let report = net.maintenance_report();
    let has_config = net.network_config.is_some();
    drop(net);
    if !had_config && has_config {
        let _ = crate::hal::net::register_active_network();
    }
    report
}

/// Send a text message over the legacy COM2 wire protocol.
pub fn send_text(text: &str) -> Result<(), NetError> {
    NET.lock().serial.send_text(text)
}

/// Send a QASM payload over the legacy COM2 wire protocol.
pub fn send_circuit(qasm: &str) -> Result<(), NetError> {
    NET.lock().serial.send_circuit(qasm)
}

/// Send a specific serial message type over COM2.
pub fn send_message(msg_type: MessageType, payload: &[u8]) -> Result<(), NetError> {
    NET.lock().serial.send(msg_type, payload)
}

/// Poll the COM2 link for one message.
#[must_use]
pub fn receive() -> Option<Message> {
    NET.lock().serial.receive()
}

/// Resolve a host name using the DHCP-provided DNS server.
pub fn resolve_host(host: &str) -> Result<ipv4::Ipv4Addr, NetError> {
    NET.lock().resolve_host(host)
}

/// Wait for a DHCP lease using the existing interface stack.
pub fn wait_for_dhcp(timeout_ms: u64) -> Result<Option<DhcpConfig>, NetError> {
    let result = NET.lock().acquire_dhcp(timeout_ms);
    if result.is_ok() {
        let _ = crate::hal::net::register_active_network();
    }
    result
}

pub(crate) fn wait_for_runtime_progress() {
    let start = interrupts::tick_count();
    if !pit::wait_for_tick_advance(start, 1) {
        core::hint::spin_loop();
    }
}

fn dhcp_retry_config() -> dhcpv4::RetryConfig {
    let mut config = dhcpv4::RetryConfig::default();
    config.discover_timeout = Duration::from_millis(DHCP_DISCOVER_RETRY_MS);
    config.initial_request_timeout = Duration::from_millis(DHCP_REQUEST_RETRY_MS);
    config.request_retries = DHCP_REQUEST_RETRIES;
    config
}

fn reset_dhcp_wire_trace() {
    *DHCP_WIRE_TRACE.lock() = DhcpWireTrace::new();
}

fn dhcp_wire_trace() -> DhcpWireTrace {
    *DHCP_WIRE_TRACE.lock()
}

fn observe_dhcp_frame(frame: &[u8], tx: bool) {
    let Ok(eth) = SmolEthernetFrame::new_checked(frame) else {
        if !tx {
            let mut trace = DHCP_WIRE_TRACE.lock();
            trace.dhcp_drop_reason = "rx-bad-ethernet";
        }
        return;
    };
    if !tx {
        let mut trace = DHCP_WIRE_TRACE.lock();
        trace.rx_eth_frames = trace.rx_eth_frames.saturating_add(1);
    }
    if eth.ethertype() != EthernetProtocol::Ipv4 {
        return;
    }
    if !tx {
        let mut trace = DHCP_WIRE_TRACE.lock();
        trace.rx_ipv4_frames = trace.rx_ipv4_frames.saturating_add(1);
    }
    let Ok(ipv4) = Ipv4Packet::new_checked(eth.payload()) else {
        if !tx {
            let mut trace = DHCP_WIRE_TRACE.lock();
            trace.dhcp_drop_reason = "rx-bad-ipv4";
        }
        return;
    };
    if ipv4.next_header() != IpProtocol::Udp {
        return;
    }
    if !tx {
        let mut trace = DHCP_WIRE_TRACE.lock();
        trace.rx_udp_frames = trace.rx_udp_frames.saturating_add(1);
    }
    let Ok(udp) = UdpPacket::new_checked(ipv4.payload()) else {
        if !tx {
            let mut trace = DHCP_WIRE_TRACE.lock();
            trace.dhcp_drop_reason = "rx-bad-udp";
        }
        return;
    };
    let src_port = udp.src_port();
    let dst_port = udp.dst_port();
    if !((src_port == DHCP_CLIENT_PORT && dst_port == DHCP_SERVER_PORT)
        || (src_port == DHCP_SERVER_PORT && dst_port == DHCP_CLIENT_PORT))
    {
        return;
    }
    if !tx {
        let mut trace = DHCP_WIRE_TRACE.lock();
        trace.rx_udp_67_68 = trace.rx_udp_67_68.saturating_add(1);
    }
    let Ok(packet) = DhcpPacket::new_checked(udp.payload()) else {
        if !tx {
            let mut trace = DHCP_WIRE_TRACE.lock();
            trace.rx_dhcp_parse_errors = trace.rx_dhcp_parse_errors.saturating_add(1);
            trace.dhcp_drop_reason = "rx-bad-dhcp-packet";
        }
        return;
    };
    let Ok(repr) = DhcpRepr::parse(&packet) else {
        if !tx {
            let mut trace = DHCP_WIRE_TRACE.lock();
            trace.rx_dhcp_parse_errors = trace.rx_dhcp_parse_errors.saturating_add(1);
            trace.dhcp_drop_reason = "rx-bad-dhcp-repr";
        }
        return;
    };
    let mut trace = DHCP_WIRE_TRACE.lock();
    if !tx {
        trace.rx_dhcp_frames = trace.rx_dhcp_frames.saturating_add(1);
    }
    match (tx, repr.message_type) {
        (true, DhcpMessageType::Discover) => {
            trace.discover_sent = trace.discover_sent.saturating_add(1);
            trace.last_event = "discover-sent";
        }
        (true, DhcpMessageType::Request) => {
            trace.request_sent = trace.request_sent.saturating_add(1);
            trace.last_event = "request-sent";
        }
        (false, DhcpMessageType::Offer) => {
            trace.offer_received = trace.offer_received.saturating_add(1);
            trace.last_event = "offer-received";
        }
        (false, DhcpMessageType::Ack) => {
            trace.ack_received = trace.ack_received.saturating_add(1);
            trace.last_event = "ack-received";
        }
        (false, DhcpMessageType::Nak) => {
            trace.nak_received = trace.nak_received.saturating_add(1);
            trace.last_event = "nak-received";
        }
        _ => {}
    }
}

/// Send one raw Ethernet frame through virtio-net.
pub fn send_raw_frame(frame: &[u8]) -> Result<(), NetError> {
    NET.lock().send_frame(frame)
}

/// Broadcast an ARP request through the hardware NIC.
pub fn send_arp_probe(target: ipv4::Ipv4Addr) -> Result<(), NetError> {
    NET.lock().send_arp_probe(target)
}

/// Receive one raw Ethernet frame harvested from the RX virtqueue.
#[must_use]
pub fn receive_raw_frame() -> Option<Vec<u8>> {
    NET.lock().recv_frame()
}

/// Send one ICMP echo request and wait for the reply.
pub fn ping_host(host: &str) -> Result<icmp::PingReply, NetError> {
    let mut stack = NET.lock();
    let target = stack.resolve_host(host)?;
    icmp::ping(&mut stack, target, 1, 3_000)
}

/// Perform an HTTP GET over the kernel TCP/IP stack.
pub fn http_get(url: &str) -> Result<http::HttpResponse, NetError> {
    http::http_get(&mut NET.lock(), url)
}

/// Perform an HTTP GET with extra request headers.
pub fn http_get_with_headers(
    url: &str,
    headers: &[(&str, &str)],
) -> Result<http::HttpResponse, NetError> {
    http::http_get_with_headers(&mut NET.lock(), url, headers)
}

/// Perform an HTTP POST over the kernel TCP/IP stack.
pub fn http_post(
    url: &str,
    content_type: &str,
    body: &[u8],
) -> Result<http::HttpResponse, NetError> {
    http::http_post(&mut NET.lock(), url, content_type, body)
}

/// Perform an HTTP POST with extra request headers.
pub fn http_post_with_headers(
    url: &str,
    content_type: &str,
    body: &[u8],
    headers: &[(&str, &str)],
) -> Result<http::HttpResponse, NetError> {
    http::http_post_with_headers(&mut NET.lock(), url, content_type, body, headers)
}

/// Snapshot the ARP cache observed by the interface.
#[must_use]
pub fn arp_entries() -> Vec<arp::ArpEntry> {
    NET.lock().arp_cache.entries().to_vec()
}

/// Look up an IPv4 address in the observed ARP cache.
#[must_use]
pub fn arp_lookup(ip: ipv4::Ipv4Addr) -> Option<[u8; 6]> {
    NET.lock().arp_cache.lookup(ip)
}

/// Snapshot the DNS cache.
#[must_use]
pub fn dns_cache() -> Vec<dns::DnsCacheEntry> {
    NET.lock().dns_resolver.entries().to_vec()
}

/// Manually attach a USB NIC, replacing the current hardware NIC.
pub fn attach_usb_nic() -> Result<(), NetError> {
    let result = NET.lock().attach_usb_nic();
    if result.is_ok() {
        let _ = crate::hal::net::register_active_network();
    }
    result
}

/// Whether a USB NIC (CDC-ECM/NCM/RNDIS) is currently visible from the xHCI driver.
#[must_use]
pub fn usb_nic_available() -> bool {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    net.usb_nic_available()
}

/// Return the first currently detected USB NIC/tethering device, if any.
#[must_use]
pub fn detected_usb_nic() -> Option<UsbNetworkInfo> {
    usb_tethering_snapshot().detected
}

/// Whether the currently active NIC is a USB NIC.
#[must_use]
pub fn active_is_usb() -> bool {
    let mut net = NET.lock();
    let _ = net.sync_active_usb_nic();
    net.active_is_usb()
}

#[must_use]
pub fn usb_tethering_snapshot() -> UsbTetheringSnapshot {
    NET.lock().usb_tethering_snapshot()
}

/// Force the active wired NIC to retry full bring-up: re-enable bus mastering,
/// wake the PHY, restart autoneg, and try a forced 100M-FD fallback if needed.
/// Returns Ok(true) if link came up, Ok(false) if it stayed down, or Err if
/// no compatible NIC is active.
pub fn retry_active_nic(timeout_ms: u64) -> Result<bool, NetError> {
    let mut stack = NET.lock();
    let nic = stack.hardware.as_mut().ok_or(NetError::NoHardware)?;
    let link_up = match nic {
        ActiveNic::Rtl8169(device) => device.force_retry(timeout_ms),
        ActiveNic::E1000(_) | ActiveNic::Virtio(_) => {
            return Err(NetError::ProtocolError(
                "net retry: active driver does not implement force-retry yet".into(),
            ));
        }
        ActiveNic::UsbNet(_) => {
            return Err(NetError::ProtocolError(
                "net retry: USB NIC has no PHY-level retry; reattach with 'net usb'".into(),
            ));
        }
    };
    drop(stack);
    if link_up {
        let _ = crate::hal::net::register_active_network();
        let _ = wait_for_dhcp(DEFAULT_DHCP_TIMEOUT_MS);
    }
    Ok(link_up)
}

#[must_use]
pub fn format_mac(mac: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
