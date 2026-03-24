#![allow(dead_code)]

use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use x86_64::PhysAddr;

use crate::memory;
use crate::net::buffer::{DmaRegion, PacketBuffer};
use crate::net::pci::{self, PciBar, PciDevice};
use crate::net::{NetError, NetworkDeviceInfo, NetworkTransport};

const NUM_TX_DESCS: usize = 64;
const NUM_RX_DESCS: usize = 64;
const BUFFER_SIZE: usize = 2048;

const E1000_CTRL: u64 = 0x0000;
const E1000_STATUS: u64 = 0x0008;
const E1000_IMC: u64 = 0x00D8;
const E1000_RCTL: u64 = 0x0100;
const E1000_TCTL: u64 = 0x0400;
const E1000_TIPG: u64 = 0x0410;
const E1000_RDBAL: u64 = 0x2800;
const E1000_RDBAH: u64 = 0x2804;
const E1000_RDLEN: u64 = 0x2808;
const E1000_RDH: u64 = 0x2810;
const E1000_RDT: u64 = 0x2818;
const E1000_TDBAL: u64 = 0x3800;
const E1000_TDBAH: u64 = 0x3804;
const E1000_TDLEN: u64 = 0x3808;
const E1000_TDH: u64 = 0x3810;
const E1000_TDT: u64 = 0x3818;
const E1000_RAL: u64 = 0x5400;
const E1000_RAH: u64 = 0x5404;

const CTRL_RST: u32 = 1 << 26;
const CTRL_SLU: u32 = 1 << 6;
const STATUS_LINK_UP: u32 = 1 << 1;

const RCTL_EN: u32 = 1 << 1;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_SECRC: u32 = 1 << 26;

const TCTL_EN: u32 = 1 << 1;
const TCTL_PSP: u32 = 1 << 3;

