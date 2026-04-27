use alloc::vec::Vec;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{fence, Ordering};

use x86_64::{PhysAddr, VirtAddr};

use crate::arch::x86_64::port;
use crate::arch::x86_64::{interrupts, pit};
use crate::memory;
use crate::net::buffer::{DmaRegion, PacketBuffer};
use crate::net::pci::{self, PciBar, PciDevice};
use crate::net::{LinkState, NetError, NetworkDeviceInfo, NetworkTransport};

const NUM_TX_DESCS: usize = 64;
const NUM_RX_DESCS: usize = 64;
const BUFFER_SIZE: usize = 2048;
const MMIO_MAP_SIZE: usize = 0x1000;

const REG_MAC0: u16 = 0x00;
const REG_MAR0: u16 = 0x08;
const REG_TX_DESC_START_LOW: u16 = 0x20;
const REG_TX_DESC_START_HIGH: u16 = 0x24;
const REG_CHIP_CMD: u16 = 0x37;
const REG_TX_POLL: u16 = 0x38;
const REG_INTR_MASK: u16 = 0x3C;
const REG_INTR_STATUS: u16 = 0x3E;
const REG_TX_CONFIG: u16 = 0x40;
const REG_RX_CONFIG: u16 = 0x44;
const REG_RX_MISSED: u16 = 0x4C;
const REG_CFG9346: u16 = 0x50;
const REG_PHY_ACCESS: u16 = 0x60;
const REG_PHY_STATUS: u16 = 0x6C;
const REG_RX_MAX_SIZE: u16 = 0xDA;
const REG_CPLUS_CMD: u16 = 0xE0;
const REG_RX_DESC_START_LOW: u16 = 0xE4;
const REG_RX_DESC_START_HIGH: u16 = 0xE8;
const REG_EARLY_TX_THRESHOLD: u16 = 0xEC;

const CMD_RESET: u8 = 1 << 4;
const CMD_RX_ENABLE: u8 = 1 << 3;
const CMD_TX_ENABLE: u8 = 1 << 2;
const TX_POLL_NORMAL: u8 = 1 << 6;

const CFG9346_UNLOCK: u8 = 0xC0;
const CFG9346_LOCK: u8 = 0x00;

const PHY_STATUS_LINK_OK: u8 = 1 << 1;
const PHY_STATUS_FULL_DUPLEX: u8 = 1 << 0;
const PHY_STATUS_10M: u8 = 1 << 2;
const PHY_STATUS_100M: u8 = 1 << 3;
const PHY_STATUS_1000M: u8 = 1 << 4;

const CPLUS_CMD_DAC: u16 = 1 << 4;

const PHY_ACCESS_BUSY: u32 = 1 << 31;
const PHY_REG_CONTROL: u8 = 0x00;
const PHY_REG_STATUS: u8 = 0x01;
const PHY_REG_AUTO_NEGOTIATION: u8 = 0x04;
const PHY_REG_GIGABIT_CONTROL: u8 = 0x09;
const PHY_CONTROL_AUTO_NEGOTIATION_ENABLE: u16 = 0x1000;
const PHY_CONTROL_RESTART_AUTO_NEGOTIATION: u16 = 0x0200;
const PHY_CONTROL_RESET: u16 = 0x8000;
const PHY_CONTROL_POWER_DOWN: u16 = 0x0800;
const PHY_CONTROL_SPEED_100: u16 = 0x2000;
const PHY_CONTROL_FULL_DUPLEX: u16 = 0x0100;
const PHY_STATUS_LINK_STATUS: u16 = 0x0004;
const PHY_STATUS_AUTO_NEGOTIATION_COMPLETE: u16 = 0x0020;
const PHY_STATUS_AUTO_NEGOTIATION_ABILITY: u16 = 0x0008;
const PHY_ADV_10_HALF: u16 = 0x0020;
const PHY_ADV_10_FULL: u16 = 0x0040;
const PHY_ADV_100_HALF: u16 = 0x0080;
const PHY_ADV_100_FULL: u16 = 0x0100;
const PHY_ADV_PAUSE: u16 = 0x0400;
const PHY_ADV_ASYM_PAUSE: u16 = 0x0800;
const PHY_ADV_GIGABIT_FULL: u16 = 0x0200;
const PHY_ADV_GIGABIT_HALF: u16 = 0x0100;
const PHY_ANAR_SELECTOR_802_3: u16 = 0x0001;
const PHY_REG_LINK_PARTNER: u8 = 0x05;
const PHY_REG_EXPANSION: u8 = 0x06;

/// Settling delay after PHY reset completes (ms).
/// RTL8168H internal PHY needs this before MDIO registers are reliable.
const PHY_POST_RESET_SETTLE_MS: u64 = 100;

const REG_CONFIG1: u16 = 0x52;
const REG_CONFIG2: u16 = 0x53;
const REG_CONFIG5: u16 = 0x56;
const REG_MISC: u16 = 0xF0;

const TX_CONFIG_DEFAULT: u32 = 0x0300_0700;
const RX_CONFIG_DEFAULT: u32 = 0x0000_E70F;
const RX_MAX_SIZE_DEFAULT: u16 = 0x0FFF;
const EARLY_TX_THRESHOLD_DEFAULT: u8 = 0x3B;

// RTL8168 specific: IFG + DMA burst config for real hardware
const TX_CONFIG_RTL8168: u32 = 0x0300_0700 | (0x07 << 8); // MaxDMA unlimited + IFG
const RX_CONFIG_RTL8168: u32 = 0x0000_C70E; // FIFO 7K, DMA unlimited, accept bcast+mcast+phys

const INTR_RX_OK: u16 = 1 << 0;
const INTR_RX_ERR: u16 = 1 << 1;
const INTR_TX_OK: u16 = 1 << 2;
const INTR_TX_ERR: u16 = 1 << 3;
const INTR_RX_OVERFLOW: u16 = 1 << 4;
const INTR_LINK_CHANGE: u16 = 1 << 5;
const INTR_RX_FIFO_OVER: u16 = 1 << 6;
const INTR_TX_DESC_UNAVAIL: u16 = 1 << 7;
const INTR_SYSTEM_ERROR: u16 = 1 << 15;
const INTR_MASK_OPERATIONAL: u16 = INTR_RX_OK
    | INTR_RX_ERR
    | INTR_TX_OK
    | INTR_TX_ERR
    | INTR_RX_OVERFLOW
    | INTR_LINK_CHANGE
    | INTR_RX_FIFO_OVER
    | INTR_TX_DESC_UNAVAIL
    | INTR_SYSTEM_ERROR;

const RESET_TIMEOUT_MS: u64 = 200;
const PHY_TIMEOUT_MS: u64 = 250;
const AUTONEG_RETRY_INTERVAL_MS: u64 = 1_500;
const BOOT_AUTONEG_TIMEOUT_MS: u64 = 2_500;
const BOOT_LINK_SETTLE_MS: u64 = 2_500;
const BOOT_FORCED_LINK_TIMEOUT_MS: u64 = 1_200;

