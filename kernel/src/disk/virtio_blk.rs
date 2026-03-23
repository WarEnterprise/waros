use core::hint::spin_loop;
use core::mem::size_of;
use core::ptr;

use x86_64::PhysAddr;

use crate::net::buffer::DmaRegion;
use crate::net::pci::{self, PciBar, PciDevice};
use crate::net::virtio::queue::Virtqueue;
use crate::net::virtio::transport::LegacyTransport;
use crate::net::virtio::{
    STATUS_ACKNOWLEDGE, STATUS_DRIVER, STATUS_DRIVER_OK, STATUS_FAILED, STATUS_FEATURES_OK,
    VIRTQ_DESC_F_NEXT, VIRTQ_DESC_F_WRITE,
};
use crate::net::NetError;

use super::format::SECTOR_SIZE;
use super::DiskError;

pub const VIRTIO_BLK_T_IN: u32 = 0;
pub const VIRTIO_BLK_T_OUT: u32 = 1;

const HEADER_DESC_ID: u16 = 0;
const DATA_DESC_ID: u16 = 1;
const STATUS_DESC_ID: u16 = 2;
const DATA_OFFSET: usize = size_of::<BlkReqHeader>();
const STATUS_OFFSET: usize = DATA_OFFSET + SECTOR_SIZE;
const REQUEST_REGION_SIZE: usize = STATUS_OFFSET + 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct BlkReqHeader {
    pub req_type: u32,
    pub reserved: u32,
    pub sector: u64,
}

/// Polled legacy virtio-blk device using one synchronous request queue.
pub struct VirtioBlk {
    pci: PciDevice,
    io_base: u16,
    transport: LegacyTransport,
    queue: Virtqueue,
    request_region: DmaRegion,
    pub capacity_sectors: u64,
    pub sector_size: u32,
    pub disk_size: u64,
}

impl VirtioBlk {
    pub fn init(pci: PciDevice) -> Result<Self, DiskError> {
        let PciBar::Io(io_base) = pci.bar(0) else {
            return Err(DiskError::UnsupportedDevice(
                "virtio-blk requires the legacy I/O transport in BAR0",
            ));
        };

        pci::enable_bus_mastering(&pci);

        let mut transport = LegacyTransport::new(io_base);
        transport.reset();
        transport.set_status(STATUS_ACKNOWLEDGE);
        transport.add_status(STATUS_DRIVER);
        transport.set_guest_features(0);
        transport.add_status(STATUS_FEATURES_OK);
        if transport.status() & STATUS_FEATURES_OK == 0 {
            transport.add_status(STATUS_FAILED);
            return Err(DiskError::InitFailed(
                "virtio-blk feature negotiation was rejected",
            ));
        }

        let capacity_lo = transport.read_config_u32(0) as u64;
        let capacity_hi = transport.read_config_u32(4) as u64;
        let capacity_sectors = (capacity_hi << 32) | capacity_lo;
        if capacity_sectors == 0 {
            return Err(DiskError::InitFailed("virtio-blk reported zero capacity"));
        }

        let queue = transport.configure_queue(0).map_err(DiskError::from)?;
        if queue.size < 3 {
            return Err(DiskError::InitFailed(
                "virtio-blk queue is too small for 3-descriptor requests",
            ));
        }

        transport.add_status(STATUS_DRIVER_OK);

        Ok(Self {
            pci,
            io_base,
            transport,
            queue,
            request_region: DmaRegion::allocate(REQUEST_REGION_SIZE).map_err(DiskError::from)?,
            capacity_sectors,
            sector_size: SECTOR_SIZE as u32,
            disk_size: capacity_sectors * SECTOR_SIZE as u64,
        })
    }

    #[must_use]
    pub fn pci_device(&self) -> PciDevice {
        self.pci
    }

    #[must_use]
    pub fn io_base(&self) -> u16 {
        self.io_base
    }

    pub fn read_sectors(
        &mut self,
        sector: u64,
        count: u32,
        buffer: &mut [u8],
    ) -> Result<(), DiskError> {
        self.validate_request(sector, count, buffer.len())?;

        for index in 0..count as usize {
            let offset = index * SECTOR_SIZE;
            self.do_request(
                VIRTIO_BLK_T_IN,
                sector + index as u64,
                &mut buffer[offset..offset + SECTOR_SIZE],
            )?;
        }

        Ok(())
    }

