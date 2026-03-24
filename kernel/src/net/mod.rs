use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use smoltcp::iface::{Config as InterfaceConfig, Interface, SocketHandle, SocketSet};
use smoltcp::socket::dhcpv4;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};
use spin::{Lazy, Mutex};

use crate::arch::x86_64::{interrupts, pit};

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
            Self::UnsupportedDevice(reason) => write!(formatter, "unsupported network device: {reason}"),
            Self::InitializationFailed(reason) => {
                write!(formatter, "network initialization failed: {reason}")
            }
            Self::OutOfMemory => formatter.write_str("network DMA allocation failed"),
            Self::QueueFull => formatter.write_str("virtqueue has no free descriptors"),
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
    pub hardware: Option<VirtioDeviceInfo>,
    pub network_config: Option<DhcpConfig>,
}

/// Read-only summary of the detected virtio-net device.
#[derive(Debug, Clone)]
pub struct VirtioDeviceInfo {
    pub mac: [u8; 6],
    pub io_base: u16,
    pub rx_queue_size: u16,
    pub tx_queue_size: u16,
    pub interrupt_line: u8,
    pub pending_frames: usize,
    pub rx_frames: u64,
    pub tx_frames: u64,
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
    hardware: Option<virtio::net::VirtioNet>,
    iface: Option<Interface>,
    sockets: SocketSet<'static>,
    dhcp_handle: Option<SocketHandle>,
    network_config: Option<DhcpConfig>,
    dns_resolver: dns::DnsResolver,
    arp_cache: arp::ArpCache,
    next_local_port: u16,
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
        }
    }

    pub fn init(&mut self) -> Result<NetInitReport, NetError> {
        self.serial.init();
        self.pci_devices = pci::enumerate_pci();

        if self.hardware.is_none() {
            if let Some(device) = pci::find_virtio_net_in(&self.pci_devices) {
                self.hardware = Some(virtio::net::VirtioNet::init(device)?);
            }
        }

        if self.hardware.is_some() {
            self.prepare_stack()?;
            let _ = self.acquire_dhcp(5_000);
        }

        Ok(self.report())
    }

    #[must_use]
    pub fn report(&self) -> NetInitReport {
        NetInitReport {
            pci_devices: self.pci_devices.len(),
            serial_status: self.serial.status(),
            hardware: self.hardware.as_ref().map(virtio::net::VirtioNet::info),
            network_config: self.network_config,
        }
    }

    #[must_use]
    pub fn status(&self) -> String {
        let ip = self
            .network_config
            .map(|config| format!("{} gw {}", config.cidr_string(), config.gateway.unwrap_or(ipv4::Ipv4Addr::ZERO)))
            .unwrap_or_else(|| "unconfigured".into());

        match self.hardware.as_ref() {
            Some(device) => {
                let info = device.info();
                format!(
                    "serial={} | virtio-net={} @ 0x{:04X} | ipv4={}",
                    self.serial.status(),
                    format_mac(&info.mac),
                    info.io_base,
                    ip
                )
            }
            None => format!("serial={} | virtio-net=offline", self.serial.status()),
        }
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
        let timestamp_ms = self.now_ms();
        let timestamp = Instant::from_millis(timestamp_ms as i64);
        let dhcp_handle = self.dhcp_handle;

        let dhcp_event = {
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

            let mut device = KernelDevice {
                nic,
                arp_cache,
                now_ms: timestamp_ms,
            };
            let _ = iface.poll(timestamp, &mut device, sockets);
            dhcp_handle.and_then(|handle| match sockets.get_mut::<dhcpv4::Socket>(handle).poll() {
                Some(dhcpv4::Event::Configured(config)) => {
                    Some(DhcpEventSnapshot::Configured(DhcpConfig::from(&config)))
                }
                Some(dhcpv4::Event::Deconfigured) => Some(DhcpEventSnapshot::Deconfigured),
                None => None,
            })
        };

        if let Some(event) = dhcp_event {
            self.apply_dhcp_event(event);
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
                addrs.push(IpCidr::Ipv4(smoltcp::wire::Ipv4Cidr::new(
                    smoltcp::wire::Ipv4Address::UNSPECIFIED,
                    0,
                )))
                .unwrap();
            }
        });

        self.iface = Some(iface);
        let dhcp_socket = dhcpv4::Socket::new();
        self.dhcp_handle = Some(self.sockets.add(dhcp_socket));
        Ok(())
    }

    fn acquire_dhcp(&mut self, timeout_ms: u64) -> Result<Option<DhcpConfig>, NetError> {
        let deadline = self.now_ms().saturating_add(timeout_ms);
        while self.now_ms() < deadline {
            self.poll_network();
            if self.network_config.is_some() {
                return Ok(self.network_config);
            }
        }
        Ok(None)
    }

    fn apply_dhcp_event(&mut self, event: DhcpEventSnapshot) {
        match event {
            DhcpEventSnapshot::Configured(converted) => {
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
                        let _ = iface.routes_mut().add_default_ipv4_route(gateway.as_smoltcp());
                    } else {
                        iface.routes_mut().remove_default_ipv4_route();
                    }
                }
                self.network_config = Some(converted);
            }
            DhcpEventSnapshot::Deconfigured => {
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

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.hardware
            .as_mut()
            .ok_or(NetError::NoHardware)?
            .send_frame(frame)
    }

    pub fn recv_frame(&mut self) -> Option<Vec<u8>> {
        self.hardware
            .as_mut()
            .and_then(virtio::net::VirtioNet::recv_frame)
    }

    pub fn hardware_diagnostics(&self) -> Option<VirtioNetDiagnostics> {
        self.hardware
            .as_ref()
            .map(virtio::net::VirtioNet::diagnostics)
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
    nic: &'a mut virtio::net::VirtioNet,
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

    fn receive(
        &mut self,
        _timestamp: Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let frame = self.nic.recv_frame()?;
        self.arp_cache.observe_frame(&frame, self.now_ms);
        Some((KernelRxToken { buffer: frame }, KernelTxToken { nic: self.nic }))
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
    nic: &'a mut virtio::net::VirtioNet,
}

impl<'a> smoltcp::phy::TxToken for KernelTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0u8; len];
        let result = f(&mut frame);
        let _ = self.nic.send_frame(&frame);
        result
    }
}

/// Initialize both the legacy serial link and the kernel networking stack.
pub fn init() -> Result<NetInitReport, NetError> {
    NET.lock().init()
}

/// Poll the hardware/network stack.
pub fn poll() -> usize {
    NET.lock().poll_network()
}

/// Snapshot every enumerated PCI device.
#[must_use]
pub fn pci_devices() -> Vec<PciDevice> {
    NET.lock().pci_devices.clone()
}

/// Return the current DHCP-derived network configuration, if any.
#[must_use]
pub fn network_config() -> Option<DhcpConfig> {
    NET.lock().network_config
}

/// Human-readable combined interface status.
#[must_use]
pub fn status() -> String {
    NET.lock().status()
}

/// Return current virtio-net details if the NIC initialized successfully.
#[must_use]
pub fn hardware_status() -> Option<VirtioDeviceInfo> {
    NET.lock().hardware.as_ref().map(virtio::net::VirtioNet::info)
}

/// Return low-level virtio queue/status counters for debugging packet flow.
#[must_use]
pub fn hardware_diagnostics() -> Option<VirtioNetDiagnostics> {
    NET.lock().hardware_diagnostics()
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
pub fn http_post(url: &str, content_type: &str, body: &[u8]) -> Result<http::HttpResponse, NetError> {
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

#[must_use]
pub fn format_mac(mac: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}