/// Indirect MDIO/PHY page-select register on RTL8168 (page select via 0x1F).
const PHY_REG_PAGE_SELECT: u8 = 0x1F;
/// MMD device address used to disable Energy Efficient Ethernet on RTL8168 PHYs.
const PHY_REG_MMD_ACCESS_CTRL: u8 = 0x0D;
const PHY_REG_MMD_ACCESS_DATA: u8 = 0x0E;
const PHY_MMD_DEVICE_AN: u16 = 0x0007;
const PHY_MMD_AN_EEE_ADV: u16 = 0x003C;
const PHY_MMD_FUNCTION_DATA_NO_INCR: u16 = 0x4000;

const DESC_OWN: u32 = 1 << 31;
const DESC_EOR: u32 = 1 << 30;
const DESC_FS: u32 = 1 << 29;
const DESC_LS: u32 = 1 << 28;
const DESC_RX_ERROR_MASK: u32 = (1 << 20) | (1 << 19) | (1 << 18) | (1 << 17) | (1 << 16);
const DESC_FRAME_LEN_MASK: u32 = 0x3FFF;

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
struct RtlDesc {
    opts1: u32,
    opts2: u32,
    addr_lo: u32,
    addr_hi: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Rtl8169Diagnostics {
    pub device_id: u16,
    pub revision_id: u8,
    pub mac_version: u32,
    pub is_rtl8168: bool,
    pub interrupt_line: u8,
    pub pci_command: u16,
    pub chip_cmd: u8,
    pub intr_status: u16,
    pub intr_mask: u16,
    pub phy_status: u8,
    pub phy_control: u16,
    pub phy_register_status: u16,
    pub phy_auto_negotiation: u16,
    pub phy_gigabit_control: u16,
    pub phy_link_partner: u16,
    pub phy_expansion: u16,
    pub link_up: bool,
    pub link_speed_mbps: u32,
    pub full_duplex: bool,
    pub reset_complete: bool,
    pub rx_head: u32,
    pub tx_tail: u32,
    pub rx_frames: u64,
    pub tx_frames: u64,
    pub tx_attempts: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub rx_interrupts: u64,
    pub tx_interrupts: u64,
    pub link_changes: u64,
}

enum RegisterWindow {
    Mmio { phys: u64, virt: VirtAddr },
    Io(u16),
}

pub struct Rtl8169 {
    pci: PciDevice,
    window: RegisterWindow,
    mac: [u8; 6],
    mac_version: u32,
    is_rtl8168: bool,
    tx_desc_region: DmaRegion,
    tx_descs: *mut RtlDesc,
    tx_buffers: Vec<PacketBuffer>,
    tx_tail: usize,
    rx_desc_region: DmaRegion,
    rx_descs: *mut RtlDesc,
    rx_buffers: Vec<PacketBuffer>,
    rx_head: usize,
    rx_frames: u64,
    tx_frames: u64,
    tx_attempts: u64,
    tx_errors: u64,
    rx_errors: u64,
    rx_interrupts: u64,
    tx_interrupts: u64,
    link_changes: u64,
    reset_complete: bool,
    irq_enabled: bool,
    last_link_up: bool,
    last_autoneg_attempt_tick: u64,
    last_tx_tick: u64,
}

unsafe impl Send for Rtl8169 {}

impl Rtl8169 {
    pub fn init(pci: PciDevice) -> Result<Self, NetError> {
        let window = Self::map_register_window(&pci)?;
        pci::enable_bus_mastering(&pci);

        // --- CRITICAL: Disable ASPM before any register access ---
        // RTL8168-family on real hardware has known instability with ASPM L0s/L1.
        // BIOS often enables it; we must disable before touching MMIO.
        pci::disable_aspm(&pci);

        // Optimize PCI Express Max Read Request Size to 4096 for DMA throughput
        pci::set_max_read_request_size(&pci, 5); // 5 = 4096 bytes

        let mut device = Self {
            pci,
            window,
            mac: [0; 6],
            mac_version: 0,
            is_rtl8168: false,
            tx_desc_region: DmaRegion::allocate(NUM_TX_DESCS * size_of::<RtlDesc>())?,
            tx_descs: core::ptr::null_mut(),
            tx_buffers: Vec::with_capacity(NUM_TX_DESCS),
            tx_tail: 0,
            rx_desc_region: DmaRegion::allocate(NUM_RX_DESCS * size_of::<RtlDesc>())?,
            rx_descs: core::ptr::null_mut(),
            rx_buffers: Vec::with_capacity(NUM_RX_DESCS),
            rx_head: 0,
            rx_frames: 0,
            tx_frames: 0,
            tx_attempts: 0,
            tx_errors: 0,
            rx_errors: 0,
            rx_interrupts: 0,
            tx_interrupts: 0,
            link_changes: 0,
            reset_complete: false,
            irq_enabled: false,
            last_link_up: false,
            last_autoneg_attempt_tick: 0,
            last_tx_tick: 0,
        };

        device.tx_descs = device.tx_desc_region.as_mut_ptr().cast::<RtlDesc>();
        device.rx_descs = device.rx_desc_region.as_mut_ptr().cast::<RtlDesc>();

        device.log_identity();
        device.detect_mac_version();
        device.dump_registers("pre-init");

        // RTL8168-specific: Disable ASF/Dashboard before reset (prevents firmware interference)
        if device.is_rtl8168 {
            device.disable_asf();
        }

        device.reset()?;
        device.write16(REG_INTR_MASK, 0);
        device.write16(REG_INTR_STATUS, u16::MAX);
        device.mac = device.read_mac();
        device.init_rings()?;
        crate::serial_println!("[rtl8169] MAC {}", crate::net::format_mac(&device.mac));
        device.configure_phy()?;
        device.configure()?;
        device.last_link_up = device.current_link_up();
        device.configure_interrupts();
        device.dump_registers("post-init");
        Ok(device)
    }

    #[must_use]
    pub fn info(&self) -> NetworkDeviceInfo {
        let link_up = self.link_up();
        NetworkDeviceInfo {
            name: "Realtek RTL8169-family",
            driver: "rtl8169",
            mac: self.mac,
            transport: match self.window {
                RegisterWindow::Mmio { phys, .. } => NetworkTransport::Mmio(phys),
                RegisterWindow::Io(base) => NetworkTransport::Io(base),
            },
            rx_queue_size: NUM_RX_DESCS as u16,
            tx_queue_size: NUM_TX_DESCS as u16,
            interrupt_line: self.pci.interrupt_line,
            pending_frames: 0,
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
            link_speed_mbps: if link_up { self.link_speed() } else { 0 },
            link_state: if link_up {
                LinkState::Up
            } else {
                LinkState::Down
            },
            full_duplex: self.full_duplex(),
        }
    }