const TX_CMD_EOP: u8 = 1 << 0;
const TX_CMD_IFCS: u8 = 1 << 1;
const TX_CMD_RS: u8 = 1 << 3;
const TX_STATUS_DD: u8 = 1 << 0;
const RX_STATUS_DD: u8 = 1 << 0;

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct E1000TxDesc {
    buffer_addr: u64,
    length: u16,
    cso: u8,
    cmd: u8,
    status: u8,
    css: u8,
    special: u16,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct E1000RxDesc {
    buffer_addr: u64,
    length: u16,
    checksum: u16,
    status: u8,
    errors: u8,
    special: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct E1000Diagnostics {
    pub ctrl: u32,
    pub status: u32,
    pub rx_head: u32,
    pub rx_tail: u32,
    pub tx_head: u32,
    pub tx_tail: u32,
    pub rx_frames: u64,
    pub tx_frames: u64,
}

pub struct E1000 {
    pci: PciDevice,
    mmio_phys: u64,
    mmio_base: u64,
    mac: [u8; 6],
    tx_desc_region: DmaRegion,
    tx_descs: *mut E1000TxDesc,
    tx_buffers: Vec<PacketBuffer>,
    tx_tail: usize,
    rx_desc_region: DmaRegion,
    rx_descs: *mut E1000RxDesc,
    rx_buffers: Vec<PacketBuffer>,
    rx_head: usize,
    rx_frames: u64,
    tx_frames: u64,
}

unsafe impl Send for E1000 {}

impl E1000 {
    pub fn init(pci: PciDevice) -> Result<Self, NetError> {
        let mmio_phys = match pci.bar(0) {
            PciBar::Memory32(base) => u64::from(base),
            PciBar::Memory64(base) => base,
            _ => {
                return Err(NetError::UnsupportedDevice(
                    "e1000 requires a memory BAR in BAR0",
                ));
            }
        };

        pci::enable_bus_mastering(&pci);
        let mmio_base = memory::phys_to_virt(PhysAddr::new(mmio_phys))
            .ok_or(NetError::InitializationFailed("e1000 MMIO mapping missing"))?
            .as_u64();

        let mut device = Self {
            pci,
            mmio_phys,
            mmio_base,
            mac: [0; 6],
            tx_desc_region: DmaRegion::allocate(NUM_TX_DESCS * size_of::<E1000TxDesc>())?,
            tx_descs: core::ptr::null_mut(),
            tx_buffers: Vec::with_capacity(NUM_TX_DESCS),
            tx_tail: 0,
            rx_desc_region: DmaRegion::allocate(NUM_RX_DESCS * size_of::<E1000RxDesc>())?,
            rx_descs: core::ptr::null_mut(),
            rx_buffers: Vec::with_capacity(NUM_RX_DESCS),
            rx_head: 0,
            rx_frames: 0,
            tx_frames: 0,
        };

        device.tx_descs = device.tx_desc_region.as_mut_ptr().cast::<E1000TxDesc>();
        device.rx_descs = device.rx_desc_region.as_mut_ptr().cast::<E1000RxDesc>();

        device.reset();
        device.write_reg(E1000_IMC, u32::MAX);
        device.mac = device.read_mac();
        device.init_tx()?;
        device.init_rx()?;

        let ctrl = device.read_reg(E1000_CTRL);
        device.write_reg(E1000_CTRL, ctrl | CTRL_SLU);
        Ok(device)
    }

    #[must_use]
    pub fn info(&self) -> NetworkDeviceInfo {
        NetworkDeviceInfo {
            name: "Intel E1000",
            driver: "e1000",
            mac: self.mac,
            transport: NetworkTransport::Mmio(self.mmio_phys),
            rx_queue_size: NUM_RX_DESCS as u16,
            tx_queue_size: NUM_TX_DESCS as u16,
            interrupt_line: self.pci.interrupt_line,
            pending_frames: 0,
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
            link_speed_mbps: self.link_speed(),
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> E1000Diagnostics {
        E1000Diagnostics {
            ctrl: self.read_reg(E1000_CTRL),
            status: self.read_reg(E1000_STATUS),
            rx_head: self.read_reg(E1000_RDH),
            rx_tail: self.read_reg(E1000_RDT),
            tx_head: self.read_reg(E1000_TDH),
            tx_tail: self.read_reg(E1000_TDT),
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
        }
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        let index = self.tx_tail;
        let status = unsafe { (*self.tx_descs.add(index)).status };
        if status & TX_STATUS_DD == 0 {
            return Err(NetError::QueueFull);
        }

        let buffer = &mut self.tx_buffers[index];
        if frame.len() > buffer.capacity() {
            return Err(NetError::PayloadTooLarge);
        }

        buffer[..frame.len()].copy_from_slice(frame);
        let desc = self.tx_desc_mut(index);
        desc.length = frame.len() as u16;
        desc.cso = 0;
        desc.cmd = TX_CMD_EOP | TX_CMD_IFCS | TX_CMD_RS;
        desc.status = 0;
        desc.css = 0;
        desc.special = 0;

        self.tx_tail = (self.tx_tail + 1) % NUM_TX_DESCS;
        fence(Ordering::SeqCst);
        self.write_reg(E1000_TDT, self.tx_tail as u32);
        self.tx_frames = self.tx_frames.saturating_add(1);
        Ok(())
    }

    #[must_use]
    pub fn recv_frame(&mut self) -> Option<Vec<u8>> {
        let index = self.rx_head;
        let (status, errors, length) = unsafe {
            let desc = &*self.rx_descs.add(index);
            (desc.status, desc.errors, desc.length)
        };
        if status & RX_STATUS_DD == 0 {
            return None;
        }

        let data = if errors == 0 && length != 0 {
            let len = length as usize;
            let packet = self.rx_buffers[index][..len].to_vec();
            self.rx_frames = self.rx_frames.saturating_add(1);
            Some(packet)
        } else {
            None
        };

        let desc = self.rx_desc_mut(index);
        desc.status = 0;
        desc.length = 0;
        desc.checksum = 0;
        desc.errors = 0;
        desc.special = 0;

        self.write_reg(E1000_RDT, index as u32);
        self.rx_head = (self.rx_head + 1) % NUM_RX_DESCS;
        data
    }

    #[must_use]
    pub fn link_up(&self) -> bool {
        self.read_reg(E1000_STATUS) & STATUS_LINK_UP != 0
    }

    #[must_use]
    pub fn link_speed(&self) -> u32 {
        match (self.read_reg(E1000_STATUS) >> 6) & 0x3 {
            0 => 10,
            1 => 100,
            2 => 1000,
            _ => 1000,
        }
    }

    fn init_tx(&mut self) -> Result<(), NetError> {
        for index in 0..NUM_TX_DESCS {
            let buffer = PacketBuffer::new(BUFFER_SIZE)?;
            let desc = self.tx_desc_mut(index);
            desc.buffer_addr = buffer.physical().as_u64();
            desc.status = TX_STATUS_DD;
            self.tx_buffers.push(buffer);
        }

        let base = self.tx_desc_region.physical().as_u64();
        self.write_reg(E1000_TDBAL, base as u32);
        self.write_reg(E1000_TDBAH, (base >> 32) as u32);
        self.write_reg(E1000_TDLEN, (NUM_TX_DESCS * size_of::<E1000TxDesc>()) as u32);
        self.write_reg(E1000_TDH, 0);
        self.write_reg(E1000_TDT, 0);
        self.write_reg(E1000_TIPG, 0x0060_200A);
        self.write_reg(E1000_TCTL, TCTL_EN | TCTL_PSP | (0x10 << 4) | (0x40 << 12));
        Ok(())
    }

    fn init_rx(&mut self) -> Result<(), NetError> {
        for index in 0..NUM_RX_DESCS {
            let buffer = PacketBuffer::new(BUFFER_SIZE)?;
            let desc = self.rx_desc_mut(index);
            desc.buffer_addr = buffer.physical().as_u64();
            desc.status = 0;
            self.rx_buffers.push(buffer);
        }

        let base = self.rx_desc_region.physical().as_u64();
        self.write_reg(E1000_RDBAL, base as u32);
        self.write_reg(E1000_RDBAH, (base >> 32) as u32);
        self.write_reg(E1000_RDLEN, (NUM_RX_DESCS * size_of::<E1000RxDesc>()) as u32);
        self.write_reg(E1000_RDH, 0);
        self.write_reg(E1000_RDT, (NUM_RX_DESCS - 1) as u32);
        self.write_reg(E1000_RCTL, RCTL_EN | RCTL_BAM | RCTL_SECRC);
        Ok(())
    }

    fn reset(&mut self) {
        let ctrl = self.read_reg(E1000_CTRL);
        self.write_reg(E1000_CTRL, ctrl | CTRL_RST);
        for _ in 0..100_000 {
            if self.read_reg(E1000_CTRL) & CTRL_RST == 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }

    fn read_mac(&self) -> [u8; 6] {
        let ral = self.read_reg(E1000_RAL);
        let rah = self.read_reg(E1000_RAH);
        [
            (ral & 0xFF) as u8,
            ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8,
            ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8,
            ((rah >> 8) & 0xFF) as u8,
        ]
    }

    fn tx_desc_mut(&mut self, index: usize) -> &mut E1000TxDesc {
        unsafe { &mut *self.tx_descs.add(index) }
    }

    fn rx_desc_mut(&mut self, index: usize) -> &mut E1000RxDesc {
        unsafe { &mut *self.rx_descs.add(index) }
    }

    fn read_reg(&self, register: u64) -> u32 {
        unsafe { read_volatile((self.mmio_base + register) as *const u32) }
    }

    fn write_reg(&self, register: u64, value: u32) {
        unsafe {
            write_volatile((self.mmio_base + register) as *mut u32, value);
        }
    }
}
