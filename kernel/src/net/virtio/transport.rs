use x86_64::instructions::port::Port;

use crate::net::NetError;

use super::queue::Virtqueue;
use super::{
    LEGACY_DEVICE_CONFIG, LEGACY_DEVICE_FEATURES, LEGACY_DEVICE_STATUS, LEGACY_GUEST_FEATURES,
    LEGACY_ISR_STATUS, LEGACY_QUEUE_ADDRESS, LEGACY_QUEUE_NOTIFY, LEGACY_QUEUE_SELECT,
    LEGACY_QUEUE_SIZE,
};

/// Shared helper for the legacy virtio-pci I/O transport used by the current NIC and disk
/// drivers.
pub struct LegacyTransport {
    io_base: u16,
}

impl LegacyTransport {
    pub fn new(io_base: u16) -> Self {
        Self { io_base }
    }

    #[must_use]
    pub fn io_base(&self) -> u16 {
        self.io_base
    }

    pub fn device_features(&mut self) -> u32 {
        self.read_u32(LEGACY_DEVICE_FEATURES)
    }

    pub fn set_guest_features(&mut self, features: u32) {
        self.write_u32(LEGACY_GUEST_FEATURES, features);
    }

    pub fn configure_queue(&mut self, queue_index: u16) -> Result<Virtqueue, NetError> {
        self.write_u16(LEGACY_QUEUE_SELECT, queue_index);
        let queue_size = self.read_u16(LEGACY_QUEUE_SIZE);
        if queue_size == 0 {
            return Err(NetError::InitializationFailed("virtqueue is not present"));
        }

        // Legacy virtio-pci exposes a fixed queue size chosen by the device. Using a smaller
        // guest-side ring corrupts the shared layout because both sides then disagree on where
        // the available and used rings live in physical memory.
        let queue = Virtqueue::new(queue_index, queue_size)?;
        self.write_u32(LEGACY_QUEUE_ADDRESS, queue.pfn());
        Ok(queue)
    }

    pub fn reset(&mut self) {
        self.write_u8(LEGACY_DEVICE_STATUS, 0);
    }

    pub fn status(&mut self) -> u8 {
        self.read_u8(LEGACY_DEVICE_STATUS)
    }

    pub fn set_status(&mut self, status: u8) {
        self.write_u8(LEGACY_DEVICE_STATUS, status);
    }

    pub fn add_status(&mut self, status: u8) {
        let current = self.status();
        self.set_status(current | status);
    }

    #[must_use]
    pub fn read_isr_status(&mut self) -> u8 {
        self.read_u8(LEGACY_ISR_STATUS)
    }

    pub fn notify_queue(&mut self, queue_index: u16) {
        self.write_u16(LEGACY_QUEUE_NOTIFY, queue_index);
    }

    #[must_use]
    pub fn read_config_u8(&mut self, offset: u16) -> u8 {
        self.read_u8(LEGACY_DEVICE_CONFIG + offset)
    }

    #[must_use]
    pub fn read_config_u16(&mut self, offset: u16) -> u16 {
        self.read_u16(LEGACY_DEVICE_CONFIG + offset)
    }

    #[must_use]
    pub fn read_config_u32(&mut self, offset: u16) -> u32 {
        self.read_u32(LEGACY_DEVICE_CONFIG + offset)
    }

    #[must_use]
    pub fn read_mac(&mut self) -> [u8; 6] {
        let mut mac = [0u8; 6];
        for (index, slot) in mac.iter_mut().enumerate() {
            *slot = self.read_config_u8(index as u16);
        }
        mac
    }

    fn read_u8(&mut self, offset: u16) -> u8 {
        let mut port = Port::<u8>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes byte-addressable I/O registers in BAR0.
        unsafe { port.read() }
    }

    fn read_u16(&mut self, offset: u16) -> u16 {
        let mut port = Port::<u16>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes 16-bit queue registers in BAR0.
        unsafe { port.read() }
    }

    fn read_u32(&mut self, offset: u16) -> u32 {
        let mut port = Port::<u32>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes 32-bit feature and queue registers in BAR0.
        unsafe { port.read() }
    }

    fn write_u8(&mut self, offset: u16, value: u8) {
        let mut port = Port::<u8>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes byte-addressable I/O registers in BAR0.
        unsafe {
            port.write(value);
        }
    }

    fn write_u16(&mut self, offset: u16, value: u16) {
        let mut port = Port::<u16>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes 16-bit queue registers in BAR0.
        unsafe {
            port.write(value);
        }
    }

    fn write_u32(&mut self, offset: u16, value: u32) {
        let mut port = Port::<u32>::new(self.io_base + offset);
        // SAFETY: The legacy virtio transport exposes 32-bit feature and queue registers in BAR0.
        unsafe {
            port.write(value);
        }
    }
}