    #[must_use]
    pub fn diagnostics(&self) -> Rtl8169Diagnostics {
        let link_up = self.current_link_up();
        Rtl8169Diagnostics {
            device_id: self.pci.device_id,
            revision_id: self.pci.revision_id,
            mac_version: self.mac_version,
            is_rtl8168: self.is_rtl8168,
            interrupt_line: self.pci.interrupt_line,
            pci_command: pci::command_register(&self.pci),
            chip_cmd: self.read8(REG_CHIP_CMD),
            intr_status: self.read16(REG_INTR_STATUS),
            intr_mask: self.read16(REG_INTR_MASK),
            phy_status: self.read8(REG_PHY_STATUS),
            phy_control: self.read_phy(PHY_REG_CONTROL).unwrap_or(0),
            phy_register_status: self.read_bmsr().unwrap_or(0),
            phy_auto_negotiation: self.read_phy(PHY_REG_AUTO_NEGOTIATION).unwrap_or(0),
            phy_gigabit_control: self.read_phy(PHY_REG_GIGABIT_CONTROL).unwrap_or(0),
            phy_link_partner: self.read_phy(PHY_REG_LINK_PARTNER).unwrap_or(0),
            phy_expansion: self.read_phy(PHY_REG_EXPANSION).unwrap_or(0),
            link_up,
            link_speed_mbps: if link_up { self.link_speed() } else { 0 },
            full_duplex: self.full_duplex(),
            reset_complete: self.reset_complete,
            rx_head: self.rx_head as u32,
            tx_tail: self.tx_tail as u32,
            rx_frames: self.rx_frames,
            tx_frames: self.tx_frames,
            tx_attempts: self.tx_attempts,
            tx_errors: self.tx_errors,
            rx_errors: self.rx_errors,
            rx_interrupts: self.rx_interrupts,
            tx_interrupts: self.tx_interrupts,
            link_changes: self.link_changes,
        }
    }

    pub fn send_frame(&mut self, frame: &[u8]) -> Result<(), NetError> {
        self.poll();
        if !self.current_link_up() {
            return Err(NetError::ProtocolError(
                "rtl8169: wired link is down; transmit blocked".into(),
            ));
        }
        self.tx_attempts = self.tx_attempts.saturating_add(1);
        let index = self.tx_tail;
        let desc = self.tx_desc(index);
        if desc.opts1 & DESC_OWN != 0 {
            // Check for TX stall: if hardware hasn't consumed this descriptor
            // within a reasonable time, attempt recovery.
            self.check_tx_stall();
            let desc = self.tx_desc(index);
            if desc.opts1 & DESC_OWN != 0 {
                return Err(NetError::QueueFull);
            }
        }

        let buffer = &mut self.tx_buffers[index];
        if frame.len() > buffer.capacity() {
            return Err(NetError::PayloadTooLarge);
        }

        buffer[..frame.len()].copy_from_slice(frame);
        let mut opts1 = DESC_OWN | DESC_FS | DESC_LS | (frame.len() as u32 & DESC_FRAME_LEN_MASK);
        if index + 1 == NUM_TX_DESCS {
            opts1 |= DESC_EOR;
        }

        let desc = self.tx_desc_mut(index);
        desc.opts2 = 0;
        fence(Ordering::Release);
        desc.opts1 = opts1;

        self.tx_tail = (self.tx_tail + 1) % NUM_TX_DESCS;
        self.last_tx_tick = interrupts::tick_count();
        fence(Ordering::SeqCst);
        self.write8(REG_TX_POLL, TX_POLL_NORMAL);
        self.tx_frames = self.tx_frames.saturating_add(1);
        crate::serial_println!(
            "[rtl8169] tx queued len={} slot={} attempts={} frames={}",
            frame.len(),
            index,
            self.tx_attempts,
            self.tx_frames
        );
        Ok(())
    }

    #[must_use]
    pub fn recv_frame(&mut self) -> Option<Vec<u8>> {
        self.poll();
        let index = self.rx_head;
        let desc = self.rx_desc(index);
        if desc.opts1 & DESC_OWN != 0 {
            return None;
        }

        let opts1 = desc.opts1;
        let length = (opts1 & DESC_FRAME_LEN_MASK) as usize;
        let good_packet = opts1 & DESC_RX_ERROR_MASK == 0
            && opts1 & DESC_FS != 0
            && opts1 & DESC_LS != 0
            && length >= 4;

        let data = if good_packet {
            let payload_len = length.saturating_sub(4);
            let packet = self.rx_buffers[index][..payload_len].to_vec();
            self.rx_frames = self.rx_frames.saturating_add(1);
            Some(packet)
        } else {
            None
        };

        let mut new_opts1 =
            DESC_OWN | (self.rx_buffers[index].capacity() as u32 & DESC_FRAME_LEN_MASK);
        if index + 1 == NUM_RX_DESCS {
            new_opts1 |= DESC_EOR;
        }

        let desc = self.rx_desc_mut(index);
        desc.opts2 = 0;
        fence(Ordering::Release);
        desc.opts1 = new_opts1;

        self.rx_head = (self.rx_head + 1) % NUM_RX_DESCS;
        if data.is_some() {
            crate::serial_println!(
                "[rtl8169] rx packet slot={} frames={}",
                index,
                self.rx_frames
            );
        }
        data
    }

    #[must_use]
    pub fn link_up(&self) -> bool {
        self.current_link_up()
    }

    #[must_use]
    pub fn link_speed(&self) -> u32 {
        let phy_status = self.read8(REG_PHY_STATUS);
        if phy_status & PHY_STATUS_1000M != 0 {
            1000
        } else if phy_status & PHY_STATUS_100M != 0 {
            100
        } else if phy_status & PHY_STATUS_10M != 0 {
            10
        } else {
            0
        }
    }

    #[must_use]
    pub fn full_duplex(&self) -> bool {
        self.read8(REG_PHY_STATUS) & PHY_STATUS_FULL_DUPLEX != 0
    }

    pub fn poll(&mut self) {
        self.service_interrupts();
        self.refresh_link_state();
    }

    fn map_register_window(pci: &PciDevice) -> Result<RegisterWindow, NetError> {
        for index in 0..pci.bars.len() {
            match pci.bar(index) {
                PciBar::Memory32(base) => {
                    let virt = memory::map_mmio(PhysAddr::new(u64::from(base)), MMIO_MAP_SIZE)
                        .map_err(|_| {
                            NetError::InitializationFailed("rtl8169 MMIO mapping failed")
                        })?;
                    return Ok(RegisterWindow::Mmio {
                        phys: u64::from(base),
                        virt,
                    });
                }
                PciBar::Memory64(base) => {
                    let virt =
                        memory::map_mmio(PhysAddr::new(base), MMIO_MAP_SIZE).map_err(|_| {
                            NetError::InitializationFailed("rtl8169 MMIO mapping failed")
                        })?;
                    return Ok(RegisterWindow::Mmio { phys: base, virt });
                }
                PciBar::Unused => {}
                PciBar::Io(_) => {}
            }
        }

        for index in 0..pci.bars.len() {
            if let PciBar::Io(base) = pci.bar(index) {
                return Ok(RegisterWindow::Io(base));
            }
        }

        Err(NetError::UnsupportedDevice(
            "rtl8169 requires a memory or I/O BAR",
        ))
    }

