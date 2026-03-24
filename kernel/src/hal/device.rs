#![allow(dead_code)]

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub u32);

#[derive(Debug, Clone)]
pub struct HardwareDevice {
    pub id: DeviceId,
    pub info: DeviceInfo,
    pub driver: DriverState,
    pub status: DeviceStatus,
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub category: DeviceCategory,
    pub bus: BusLocation,
    pub vendor_id: u16,
    pub product_id: u16,
    pub capabilities: DeviceCapabilities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCategory {
    Processor,
    Memory,
    Input,
    Network,
    Storage,
    Display,
    UsbController,
    UsbDevice,
    Audio,
    QuantumProcessor,
    PowerManagement,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BusLocation {
    Pci { bus: u8, device: u8, function: u8 },
    Usb { controller: DeviceId, port: u8, address: u8 },
    Platform,
    Virtual,
}

#[derive(Debug, Clone)]
pub enum DeviceCapabilities {
    Input(InputCapabilities),
    Network(NetworkCapabilities),
    Storage(StorageCapabilities),
    Display(DisplayCapabilities),
    Usb(UsbCapabilities),
    Quantum(QuantumCapabilities),
    None,
}

#[derive(Debug, Clone)]
pub struct InputCapabilities {
    pub has_keyboard: bool,
    pub has_pointer: bool,
    pub has_touch: bool,
    pub layout: KeyboardLayout,
}

#[derive(Debug, Clone)]
pub struct NetworkCapabilities {
    pub max_speed_mbps: u32,
    pub has_wifi: bool,
    pub has_ethernet: bool,
    pub mac_address: [u8; 6],
    pub pq_tls_offload: bool,
}

#[derive(Debug, Clone)]
pub struct StorageCapabilities {
    pub capacity_bytes: u64,
    pub sector_size: u32,
    pub is_removable: bool,
    pub is_readonly: bool,
    pub supports_trim: bool,
}

#[derive(Debug, Clone)]
pub struct DisplayCapabilities {
    pub width: u32,
    pub height: u32,
    pub bpp: u8,
    pub pixel_format: PixelFormat,
}

#[derive(Debug, Clone)]
pub struct UsbCapabilities {
    pub speed: UsbSpeed,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub max_packet_size: u16,
    pub num_endpoints: u8,
}

#[derive(Debug, Clone)]
pub struct QuantumCapabilities {
    pub num_qubits: usize,
    pub native_gates: Vec<String>,
    pub connectivity: Vec<(usize, usize)>,
    pub is_simulator: bool,
    pub coherence_time_us: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStatus {
    Discovered,
    Initialized,
    Active,
    Suspended,
    Error,
    Removed,
}

#[derive(Debug, Clone)]
pub enum DriverState {
    None,
    Loaded(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,
    Full,
    High,
    Super,
    SuperPlus,
    Super2x2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgr,
    Rgb,
    Bgra,
    Rgba,
    Mono,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardLayout {
    UsQwerty,
    BrazilAbnt2,
    German,
    French,
    Japanese,
    UkQwerty,
    Custom,
}

impl DeviceCategory {
    #[must_use]
    pub fn short_name(self) -> &'static str {
        match self {
            Self::Processor => "Processor",
            Self::Memory => "Memory",
            Self::Input => "Input",
            Self::Network => "Network",
            Self::Storage => "Storage",
            Self::Display => "Display",
            Self::UsbController => "UsbCtrl",
            Self::UsbDevice => "UsbDev",
            Self::Audio => "Audio",
            Self::QuantumProcessor => "Quantum",
            Self::PowerManagement => "Power",
            Self::Other => "Other",
        }
    }
}

impl DeviceStatus {
    #[must_use]
    pub fn short_name(self) -> &'static str {
        match self {
            Self::Discovered => "disc",
            Self::Initialized => "init",
            Self::Active => "active",
            Self::Suspended => "suspend",
            Self::Error => "error",
            Self::Removed => "removed",
        }
    }
}

impl DriverState {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::None => "none",
            Self::Loaded(name) => name.as_str(),
        }
    }
}

impl KeyboardLayout {
    #[must_use]
    pub fn short_name(self) -> &'static str {
        match self {
            Self::UsQwerty => "us",
            Self::BrazilAbnt2 => "br",
            Self::German => "de",
            Self::French => "fr",
            Self::Japanese => "jp",
            Self::UkQwerty => "uk",
            Self::Custom => "custom",
        }
    }
}
