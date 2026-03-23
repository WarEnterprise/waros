pub mod net;
pub mod queue;

pub const STATUS_ACKNOWLEDGE: u8 = 0x01;
pub const STATUS_DRIVER: u8 = 0x02;
pub const STATUS_DRIVER_OK: u8 = 0x04;
pub const STATUS_FEATURES_OK: u8 = 0x08;
pub const STATUS_FAILED: u8 = 0x80;

pub const VIRTQ_DESC_F_NEXT: u16 = 0x0001;
pub const VIRTQ_DESC_F_WRITE: u16 = 0x0002;

pub const LEGACY_DEVICE_FEATURES: u16 = 0x00;
pub const LEGACY_GUEST_FEATURES: u16 = 0x04;
pub const LEGACY_QUEUE_ADDRESS: u16 = 0x08;
pub const LEGACY_QUEUE_SIZE: u16 = 0x0C;
pub const LEGACY_QUEUE_SELECT: u16 = 0x0E;
pub const LEGACY_QUEUE_NOTIFY: u16 = 0x10;
pub const LEGACY_DEVICE_STATUS: u16 = 0x12;
pub const LEGACY_ISR_STATUS: u16 = 0x13;
pub const LEGACY_DEVICE_CONFIG: u16 = 0x14;

pub const VIRTIO_NET_F_MAC: u32 = 5;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioNetHeader {
    pub flags: u8,
    pub gso_type: u8,
    pub hdr_len: u16,
    pub gso_size: u16,
    pub csum_start: u16,
    pub csum_offset: u16,
}