    fn configure(&mut self) -> Result<(), NetError> {
        crate::serial_println!("[rtl8169] datapath configure start is_rtl8168={}", self.is_rtl8168);

        // DMA barrier: init_rings() wrote descriptors to DMA memory; ensure
        // all those writes are globally visible before we program the device
        // with their physical addresses.
        fence(Ordering::SeqCst);

        // STEP 1: Ensure Rx/Tx disabled BEFORE touching any config register.
        self.write8(REG_CHIP_CMD, 0);
        self.write8(REG_CFG9346, CFG9346_UNLOCK);

        // RTL8168: Disable power-saving clock request EARLY (can prevent
        // link establishment if done after MAC enable).
        if self.is_rtl8168 {
            let config2 = self.read8(REG_CONFIG2);
            self.write8(REG_CONFIG2, config2 & !0x20); // Clear ClkReqEn
            let config5 = self.read8(REG_CONFIG5);
            self.write8(REG_CONFIG5, config5 & !0x01); // Clear PME_STS wake
            crate::serial_println!(
                "[rtl8169] rtl8168 power quirks config2=0x{:02X}->0x{:02X} config5=0x{:02X}->0x{:02X}",
                config2, self.read8(REG_CONFIG2), config5, self.read8(REG_CONFIG5)
            );
        }

        // STEP 2: C+ command features.
        let mut cplus = self.read16(REG_CPLUS_CMD);
        if self.requires_dac() {
            cplus |= CPLUS_CMD_DAC;
        }
        cplus |= 1 << 3; // PCI MRW enable
        self.write16(REG_CPLUS_CMD, cplus);

        // STEP 3: Descriptor ring base addresses.
        let tx_base = self.tx_desc_region.physical().as_u64();
        self.write32(REG_TX_DESC_START_LOW, tx_base as u32);
        self.write32(REG_TX_DESC_START_HIGH, (tx_base >> 32) as u32);

        let rx_base = self.rx_desc_region.physical().as_u64();
        self.write32(REG_RX_DESC_START_LOW, rx_base as u32);
        self.write32(REG_RX_DESC_START_HIGH, (rx_base >> 32) as u32);

        // STEP 4: Multicast accept-all (needed for DHCP broadcast).
        self.write32(REG_MAR0, u32::MAX);
        self.write32(REG_MAR0 + 4, u32::MAX);

        // STEP 5: TX + RX configuration (chip-specific values).
        let (tx_cfg, rx_cfg) = if self.is_rtl8168 {
            (TX_CONFIG_RTL8168, RX_CONFIG_RTL8168)
        } else {
            (TX_CONFIG_DEFAULT, RX_CONFIG_DEFAULT)
        };
        self.write32(REG_TX_CONFIG, tx_cfg);
        self.write32(REG_RX_CONFIG, rx_cfg);
        self.write16(REG_RX_MAX_SIZE, RX_MAX_SIZE_DEFAULT);
        self.write8(REG_EARLY_TX_THRESHOLD, EARLY_TX_THRESHOLD_DEFAULT);

        // STEP 6: Lock config, clear counters, then enable Rx/Tx last.
        self.write8(REG_CFG9346, CFG9346_LOCK);
        self.write32(REG_RX_MISSED, 0);
        self.write16(REG_INTR_STATUS, u16::MAX);
        self.write8(REG_CHIP_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);
        crate::serial_println!(
            "[rtl8169] datapath configured tx=0x{:08X} rx=0x{:08X} cmd=0x{:02X} cplus=0x{:04X}",
            self.read32(REG_TX_CONFIG),
            self.read32(REG_RX_CONFIG),
            self.read8(REG_CHIP_CMD),
            self.read16(REG_CPLUS_CMD)
        );
        Ok(())
    }

    fn init_rings(&mut self) -> Result<(), NetError> {
        for index in 0..NUM_TX_DESCS {
            let buffer = PacketBuffer::new(BUFFER_SIZE)?;
            let desc = self.tx_desc_mut(index);
            desc.opts1 = if index + 1 == NUM_TX_DESCS {
                DESC_EOR
            } else {
                0
            };
            desc.opts2 = 0;
            desc.addr_lo = buffer.physical().as_u64() as u32;
            desc.addr_hi = (buffer.physical().as_u64() >> 32) as u32;
            self.tx_buffers.push(buffer);
        }

        for index in 0..NUM_RX_DESCS {
            let buffer = PacketBuffer::new(BUFFER_SIZE)?;
            let mut opts1 = DESC_OWN | (buffer.capacity() as u32 & DESC_FRAME_LEN_MASK);
            if index + 1 == NUM_RX_DESCS {
                opts1 |= DESC_EOR;
            }

            let desc = self.rx_desc_mut(index);
            desc.opts1 = opts1;
            desc.opts2 = 0;
            desc.addr_lo = buffer.physical().as_u64() as u32;
            desc.addr_hi = (buffer.physical().as_u64() >> 32) as u32;
            self.rx_buffers.push(buffer);
        }

        Ok(())
    }

    fn requires_dac(&self) -> bool {
        let bases = [
            self.tx_desc_region.physical().as_u64(),
            self.rx_desc_region.physical().as_u64(),
        ];

        bases.iter().any(|base| *base >> 32 != 0)
            || self
                .tx_buffers
                .iter()
                .any(|buffer| buffer.physical().as_u64() >> 32 != 0)
            || self
                .rx_buffers
                .iter()
                .any(|buffer| buffer.physical().as_u64() >> 32 != 0)
    }

    fn reset(&mut self) -> Result<(), NetError> {
        crate::serial_println!("[rtl8169] reset start");

        // Pre-reset: stop all DMA activity cleanly to avoid hung state
        self.write16(REG_INTR_MASK, 0);
        self.write16(REG_INTR_STATUS, u16::MAX);
        self.write8(REG_CHIP_CMD, 0); // Disable Rx/Tx
        // Small delay for DMA drain (real hardware needs this)
        for _ in 0..2048 {
            core::hint::spin_loop();
        }

        self.write8(REG_CHIP_CMD, CMD_RESET);
        if self.wait_until(RESET_TIMEOUT_MS, || {
            self.read8(REG_CHIP_CMD) & CMD_RESET == 0
        }) {
            self.reset_complete = true;
            // CRITICAL: post-reset settling.  The CMD_RESET bit clearing
            // only means the *MAC* state machine has restarted.  The
            // internal PHY, PHYAR MDIO bridge, and config registers need
            // additional stabilisation time before they are reliable.
            // Linux r8169 inserts msleep(1) here; we use 20 ms because
            // we have coarser PIT granularity and this runs exactly once.
            self.wait_until(20, || false);
            crate::serial_println!(
                "[rtl8169] reset complete cmd=0x{:02X} (post-settle done)",
                self.read8(REG_CHIP_CMD)
            );
            Ok(())
        } else {
            self.reset_complete = false;
            crate::serial_println!(
                "[rtl8169] reset timeout cmd=0x{:02X} isr=0x{:04X}",
                self.read8(REG_CHIP_CMD),
                self.read16(REG_INTR_STATUS)
            );
            Err(NetError::InitializationFailed(
                "rtl8169 reset did not complete",
            ))
        }
    }

    fn read_mac(&self) -> [u8; 6] {
        let mut mac = [0u8; 6];
        for (index, slot) in mac.iter_mut().enumerate() {
            *slot = self.read8(REG_MAC0 + index as u16);
        }
        mac
    }

