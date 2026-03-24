use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr;

use x86_64::instructions::port::Port;

use crate::net::buffer::PacketBuffer;
use crate::net::pci::{self, PciBar, PciDevice};
use crate::net::{NetError, NetworkDeviceInfo, NetworkTransport};

use super::queue::{Virtqueue, VirtqueueSnapshot};
use super::transport::LegacyTransport;
use super::{
    LEGACY_DEVICE_STATUS, LEGACY_ISR_STATUS, LEGACY_QUEUE_NOTIFY, STATUS_ACKNOWLEDGE,
    STATUS_DRIVER, STATUS_DRIVER_OK, STATUS_FEATURES_OK, STATUS_FAILED, VIRTIO_NET_F_MAC,
    VIRTQ_DESC_F_WRITE, VirtioNetHeader,
};

const NETWORK_BUFFER_SIZE: usize = 2048;

#[derive(Debug, Clone, Copy)]
pub struct VirtioNetDiagnostics {
    pub device_status: u8,
    pub pci_command: u16,
    pub rx_queue: VirtqueueSnapshot,
    pub tx_queue: VirtqueueSnapshot,
    pub rx_buffers: usize,
    pub tx_buffers: usize,
    pub tx_free: usize,
    pub pending_frames: usize,
    pub rx_frames: u64,
    pub tx_frames: u64,
}

/// Polled virtio-net device backed by the legacy PCI I/O transport.
pub struct VirtioNet {
    pci: PciDevice,
    io_base: u16,
    mac: [u8; 6],
    rx_queue: Virtqueue,
    tx_queue: Virtqueue,
    rx_buffers: Vec<PacketBuffer>,
    tx_buffers: Vec<PacketBuffer>,
    tx_free: Vec<u16>,
    pending_rx: VecDeque<Vec<u8>>,
    rx_frames: u64,
    tx_frames: u64,
}

impl VirtioNet {
    pub fn init(pci: PciDevice) -> Result<Self, NetError> {
        let PciBar::Io(io_base) = pci.bar(0) else {
            return Err(NetError::UnsupportedDevice(
                "virtio-net Phase A expects a legacy I/O BAR in BAR0",
            ));
        };

        pci::enable_bus_mastering(&pci);
        let mut transport = LegacyTransport::new(io_base);
        transport.reset();
        transport.set_status(STATUS_ACKNOWLEDGE);
        transport.add_status(STATUS_DRIVER);

        let device_features = transport.device_features();
        let guest_features = if device_features & (1 << VIRTIO_NET_F_MAC) != 0 {
            1 << VIRTIO_NET_F_MAC
        } else {
            0
        };
        transport.set_guest_features(guest_features);
        transport.add_status(STATUS_FEATURES_OK);
        if transport.status() & STATUS_FEATURES_OK == 0 {
            transport.add_status(STATUS_FAILED);
            return Err(NetError::InitializationFailed(
                "virtio feature negotiation was rejected",
            ));
        }

        let mac = transport.read_mac();
        let rx_queue = transport.configure_queue(0)?;
        let tx_queue = transport.configure_queue(1)?;

        let mut device = Self {
            pci,
            io_base,
            mac,
            rx_queue,
            tx_queue,
            rx_buffers: Vec::new(),
            tx_buffers: Vec::new(),
            tx_free: Vec::new(),
            pending_rx: VecDeque::new(),
            rx_frames: 0,
            tx_frames: 0,
        };

        device.prime_rx_queue()?;
        device.prime_tx_queue()?;
        transport.add_status(STATUS_DRIVER_OK);
        Ok(device)
    }

