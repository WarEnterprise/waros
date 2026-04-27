#![allow(dead_code)]

use alloc::vec::Vec;

use crate::hal::device::DeviceCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointDirection {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Control,
    Isochronous,
    Bulk,
    Interrupt,
}

#[derive(Debug, Clone)]
pub struct UsbEndpoint {
    pub address: u8,
    pub direction: EndpointDirection,
    pub transfer_type: TransferType,
    pub max_packet_size: u16,
    pub interval: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HidDescriptorInfo {
    pub report_descriptor_length: u16,
}

#[derive(Debug, Clone)]
pub struct UsbInterface {
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoints: Vec<UsbEndpoint>,
    pub hid: Option<HidDescriptorInfo>,
}

#[derive(Debug, Clone)]
pub struct UsbConfiguration {
    pub configuration_value: u8,
    pub attributes: u8,
    pub max_power_ma: u16,
    pub interfaces: Vec<UsbInterface>,
}

impl UsbInterface {
    #[must_use]
    pub fn is_hid(&self) -> bool {
        self.class == 0x03
    }

    #[must_use]
    pub fn is_mass_storage(&self) -> bool {
        self.class == 0x08
    }

    #[must_use]
    pub fn boot_keyboard(&self) -> bool {
        self.class == 0x03 && self.subclass == 0x01 && self.protocol == 0x01
    }

    #[must_use]
    pub fn boot_mouse(&self) -> bool {
        self.class == 0x03 && self.subclass == 0x01 && self.protocol == 0x02
    }

    /// CDC-ECM: Communications Device Class - Ethernet Control Model
    /// Class 0x02 (Communications), Subclass 0x06 (ECM)
    #[must_use]
    pub fn is_cdc_ecm(&self) -> bool {
        self.class == 0x02 && self.subclass == 0x06
    }

    /// CDC-NCM: Communications Device Class - Network Control Model
    /// Class 0x02 (Communications), Subclass 0x0D (NCM)
    #[must_use]
    pub fn is_cdc_ncm(&self) -> bool {
        self.class == 0x02 && self.subclass == 0x0D
    }

    /// RNDIS: Microsoft Remote NDIS (class 0x02, subclass 0x02, protocol 0xFF),
    /// wireless-class RNDIS (0xE0, 0x01, 0x03), or the Android/Windows
    /// composite profile exposed as MISC/04/01.
    #[must_use]
    pub fn is_rndis(&self) -> bool {
        (self.class == 0x02 && self.subclass == 0x02 && self.protocol == 0xFF)
            || (self.class == 0xE0 && self.subclass == 0x01 && self.protocol == 0x03)
            || (self.class == 0xEF && self.subclass == 0x04 && self.protocol == 0x01)
            || (self.class == 0xEF && self.subclass == 0x01 && self.protocol == 0x01)
    }

    /// CDC-EEM: Ethernet Emulation Model (used by some USB tethering)
    #[must_use]
    pub fn is_cdc_eem(&self) -> bool {
        self.class == 0x02 && self.subclass == 0x0C
    }

    /// Any USB network-class interface (CDC-ECM, CDC-NCM, RNDIS, CDC-EEM)
    #[must_use]
    pub fn is_network(&self) -> bool {
        self.is_cdc_ecm() || self.is_cdc_ncm() || self.is_rndis() || self.is_cdc_eem()
    }

    /// CDC Data interface (paired with a CDC control interface)
    #[must_use]
    pub fn is_cdc_data(&self) -> bool {
        self.class == 0x0A
    }

    #[must_use]
    pub fn is_ptp_mtp_like(&self) -> bool {
        self.class == 0x06
    }

    #[must_use]
    pub fn is_adb(&self) -> bool {
        self.class == 0xFF && self.subclass == 0x42 && self.protocol == 0x01
    }

    #[must_use]
    pub fn is_vendor_specific(&self) -> bool {
        self.class == 0xFF
    }

    #[must_use]
    pub fn has_bulk_in(&self) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.direction == EndpointDirection::In
                && endpoint.transfer_type == TransferType::Bulk
        })
    }

    #[must_use]
    pub fn has_bulk_out(&self) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.direction == EndpointDirection::Out
                && endpoint.transfer_type == TransferType::Bulk
        })
    }

    #[must_use]
    pub fn has_interrupt_in(&self) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.direction == EndpointDirection::In
                && endpoint.transfer_type == TransferType::Interrupt
        })
    }

    #[must_use]
    pub fn has_interrupt_out(&self) -> bool {
        self.endpoints.iter().any(|endpoint| {
            endpoint.direction == EndpointDirection::Out
                && endpoint.transfer_type == TransferType::Interrupt
        })
    }

    /// Human-readable protocol name for USB network interfaces.
    #[must_use]
    pub fn usb_net_protocol_name(&self) -> &'static str {
        if self.is_cdc_ecm() {
            "CDC-ECM"
        } else if self.is_cdc_ncm() {
            "CDC-NCM"
        } else if self.is_rndis() {
            "RNDIS"
        } else if self.is_cdc_eem() {
            "CDC-EEM"
        } else if self.is_cdc_data() {
            "CDC-Data"
        } else {
            "unknown"
        }
    }
}