    fn configure_phy(&mut self) -> Result<(), NetError> {
        crate::serial_println!("[rtl8169] phy init start");
        self.dump_registers("phy-before");

        // ---- STEP 1: PHY reset ----
        self.write_phy(PHY_REG_CONTROL, PHY_CONTROL_RESET)?;
        if !self.wait_until(PHY_TIMEOUT_MS, || {
            self.read_phy(PHY_REG_CONTROL)
                .map(|bmcr| bmcr & PHY_CONTROL_RESET == 0)
                .unwrap_or(false)
        }) {
            crate::serial_println!("[rtl8169] phy reset timeout");
            return Err(NetError::InitializationFailed(
                "rtl8169 PHY reset did not complete",
            ));
        }

        // ---- STEP 2: Post-reset settling delay ----
        // CRITICAL: RTL8168H internal PHY needs time after reset bit clears
        // before MDIO registers are stable. Without this, ANAR/GBCR writes
        // can be silently lost. Linux r8169 uses msleep(1) here; we use 100ms
        // to be safe on first bring-up.
        crate::serial_println!("[rtl8169] phy reset done, settling {}ms", PHY_POST_RESET_SETTLE_MS);
        self.wait_until(PHY_POST_RESET_SETTLE_MS, || false);

        // ---- STEP 3: Clear power-down bit ----
        // After chip reset or BIOS handoff, the PHY can be in power-down state.
        // A powered-down PHY cannot negotiate. Explicitly clear BMCR bit 11.
        let bmcr_now = self.read_phy(PHY_REG_CONTROL).unwrap_or(0);
        if bmcr_now & PHY_CONTROL_POWER_DOWN != 0 {
            crate::serial_println!(
                "[rtl8169] PHY was in POWER DOWN (bmcr=0x{:04X}), waking up",
                bmcr_now
            );
            self.write_phy(PHY_REG_CONTROL, bmcr_now & !PHY_CONTROL_POWER_DOWN)?;
            // Additional settling after power-up
            self.wait_until(50, || false);
        }

        // ---- STEP 4: Dump PHY state after reset for debug ----
        let bmsr_post_reset = self.read_bmsr().unwrap_or(0);
        let bmcr_post_reset = self.read_phy(PHY_REG_CONTROL).unwrap_or(0);
        crate::serial_println!(
            "[rtl8169] phy post-reset bmcr=0x{:04X} bmsr=0x{:04X} physt=0x{:02X}",
            bmcr_post_reset,
            bmsr_post_reset,
            self.read8(REG_PHY_STATUS)
        );

        // Verify PHY is alive: BMSR should have non-zero capability bits
        if bmsr_post_reset == 0x0000 || bmsr_post_reset == 0xFFFF {
            crate::serial_println!(
                "[rtl8169] WARNING: BMSR reads 0x{:04X} - PHY may be dead or MDIO bus broken",
                bmsr_post_reset
            );
        }

        // ---- STEP 4.5: Disable Energy Efficient Ethernet (EEE) ----
        // EEE LPI (low-power idle) is a known cause of link drops, packet loss
        // and full link-down events on RTL8168H/I/J family in real notebooks
        // when paired with switches that mis-handle EEE wake. Linux r8169
        // disables EEE by default for these chips. We do the same: clear
        // the IEEE 802.3az AN advertisement (MMD 7 register 0x3C).
        if self.is_rtl8168 {
            self.disable_eee();
        }

        // ---- STEP 5: Fast boot bring-up, with long retries deferred to background ----
        self.program_autoneg_advertisement()?;
        self.restart_autoneg("boot")?;
        let autoneg_complete = self.wait_for_auto_negotiation(BOOT_AUTONEG_TIMEOUT_MS);
        if autoneg_complete {
            let anlpar = self.read_phy(PHY_REG_LINK_PARTNER).unwrap_or(0);
            let aner = self.read_phy(PHY_REG_EXPANSION).unwrap_or(0);
            crate::serial_println!(
                "[rtl8169] boot autoneg complete anlpar=0x{:04X} aner=0x{:04X}",
                anlpar,
                aner
            );
        }

        let mut link_up = self.wait_for_link_up(BOOT_LINK_SETTLE_MS);

        // ---- STEP 6: If autoneg did not converge quickly, try one fast forced-mode probe ----
        if !link_up {
            crate::serial_println!(
                "[rtl8169] boot link still down after {}ms; trying forced 100M full duplex for {}ms",
                BOOT_AUTONEG_TIMEOUT_MS + BOOT_LINK_SETTLE_MS,
                BOOT_FORCED_LINK_TIMEOUT_MS
            );
            self.write_phy(
                PHY_REG_CONTROL,
                PHY_CONTROL_SPEED_100 | PHY_CONTROL_FULL_DUPLEX,
            )?;
            link_up = self.wait_for_link_up(BOOT_FORCED_LINK_TIMEOUT_MS);
            if link_up {
                crate::serial_println!(
                    "[rtl8169] FORCED 100M link UP - autoneg was the problem"
                );
            } else {
                crate::serial_println!(
                    "[rtl8169] forced 100M also failed bmcr=0x{:04X} bmsr=0x{:04X} physt=0x{:02X}",
                    self.read_phy(PHY_REG_CONTROL).unwrap_or(0),
                    self.read_bmsr().unwrap_or(0),
                    self.read8(REG_PHY_STATUS)
                );

                // Leave the PHY alive and let refresh_link_state() retry in background.
                self.restart_autoneg("background")?;
                crate::serial_println!(
                    "[rtl8169] boot bring-up deferred; background retries will continue every {}ms",
                    AUTONEG_RETRY_INTERVAL_MS
                );
            }
        }

        crate::serial_println!(
            "[rtl8169] phy init done link_up={} speed={} duplex={}",
            link_up,
            self.link_speed(),
            if self.full_duplex() { "full" } else { "half" }
        );
        self.dump_registers("phy-after");
        Ok(())
    }

    fn program_autoneg_advertisement(&self) -> Result<(), NetError> {
        let anar = PHY_ANAR_SELECTOR_802_3
            | PHY_ADV_10_HALF
            | PHY_ADV_10_FULL
            | PHY_ADV_100_HALF
            | PHY_ADV_100_FULL
            | PHY_ADV_PAUSE
            | PHY_ADV_ASYM_PAUSE;
        self.write_phy(PHY_REG_AUTO_NEGOTIATION, anar)?;
        self.write_phy(
            PHY_REG_GIGABIT_CONTROL,
            PHY_ADV_GIGABIT_FULL | PHY_ADV_GIGABIT_HALF,
        )?;
        crate::serial_println!(
            "[rtl8169] advertisement anar=0x{:04X} gbcr=0x{:04X}",
            self.read_phy(PHY_REG_AUTO_NEGOTIATION).unwrap_or(0),
            self.read_phy(PHY_REG_GIGABIT_CONTROL).unwrap_or(0)
        );
        Ok(())
    }

    fn restart_autoneg(&mut self, reason: &str) -> Result<(), NetError> {
        crate::serial_println!(
            "[rtl8169] autoneg restart reason={} bmsr=0x{:04X} physt=0x{:02X}",
            reason,
            self.read_bmsr().unwrap_or(0),
            self.read8(REG_PHY_STATUS)
        );
        self.write_phy(
            PHY_REG_CONTROL,
            PHY_CONTROL_AUTO_NEGOTIATION_ENABLE | PHY_CONTROL_RESTART_AUTO_NEGOTIATION,
        )?;
        self.last_autoneg_attempt_tick = interrupts::tick_count();
        Ok(())
    }

