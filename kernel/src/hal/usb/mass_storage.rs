#![allow(dead_code)]

use alloc::string::{String, ToString};

use crate::disk::DiskError;
use crate::hal::traits::StorageDriver;
use crate::hal::DeviceId;

pub const USB_MASS_STORAGE_CLASS: u8 = 0x08;
pub const USB_MASS_STORAGE_SCSI_SUBCLASS: u8 = 0x06;
pub const USB_MASS_STORAGE_BULK_ONLY_PROTOCOL: u8 = 0x50;

pub const CBW_SIGNATURE: u32 = 0x4342_5355;
pub const CSW_SIGNATURE: u32 = 0x5342_5355;

#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct Cbw {
    pub signature: u32,
    pub tag: u32,
    pub data_length: u32,
    pub flags: u8,
    pub lun: u8,
    pub cb_length: u8,
    pub cb: [u8; 16],
}

impl Cbw {
    #[must_use]
    pub fn new(tag: u32, data_length: u32, data_in: bool, command: &[u8], cmd_len: u8) -> Self {
        let mut cb = [0u8; 16];
        let copy_len = command.len().min(cb.len());
        cb[..copy_len].copy_from_slice(&command[..copy_len]);
        Self {
            signature: CBW_SIGNATURE,
            tag,
            data_length,
            flags: if data_in { 0x80 } else { 0x00 },
            lun: 0,
            cb_length: cmd_len,
            cb,
        }
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const Self as *const u8, 31) }
    }
}

#[repr(C, packed)]
#[derive(Clone, Copy, Default)]
pub struct Csw {
    pub signature: u32,
    pub tag: u32,
    pub data_residue: u32,
    pub status: u8,
}

impl Csw {
    pub fn from_bytes(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 13 {
            return Err("USB CSW too short");
        }
        Ok(Self {
            signature: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            tag: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            data_residue: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
            status: data[12],
        })
    }

    pub fn validate(&self, expected_tag: u32) -> Result<(), &'static str> {
        if self.signature != CSW_SIGNATURE {
            return Err("USB CSW signature mismatch");
        }
        if self.tag != expected_tag {
            return Err("USB CSW tag mismatch");
        }
        if self.status != 0 {
            return Err("USB mass storage command failed");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UsbMassStorageInfo {
    pub capacity_sectors: u64,
    pub sector_size: u32,
}

impl UsbMassStorageInfo {
    #[must_use]
    pub fn capacity_bytes(self) -> u64 {
        self.capacity_sectors.saturating_mul(u64::from(self.sector_size))
    }
}

pub struct UsbMassStorageDevice {
    pub hal_id: DeviceId,
    pub controller_id: DeviceId,
    pub slot_id: u8,
    pub name: String,
    pub info: UsbMassStorageInfo,
}

impl StorageDriver for UsbMassStorageDevice {
    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn device_id(&self) -> DeviceId {
        self.hal_id
    }

    fn capacity_sectors(&self) -> u64 {
        self.info.capacity_sectors
    }

    fn sector_size(&self) -> u32 {
        self.info.sector_size
    }

    fn read_sectors(
        &mut self,
        _sector: u64,
        _count: u32,
        _buf: &mut [u8],
    ) -> Result<(), DiskError> {
        Err(DiskError::UnsupportedDevice(
            "USB storage reads are mediated by the xHCI controller registry",
        ))
    }

    fn write_sectors(&mut self, _sector: u64, _count: u32, _buf: &[u8]) -> Result<(), DiskError> {
        Err(DiskError::UnsupportedDevice(
            "USB storage writes are mediated by the xHCI controller registry",
        ))
    }

    fn flush(&mut self) -> Result<(), DiskError> {
        Ok(())
    }

    fn is_removable(&self) -> bool {
        true
    }
}

#[must_use]
pub fn is_mass_storage_interface(class: u8, subclass: u8, protocol: u8) -> bool {
    class == USB_MASS_STORAGE_CLASS
        && subclass == USB_MASS_STORAGE_SCSI_SUBCLASS
        && protocol == USB_MASS_STORAGE_BULK_ONLY_PROTOCOL
}

#[must_use]
pub fn scsi_inquiry_command() -> [u8; 16] {
    let mut command = [0u8; 16];
    command[0] = 0x12;
    command[4] = 36;
    command
}

#[must_use]
pub fn scsi_read_capacity_10_command() -> [u8; 16] {
    let mut command = [0u8; 16];
    command[0] = 0x25;
    command
}

#[must_use]
pub fn scsi_read_10_command(lba: u32, sectors: u16) -> [u8; 16] {
    let mut command = [0u8; 16];
    command[0] = 0x28;
    command[2..6].copy_from_slice(&lba.to_be_bytes());
    command[7..9].copy_from_slice(&sectors.to_be_bytes());
    command
}

#[must_use]
pub fn scsi_write_10_command(lba: u32, sectors: u16) -> [u8; 16] {
    let mut command = [0u8; 16];
    command[0] = 0x2A;
    command[2..6].copy_from_slice(&lba.to_be_bytes());
    command[7..9].copy_from_slice(&sectors.to_be_bytes());
    command
}

pub fn parse_inquiry_strings(data: &[u8]) -> (String, String) {
    let vendor = core::str::from_utf8(data.get(8..16).unwrap_or(&[]))
        .unwrap_or("")
        .trim()
        .to_string();
    let product = core::str::from_utf8(data.get(16..32).unwrap_or(&[]))
        .unwrap_or("")
        .trim()
        .to_string();
    (vendor, product)
}

pub fn parse_read_capacity_10(data: &[u8]) -> Result<UsbMassStorageInfo, &'static str> {
    if data.len() < 8 {
        return Err("USB READ CAPACITY(10) reply too short");
    }

    let last_lba = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let sector_size = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    Ok(UsbMassStorageInfo {
        capacity_sectors: u64::from(last_lba) + 1,
        sector_size,
    })
}