pub fn parse_configuration_descriptors(data: &[u8]) -> Result<UsbConfiguration, &'static str> {
    if data.len() < 9 {
        return Err("USB configuration descriptor too short");
    }
    if data[1] != 0x02 {
        return Err("USB configuration descriptor missing header");
    }

    let total_length = u16::from_le_bytes([data[2], data[3]]) as usize;
    let parse_len = total_length.min(data.len());
    let configuration_value = data[5];
    let attributes = data[7];
    let max_power_ma = u16::from(data[8]) * 2;

    let mut interfaces = Vec::new();
    let mut offset = 9;

    while offset + 2 <= parse_len {
        let length = data[offset] as usize;
        let descriptor_type = data[offset + 1];
        if length < 2 || offset + length > parse_len {
            break;
        }

        let descriptor = &data[offset..offset + length];
        match descriptor_type {
            0x04 if length >= 9 => {
                interfaces.push(UsbInterface {
                    number: descriptor[2],
                    alternate_setting: descriptor[3],
                    class: descriptor[5],
                    subclass: descriptor[6],
                    protocol: descriptor[7],
                    endpoints: Vec::new(),
                    hid: None,
                });
            }
            0x05 if length >= 7 => {
                let Some(interface) = interfaces.last_mut() else {
                    offset += length;
                    continue;
                };
                let address = descriptor[2];
                interface.endpoints.push(UsbEndpoint {
                    address,
                    direction: if address & 0x80 != 0 {
                        EndpointDirection::In
                    } else {
                        EndpointDirection::Out
                    },
                    transfer_type: match descriptor[3] & 0x03 {
                        0 => TransferType::Control,
                        1 => TransferType::Isochronous,
                        2 => TransferType::Bulk,
                        _ => TransferType::Interrupt,
                    },
                    max_packet_size: u16::from_le_bytes([descriptor[4], descriptor[5]]),
                    interval: descriptor[6],
                });
            }
            0x21 if length >= 9 => {
                if let Some(interface) = interfaces.last_mut() {
                    interface.hid = Some(HidDescriptorInfo {
                        report_descriptor_length: u16::from_le_bytes([
                            descriptor[7],
                            descriptor[8],
                        ]),
                    });
                }
            }
            _ => {}
        }

        offset += length;
    }

    Ok(UsbConfiguration {
        configuration_value,
        attributes,
        max_power_ma,
        interfaces,
    })
}

#[must_use]
pub fn classify_device(device_class: u8, interfaces: &[UsbInterface]) -> DeviceCategory {
    match device_class {
        0x02 => {
            // Communications Device Class — check for network sub-protocols
            if interfaces.iter().any(UsbInterface::is_network) {
                return DeviceCategory::Network;
            }
            return DeviceCategory::UsbDevice;
        }
        0x03 => return DeviceCategory::Input,
        0x08 => return DeviceCategory::Storage,
        0x09 => return DeviceCategory::UsbDevice,
        0xE0 | 0xEF => {
            // Wireless / Miscellaneous — RNDIS can live here
            if interfaces.iter().any(UsbInterface::is_rndis) {
                return DeviceCategory::Network;
            }
            return DeviceCategory::UsbDevice;
        }
        _ => {}
    }

    // Interface-level classification (composite devices often set device_class=0x00)
    if interfaces.iter().any(UsbInterface::is_network) {
        return DeviceCategory::Network;
    }
    if interfaces.iter().any(UsbInterface::is_hid) {
        return DeviceCategory::Input;
    }
    if interfaces.iter().any(UsbInterface::is_mass_storage) {
        return DeviceCategory::Storage;
    }

    DeviceCategory::UsbDevice
}

/// For a USB network device, identify the specific protocol in use.
#[must_use]
pub fn usb_net_protocol(interfaces: &[UsbInterface]) -> &'static str {
    for iface in interfaces {
        if iface.is_cdc_ecm() {
            return "CDC-ECM";
        }
        if iface.is_cdc_ncm() {
            return "CDC-NCM";
        }
        if iface.is_rndis() {
            return "RNDIS";
        }
        if iface.is_cdc_eem() {
            return "CDC-EEM";
        }
    }
    "unknown"
}