    fn wait_for_auto_negotiation(&self, timeout_ms: u64) -> bool {
        crate::serial_println!("[rtl8169] autoneg wait {}ms", timeout_ms);
        let completed = self.wait_until(timeout_ms, || {
            self.read_bmsr()
                .map(|bmsr| bmsr & PHY_STATUS_AUTO_NEGOTIATION_COMPLETE != 0)
                .unwrap_or(false)
        });
        crate::serial_println!(
            "[rtl8169] autoneg {} after {}ms bmsr=0x{:04X}",
            if completed { "complete" } else { "timeout" },
            timeout_ms,
            self.read_bmsr().unwrap_or(0)
        );
        completed
    }

    fn wait_for_link_up(&self, timeout_ms: u64) -> bool {
        let linked = self.wait_until(timeout_ms, || self.current_link_up());
        crate::serial_println!(
            "[rtl8169] link {} phy=0x{:02X} bmsr=0x{:04X}",
            if linked { "up" } else { "down" },
            self.read8(REG_PHY_STATUS),
            self.read_bmsr().unwrap_or(0)
        );
        linked
    }

    fn current_link_up(&self) -> bool {
        let phy_status = self.read8(REG_PHY_STATUS);
        let bmsr = self.read_bmsr().unwrap_or(0);
        (phy_status & PHY_STATUS_LINK_OK != 0) || (bmsr & PHY_STATUS_LINK_STATUS != 0)
    }

    fn read_bmsr(&self) -> Option<u16> {
        let _ = self.read_phy(PHY_REG_STATUS);
        self.read_phy(PHY_REG_STATUS)
    }

    fn configure_interrupts(&mut self) {
        let irq_line = self.pci.interrupt_line;
        self.irq_enabled = interrupts::register_network_irq(irq_line);
        if self.irq_enabled {
            self.write16(REG_INTR_STATUS, u16::MAX);
            self.write16(REG_INTR_MASK, INTR_MASK_OPERATIONAL);
            crate::serial_println!(
                "[rtl8169] IRQ armed line={} imr=0x{:04X}",
                irq_line,
                self.read16(REG_INTR_MASK)
            );
        } else {
            self.write16(REG_INTR_MASK, 0);
            crate::serial_println!(
                "[rtl8169] IRQ line {} not routed by WarOS PIC path; using polling",
                irq_line
            );
        }
    }

    fn service_interrupts(&mut self) {
        let status = self.read16(REG_INTR_STATUS);
        if status == 0 || status == u16::MAX {
            return;
        }

        self.write16(REG_INTR_STATUS, status);

        if status & INTR_RX_OK != 0 {
            self.rx_interrupts = self.rx_interrupts.saturating_add(1);
        }
        if status & INTR_TX_OK != 0 {
            self.tx_interrupts = self.tx_interrupts.saturating_add(1);
        }
        if status & INTR_LINK_CHANGE != 0 {
            crate::serial_println!(
                "[rtl8169] link-change isr=0x{:04X} phy=0x{:02X} bmsr=0x{:04X}",
                status,
                self.read8(REG_PHY_STATUS),
                self.read_bmsr().unwrap_or(0)
            );
        }
        if status & INTR_TX_ERR != 0 {
            self.tx_errors = self.tx_errors.saturating_add(1);
        }
        if status & (INTR_RX_ERR | INTR_RX_OVERFLOW | INTR_RX_FIFO_OVER) != 0 {
            self.rx_errors = self.rx_errors.saturating_add(1);
        }
        if status
            & (INTR_RX_ERR | INTR_TX_ERR | INTR_RX_OVERFLOW | INTR_RX_FIFO_OVER | INTR_SYSTEM_ERROR)
            != 0
        {
            crate::serial_println!(
                "[rtl8169] error isr=0x{:04X} cmd=0x{:02X} missed=0x{:08X}",
                status,
                self.read8(REG_CHIP_CMD),
                self.read32(REG_RX_MISSED)
            );
            // On system error, re-enable Rx/Tx (hardware may have auto-disabled)
            if status & INTR_SYSTEM_ERROR != 0 {
                crate::serial_println!("[rtl8169] system error recovery: re-enabling datapath");
                self.write8(REG_CHIP_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);
            }
        }
    }

    fn refresh_link_state(&mut self) {
        let link_up = self.current_link_up();
        if link_up != self.last_link_up {
            self.link_changes = self.link_changes.saturating_add(1);
            self.last_link_up = link_up;
            crate::serial_println!(
                "[rtl8169] link transition {} speed={} duplex={} phy=0x{:02X} bmsr=0x{:04X}",
                if link_up { "UP" } else { "DOWN" },
                self.link_speed(),
                if self.full_duplex() { "full" } else { "half" },
                self.read8(REG_PHY_STATUS),
                self.read_bmsr().unwrap_or(0)
            );
            if link_up {
                // Ensure MAC datapath is running when link arrives.
                self.write8(REG_CHIP_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);
                self.write32(REG_RX_MISSED, 0);
            }
        } else if !link_up {
            let now = interrupts::tick_count();
            let retry_ticks = AUTONEG_RETRY_INTERVAL_MS
                .saturating_mul(u64::from(pit::PIT_FREQUENCY_HZ))
                .div_ceil(1_000)
                .max(1);
            if now.saturating_sub(self.last_autoneg_attempt_tick) >= retry_ticks {
                // Alternate between autoneg restart and forced 100M full duplex
                // every other attempt (faster than the original 1-in-4 cadence,
                // which left users waiting up to 8 s on real notebooks for the
                // forced fallback to even be tried).
                let attempt_number = self.link_changes.saturating_add(
                    now.saturating_sub(self.last_autoneg_attempt_tick) / retry_ticks.max(1),
                );

                // Defensive: BIOS/firmware can clear bus-mastering after sleep
                // resume; reassert it on every retry while link is down.
                pci::enable_bus_mastering(&self.pci);

                // Wake PHY if it slipped back to power-down (BMCR bit 11).
                if let Some(bmcr) = self.read_phy(PHY_REG_CONTROL) {
                    if bmcr & PHY_CONTROL_POWER_DOWN != 0 {
                        let _ = self.write_phy(PHY_REG_CONTROL, bmcr & !PHY_CONTROL_POWER_DOWN);
                        crate::serial_println!(
                            "[rtl8169] link down: PHY was POWER_DOWN, woke it (bmcr=0x{:04X})",
                            bmcr
                        );
                    }
                }

                if attempt_number % 2 == 1 {
                    // Every other retry: forced 100M full duplex
                    crate::serial_println!(
                        "[rtl8169] link down; trying forced 100M-FD bmsr=0x{:04X} physt=0x{:02X}",
                        self.read_bmsr().unwrap_or(0),
                        self.read8(REG_PHY_STATUS)
                    );
                    let _ = self.write_phy(
                        PHY_REG_CONTROL,
                        PHY_CONTROL_SPEED_100 | PHY_CONTROL_FULL_DUPLEX,
                    );
                } else {
                    crate::serial_println!(
                        "[rtl8169] link down; restarting autoneg bmsr=0x{:04X} physt=0x{:02X}",
                        self.read_bmsr().unwrap_or(0),
                        self.read8(REG_PHY_STATUS)
                    );
                    let _ = self.program_autoneg_advertisement();
                    let _ = self.restart_autoneg("link-down");
                }
                self.last_autoneg_attempt_tick = now;
            }
        }
    }

