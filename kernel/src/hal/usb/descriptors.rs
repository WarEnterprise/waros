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
                        report_descriptor_length: u16::from_le_bytes([descriptor[7], descriptor[8]]),
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
        0x03 => return DeviceCategory::Input,
        0x08 => return DeviceCategory::Storage,
        0x09 => return DeviceCategory::UsbDevice,
        _ => {}
    }

    if interfaces.iter().any(UsbInterface::is_hid) {
        return DeviceCategory::Input;
    }
    if interfaces.iter().any(UsbInterface::is_mass_storage) {
        return DeviceCategory::Storage;
    }

    DeviceCategory::UsbDevice
}