    pub fn write_sectors(
        &mut self,
        sector: u64,
        count: u32,
        buffer: &[u8],
    ) -> Result<(), DiskError> {
        self.validate_request(sector, count, buffer.len())?;

        for index in 0..count as usize {
            let offset = index * SECTOR_SIZE;
            let mut sector_buffer = [0u8; SECTOR_SIZE];
            sector_buffer.copy_from_slice(&buffer[offset..offset + SECTOR_SIZE]);
            self.do_request(VIRTIO_BLK_T_OUT, sector + index as u64, &mut sector_buffer)?;
        }

        Ok(())
    }

    fn validate_request(
        &self,
        sector: u64,
        count: u32,
        buffer_len: usize,
    ) -> Result<(), DiskError> {
        if sector.saturating_add(count as u64) > self.capacity_sectors {
            return Err(DiskError::OutOfBounds);
        }
        if buffer_len < count as usize * SECTOR_SIZE {
            return Err(DiskError::BufferTooSmall);
        }
        Ok(())
    }

    fn do_request(
        &mut self,
        req_type: u32,
        sector: u64,
        buffer: &mut [u8],
    ) -> Result<(), DiskError> {
        if buffer.len() != SECTOR_SIZE {
            return Err(DiskError::BufferTooSmall);
        }

        let header = BlkReqHeader {
            req_type,
            reserved: 0,
            sector,
        };

        let base = self.request_region.physical().as_u64();
        let request = self.request_region.slice_mut();
        request.fill(0);
        // SAFETY: `request` points to a DMA region that is large enough for the header.
        unsafe {
            ptr::write_unaligned(request.as_mut_ptr().cast::<BlkReqHeader>(), header);
        }

        if req_type == VIRTIO_BLK_T_OUT {
            request[DATA_OFFSET..DATA_OFFSET + SECTOR_SIZE].copy_from_slice(buffer);
        }

        let header_phys = PhysAddr::new(base);
        let data_phys = PhysAddr::new(base + DATA_OFFSET as u64);
        let status_phys = PhysAddr::new(base + STATUS_OFFSET as u64);

        self.queue.set_descriptor(
            HEADER_DESC_ID,
            header_phys,
            size_of::<BlkReqHeader>() as u32,
            VIRTQ_DESC_F_NEXT,
            DATA_DESC_ID,
        )?;

        let data_flags = if req_type == VIRTIO_BLK_T_IN {
            VIRTQ_DESC_F_NEXT | VIRTQ_DESC_F_WRITE
        } else {
            VIRTQ_DESC_F_NEXT
        };
        self.queue.set_descriptor(
            DATA_DESC_ID,
            data_phys,
            SECTOR_SIZE as u32,
            data_flags,
            STATUS_DESC_ID,
        )?;
        self.queue.set_descriptor(
            STATUS_DESC_ID,
            status_phys,
            1,
            VIRTQ_DESC_F_WRITE,
            0,
        )?;
        self.queue.add_available(HEADER_DESC_ID)?;
        self.transport.notify_queue(0);

        while self.queue.pop_used().is_none() {
            spin_loop();
        }

        let status = request[STATUS_OFFSET];
        let _ = self.transport.read_isr_status();
        if status != 0 {
            return Err(DiskError::DeviceError);
        }

        if req_type == VIRTIO_BLK_T_IN {
            buffer.copy_from_slice(&request[DATA_OFFSET..DATA_OFFSET + SECTOR_SIZE]);
        }

        Ok(())
    }
}

#[must_use]
pub fn find_virtio_blk() -> Option<PciDevice> {
    pci::enumerate_pci().into_iter().find(|device| {
        device.vendor_id == 0x1AF4
            && matches!(device.device_id, 0x1001 | 0x1042)
            && device.class_code == 0x01
    })
}

impl From<NetError> for DiskError {
    fn from(error: NetError) -> Self {
        match error {
            NetError::InitializationFailed(reason) => Self::InitFailed(reason),
            NetError::UnsupportedDevice(reason) => Self::UnsupportedDevice(reason),
            NetError::OutOfMemory => Self::OutOfMemory,
            _ => Self::DeviceError,
        }
    }
}