    fn wait_until<F>(&self, timeout_ms: u64, mut ready: F) -> bool
    where
        F: FnMut() -> bool,
    {
        if ready() {
            return true;
        }

        let start_tick = interrupts::tick_count();
        let timeout_ticks = timeout_ms
            .saturating_mul(u64::from(pit::PIT_FREQUENCY_HZ))
            .div_ceil(1_000)
            .max(1);
        while interrupts::tick_count().saturating_sub(start_tick) < timeout_ticks {
            if ready() {
                return true;
            }
            let observed_tick = interrupts::tick_count();
            if !pit::wait_for_tick_advance(observed_tick, 1) {
                core::hint::spin_loop();
            }
        }
        ready()
    }

    fn log_identity(&self) {
        let command = pci::command_register(&self.pci);
        let status = pci::status_register(&self.pci);
        match self.window {
            RegisterWindow::Mmio { phys, .. } => crate::serial_println!(
                "[rtl8169] pci {:02X}:{:02X}.{} dev={:04X} rev=0x{:02X} mmio=0x{:08X} irq={} cmd=0x{:04X} sts=0x{:04X}",
                self.pci.bus,
                self.pci.device,
                self.pci.function,
                self.pci.device_id,
                self.pci.revision_id,
                phys,
                self.pci.interrupt_line,
                command,
                status
            ),
            RegisterWindow::Io(base) => crate::serial_println!(
                "[rtl8169] pci {:02X}:{:02X}.{} dev={:04X} rev=0x{:02X} io=0x{:04X} irq={} cmd=0x{:04X} sts=0x{:04X}",
                self.pci.bus,
                self.pci.device,
                self.pci.function,
                self.pci.device_id,
                self.pci.revision_id,
                base,
                self.pci.interrupt_line,
                command,
                status
            ),
        }
    }

    fn dump_registers(&self, stage: &str) {
        crate::serial_println!(
            "[rtl8169] {} cmd=0x{:02X} isr=0x{:04X} imr=0x{:04X} physt=0x{:02X} bmcr=0x{:04X} bmsr=0x{:04X} anar=0x{:04X} gbcr=0x{:04X}",
            stage,
            self.read8(REG_CHIP_CMD),
            self.read16(REG_INTR_STATUS),
            self.read16(REG_INTR_MASK),
            self.read8(REG_PHY_STATUS),
            self.read_phy(PHY_REG_CONTROL).unwrap_or(0),
            self.read_bmsr().unwrap_or(0),
            self.read_phy(PHY_REG_AUTO_NEGOTIATION).unwrap_or(0),
            self.read_phy(PHY_REG_GIGABIT_CONTROL).unwrap_or(0)
        );
    }

    fn tx_desc(&self, index: usize) -> &RtlDesc {
        unsafe { &*self.tx_descs.add(index) }
    }

    fn tx_desc_mut(&mut self, index: usize) -> &mut RtlDesc {
        unsafe { &mut *self.tx_descs.add(index) }
    }

    fn rx_desc(&self, index: usize) -> &RtlDesc {
        unsafe { &*self.rx_descs.add(index) }
    }

    fn rx_desc_mut(&mut self, index: usize) -> &mut RtlDesc {
        unsafe { &mut *self.rx_descs.add(index) }
    }