    #[must_use]
    pub fn info(&self) -> NetworkDeviceInfo {
        NetworkDeviceInfo {
            name: "VirtIO Network",
            driver: "virtio-net",
            mac: self.mac,
            transport: NetworkTransport::Io(self.io_base),
            rx_queue_size: self.rx_queue.size,
            tx_queue_size: self.tx_queue.size,
            interrupt_line: self.pci.interrupt_line,
            pending_frames: self.pending_rx.len(),
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
            link_speed_mbps: 1000,
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> VirtioNetDiagnostics {
        let pci_command =
            pci::pci_config_read32(self.pci.bus, self.pci.device, self.pci.function, 0x04) as u16;

        VirtioNetDiagnostics {
            device_status: self.read_device_status(),
            pci_command,
            rx_queue: self.rx_queue.snapshot(),
            tx_queue: self.tx_queue.snapshot(),
            rx_buffers: self.rx_buffers.len(),
            tx_buffers: self.tx_buffers.len(),
            tx_free: self.tx_free.len(),
            pending_frames: self.pending_rx.len(),
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
        }
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.reclaim_tx_descriptors();

        let descriptor_id = self.tx_free.pop().ok_or(NetError::QueueFull)?;
        let buffer = &mut self.tx_buffers[usize::from(descriptor_id)];
        let header_len = size_of::<VirtioNetHeader>();
        let total_len = header_len + frame.len();
        if total_len > buffer.capacity() {
            self.tx_free.push(descriptor_id);
            return Err(NetError::PayloadTooLarge);
        }

        // SAFETY: `buffer` owns `capacity()` bytes and `header_len <= capacity()`.
        unsafe {
            ptr::write_unaligned(
                buffer.as_mut_ptr().cast::<VirtioNetHeader>(),
                VirtioNetHeader::default(),
            );
        }
        buffer[header_len..total_len].copy_from_slice(frame);

        self.tx_queue.set_descriptor(
            descriptor_id,
            buffer.physical(),
            total_len as u32,
            0,
            0,
        )?;
        self.tx_queue.add_available(descriptor_id)?;
        self.notify_queue(1);
        self.tx_frames = self.tx_frames.saturating_add(1);
        Ok(())
    }

    #[must_use]
    pub fn recv_frame(&mut self) -> Option<Vec<u8>> {
        self.poll();
        self.pending_rx.pop_front()
    }

    pub fn poll(&mut self) -> usize {
        self.reclaim_tx_descriptors();
        let mut harvested = 0usize;
        let header_len = size_of::<VirtioNetHeader>();

        while let Some(used) = self.rx_queue.pop_used() {
            let descriptor_id = used.id as usize;
            if descriptor_id >= self.rx_buffers.len() {
                continue;
            }

            let written = used.len as usize;
            if written > header_len {
                let payload = self.rx_buffers[descriptor_id][header_len..written].to_vec();
                self.pending_rx.push_back(payload);
                self.rx_frames = self.rx_frames.saturating_add(1);
                harvested += 1;
            }

            let descriptor_index = descriptor_id as u16;
            let buffer = &self.rx_buffers[descriptor_id];
            let _ = self.rx_queue.set_descriptor(
                descriptor_index,
                buffer.physical(),
                buffer.capacity() as u32,
                VIRTQ_DESC_F_WRITE,
                0,
            );
            let _ = self.rx_queue.add_available(descriptor_index);
        }

        if harvested != 0 {
            self.notify_queue(0);
        }

        let _ = self.read_isr_status();
        harvested
    }

    #[allow(dead_code)]
    pub fn handle_interrupt(&mut self) {
        let _ = self.read_isr_status();
        let _ = self.poll();
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn mac_address(&self) -> [u8; 6] {
        self.mac
    }

    fn prime_rx_queue(&mut self) -> Result<(), NetError> {
        self.rx_buffers = Vec::with_capacity(usize::from(self.rx_queue.size));
        for descriptor_id in 0..self.rx_queue.size {
            let buffer = PacketBuffer::new(NETWORK_BUFFER_SIZE)?;
            self.rx_queue.set_descriptor(
                descriptor_id,
                buffer.physical(),
                buffer.capacity() as u32,
                VIRTQ_DESC_F_WRITE,
                0,
            )?;
            self.rx_queue.add_available(descriptor_id)?;
            self.rx_buffers.push(buffer);
        }
        self.notify_queue(0);
        Ok(())
    }

    fn prime_tx_queue(&mut self) -> Result<(), NetError> {
        self.tx_buffers = Vec::with_capacity(usize::from(self.tx_queue.size));
        self.tx_free = Vec::with_capacity(usize::from(self.tx_queue.size));
        for descriptor_id in 0..self.tx_queue.size {
            let buffer = PacketBuffer::new(NETWORK_BUFFER_SIZE)?;
            self.tx_queue
                .set_descriptor(descriptor_id, buffer.physical(), 0, 0, 0)?;
            self.tx_buffers.push(buffer);
            self.tx_free.push(descriptor_id);
        }
        Ok(())
    }

    fn reclaim_tx_descriptors(&mut self) {
        while let Some(used) = self.tx_queue.pop_used() {
            let descriptor_id = used.id as u16;
            if !self.tx_free.contains(&descriptor_id) {
                self.tx_free.push(descriptor_id);
            }
        }
    }

    fn notify_queue(&self, queue_index: u16) {
        let mut notify = Port::<u16>::new(self.io_base + LEGACY_QUEUE_NOTIFY);
        // SAFETY: `self.io_base` points to the virtio legacy I/O BAR, and queue indices are device-defined.
        unsafe {
            notify.write(queue_index);
        }
    }

    fn read_isr_status(&self) -> u8 {
        let mut isr = Port::<u8>::new(self.io_base + LEGACY_ISR_STATUS);
        // SAFETY: `self.io_base` points to the virtio legacy I/O BAR ISR status register.
        unsafe { isr.read() }
    }

    fn read_device_status(&self) -> u8 {
        let mut status = Port::<u8>::new(self.io_base + LEGACY_DEVICE_STATUS);
        // SAFETY: `self.io_base` points to the virtio legacy I/O BAR status register.
        unsafe { status.read() }
    }
}