    fn read8(&self, register: u16) -> u8 {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                read_volatile((virt.as_u64() + u64::from(register)) as *const u8)
            },
            RegisterWindow::Io(base) => port::inb(base + register),
        }
    }

    fn read16(&self, register: u16) -> u16 {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                read_volatile((virt.as_u64() + u64::from(register)) as *const u16)
            },
            RegisterWindow::Io(base) => port::inw(base + register),
        }
    }

    fn read32(&self, register: u16) -> u32 {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                read_volatile((virt.as_u64() + u64::from(register)) as *const u32)
            },
            RegisterWindow::Io(base) => port::inl(base + register),
        }
    }

    fn read_phy(&self, register: u8) -> Option<u16> {
        // Ensure any prior PHYAR transaction has drained.
        // Without this, a fast immediate poll can see stale bit-31 from
        // the previous read/write and return garbage.
        self.post_mdio_delay();

        // Initiate read: write register address with bit31 = 0.
        self.write32(REG_PHY_ACCESS, u32::from(register) << 16);

        // MMIO write → device latency gap: the register won't reflect the
        // new transaction instantly.  Burn a spin so the first poll reads
        // post-write state, not the stale previous value.
        for _ in 0..1024 {
            core::hint::spin_loop();
        }

        // Poll until bit 31 becomes SET (hardware signals data ready).
        if self.wait_until(PHY_TIMEOUT_MS, || {
            self.read32(REG_PHY_ACCESS) & PHY_ACCESS_BUSY != 0
        }) {
            let value = self.read32(REG_PHY_ACCESS);
            self.post_mdio_delay();
            Some((value & 0xFFFF) as u16)
        } else {
            crate::serial_println!("[rtl8169] phy read timeout reg=0x{:02X}", register);
            None
        }
    }

    fn write8(&self, register: u16, value: u8) {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                write_volatile((virt.as_u64() + u64::from(register)) as *mut u8, value);
            },
            RegisterWindow::Io(base) => port::outb(base + register, value),
        }
    }

    fn write16(&self, register: u16, value: u16) {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                write_volatile((virt.as_u64() + u64::from(register)) as *mut u16, value);
            },
            RegisterWindow::Io(base) => port::outw(base + register, value),
        }
    }

    fn write32(&self, register: u16, value: u32) {
        match self.window {
            RegisterWindow::Mmio { virt, .. } => unsafe {
                write_volatile((virt.as_u64() + u64::from(register)) as *mut u32, value);
            },
            RegisterWindow::Io(base) => port::outl(base + register, value),
        }
    }

    fn write_phy(&self, register: u8, value: u16) -> Result<(), NetError> {
        // Drain any prior PHYAR transaction before starting a new one.
        self.post_mdio_delay();

        self.write32(
            REG_PHY_ACCESS,
            PHY_ACCESS_BUSY | (u32::from(register) << 16) | u32::from(value),
        );

        // MMIO write → device latency gap (same rationale as read_phy).
        for _ in 0..1024 {
            core::hint::spin_loop();
        }

        // Poll until bit 31 clears (write accepted).
        if self.wait_until(PHY_TIMEOUT_MS, || {
            self.read32(REG_PHY_ACCESS) & PHY_ACCESS_BUSY == 0
        }) {
            self.post_mdio_delay();
            crate::serial_println!(
                "[rtl8169] phy write reg=0x{:02X} val=0x{:04X}",
                register,
                value
            );
            Ok(())
        } else {
            crate::serial_println!(
                "[rtl8169] phy write timeout reg=0x{:02X} val=0x{:04X}",
                register,
                value
            );
            Err(NetError::InitializationFailed(
                "rtl8169 PHY write timed out",
            ))
        }
    }

    fn post_mdio_delay(&self) {
        // Linux r8169 uses udelay(20) (~60k cycles at 3GHz).
        // PAUSE on post-Skylake x86 takes ~140 cycles/iter, pre-Skylake ~10.
        // 2048 iterations: ~287k cycles (post-Skylake) or ~20k (pre-Skylake).
        // Provides safe margin for MDIO bridge settling on real hardware.
        for _ in 0..2048 {
            core::hint::spin_loop();
        }
    }

    /// Identify the exact MAC version from TxConfig register bits [30:20].
    /// This determines chip-specific quirks (RTL8168B/C/D/E/F/G/H vs RTL8169).
    fn detect_mac_version(&mut self) {
        let tx_config = self.read32(REG_TX_CONFIG);
        self.mac_version = (tx_config >> 20) & 0xFFC;
        self.is_rtl8168 = self.pci.device_id == 0x8168 || self.pci.device_id == 0x8161;

        // Also check TxConfig version bits for RTL8168 variants that report as 8169
        let hwver = (tx_config >> 26) & 0x0F;
        if hwver >= 0x04 && !self.is_rtl8168 {
            self.is_rtl8168 = true;
        }

        crate::serial_println!(
            "[rtl8169] mac_version=0x{:03X} is_rtl8168={} device_id=0x{:04X} rev=0x{:02X} txcfg=0x{:08X}",
            self.mac_version,
            self.is_rtl8168,
            self.pci.device_id,
            self.pci.revision_id,
            tx_config
        );
    }

    /// Disable ASF (Alert Standard Format) / Dashboard on RTL8168.
    /// ASF firmware can interfere with driver operation on real hardware.
    fn disable_asf(&self) {
        // Write to misc register to disable ASF handshake
        self.write8(REG_CFG9346, CFG9346_UNLOCK);
        let config1 = self.read8(REG_CONFIG1);
        // Disable PMEn (Power Management Event) which can cause spurious wakes
        self.write8(REG_CONFIG1, config1 & !0x01);
        let misc = self.read32(REG_MISC);
        // Disable write-enable bit for magic packet
        self.write32(REG_MISC, misc & !0x0020);
        self.write8(REG_CFG9346, CFG9346_LOCK);
        crate::serial_println!(
            "[rtl8169] ASF/PME disabled config1=0x{:02X}->0x{:02X}",
            config1,
            self.read8(REG_CONFIG1)
        );
    }

    /// Disable IEEE 802.3az Energy Efficient Ethernet on the internal PHY.
    /// EEE is the leading cause of intermittent link drops on RTL8168 family
    /// notebooks. Clearing MMD 7 register 0x3C disables EEE advertisement so
    /// the PHY never enters LPI mode after link-up.
    fn disable_eee(&self) {
        // Step 1: Direct MMD access via reg 0x0D / 0x0E
        // Set function = address (00), devad = 7
        let _ = self.write_phy(PHY_REG_MMD_ACCESS_CTRL, PHY_MMD_DEVICE_AN);
        let _ = self.write_phy(PHY_REG_MMD_ACCESS_DATA, PHY_MMD_AN_EEE_ADV);
        // Set function = data, no post-increment
        let _ = self.write_phy(
            PHY_REG_MMD_ACCESS_CTRL,
            PHY_MMD_FUNCTION_DATA_NO_INCR | PHY_MMD_DEVICE_AN,
        );
        // Write 0x0000 to MMD 7.0x3C (clear all EEE advertisement bits)
        let _ = self.write_phy(PHY_REG_MMD_ACCESS_DATA, 0x0000);
        crate::serial_println!(
            "[rtl8169] EEE disabled via MMD 7.{:04X} (link-stability fix for RTL8168)",
            PHY_MMD_AN_EEE_ADV
        );
    }

    /// Force a complete bring-up retry from shell or background. Re-arms the
    /// PHY power-up, advertisement, autoneg restart, and re-enables datapath.
    /// Returns true if the link came up within `timeout_ms`.
    pub fn force_retry(&mut self, timeout_ms: u64) -> bool {
        crate::serial_println!("[rtl8169] force_retry begin timeout={}ms", timeout_ms);

        // Defensive: re-enable bus mastering in case BIOS/firmware cleared it
        // after a sleep/resume cycle.
        pci::enable_bus_mastering(&self.pci);
        pci::disable_aspm(&self.pci);

        // Wake the PHY if it slipped back into power-down.
        if let Some(bmcr) = self.read_phy(PHY_REG_CONTROL) {
            if bmcr & PHY_CONTROL_POWER_DOWN != 0 {
                let _ = self.write_phy(PHY_REG_CONTROL, bmcr & !PHY_CONTROL_POWER_DOWN);
                self.wait_until(50, || false);
                crate::serial_println!("[rtl8169] force_retry: woke PHY from power-down");
            }
        }

        // Re-program advertisement and restart autoneg.
        let _ = self.program_autoneg_advertisement();
        let _ = self.restart_autoneg("force-retry");

        // Re-enable RX/TX (defensive).
        self.write8(REG_CHIP_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);
        self.write32(REG_RX_MISSED, 0);

        let autoneg_done = self.wait_for_auto_negotiation(timeout_ms.min(2_000));
        if autoneg_done {
            crate::serial_println!(
                "[rtl8169] force_retry: autoneg complete anlpar=0x{:04X}",
                self.read_phy(PHY_REG_LINK_PARTNER).unwrap_or(0)
            );
        }

        let mut link_up = self.wait_for_link_up(timeout_ms);
        if !link_up {
            // Last resort: forced 100M-FD probe.
            crate::serial_println!("[rtl8169] force_retry: trying forced 100M-FD");
            let _ = self.write_phy(
                PHY_REG_CONTROL,
                PHY_CONTROL_SPEED_100 | PHY_CONTROL_FULL_DUPLEX,
            );
            link_up = self.wait_for_link_up(BOOT_FORCED_LINK_TIMEOUT_MS);
            if !link_up {
                // Re-enable autoneg in background so it keeps trying.
                let _ = self.program_autoneg_advertisement();
                let _ = self.restart_autoneg("force-retry-background");
            }
        }

        if link_up {
            self.last_link_up = true;
            self.write8(REG_CHIP_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);
        }
        crate::serial_println!(
            "[rtl8169] force_retry done link_up={} bmsr=0x{:04X} physt=0x{:02X}",
            link_up,
            self.read_bmsr().unwrap_or(0),
            self.read8(REG_PHY_STATUS)
        );
        link_up
    }

    /// Check for TX stall condition and attempt recovery.
    /// On real hardware, TX can hang if the chip enters a bad state.
    fn check_tx_stall(&mut self) {
        let now = interrupts::tick_count();
        let stall_threshold_ticks = 500u64
            .saturating_mul(u64::from(pit::PIT_FREQUENCY_HZ))
            .div_ceil(1_000)
            .max(1); // 500ms

        if self.last_tx_tick > 0
            && now.saturating_sub(self.last_tx_tick) > stall_threshold_ticks
        {
            crate::serial_println!(
                "[rtl8169] TX stall detected ({}ms), re-poking TX poll",
                now.saturating_sub(self.last_tx_tick) * 1000 / u64::from(pit::PIT_FREQUENCY_HZ).max(1)
            );
            self.tx_errors = self.tx_errors.saturating_add(1);
            // Re-enable TX and poke the poll register
            let cmd = self.read8(REG_CHIP_CMD);
            if cmd & CMD_TX_ENABLE == 0 {
                self.write8(REG_CHIP_CMD, cmd | CMD_TX_ENABLE | CMD_RX_ENABLE);
            }
            self.write8(REG_TX_POLL, TX_POLL_NORMAL);
            self.last_tx_tick = now;
        }
    }
}
