#![allow(dead_code)]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::spin_loop;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use core::slice;
use core::sync::atomic::{fence, AtomicU32, Ordering};

use x86_64::PhysAddr;

use crate::arch::x86_64::{interrupts, pit};
use crate::boot::trace::{self, BreadcrumbTag};
use crate::memory;
use crate::net::buffer::DmaRegion;
use crate::net::pci::{self, PciBar, PciDevice};

use super::super::device::{DeviceCategory, DeviceId, UsbSpeed};
use super::descriptors::{self, EndpointDirection, TransferType, UsbEndpoint, UsbInterface};
use super::hid::{self, HidKind, KeyboardBootState};
use super::mass_storage::{self, Cbw, Csw, UsbMassStorageInfo};

const CAPLENGTH: u64 = 0x00;
const HCSPARAMS1: u64 = 0x04;
const HCSPARAMS2: u64 = 0x08;
const HCCPARAMS1: u64 = 0x10;
const DBOFF: u64 = 0x14;
const RTSOFF: u64 = 0x18;

const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
const PAGESIZE: u64 = 0x08;
const CRCR: u64 = 0x18;
const DCBAAP: u64 = 0x30;
const CONFIG: u64 = 0x38;

const PORT_REGS_BASE: u64 = 0x400;
const PORT_REGS_STRIDE: u64 = 0x10;
const PORTSC: u64 = 0x00;

const IMAN: u64 = 0x00;
const ERSTSZ: u64 = 0x08;
const ERSTBA: u64 = 0x10;
const ERDP: u64 = 0x18;

const TRB_NORMAL: u8 = 1;
const TRB_SETUP_STAGE: u8 = 2;
const TRB_DATA_STAGE: u8 = 3;
const TRB_STATUS_STAGE: u8 = 4;
const TRB_LINK: u8 = 6;
const TRB_ENABLE_SLOT: u8 = 9;
const TRB_DISABLE_SLOT: u8 = 10;
const TRB_ADDRESS_DEVICE: u8 = 11;
const TRB_CONFIGURE_ENDPOINT: u8 = 12;
const TRB_EVALUATE_CONTEXT: u8 = 13;
const TRB_TRANSFER_EVENT: u8 = 32;
const TRB_COMMAND_COMPLETION: u8 = 33;
const TRB_PORT_STATUS_CHANGE: u8 = 34;

const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_PLS_MASK: u32 = 0xF << 5;
const PORTSC_PP: u32 = 1 << 9;
const PORTSC_PIC_MASK: u32 = 0x3 << 14;
const PORTSC_WCE: u32 = 1 << 25;
const PORTSC_WDE: u32 = 1 << 26;
const PORTSC_WOE: u32 = 1 << 27;
const PORTSC_CSC: u32 = 1 << 17;
const PORTSC_PEC: u32 = 1 << 18;
const PORTSC_PRC: u32 = 1 << 21;
const PORTSC_PLC: u32 = 1 << 22;
const PORTSC_CEC: u32 = 1 << 23;
const PORTSC_WRC: u32 = 1 << 19;
const PORTSC_WPR: u32 = 1 << 31; // Warm Port Reset (USB 3.0+)
const PORTSC_CHANGE_BITS: u32 = PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;
const PORTSC_PRESERVE_WRITE_BITS: u32 =
    PORTSC_PP | PORTSC_PIC_MASK | PORTSC_WCE | PORTSC_WDE | PORTSC_WOE;

const EP_TYPE_INTERRUPT_OUT: u32 = 3;
const EP_TYPE_CONTROL: u32 = 4;
const EP_TYPE_BULK_OUT: u32 = 2;
const EP_TYPE_BULK_IN: u32 = 6;
const EP_TYPE_INTERRUPT_IN: u32 = 7;

const SETUP_TRT_NO_DATA: u32 = 0;
const SETUP_TRT_OUT: u32 = 2;
const SETUP_TRT_IN: u32 = 3;

const IOC: u32 = 1 << 5;
const IDT: u32 = 1 << 6;
const ISP: u32 = 1 << 2;

const COMPLETION_SUCCESS: u8 = 1;
const COMPLETION_SHORT_PACKET: u8 = 13;
const INITIAL_MMIO_MAP_SIZE: usize = 4096;
const PORT_RESET_TIMEOUT_MS: u64 = 500;
const PORT_ENABLE_TIMEOUT_MS: u64 = 1500;
const PORT_RESET_SETTLE_DELAY_MS: u64 = 20;
const PORT_ENABLE_SETTLE_DELAY_MS: u64 = 10;
const USB2_CONNECT_DEBOUNCE_MS: u64 = 100;
const MAX_EVENTS_PER_POLL: usize = 256;
const XHCI_HID_INPUT_TRACE_ENABLED: bool = false;
const XHCI_RUNTIME_EVENT_TRACE_ENABLED: bool = false;
const PORT_RESCAN_COOLDOWN_MS: u64 = 500;
const RUNTIME_TOPOLOGY_CATCH_UP_BUDGET_MS: u64 = 3000;
const RETIRED_PORT_FAILURE_RETENTION_MS: u64 = 15_000;

// --- USB networking protocol constants ---
const RNDIS_MSG_INIT: u32 = 0x0000_0002;
const RNDIS_MSG_INIT_C: u32 = 0x8000_0002;
const RNDIS_MSG_SET: u32 = 0x0000_0005;
const RNDIS_MSG_SET_C: u32 = 0x8000_0005;
const RNDIS_MSG_QUERY: u32 = 0x0000_0004;
const RNDIS_MSG_QUERY_C: u32 = 0x8000_0004;
const RNDIS_MSG_PACKET: u32 = 0x0000_0001;
const OID_GEN_CURRENT_PACKET_FILTER: u32 = 0x0001_010E;
const OID_802_3_CURRENT_ADDRESS: u32 = 0x0101_0102;
const NDIS_PACKET_FILTER_ALL: u32 = 0x000F; // directed+multicast+all_multicast+broadcast
const NDIS_PACKET_FILTER_PROMISCUOUS: u32 = 0x0020;
const RNDIS_PACKET_HEADER_SIZE: usize = 44;
const USB_NET_MAX_TRANSFER: u32 = 0x4000; // 16 KB

static HID_ARM_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);
static HID_TRANSFER_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);
static HID_TRANSFER_FAIL_TRACE_COUNT: AtomicU32 = AtomicU32::new(0);

#[repr(C, align(16))]
#[derive(Clone, Copy, Default)]
pub struct Trb {
    pub parameter: u64,
    pub status: u32,
    pub control: u32,
}

impl Trb {
    #[must_use]
    pub fn trb_type(self) -> u8 {
        ((self.control >> 10) & 0x3F) as u8
    }

    #[must_use]
    pub fn completion_code(self) -> u8 {
        ((self.status >> 24) & 0xFF) as u8
    }

    #[must_use]
    pub fn slot_id(self) -> u8 {
        ((self.control >> 24) & 0xFF) as u8
    }

    #[must_use]
    pub fn endpoint_id(self) -> u8 {
        ((self.control >> 16) & 0x1F) as u8
    }

    #[must_use]
    pub fn port_id(self) -> u8 {
        ((self.parameter >> 24) & 0xFF) as u8
    }

    #[must_use]
    pub fn transfer_residue(self) -> u32 {
        self.status & 0x00FF_FFFF
    }

    #[must_use]
    pub fn cycle(self) -> bool {
        self.control & 1 != 0
    }

    pub fn set_cycle(&mut self, value: bool) {
        if value {
            self.control |= 1;
        } else {
            self.control &= !1;
        }
    }
}

pub struct XhciRing {
    _region: DmaRegion,
    pub phys_addr: u64,
    trbs: *mut Trb,
    pub size: usize,
    enqueue_index: usize,
    dequeue_index: usize,
    cycle_state: bool,
    has_link: bool,
}

#[derive(Debug, Clone)]
pub struct UsbInterfaceStatus {
    pub number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub endpoint_count: u8,
    pub has_bulk_in: bool,
    pub has_bulk_out: bool,
    pub has_interrupt_in: bool,
    pub has_interrupt_out: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbPortVisibility {
    Generic,
    Input,
    Storage,
    PhoneMedia,
    PhoneAdb,
    VendorSpecific,
    NetworkCandidate,
    NetworkReady,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbAttachState {
    NotApplicable,
    Unsupported,
    Candidate,
    Ready,
}

#[derive(Clone)]
pub struct UsbPortStatus {
    pub port: u8,
    pub connected: bool,
    pub enabled: bool,
    pub speed: UsbSpeed,
    pub slot_id: Option<u8>,
    pub addressed: bool,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub category: DeviceCategory,
    pub driver: &'static str,
    pub name: String,
    pub configured: bool,
    pub configuration_count: u8,
    pub active_configuration: Option<u8>,
    pub class_code: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub interface_count: u8,
    pub interfaces: Vec<UsbInterfaceStatus>,
    pub visibility: UsbPortVisibility,
    pub attach_state: UsbAttachState,
    pub attach_reason: &'static str,
    pub enumeration_note: &'static str,
    pub hid_kind: Option<HidKind>,
    pub storage: Option<UsbMassStorageInfo>,
    pub network: Option<UsbNetworkInfo>,
}

pub(super) struct HidEndpoint {
    endpoint_address: u8,
    pub(super) endpoint_id: u8,
    ring: XhciRing,
    report_buffer: DmaRegion,
    report_size: usize,
    pub(super) in_flight_trb: Option<u64>,
    pub(super) hid_kind: HidKind,
    keyboard_state: KeyboardBootState,
}

struct StorageEndpoint {
    bulk_out_address: u8,
    bulk_out_id: u8,
    bulk_out_ring: XhciRing,
    bulk_in_address: u8,
    bulk_in_id: u8,
    bulk_in_ring: XhciRing,
    tag: u32,
    info: UsbMassStorageInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsbNetProtocol {
    CdcEcm,
    Rndis,
    CdcNcm,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbNetworkInfo {
    pub protocol: UsbNetProtocol,
    pub mac: [u8; 6],
    pub max_frame_size: u16,
    pub controller_index: usize,
    pub slot_id: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbNetDiagnostics {
    pub protocol: UsbNetProtocol,
    pub mac: [u8; 6],
    pub rx_raw_events: u64,
    pub rx_frames: u64,
    pub tx_frames: u64,
    pub tx_errors: u64,
    pub rx_errors: u64,
    pub rx_extract_errors: u64,
    pub rx_short_frames: u64,
    pub rx_last_raw_len: usize,
    pub rx_last_frame_len: usize,
    pub rx_last_drop: &'static str,
    pub rx_queue_depth: usize,
    pub rx_armed: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbRuntimeControllerStatus {
    pub pending_topology: bool,
    pub last_runtime_event: &'static str,
    pub last_runtime_event_port: Option<u8>,
    pub last_runtime_event_ms: u64,
    pub last_topology_service_result: &'static str,
    pub last_topology_service_ms: u64,
    pub last_inventory_sync_ms: u64,
    pub last_inventory_sync_devices: usize,
    pub cached_ports: usize,
    pub cached_connected_devices: usize,
    pub active_attempt_port: Option<u8>,
    pub progress: UsbRuntimePortProgress,
    pub last_enumeration_failure_port: Option<u8>,
    pub last_enumeration_failure_reason: &'static str,
    pub stale_failed_ports: [Option<UsbRetiredPortFailure>; 4],
}

#[derive(Debug, Clone, Copy)]
pub struct UsbRetiredPortFailure {
    pub port: u8,
    pub stage: &'static str,
    pub reason: &'static str,
    pub noted_ms: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct UsbRuntimePortProgress {
    pub port: Option<u8>,
    pub connect_seen: bool,
    pub reset_attempted: bool,
    pub reset_completed: bool,
    pub reset_completion_source: &'static str,
    pub warm_reset_attempted: bool,
    pub port_enabled: bool,
    pub ped_observed: bool,
    pub ped_first_seen_ms: u64,
    pub ped_last_seen_ms: u64,
    pub ped_lost_after_observed: bool,
    pub slot_allocated: bool,
    pub address_assigned: bool,
    pub descriptors_fetched: bool,
    pub configuration_attempted: bool,
    pub configured: bool,
    pub budget_exhausted_after_reset: bool,
    pub last_stage: &'static str,
    pub last_stage_ms: u64,
    pub failure_step: &'static str,
    pub last_portsc: u32,
    pub last_portsc_ms: u64,
    pub last_change_bits: u32,
    pub last_ack_bits: u32,
    pub observed_change_bits: u32,
    pub observed_ack_bits: u32,
    pub last_link_state: &'static str,
}

impl Default for UsbRuntimePortProgress {
    fn default() -> Self {
        Self {
            port: None,
            connect_seen: false,
            reset_attempted: false,
            reset_completed: false,
            reset_completion_source: "none",
            warm_reset_attempted: false,
            port_enabled: false,
            ped_observed: false,
            ped_first_seen_ms: 0,
            ped_last_seen_ms: 0,
            ped_lost_after_observed: false,
            slot_allocated: false,
            address_assigned: false,
            descriptors_fetched: false,
            configuration_attempted: false,
            configured: false,
            budget_exhausted_after_reset: false,
            last_stage: "none",
            last_stage_ms: 0,
            failure_step: "none",
            last_portsc: 0,
            last_portsc_ms: 0,
            last_change_bits: 0,
            last_ack_bits: 0,
            observed_change_bits: 0,
            observed_ack_bits: 0,
            last_link_state: "unknown",
        }
    }
}

struct NetworkEndpoint {
    bulk_out_address: u8,
    bulk_out_id: u8,
    bulk_out_ring: XhciRing,
    bulk_in_address: u8,
    bulk_in_id: u8,
    bulk_in_ring: XhciRing,
    protocol: UsbNetProtocol,
    mac: [u8; 6],
    max_transfer_size: u32,
    control_interface: u8,
    data_interface: u8,
    rx_buffer: DmaRegion,
    rx_armed: bool,
    rx_queue: Vec<Vec<u8>>,
    rx_raw_events: u64,
    rx_frames: u64,
    tx_frames: u64,
    tx_errors: u64,
    rx_errors: u64,
    rx_extract_errors: u64,
    rx_short_frames: u64,
    rx_last_raw_len: usize,
    rx_last_frame_len: usize,
    rx_last_drop: &'static str,
}

pub(super) struct UsbSlotState {
    port: u8,
    speed: UsbSpeed,
    input_context: DmaRegion,
    output_context: DmaRegion,
    ep0_ring: XhciRing,
    max_packet_size0: u16,
    addressed: bool,
    pub(super) hid: Option<HidEndpoint>,
    storage: Option<StorageEndpoint>,
    network: Option<NetworkEndpoint>,
}

pub struct XhciController {
    pub pci: PciDevice,
    pub controller_device_id: Option<DeviceId>,
    pub mmio_phys: u64,
    pub mmio_base: u64,
    pub op_base: u64,
    pub rt_base: u64,
    pub db_base: u64,
    pub max_slots: u8,
    pub max_ports: u8,
    pub context_size: usize,
    pub command_ring: XhciRing,
    pub event_ring: XhciRing,
    dcbaa: DmaRegion,
    erst: DmaRegion,
    scratchpad_array: Option<DmaRegion>,
    scratchpad_buffers: Vec<DmaRegion>,
    pub(super) slots: Vec<Option<UsbSlotState>>,
    pub ports: Vec<UsbPortStatus>,
    last_command_name: &'static str,
    last_command_trb_phys: u64,
    last_command_trb: Option<Trb>,
    last_command_completion: Option<Trb>,
    last_transfer_completion: Option<Trb>,
    pending_port_rescan: bool,
    last_port_rescan_ms: u64,
    last_runtime_event: &'static str,
    last_runtime_event_port: Option<u8>,
    last_runtime_event_ms: u64,
    last_topology_service_result: &'static str,
    last_topology_service_ms: u64,
    last_inventory_sync_ms: u64,
    last_inventory_sync_devices: usize,
    runtime_catch_up_deadline_ms: u64,
    runtime_catch_up_aborted: bool,
    last_port_progress: UsbRuntimePortProgress,
    last_enumeration_failure_port: Option<u8>,
    last_enumeration_failure_reason: &'static str,
    stale_failed_ports: [Option<UsbRetiredPortFailure>; 4],
}

struct SetupPacket {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
}

unsafe impl Send for XhciRing {}
unsafe impl Send for XhciController {}

impl XhciRing {
    pub fn new(size: usize, has_link: bool) -> Result<Self, &'static str> {
        let size = size.max(16).next_power_of_two();
        let mut region = DmaRegion::allocate(size * size_of::<Trb>())
            .map_err(|_| "xHCI ring allocation failed")?;
        let phys_addr = region.physical().as_u64();
        let trbs = region.as_mut_ptr().cast::<Trb>();

        for index in 0..size {
            unsafe {
                write_volatile(trbs.add(index), Trb::default());
            }
        }

        let mut ring = Self {
            _region: region,
            phys_addr,
            trbs,
            size,
            enqueue_index: 0,
            dequeue_index: 0,
            cycle_state: true,
            has_link,
        };

        if has_link {
            ring.write_link_trb();
        }

        Ok(ring)
    }

    pub fn enqueue(&mut self, mut trb: Trb) -> Result<u64, &'static str> {
        if self.has_link && self.enqueue_index == self.size - 1 {
            self.wrap_enqueue();
        }

        let index = self.enqueue_index;
        trb.set_cycle(self.cycle_state);
        unsafe {
            write_volatile(self.trbs.add(index), trb);
        }
        let phys = self.phys_addr + (index as u64) * size_of::<Trb>() as u64;
        self.enqueue_index += 1;
        if self.has_link && self.enqueue_index == self.size - 1 {
            self.wrap_enqueue();
        } else if !self.has_link && self.enqueue_index == self.size {
            self.enqueue_index = 0;
            self.cycle_state = !self.cycle_state;
        }

        Ok(phys)
    }

    pub fn dequeue(&mut self) -> Option<Trb> {
        let trb = unsafe { read_volatile(self.trbs.add(self.dequeue_index)) };
        if trb.cycle() != self.cycle_state {
            return None;
        }

        self.dequeue_index += 1;
        if self.dequeue_index >= self.size {
            self.dequeue_index = 0;
            self.cycle_state = !self.cycle_state;
        }

        Some(trb)
    }

    #[must_use]
    pub fn dequeue_pointer(&self) -> u64 {
        self.phys_addr + (self.dequeue_index as u64) * size_of::<Trb>() as u64
    }

    fn wrap_enqueue(&mut self) {
        if !self.has_link {
            return;
        }
        self.write_link_trb();
        self.enqueue_index = 0;
        self.cycle_state = !self.cycle_state;
    }

    fn write_link_trb(&mut self) {
        let mut trb = Trb {
            parameter: self.phys_addr,
            status: 0,
            control: ((TRB_LINK as u32) << 10) | (1 << 1),
        };
        trb.set_cycle(self.cycle_state);
        unsafe {
            write_volatile(self.trbs.add(self.size - 1), trb);
        }
    }
}

impl SetupPacket {
    #[must_use]
    fn direction_in(&self) -> bool {
        self.request_type & 0x80 != 0
    }

    #[must_use]
    fn as_u64(&self) -> u64 {
        u64::from(self.request_type)
            | (u64::from(self.request) << 8)
            | (u64::from(self.value) << 16)
            | (u64::from(self.index) << 32)
            | (u64::from(self.length) << 48)
    }

    #[must_use]
    fn transfer_type(&self) -> u32 {
        if self.length == 0 {
            SETUP_TRT_NO_DATA
        } else if self.direction_in() {
            SETUP_TRT_IN
        } else {
            SETUP_TRT_OUT
        }
    }
}

impl XhciController {
    pub fn init(pci: PciDevice) -> Result<Self, &'static str> {
        let mmio_phys = match pci.bar(0) {
            PciBar::Memory32(base) => u64::from(base),
            PciBar::Memory64(base) => base,
            _ => return Err("xHCI requires a memory BAR"),
        };

        pci::enable_bus_mastering(&pci);
        let mut mmio_base = map_xhci_mmio(mmio_phys, INITIAL_MMIO_MAP_SIZE)?;

        let cap_length = read_mmio8(mmio_base + CAPLENGTH) as u64;
        let hcs_params1 = read_mmio32(mmio_base + HCSPARAMS1);
        let hcs_params2 = read_mmio32(mmio_base + HCSPARAMS2);
        let hcc_params1 = read_mmio32(mmio_base + HCCPARAMS1);
        let db_offset = read_mmio32(mmio_base + DBOFF);
        let rt_offset = read_mmio32(mmio_base + RTSOFF);
        let max_slots = (hcs_params1 & 0xFF) as u8;
        let max_ports = ((hcs_params1 >> 24) & 0xFF) as u8;
        let scratchpad_count = scratchpad_buffer_count(hcs_params2) as usize;
        let required_mmio_len = required_mmio_span(
            cap_length,
            db_offset as u64,
            rt_offset as u64,
            max_slots,
            max_ports,
        );
        if required_mmio_len > INITIAL_MMIO_MAP_SIZE {
            mmio_base = map_xhci_mmio(mmio_phys, required_mmio_len)?;
        }

        let op_base = mmio_base + cap_length;
        let rt_base = mmio_base + (rt_offset as u64);
        let db_base = mmio_base + (db_offset as u64);
        let page_size_mask = read_mmio32(op_base + PAGESIZE);
        if page_size_mask & 0x1 == 0 {
            return Err("xHCI does not advertise 4 KiB page support");
        }
        let context_size = if hcc_params1 & (1 << 2) != 0 { 64 } else { 32 };

        let command_ring = XhciRing::new(256, true)?;
        let event_ring = XhciRing::new(256, false)?;
        let dcbaa = DmaRegion::allocate(256 * size_of::<u64>())
            .map_err(|_| "xHCI DCBAA allocation failed")?;
        let erst = DmaRegion::allocate(16).map_err(|_| "xHCI ERST allocation failed")?;
        let (scratchpad_array, scratchpad_buffers) = allocate_scratchpads(scratchpad_count)?;

        let mut slots = Vec::new();
        slots.resize_with(max_slots as usize + 1, || None);

        let mut controller = Self {
            pci,
            controller_device_id: None,
            mmio_phys,
            mmio_base,
            op_base,
            rt_base,
            db_base,
            max_slots,
            max_ports,
            context_size,
            command_ring,
            event_ring,
            dcbaa,
            erst,
            scratchpad_array,
            scratchpad_buffers,
            slots,
            ports: Vec::new(),
            last_command_name: "none",
            last_command_trb_phys: 0,
            last_command_trb: None,
            last_command_completion: None,
            last_transfer_completion: None,
            pending_port_rescan: false,
            last_port_rescan_ms: 0,
            last_runtime_event: "init",
            last_runtime_event_port: None,
            last_runtime_event_ms: 0,
            last_topology_service_result: "startup",
            last_topology_service_ms: 0,
            last_inventory_sync_ms: 0,
            last_inventory_sync_devices: 0,
            runtime_catch_up_deadline_ms: 0,
            runtime_catch_up_aborted: false,
            last_port_progress: UsbRuntimePortProgress::default(),
            last_enumeration_failure_port: None,
            last_enumeration_failure_reason: "none",
            stale_failed_ports: [None; 4],
        };

        crate::serial_println!(
            "[xHCI] controller {:02X}:{:02X}.{} mmio=0x{:X} virt=0x{:X} span=0x{:X} slots={} ports={} ctx={} page-mask=0x{:X} scratchpads={}",
            controller.pci.bus,
            controller.pci.device,
            controller.pci.function,
            controller.mmio_phys,
            controller.mmio_base,
            required_mmio_len,
            controller.max_slots,
            controller.max_ports,
            controller.context_size,
            page_size_mask,
            controller.scratchpad_buffers.len()
        );
        controller.reset()?;
        controller.configure_runtime()?;
        controller.start()?;
        let _ = controller.scan_ports();
        controller.last_port_rescan_ms = current_time_ms();
        controller.last_runtime_event = "startup-scan";
        controller.last_runtime_event_ms = controller.last_port_rescan_ms;
        controller.last_topology_service_result = "startup-scan";
        controller.last_topology_service_ms = controller.last_port_rescan_ms;
        Ok(controller)
    }

    pub fn scan_ports(&mut self) -> bool {
        self.prune_stale_failed_ports();
        let previous = self.ports.clone();
        let mut retained_slots = [false; 256];
        self.ports.clear();
        self.runtime_catch_up_aborted = false;
        self.last_enumeration_failure_port = None;
        self.last_enumeration_failure_reason = "none";

        for index in 0..self.max_ports {
            let port = index + 1;
            let portsc = self.power_port_if_needed(port);
            if portsc & PORTSC_CCS == 0 {
                continue;
            }
            let port_changed =
                portsc & (PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC)
                    != 0;

            let speed = decode_speed(((portsc >> 10) & 0xF) as u8);
            log_portsc("connected", port, portsc);
            self.note_portsc_snapshot(port, "connect-seen", portsc, 0);
            if let Err(error) = self.check_runtime_budget(port, "connect-seen") {
                self.ports.push(UsbPortStatus {
                    port,
                    connected: true,
                    enabled: false,
                    speed,
                    slot_id: None,
                    addressed: false,
                    vendor_id: None,
                    product_id: None,
                    category: DeviceCategory::UsbDevice,
                    driver: "xhci-runtime-budget",
                    name: format!(
                        "USB device on port {} ({:?}) runtime catch-up budget exhausted",
                        port, speed
                    ),
                    configured: false,
                    configuration_count: 0,
                    active_configuration: None,
                    class_code: 0,
                    subclass: 0,
                    protocol: 0,
                    interface_count: 0,
                    interfaces: Vec::new(),
                    visibility: UsbPortVisibility::Generic,
                    attach_state: UsbAttachState::NotApplicable,
                    attach_reason: "runtime catch-up budget exhausted before enumeration could start",
                    enumeration_note: error,
                    hid_kind: None,
                    storage: None,
                    network: None,
                });
                break;
            }
            let port_enabled = portsc & PORTSC_PED != 0;

            if let Some(previous_status) = previous.iter().find(|status| status.port == port) {
                if let Some(slot_id) = previous_status.slot_id {
                    let slot_alive = self
                        .slots
                        .get(slot_id as usize)
                        .and_then(Option::as_ref)
                        .is_some();
                    if previous_status.connected && port_enabled && slot_alive && !port_changed {
                        retained_slots[slot_id as usize] = true;
                        let mut preserved = previous_status.clone();
                        preserved.connected = true;
                        preserved.enabled = true;
                        preserved.speed = speed;
                        self.ports.push(preserved);
                        continue;
                    }

                    self.teardown_slot(slot_id, false);
                }
            }

            let mut status = UsbPortStatus {
                port,
                connected: true,
                enabled: false,
                speed,
                slot_id: None,
                addressed: false,
                vendor_id: None,
                product_id: None,
                category: DeviceCategory::UsbDevice,
                driver: "xhci-port",
                name: format!("USB device on port {} ({:?})", port, speed),
                configured: false,
                configuration_count: 0,
                active_configuration: None,
                class_code: 0,
                subclass: 0,
                protocol: 0,
                interface_count: 0,
                interfaces: Vec::new(),
                visibility: UsbPortVisibility::Generic,
                attach_state: UsbAttachState::NotApplicable,
                attach_reason: "no supported USB network interfaces visible",
                enumeration_note: "port connected; reset pending",
                hid_kind: None,
                storage: None,
                network: None,
            };

            let enabled_portsc = match self.reset_port(port) {
                Ok(portsc) => portsc,
                Err(error) => {
                    crate::serial_println!("[xHCI] port {} reset/enable failed: {}", port, error);
                    self.note_enumeration_failure(port, self.last_port_progress.last_stage, error);
                    status.driver = "xhci-port-not-enabled";
                    status.name = format!(
                        "USB device on port {} ({:?}) not enabled: {}",
                        port, speed, error
                    );
                    status.enumeration_note = error;
                    self.ports.push(status);
                    if self.runtime_catch_up_aborted {
                        break;
                    }
                    continue;
                }
            };
            status.enabled = enabled_portsc & PORTSC_PED != 0;
            self.note_portsc_snapshot(port, "port-enabled", enabled_portsc, 0);
            status.speed = decode_speed(((enabled_portsc >> 10) & 0xF) as u8);
            if !status.enabled {
                crate::serial_println!(
                    "[xHCI] port {} did not reach PED after reset portsc=0x{:08X}",
                    port,
                    enabled_portsc
                );
                self.note_enumeration_failure(
                    port,
                    "port-enable",
                    "port reset completed but the port never reached enabled state",
                );
                status.driver = "xhci-port-not-enabled";
                status.name = format!("USB device on port {} ({:?}) not enabled", port, speed);
                status.enumeration_note = "port reset completed but the port never reached enabled state";
                self.ports.push(status);
                if self.runtime_catch_up_aborted {
                    break;
                }
                continue;
            }

            delay_ms(PORT_ENABLE_SETTLE_DELAY_MS);
            log_portsc("before-enable-slot", port, self.read_portsc(port));

            if let Err(error) = self.check_runtime_budget(port, "enable-slot") {
                status.driver = "xhci-runtime-budget";
                status.name = format!(
                    "USB device on port {} ({:?}) runtime catch-up budget exhausted",
                    port, status.speed
                );
                status.enumeration_note = error;
                self.ports.push(status);
                break;
            }
            self.note_port_stage(port, "enable-slot");
            let slot_id = match self.enable_slot() {
                Ok(slot_id) => slot_id,
                Err(error) => {
                    crate::serial_println!("[xHCI] port {} enable-slot failed: {}", port, error);
                    self.note_enumeration_failure(port, "enable-slot", error);
                    status.driver = "xhci-enable-slot-failed";
                    status.name = format!(
                        "USB device on port {} ({:?}) enable-slot failed: {}",
                        port, status.speed, error
                    );
                    self.ports.push(status);
                    if self.runtime_catch_up_aborted {
                        break;
                    }
                    continue;
                }
            };
            self.note_port_stage(port, "slot-allocated");
            status.slot_id = Some(slot_id);
            match self.enumerate_device(port, status.speed, slot_id) {
                Ok(enumerated) => {
                    crate::serial_println!(
                        "[xHCI] attach success port={} slot={} {:04X}:{:04X} driver={} name={}",
                        enumerated.port,
                        enumerated.slot_id.unwrap_or(0),
                        enumerated.vendor_id.unwrap_or(0),
                        enumerated.product_id.unwrap_or(0),
                        enumerated.driver,
                        enumerated.name
                    );
                    status = enumerated;
                }
                Err(error) => {
                    crate::serial_println!(
                        "[xHCI] attach failed port={} slot={} reason={}",
                        port,
                        slot_id,
                        error
                    );
                    self.note_enumeration_failure(port, self.last_port_progress.last_stage, error);
                    status.driver = "xhci-enum-failed";
                    status.name = format!(
                        "USB device on port {} ({:?}) enum failed: {}",
                        port, status.speed, error
                    );
                    self.teardown_slot(slot_id, true);
                    status.slot_id = None;
                }
            }

            if let Some(slot_id) = status.slot_id {
                retained_slots[slot_id as usize] = true;
            }
            self.ports.push(status);
            if self.runtime_catch_up_aborted {
                break;
            }
        }

        for status in &previous {
            if let Some(slot_id) = status.slot_id {
                if !retained_slots[slot_id as usize] {
                    self.teardown_slot(slot_id, false);
                }
            }
        }

        self.log_topology_changes(&previous);
        true
    }

    fn poll_inner(&mut self, allow_topology_rescan: bool) -> bool {
        let mut topology_changed = false;
        let mut processed = 0usize;
        while processed < MAX_EVENTS_PER_POLL {
            let Some(event) = self.next_event() else {
                break;
            };
            processed = processed.saturating_add(1);
            match event.trb_type() {
                TRB_TRANSFER_EVENT => self.handle_transfer_event(event),
                TRB_PORT_STATUS_CHANGE => {
                    let port = match event.port_id() {
                        0 => None,
                        value => Some(value),
                    };
                    if XHCI_RUNTIME_EVENT_TRACE_ENABLED {
                        crate::serial_println!(
                            "[xHCI] port status change event port={} slot={} ep={}",
                            port.unwrap_or(0),
                            event.slot_id(),
                            event.endpoint_id()
                        );
                    }
                    self.pending_port_rescan = true;
                    self.note_runtime_event("port-change", port);
                }
                _ => {}
            }
        }
        if processed == MAX_EVENTS_PER_POLL && XHCI_RUNTIME_EVENT_TRACE_ENABLED {
            crate::serial_println!(
                "[xHCI] poll budget hit on {:02X}:{:02X}.{} (processed={})",
                self.pci.bus,
                self.pci.device,
                self.pci.function,
                processed
            );
        }

        if allow_topology_rescan && self.pending_port_rescan {
            let now = current_time_ms();
            let due = now.saturating_sub(self.last_port_rescan_ms) >= PORT_RESCAN_COOLDOWN_MS;
            if due {
                let _ = self.scan_ports();
                self.pending_port_rescan = false;
                self.last_port_rescan_ms = now;
                self.last_topology_service_ms = now;
                self.last_topology_service_result = "rescanned";
                topology_changed = true;
            } else {
                self.last_topology_service_ms = now;
                self.last_topology_service_result = "pending-cooldown";
            }
        }

        // Re-arm any HID endpoints that have no in-flight TRB.
        // This recovers from lost events, initial arm failures, or controller resets.
        self.rearm_stalled_hid_endpoints();

        topology_changed
    }

    pub fn poll(&mut self) -> bool {
        self.poll_inner(true)
    }

    pub fn poll_runtime(&mut self) -> bool {
        self.poll_inner(false)
    }

    pub fn rescan_if_pending(&mut self) -> bool {
        self.rescan_if_pending_with_mode(false)
    }

    pub fn rescan_if_pending_forced(&mut self) -> bool {
        self.rescan_if_pending_with_mode(true)
    }

    fn rescan_if_pending_with_mode(&mut self, force_due: bool) -> bool {
        if !self.pending_port_rescan {
            self.last_topology_service_result = "no-pending";
            self.last_topology_service_ms = current_time_ms();
            return false;
        }

        let now = current_time_ms();
        let due = force_due || now.saturating_sub(self.last_port_rescan_ms) >= PORT_RESCAN_COOLDOWN_MS;
        if !due {
            self.last_topology_service_ms = now;
            self.last_topology_service_result = "pending-cooldown";
            return false;
        }

        let rescanned = self.scan_ports();
        self.last_topology_service_ms = now;
        if self.runtime_catch_up_aborted {
            self.pending_port_rescan = true;
            self.last_topology_service_result = "budget-exhausted";
        } else {
            self.pending_port_rescan = false;
            self.last_port_rescan_ms = now;
            self.last_topology_service_result = if rescanned {
                "rescanned"
            } else {
                "rescan-no-change"
            };
        }
        rescanned
    }

    pub fn service_runtime_topology(&mut self, force_due: bool, budget_ms: u64) -> bool {
        self.runtime_catch_up_deadline_ms = if budget_ms == 0 {
            0
        } else {
            current_time_ms().saturating_add(budget_ms)
        };
        self.runtime_catch_up_aborted = false;
        self.last_topology_service_result = if self.pending_port_rescan {
            "catch-up-running"
        } else {
            "no-pending"
        };
        self.last_topology_service_ms = current_time_ms();
        let _ = self.poll_runtime();
        let rescanned = if force_due {
            self.rescan_if_pending_forced()
        } else {
            self.rescan_if_pending()
        };
        self.runtime_catch_up_deadline_ms = 0;
        rescanned
    }

    pub fn runtime_status(&self) -> UsbRuntimeControllerStatus {
        let active_attempt_port = self.last_port_progress.port.filter(|port| {
            self.pending_port_rescan || self.ports.iter().any(|status| status.port == *port)
        });
        UsbRuntimeControllerStatus {
            pending_topology: self.pending_port_rescan,
            last_runtime_event: self.last_runtime_event,
            last_runtime_event_port: self.last_runtime_event_port,
            last_runtime_event_ms: self.last_runtime_event_ms,
            last_topology_service_result: self.last_topology_service_result,
            last_topology_service_ms: self.last_topology_service_ms,
            last_inventory_sync_ms: self.last_inventory_sync_ms,
            last_inventory_sync_devices: self.last_inventory_sync_devices,
            cached_ports: self.ports.len(),
            cached_connected_devices: self.ports.iter().filter(|port| port.connected).count(),
            active_attempt_port,
            progress: self.last_port_progress,
            last_enumeration_failure_port: self.last_enumeration_failure_port,
            last_enumeration_failure_reason: self.last_enumeration_failure_reason,
            stale_failed_ports: self.stale_failed_ports,
        }
    }

    pub fn note_inventory_sync(&mut self) {
        self.last_inventory_sync_ms = current_time_ms();
        self.last_inventory_sync_devices = self.ports.iter().filter(|port| port.connected).count();
    }

    /// Check all slots for HID endpoints with no in-flight transfer and re-arm them.
    fn rearm_stalled_hid_endpoints(&mut self) {
        let stalled: alloc::vec::Vec<u8> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| {
                let slot = slot.as_ref()?;
                let hid = slot.hid.as_ref()?;
                if hid.in_flight_trb.is_none() {
                    Some(idx as u8)
                } else {
                    None
                }
            })
            .collect();

        for slot_id in stalled {
            if let Ok(slot) = self.slot_mut(slot_id) {
                if let Some(hid) = slot.hid.as_mut() {
                    if matches!(hid.hid_kind, HidKind::Keyboard) {
                        hid::reset_keyboard_boot_state(&mut hid.keyboard_state);
                    }
                }
            }
            let _ = self.arm_hid_endpoint(slot_id);
        }
    }

    fn log_topology_changes(&self, previous: &[UsbPortStatus]) {
        if !XHCI_RUNTIME_EVENT_TRACE_ENABLED {
            let _ = previous;
            return;
        }
        for old in previous {
            if self.ports.iter().all(|port| port.port != old.port) {
                crate::serial_println!(
                    "[xHCI] detach port={} slot={} {:04X}:{:04X} driver={}",
                    old.port,
                    old.slot_id.unwrap_or(0),
                    old.vendor_id.unwrap_or(0),
                    old.product_id.unwrap_or(0),
                    old.driver
                );
            }
        }

        for current in &self.ports {
            let Some(previous_port) = previous.iter().find(|old| old.port == current.port) else {
                crate::serial_println!(
                    "[xHCI] connect port={} enabled={} speed={:?}",
                    current.port,
                    current.enabled,
                    current.speed
                );
                continue;
            };

            if previous_port.vendor_id != current.vendor_id
                || previous_port.product_id != current.product_id
                || previous_port.driver != current.driver
                || previous_port.connected != current.connected
            {
                crate::serial_println!(
                    "[xHCI] reattach port={} old={:04X}:{:04X}/{} new={:04X}:{:04X}/{}",
                    current.port,
                    previous_port.vendor_id.unwrap_or(0),
                    previous_port.product_id.unwrap_or(0),
                    previous_port.driver,
                    current.vendor_id.unwrap_or(0),
                    current.product_id.unwrap_or(0),
                    current.driver
                );
            }
        }
    }

    fn enumerate_device(
        &mut self,
        port: u8,
        speed: UsbSpeed,
        slot_id: u8,
    ) -> Result<UsbPortStatus, &'static str> {
        self.note_port_stage(port, "allocate-slot");
        self.allocate_slot(slot_id, port, speed)?;
        self.note_port_stage(port, "address-device");
        self.address_device(slot_id)?;
        self.note_port_stage(port, "address-assigned");
        crate::serial_println!(
            "[xHCI] address assigned port={} slot={} speed={:?}",
            port,
            slot_id,
            speed
        );

        self.note_port_stage(port, "read-device-header");
        let header = self.read_descriptor(slot_id, 0x01, 0, 0, 8)?;
        if header.len() < 8 {
            return Err("xHCI short device descriptor header");
        }

        let max_packet_size0 = actual_max_packet_size0(speed, header[7]);
        if max_packet_size0 != 0 && max_packet_size0 != self.slot(slot_id)?.max_packet_size0 {
            self.note_port_stage(port, "evaluate-ep0");
            self.evaluate_ep0(slot_id, max_packet_size0)?;
            self.slot_mut(slot_id)?.max_packet_size0 = max_packet_size0;
        }

        self.note_port_stage(port, "read-device-descriptor");
        let device_desc = self.read_descriptor(slot_id, 0x01, 0, 0, 18)?;
        if device_desc.len() < 18 {
            return Err("xHCI short device descriptor");
        }

        let vendor_id = u16::from_le_bytes([device_desc[8], device_desc[9]]);
        let product_id = u16::from_le_bytes([device_desc[10], device_desc[11]]);
        let device_class = device_desc[4];
        let configuration_count = device_desc[17];

        let mut category = DeviceCategory::UsbDevice;
        let mut driver = "usb-generic";
        let mut configured = false;
        let configuration_count = configuration_count;
        let mut active_configuration = None;
        let mut interface_count = 0u8;
        let mut interfaces_summary = Vec::new();
        let mut visibility = UsbPortVisibility::Generic;
        let mut attach_state = UsbAttachState::NotApplicable;
        let mut attach_reason = "no supported USB network interfaces visible";
        let mut enumeration_note = "device addressed; descriptors pending";
        let mut hid_kind = None;
        let mut storage_info = None;
        let mut network_info = None;
        let mut name = format!("USB {:04X}:{:04X}", vendor_id, product_id);
        let addressed = self.slot(slot_id).map(|slot| slot.addressed).unwrap_or(false);

        crate::serial_println!(
            "[xHCI] descriptor read port={} slot={} vid={:04X} pid={:04X} dev_class={:02X}/{:02X}/{:02X} cfgs={}",
            port,
            slot_id,
            vendor_id,
            product_id,
            device_class,
            device_desc[5],
            device_desc[6],
            configuration_count
        );

        if configuration_count > 0 {
            self.note_port_stage(port, "read-config-header");
            let config_header = self.read_descriptor(slot_id, 0x02, 0, 0, 9)?;
            if config_header.len() >= 9 {
                let total_length =
                    u16::from_le_bytes([config_header[2], config_header[3]]) as usize;
                self.note_port_stage(port, "read-config-descriptors");
                let config_blob = self.read_descriptor(slot_id, 0x02, 0, 0, total_length)?;
                match descriptors::parse_configuration_descriptors(&config_blob) {
                    Ok(configuration) => {
                    active_configuration = Some(configuration.configuration_value);
                    interface_count = configuration.interfaces.len().min(u8::MAX as usize) as u8;
                    interfaces_summary = configuration
                        .interfaces
                        .iter()
                        .map(summarize_interface)
                        .collect();
                    crate::serial_println!(
                        "[xHCI] interface parse port={} slot={} config={} interfaces={} power={}mA attrs=0x{:02X}",
                        port,
                        slot_id,
                        configuration.configuration_value,
                        configuration.interfaces.len(),
                        configuration.max_power_ma,
                        configuration.attributes
                    );
                    for interface in &configuration.interfaces {
                        crate::serial_println!(
                            "[xHCI] interface slot={} if={} alt={} class={:02X}/{:02X}/{:02X} eps={}",
                            slot_id,
                            interface.number,
                            interface.alternate_setting,
                            interface.class,
                            interface.subclass,
                            interface.protocol,
                            interface.endpoints.len()
                        );
                    }
                    category =
                        descriptors::classify_device(device_class, &configuration.interfaces);
                    let inferred_attach =
                        infer_attach_state(&configuration.interfaces, network_info, None);
                    attach_state = inferred_attach.0;
                    attach_reason = inferred_attach.1;
                    self.note_port_stage(port, "descriptors-fetched");
                    self.note_port_stage(port, "configuration-attempted");
                    let set_config = self.set_configuration(slot_id, configuration.configuration_value);
                    configured = set_config.is_ok();
                    enumeration_note = if let Err(error) = set_config {
                        self.note_enumeration_failure(port, "configuration-attempted", error);
                        error
                    } else {
                        "device configured"
                    };
                    if configured {
                        if let Ok(kind) = self.configure_hid(slot_id, speed, &configuration.interfaces) {
                            hid_kind = Some(kind);
                            category = DeviceCategory::Input;
                            driver = "usb-hid";
                            name = match kind {
                                HidKind::Keyboard => {
                                    format!("USB Keyboard {:04X}:{:04X}", vendor_id, product_id)
                                }
                                HidKind::Mouse => {
                                    format!("USB Mouse {:04X}:{:04X}", vendor_id, product_id)
                                }
                                HidKind::Combined | HidKind::Unknown => {
                                    format!("USB HID {:04X}:{:04X}", vendor_id, product_id)
                                }
                            };
                        } else if category == DeviceCategory::Input {
                            crate::serial_println!(
                                "[xHCI] HID configure failed slot={} port={} vendor={:04X} product={:04X}",
                                slot_id,
                                port,
                                vendor_id,
                                product_id
                            );
                        } else if category == DeviceCategory::Storage {
                            if let Ok(info) = self.configure_mass_storage(
                                slot_id,
                                speed,
                                &configuration.interfaces,
                            ) {
                                storage_info = Some(info);
                                driver = "usb-mass-storage";
                                name = format!(
                                    "USB Storage {:04X}:{:04X} ({} MB)",
                                    vendor_id,
                                    product_id,
                                    info.capacity_bytes() / (1024 * 1024)
                                );
                            } else {
                                driver = "usb-storage-probe";
                                name = format!("USB Storage {:04X}:{:04X}", vendor_id, product_id);
                            }
                        } else if category == DeviceCategory::Network {
                            let protocol_str =
                                descriptors::usb_net_protocol(&configuration.interfaces);
                            match self.configure_network(
                                slot_id,
                                speed,
                                &configuration.interfaces,
                            ) {
                                Ok(info) => {
                                    network_info = Some(info);
                                    attach_state = UsbAttachState::Ready;
                                    attach_reason =
                                        "supported USB network interface detected; manual attach is available";
                                    driver = "usb-net";
                                    name = format!(
                                        "USB Network {} {:04X}:{:04X} [{}]",
                                        protocol_str,
                                        vendor_id,
                                        product_id,
                                        crate::net::format_mac(&info.mac)
                                    );
                                    crate::serial_println!(
                                        "[xHCI] USB network ACTIVE: {} slot={} mac={}",
                                        protocol_str,
                                        slot_id,
                                        crate::net::format_mac(&info.mac)
                                    );
                                    enumeration_note = "USB network interfaces configured";
                                }
                                Err(err) => {
                                    let inferred_attach = infer_attach_state(
                                        &configuration.interfaces,
                                        None,
                                        Some(err),
                                    );
                                    attach_state = inferred_attach.0;
                                    attach_reason = inferred_attach.1;
                                    driver = "usb-net-staged";
                                    name = format!(
                                        "USB Network {} {:04X}:{:04X}",
                                        protocol_str, vendor_id, product_id
                                    );
                                    crate::serial_println!(
                                        "[xHCI] USB network configure failed: {} ({})",
                                        err,
                                        protocol_str
                                    );
                                    enumeration_note = err;
                                }
                            }
                        }
                    }
                    if configured {
                        self.note_port_stage(port, "configured");
                    }
                    visibility =
                        classify_port_visibility(category, &configuration.interfaces, network_info);
                    if network_info.is_none() {
                        let inferred_attach = infer_attach_state(
                            &configuration.interfaces,
                            network_info,
                            if matches!(attach_state, UsbAttachState::Ready) {
                                None
                            } else {
                                Some(attach_reason)
                            },
                        );
                        attach_state = inferred_attach.0;
                        attach_reason = inferred_attach.1;
                    }
                    }
                    Err(error) => {
                        enumeration_note = error;
                    }
                }
            }
        } else {
            enumeration_note = "device exposes no configurations";
        }

        Ok(UsbPortStatus {
            port,
            connected: true,
            enabled: self.read_portsc(port) & PORTSC_PED != 0,
            speed,
            slot_id: Some(slot_id),
            addressed,
            vendor_id: Some(vendor_id),
            product_id: Some(product_id),
            category,
            driver,
            name,
            configured,
            configuration_count,
            active_configuration: if configured {
                active_configuration
            } else {
                None
            },
            class_code: device_class,
            subclass: device_desc[5],
            protocol: device_desc[6],
            interface_count,
            interfaces: interfaces_summary,
            visibility,
            attach_state,
            attach_reason,
            enumeration_note,
            hid_kind,
            storage: storage_info,
            network: network_info,
        })
    }

    fn configure_hid(
        &mut self,
        slot_id: u8,
        speed: UsbSpeed,
        interfaces: &[UsbInterface],
    ) -> Result<HidKind, &'static str> {
        for interface in interfaces {
            let hid_kind = hid::classify_interface(interface);
            if !matches!(hid_kind, HidKind::Keyboard | HidKind::Mouse) {
                if interface.is_hid() {
                    crate::serial_println!(
                        "[xHCI] HID interface unsupported slot={} if={} class={:02X}/{:02X}/{:02X}",
                        slot_id,
                        interface.number,
                        interface.class,
                        interface.subclass,
                        interface.protocol
                    );
                }
                continue;
            }

            let endpoint = interface
                .endpoints
                .iter()
                .find(|endpoint| {
                    endpoint.direction == EndpointDirection::In
                        && endpoint.transfer_type == TransferType::Interrupt
                })
                .ok_or("USB HID interrupt endpoint missing")?
                .clone();

            if let Err(error) = self.set_boot_protocol(slot_id, interface.number) {
                crate::serial_println!(
                    "[xHCI] HID set-boot-protocol failed slot={} if={} kind={:?}: {}",
                    slot_id,
                    interface.number,
                    hid_kind,
                    error
                );
            }
            self.configure_interrupt_in(slot_id, speed, &endpoint, hid_kind)?;
            return Ok(hid_kind);
        }

        Err("USB HID boot interface not found")
    }

    fn configure_mass_storage(
        &mut self,
        slot_id: u8,
        speed: UsbSpeed,
        interfaces: &[UsbInterface],
    ) -> Result<UsbMassStorageInfo, &'static str> {
        let interface = interfaces
            .iter()
            .find(|interface| {
                mass_storage::is_mass_storage_interface(
                    interface.class,
                    interface.subclass,
                    interface.protocol,
                )
            })
            .ok_or("USB mass storage interface not found")?;

        let bulk_in = interface
            .endpoints
            .iter()
            .find(|endpoint| {
                endpoint.direction == EndpointDirection::In
                    && endpoint.transfer_type == TransferType::Bulk
            })
            .ok_or("USB mass storage bulk IN missing")?
            .clone();
        let bulk_out = interface
            .endpoints
            .iter()
            .find(|endpoint| {
                endpoint.direction == EndpointDirection::Out
                    && endpoint.transfer_type == TransferType::Bulk
            })
            .ok_or("USB mass storage bulk OUT missing")?
            .clone();

        let bulk_in_id = endpoint_id_from_address(bulk_in.address);
        let bulk_out_id = endpoint_id_from_address(bulk_out.address);
        let bulk_in_ring = XhciRing::new(32, true)?;
        let bulk_out_ring = XhciRing::new(32, true)?;

        let input_phys = {
            let context_size = self.context_size;
            let slot = self.slot_mut(slot_id)?;
            slot.input_context.zero();

            let add_flags = 1 | (1 << bulk_out_id) | (1 << bulk_in_id);
            let last_context = bulk_out_id.max(bulk_in_id);
            write_ctx32(&mut slot.input_context, 4, add_flags);
            write_ctx32(
                &mut slot.input_context,
                context_size,
                u32::from(last_context) << 27,
            );

            let bulk_out_offset = input_ep_context_offset(context_size, bulk_out_id);
            let bulk_in_offset = input_ep_context_offset(context_size, bulk_in_id);

            write_non_control_endpoint_context(
                &mut slot.input_context,
                bulk_out_offset,
                &bulk_out,
                speed,
                EP_TYPE_BULK_OUT,
                bulk_out_ring.phys_addr,
            );
            write_non_control_endpoint_context(
                &mut slot.input_context,
                bulk_in_offset,
                &bulk_in,
                speed,
                EP_TYPE_BULK_IN,
                bulk_in_ring.phys_addr,
            );

            slot.input_context.physical().as_u64()
        };

        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_CONFIGURE_ENDPOINT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Configure Endpoint failed for storage");
        }

        {
            let slot = self.slot_mut(slot_id)?;
            slot.storage = Some(StorageEndpoint {
                bulk_out_address: bulk_out.address,
                bulk_out_id,
                bulk_out_ring,
                bulk_in_address: bulk_in.address,
                bulk_in_id,
                bulk_in_ring,
                tag: 1,
                info: UsbMassStorageInfo {
                    capacity_sectors: 0,
                    sector_size: 512,
                },
            });
        }

        let inquiry = self.scsi_data_in(slot_id, &mass_storage::scsi_inquiry_command(), 6, 36)?;
        let (vendor, product) = mass_storage::parse_inquiry_strings(&inquiry);
        crate::serial_println!(
            "[xHCI] USB storage slot {} inquiry: vendor='{}' product='{}'",
            slot_id,
            vendor,
            product
        );

        let capacity = self.scsi_data_in(
            slot_id,
            &mass_storage::scsi_read_capacity_10_command(),
            10,
            8,
        )?;
        let info = mass_storage::parse_read_capacity_10(&capacity)?;
        self.slot_mut(slot_id)?
            .storage
            .as_mut()
            .ok_or("USB storage state missing")?
            .info = info;
        Ok(info)
    }

    // ================================================================
    // USB Networking: CDC-ECM and RNDIS data path
    // ================================================================

    fn configure_network(
        &mut self,
        slot_id: u8,
        speed: UsbSpeed,
        interfaces: &[UsbInterface],
    ) -> Result<UsbNetworkInfo, &'static str> {
        // Detect protocol and find control + data interfaces
        let (protocol, ctrl_iface, data_iface) = self.find_network_interfaces(interfaces)?;
        crate::serial_println!(
            "[xHCI] net: protocol={:?} ctrl_iface={} data_iface={} slot={}",
            protocol,
            ctrl_iface,
            data_iface,
            slot_id
        );

        let ctrl_if = &interfaces[ctrl_iface];
        let data_if = &interfaces[data_iface];

        if data_if.alternate_setting != 0 {
            self.set_interface(slot_id, data_if.number, data_if.alternate_setting)?;
            crate::serial_println!(
                "[xHCI] net: activated interface {} alt {} (ctrl if {} alt {})",
                data_if.number,
                data_if.alternate_setting,
                ctrl_if.number,
                ctrl_if.alternate_setting
            );
        }

        // Find bulk endpoints on the active data interface
        let bulk_in = data_if
            .endpoints
            .iter()
            .find(|ep| {
                ep.direction == EndpointDirection::In && ep.transfer_type == TransferType::Bulk
            })
            .ok_or("USB net bulk IN missing")?
            .clone();
        let bulk_out = data_if
            .endpoints
            .iter()
            .find(|ep| {
                ep.direction == EndpointDirection::Out && ep.transfer_type == TransferType::Bulk
            })
            .ok_or("USB net bulk OUT missing")?
            .clone();

        crate::serial_println!(
            "[xHCI] net: bulk_in addr=0x{:02X} mps={} bulk_out addr=0x{:02X} mps={}",
            bulk_in.address,
            bulk_in.max_packet_size,
            bulk_out.address,
            bulk_out.max_packet_size
        );

        let bulk_in_id = endpoint_id_from_address(bulk_in.address);
        let bulk_out_id = endpoint_id_from_address(bulk_out.address);
        let bulk_in_ring = XhciRing::new(64, true)?;
        let bulk_out_ring = XhciRing::new(64, true)?;

        // Configure xHCI endpoints
        let input_phys = {
            let context_size = self.context_size;
            let slot = self.slot_mut(slot_id)?;
            slot.input_context.zero();

            let add_flags = 1 | (1 << bulk_out_id) | (1 << bulk_in_id);
            let last_context = bulk_out_id.max(bulk_in_id);
            write_ctx32(&mut slot.input_context, 4, add_flags);
            write_ctx32(
                &mut slot.input_context,
                context_size,
                u32::from(last_context) << 27,
            );

            let bulk_out_offset = input_ep_context_offset(context_size, bulk_out_id);
            let bulk_in_offset = input_ep_context_offset(context_size, bulk_in_id);

            write_non_control_endpoint_context(
                &mut slot.input_context,
                bulk_out_offset,
                &bulk_out,
                speed,
                EP_TYPE_BULK_OUT,
                bulk_out_ring.phys_addr,
            );
            write_non_control_endpoint_context(
                &mut slot.input_context,
                bulk_in_offset,
                &bulk_in,
                speed,
                EP_TYPE_BULK_IN,
                bulk_in_ring.phys_addr,
            );

            slot.input_context.physical().as_u64()
        };

        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_CONFIGURE_ENDPOINT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Configure Endpoint failed for network");
        }
        crate::serial_println!("[xHCI] net: endpoints configured");

        // Allocate RX buffer
        let rx_buffer = DmaRegion::allocate(USB_NET_MAX_TRANSFER as usize)
            .map_err(|_| "USB net RX buffer allocation failed")?;

        // Store network endpoint state
        let ctrl_num = interfaces[ctrl_iface].number;
        let data_num = interfaces[data_iface].number;
        {
            let slot = self.slot_mut(slot_id)?;
            slot.network = Some(NetworkEndpoint {
                bulk_out_address: bulk_out.address,
                bulk_out_id,
                bulk_out_ring,
                bulk_in_address: bulk_in.address,
                bulk_in_id,
                bulk_in_ring,
                protocol,
                mac: [0; 6],
                max_transfer_size: USB_NET_MAX_TRANSFER,
                control_interface: ctrl_num,
                data_interface: data_num,
                rx_buffer,
                rx_armed: false,
                rx_queue: Vec::new(),
                rx_raw_events: 0,
                rx_frames: 0,
                tx_frames: 0,
                tx_errors: 0,
                rx_errors: 0,
                rx_extract_errors: 0,
                rx_short_frames: 0,
                rx_last_raw_len: 0,
                rx_last_frame_len: 0,
                rx_last_drop: "none",
            });
        }

        // Protocol-specific initialization
        let mac = match protocol {
            UsbNetProtocol::Rndis => {
                self.rndis_init(slot_id)?;
                self.rndis_set_packet_filter(slot_id)?;
                self.rndis_query_mac(slot_id).unwrap_or([0x02, 0x00, 0x00, 0x00, 0x00, 0x01])
            }
            UsbNetProtocol::CdcEcm | UsbNetProtocol::CdcNcm => {
                // Set packet filter: all frames
                let _ = self.cdc_set_packet_filter(slot_id, data_num);
                // Generate locally-administered MAC (real MAC comes from descriptor)
                [0x02, 0x57, 0x41, 0x52, (slot_id & 0xFF), (ctrl_num & 0xFF)]
            }
        };

        // Store MAC
        if let Ok(slot) = self.slot_mut(slot_id) {
            if let Some(net) = slot.network.as_mut() {
                net.mac = mac;
            }
        }

        // Arm initial RX transfer
        self.arm_network_rx(slot_id)?;

        crate::serial_println!(
            "[xHCI] net: ready protocol={:?} mac={} slot={}",
            protocol,
            crate::net::format_mac(&mac),
            slot_id
        );

        Ok(UsbNetworkInfo {
            protocol,
            mac,
            max_frame_size: 1514,
            controller_index: 0, // filled by caller
            slot_id,
        })
    }

    fn find_network_interfaces(
        &self,
        interfaces: &[UsbInterface],
    ) -> Result<(UsbNetProtocol, usize, usize), &'static str> {
        let mut rndis_control_seen = false;

        // Try RNDIS first (Android tethering, most common)
        for (idx, iface) in interfaces.iter().enumerate() {
            if iface.is_rndis() && iface.has_interrupt_in() {
                rndis_control_seen = true;
                // Find matching data interface (class 0x0A)
                for (didx, diface) in interfaces.iter().enumerate() {
                    if diface.is_cdc_data()
                        && diface.has_bulk_in()
                        && diface.has_bulk_out()
                        && (diface.number == iface.number
                            || diface.number == iface.number.saturating_add(1)
                            || didx == idx.saturating_add(1))
                    {
                        return Ok((UsbNetProtocol::Rndis, idx, didx));
                    }
                }
                // RNDIS control found but no data interface with bulk endpoints.
                // Some devices put bulk endpoints on the RNDIS interface itself.
                if iface.has_bulk_in() && iface.has_bulk_out() {
                    return Ok((UsbNetProtocol::Rndis, idx, idx));
                }
            }
        }

        if rndis_control_seen {
            return Err(
                "RNDIS control interface detected, but no matching CDC data interface with bulk IN/OUT endpoints was found",
            );
        }

        // Try CDC-ECM
        for (idx, iface) in interfaces.iter().enumerate() {
            if iface.is_cdc_ecm() {
                for (didx, diface) in interfaces.iter().enumerate() {
                    if diface.is_cdc_data()
                        && diface.has_bulk_in()
                        && diface.has_bulk_out()
                    {
                        return Ok((UsbNetProtocol::CdcEcm, idx, didx));
                    }
                }
            }
        }

        // Try CDC-NCM
        for (idx, iface) in interfaces.iter().enumerate() {
            if iface.is_cdc_ncm() {
                for (didx, diface) in interfaces.iter().enumerate() {
                    if diface.is_cdc_data()
                        && diface.has_bulk_in()
                        && diface.has_bulk_out()
                    {
                        return Ok((UsbNetProtocol::CdcNcm, idx, didx));
                    }
                }
            }
        }

        Err("USB network: no supported protocol interfaces found")
    }

    fn arm_network_rx(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let endpoint_id = {
            let slot = self.slot_mut(slot_id)?;
            let net = slot.network.as_mut().ok_or("USB net state missing")?;
            if net.rx_armed {
                return Ok(()); // Already armed
            }
            let transfer = Trb {
                parameter: net.rx_buffer.physical().as_u64(),
                status: net.max_transfer_size,
                control: ((TRB_NORMAL as u32) << 10) | IOC | ISP,
            };
            net.bulk_in_ring.enqueue(transfer)?;
            net.rx_armed = true;
            net.bulk_in_id
        };

        self.ring_doorbell(slot_id, endpoint_id);
        Ok(())
    }

    /// Send an Ethernet frame over the USB network device.
    pub fn network_send_frame(&mut self, slot_id: u8, frame: &[u8]) -> Result<(), &'static str> {
        let protocol = self
            .slot(slot_id)?
            .network
            .as_ref()
            .ok_or("USB net state missing")?
            .protocol;

        // Build the wire format
        let wire_data = match protocol {
            UsbNetProtocol::Rndis => {
                let total_len = RNDIS_PACKET_HEADER_SIZE + frame.len();
                let mut buf = Vec::with_capacity(total_len);
                buf.extend_from_slice(&RNDIS_MSG_PACKET.to_le_bytes()); // MessageType
                buf.extend_from_slice(&(total_len as u32).to_le_bytes()); // MessageLength
                buf.extend_from_slice(&36u32.to_le_bytes()); // DataOffset (from byte 8)
                buf.extend_from_slice(&(frame.len() as u32).to_le_bytes()); // DataLength
                buf.extend_from_slice(&[0u8; 28]); // OOB + PerPacket + Reserved
                buf.extend_from_slice(frame);
                buf
            }
            UsbNetProtocol::CdcEcm | UsbNetProtocol::CdcNcm => frame.to_vec(),
        };

        // DMA-copy and transmit
        let mut region = DmaRegion::allocate(wire_data.len().max(1))
            .map_err(|_| "USB net TX allocation failed")?;
        region.slice_mut()[..wire_data.len()].copy_from_slice(&wire_data);

        let (endpoint_id, trb_phys) = {
            let slot = self.slot_mut(slot_id)?;
            let net = slot.network.as_mut().ok_or("USB net state missing")?;
            let transfer = Trb {
                parameter: region.physical().as_u64(),
                status: wire_data.len() as u32,
                control: ((TRB_NORMAL as u32) << 10) | IOC,
            };
            let trb_phys = net.bulk_out_ring.enqueue(transfer)?;
            (net.bulk_out_id, trb_phys)
        };

        self.ring_doorbell(slot_id, endpoint_id);

        // Wait for TX completion, handling other events while waiting
        for _ in 0..2_000_000 {
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_TRANSFER_EVENT
                    && event.slot_id() == slot_id
                    && event.endpoint_id() == endpoint_id
                    && event.parameter == trb_phys
                {
                    if event.completion_code() == COMPLETION_SUCCESS {
                        if let Ok(slot) = self.slot_mut(slot_id) {
                            if let Some(net) = slot.network.as_mut() {
                                net.tx_frames += 1;
                            }
                        }
                        return Ok(());
                    }
                    if let Ok(slot) = self.slot_mut(slot_id) {
                        if let Some(net) = slot.network.as_mut() {
                            net.tx_errors += 1;
                        }
                    }
                    return Err("USB net TX transfer failed");
                }
                // Handle other events while waiting
                self.dispatch_non_net_event(event, slot_id);
            }
            spin_loop();
        }

        if let Ok(slot) = self.slot_mut(slot_id) {
            if let Some(net) = slot.network.as_mut() {
                net.tx_errors += 1;
            }
        }
        Err("USB net TX timed out")
    }

    /// Poll for received frames. Non-blocking: drains event ring and returns
    /// one queued frame if available.
    pub fn network_recv_frame(&mut self, slot_id: u8) -> Option<Vec<u8>> {
        // Drain all pending events
        let rx_data: Option<Vec<u8>> = None;
        loop {
            let event = match self.next_event() {
                Some(e) => e,
                None => break,
            };

            match event.trb_type() {
                TRB_TRANSFER_EVENT => {
                    let ev_slot = event.slot_id();
                    let ev_ep = event.endpoint_id();

                    // Check if this is our network RX
                    let is_our_rx = ev_slot == slot_id
                        && self
                            .slot(slot_id)
                            .ok()
                            .and_then(|s| s.network.as_ref())
                            .map_or(false, |n| n.bulk_in_id == ev_ep && n.rx_armed);

                    if is_our_rx {
                        self.process_network_rx_event(&event, slot_id);
                    } else {
                        self.dispatch_non_net_event(event, slot_id);
                    }
                }
                TRB_PORT_STATUS_CHANGE => {
                    // Don't rescan ports during active networking
                }
                _ => {}
            }
        }

        // Re-arm RX if needed
        let _ = self.arm_network_rx(slot_id);

        // Dequeue one frame
        if let Ok(slot) = self.slot_mut(slot_id) {
            if let Some(net) = slot.network.as_mut() {
                if !net.rx_queue.is_empty() {
                    return Some(net.rx_queue.remove(0));
                }
            }
        }

        rx_data
    }

    fn process_network_rx_event(&mut self, event: &Trb, slot_id: u8) {
        let Ok(slot) = self.slot_mut(slot_id) else {
            return;
        };
        let Some(net) = slot.network.as_mut() else {
            return;
        };

        net.rx_armed = false;

        let success = event.completion_code() == COMPLETION_SUCCESS
            || event.completion_code() == COMPLETION_SHORT_PACKET;
        if !success {
            net.rx_errors += 1;
            net.rx_last_drop = "transfer-error";
            crate::serial_println!(
                "[xHCI] net: RX error code={}",
                event.completion_code()
            );
            return;
        }

        let buf_len = net.max_transfer_size as usize;
        let actual = buf_len.saturating_sub(event.transfer_residue() as usize);
        if actual == 0 {
            net.rx_errors += 1;
            net.rx_last_drop = "empty-transfer";
            return;
        }

        let raw = net.rx_buffer.slice()[..actual].to_vec();
        net.rx_raw_events += 1;
        net.rx_last_raw_len = actual;

        // Protocol-specific frame extraction
        let frame = match net.protocol {
            UsbNetProtocol::Rndis => extract_rndis_frame(&raw),
            UsbNetProtocol::CdcEcm | UsbNetProtocol::CdcNcm => Some(raw),
        };

        if let Some(frame) = frame {
            if frame.len() >= 14 {
                // Minimum Ethernet frame
                net.rx_frames += 1;
                net.rx_last_frame_len = frame.len();
                net.rx_last_drop = "none";
                net.rx_queue.push(frame);
            } else {
                net.rx_short_frames += 1;
                net.rx_last_frame_len = frame.len();
                net.rx_last_drop = "short-ethernet";
            }
        } else {
            net.rx_extract_errors += 1;
            net.rx_last_frame_len = 0;
            net.rx_last_drop = match net.protocol {
                UsbNetProtocol::Rndis => "rndis-extract-failed",
                UsbNetProtocol::CdcEcm | UsbNetProtocol::CdcNcm => "frame-extract-failed",
            };
        }
    }

    fn dispatch_non_net_event(&mut self, event: Trb, _net_slot: u8) {
        match event.trb_type() {
            TRB_TRANSFER_EVENT => self.handle_transfer_event(event),
            TRB_PORT_STATUS_CHANGE => {} // Defer port rescan
            _ => {}
        }
    }

    pub fn network_diagnostics(&self, slot_id: u8) -> Option<UsbNetDiagnostics> {
        let slot = self.slot(slot_id).ok()?;
        let net = slot.network.as_ref()?;
        Some(UsbNetDiagnostics {
            protocol: net.protocol,
            mac: net.mac,
            rx_raw_events: net.rx_raw_events,
            rx_frames: net.rx_frames,
            tx_frames: net.tx_frames,
            tx_errors: net.tx_errors,
            rx_errors: net.rx_errors,
            rx_extract_errors: net.rx_extract_errors,
            rx_short_frames: net.rx_short_frames,
            rx_last_raw_len: net.rx_last_raw_len,
            rx_last_frame_len: net.rx_last_frame_len,
            rx_last_drop: net.rx_last_drop,
            rx_queue_depth: net.rx_queue.len(),
            rx_armed: net.rx_armed,
        })
    }

    pub fn has_network_slot(&self, slot_id: u8) -> bool {
        self.slot(slot_id)
            .ok()
            .and_then(|s| s.network.as_ref())
            .is_some()
    }

    // --- RNDIS protocol ---

    fn rndis_init(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let ctrl_iface = self
            .slot(slot_id)?
            .network
            .as_ref()
            .ok_or("USB net state missing")?
            .control_interface;

        // Build RNDIS_INITIALIZE_MSG (24 bytes)
        let mut msg = [0u8; 24];
        msg[0..4].copy_from_slice(&RNDIS_MSG_INIT.to_le_bytes());
        msg[4..8].copy_from_slice(&24u32.to_le_bytes()); // MessageLength
        msg[8..12].copy_from_slice(&1u32.to_le_bytes()); // RequestId
        msg[12..16].copy_from_slice(&1u32.to_le_bytes()); // MajorVersion
        msg[16..20].copy_from_slice(&0u32.to_le_bytes()); // MinorVersion
        msg[20..24].copy_from_slice(&USB_NET_MAX_TRANSFER.to_le_bytes());

        // Send via SEND_ENCAPSULATED_COMMAND
        self.rndis_send_encapsulated(slot_id, ctrl_iface, &msg)?;

        // Get response via GET_ENCAPSULATED_RESPONSE
        let response = self.rndis_get_encapsulated(slot_id, ctrl_iface, 52)?;
        if response.len() < 16 {
            return Err("RNDIS INIT response too short");
        }
        let msg_type = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
        let status = u32::from_le_bytes([response[12], response[13], response[14], response[15]]);
        if msg_type != RNDIS_MSG_INIT_C {
            crate::serial_println!(
                "[xHCI] RNDIS: unexpected init response type 0x{:08X}",
                msg_type
            );
            return Err("RNDIS INIT unexpected response");
        }
        if status != 0 {
            crate::serial_println!("[xHCI] RNDIS: init failed status=0x{:08X}", status);
            return Err("RNDIS INIT failed");
        }

        // Parse max transfer size from response if available
        if response.len() >= 40 {
            let max_xfer = u32::from_le_bytes([
                response[36],
                response[37],
                response[38],
                response[39],
            ]);
            if max_xfer > 0 {
                if let Ok(slot) = self.slot_mut(slot_id) {
                    if let Some(net) = slot.network.as_mut() {
                        net.max_transfer_size = max_xfer.min(0x10000);
                    }
                }
            }
            crate::serial_println!("[xHCI] RNDIS: init OK max_transfer={}", max_xfer);
        } else {
            crate::serial_println!("[xHCI] RNDIS: init OK (short response)");
        }

        Ok(())
    }

    fn rndis_set_packet_filter(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let ctrl_iface = self
            .slot(slot_id)?
            .network
            .as_ref()
            .ok_or("USB net state missing")?
            .control_interface;

        self.rndis_set_packet_filter_value(slot_id, ctrl_iface, NDIS_PACKET_FILTER_ALL)?;
        // Best-effort: Android DHCP offers may arrive as directed unicast during early
        // bring-up. Promiscuous acceptance avoids losing that first lease path if the
        // function's station address and the DHCP chaddr disagree transiently.
        let _ = self.rndis_set_packet_filter_value(
            slot_id,
            ctrl_iface,
            NDIS_PACKET_FILTER_ALL | NDIS_PACKET_FILTER_PROMISCUOUS,
        );
        Ok(())
    }

    fn rndis_set_packet_filter_value(
        &mut self,
        slot_id: u8,
        ctrl_iface: u8,
        filter: u32,
    ) -> Result<(), &'static str> {
        // RNDIS_SET_MSG for OID_GEN_CURRENT_PACKET_FILTER
        let mut msg = [0u8; 32];
        msg[0..4].copy_from_slice(&RNDIS_MSG_SET.to_le_bytes());
        msg[4..8].copy_from_slice(&32u32.to_le_bytes()); // MessageLength
        msg[8..12].copy_from_slice(&2u32.to_le_bytes()); // RequestId
        msg[12..16].copy_from_slice(&OID_GEN_CURRENT_PACKET_FILTER.to_le_bytes());
        msg[16..20].copy_from_slice(&4u32.to_le_bytes()); // InformationBufferLength
        msg[20..24].copy_from_slice(&20u32.to_le_bytes()); // InformationBufferOffset
        // msg[24..28] = DeviceVcHandle = 0
        msg[28..32].copy_from_slice(&filter.to_le_bytes());

        self.rndis_send_encapsulated(slot_id, ctrl_iface, &msg)?;
        let response = self.rndis_get_encapsulated(slot_id, ctrl_iface, 16)?;
        if response.len() >= 16 {
            let msg_type =
                u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
            let status =
                u32::from_le_bytes([response[12], response[13], response[14], response[15]]);
            if msg_type == RNDIS_MSG_SET_C {
                if status == 0 {
                    crate::serial_println!("[xHCI] RNDIS: packet filter 0x{:08X} set OK", filter);
                    return Ok(());
                }
                crate::serial_println!(
                    "[xHCI] RNDIS: packet filter 0x{:08X} failed status=0x{:08X}",
                    filter,
                    status
                );
                return Err("RNDIS packet filter rejected");
            }
        }
        Err("RNDIS packet filter unexpected response")
    }

    fn rndis_query_mac(&mut self, slot_id: u8) -> Result<[u8; 6], &'static str> {
        let ctrl_iface = self
            .slot(slot_id)?
            .network
            .as_ref()
            .ok_or("USB net state missing")?
            .control_interface;

        // RNDIS_QUERY_MSG for OID_802_3_CURRENT_ADDRESS
        let mut msg = [0u8; 28];
        msg[0..4].copy_from_slice(&RNDIS_MSG_QUERY.to_le_bytes());
        msg[4..8].copy_from_slice(&28u32.to_le_bytes()); // MessageLength
        msg[8..12].copy_from_slice(&3u32.to_le_bytes()); // RequestId
        msg[12..16].copy_from_slice(&OID_802_3_CURRENT_ADDRESS.to_le_bytes());
        msg[16..20].copy_from_slice(&0u32.to_le_bytes()); // InformationBufferLength
        msg[20..24].copy_from_slice(&0u32.to_le_bytes()); // InformationBufferOffset

        self.rndis_send_encapsulated(slot_id, ctrl_iface, &msg)?;
        let response = self.rndis_get_encapsulated(slot_id, ctrl_iface, 52)?;

        if response.len() < 28 {
            return Err("RNDIS MAC query response too short");
        }
        let msg_type = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
        if msg_type != RNDIS_MSG_QUERY_C {
            return Err("RNDIS MAC query unexpected response");
        }

        let info_len =
            u32::from_le_bytes([response[16], response[17], response[18], response[19]]) as usize;
        let info_off =
            u32::from_le_bytes([response[20], response[21], response[22], response[23]]) as usize;
        let data_start = 8 + info_off; // Offset is from byte 8 (after RequestId)
        if info_len >= 6 && data_start + 6 <= response.len() {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&response[data_start..data_start + 6]);
            crate::serial_println!(
                "[xHCI] RNDIS: MAC = {}",
                crate::net::format_mac(&mac)
            );
            Ok(mac)
        } else {
            Err("RNDIS MAC not in response")
        }
    }

    fn rndis_send_encapsulated(
        &mut self,
        slot_id: u8,
        interface: u8,
        data: &[u8],
    ) -> Result<(), &'static str> {
        let setup = SetupPacket {
            request_type: 0x21, // Class, Interface, Host-to-Device
            request: 0x00,      // SEND_ENCAPSULATED_COMMAND
            value: 0,
            index: u16::from(interface),
            length: data.len() as u16,
        };
        self.control_transfer(slot_id, setup, None, Some(data))
    }

    fn rndis_get_encapsulated(
        &mut self,
        slot_id: u8,
        interface: u8,
        max_length: usize,
    ) -> Result<Vec<u8>, &'static str> {
        let mut buffer = DmaRegion::allocate(max_length.max(1))
            .map_err(|_| "RNDIS response buffer allocation failed")?;
        let setup = SetupPacket {
            request_type: 0xA1, // Class, Interface, Device-to-Host
            request: 0x01,      // GET_ENCAPSULATED_RESPONSE
            value: 0,
            index: u16::from(interface),
            length: max_length as u16,
        };
        self.control_transfer(slot_id, setup, Some(&mut buffer), None)?;
        Ok(buffer.slice()[..max_length].to_vec())
    }

    fn cdc_set_packet_filter(
        &mut self,
        slot_id: u8,
        interface: u8,
    ) -> Result<(), &'static str> {
        let setup = SetupPacket {
            request_type: 0x21, // Class, Interface, Host-to-Device
            request: 0x43,      // SET_ETHERNET_PACKET_FILTER
            value: 0x000F,      // directed+multicast+all_multicast+broadcast
            index: u16::from(interface),
            length: 0,
        };
        self.control_transfer(slot_id, setup, None, None)
    }

    fn configure_interrupt_in(
        &mut self,
        slot_id: u8,
        speed: UsbSpeed,
        endpoint: &UsbEndpoint,
        hid_kind: HidKind,
    ) -> Result<(), &'static str> {
        let endpoint_id = endpoint_id_from_address(endpoint.address);
        let report_size = match hid_kind {
            // Boot keyboard reports are 8 bytes; allow 9 for devices that prepend
            // a report-id while keeping transfer size tight to reduce latency/jitter.
            HidKind::Keyboard => usize::from(endpoint.max_packet_size.clamp(8, 9)),
            HidKind::Mouse => 4usize,
            HidKind::Combined | HidKind::Unknown => usize::from(endpoint.max_packet_size.max(8)),
        };
        let ring = XhciRing::new(32, true)?;
        let report_buffer = DmaRegion::allocate(report_size.max(8))
            .map_err(|_| "xHCI HID buffer allocation failed")?;
        let input_phys = {
            let context_size = self.context_size;
            let slot = self.slot_mut(slot_id)?;
            slot.input_context.zero();

            write_ctx32(&mut slot.input_context, 4, 1 | (1 << endpoint_id));
            write_ctx32(
                &mut slot.input_context,
                context_size,
                u32::from(endpoint_id) << 27,
            );

            let ep_offset = input_ep_context_offset(context_size, endpoint_id);
            let ep_type = endpoint_type(endpoint);
            let interval = endpoint_interval(speed, endpoint.interval);
            let max_packet = u32::from(endpoint.max_packet_size);

            write_ctx32(&mut slot.input_context, ep_offset, interval << 16);
            write_ctx32(
                &mut slot.input_context,
                ep_offset + 4,
                (3 << 1) | (ep_type << 3) | (max_packet << 16),
            );
            write_ctx64(&mut slot.input_context, ep_offset + 8, ring.phys_addr | 1);
            write_ctx32(
                &mut slot.input_context,
                ep_offset + 16,
                (report_size as u32) | (max_packet << 16),
            );

            slot.input_context.physical().as_u64()
        };

        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_CONFIGURE_ENDPOINT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Configure Endpoint failed");
        }

        {
            let slot = self.slot_mut(slot_id)?;
            slot.hid = Some(HidEndpoint {
                endpoint_address: endpoint.address,
                endpoint_id,
                ring,
                report_buffer,
                report_size,
                in_flight_trb: None,
                hid_kind,
                keyboard_state: KeyboardBootState::default(),
            });
        }

        self.arm_hid_endpoint(slot_id)
    }

    fn arm_hid_endpoint(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let endpoint_id = {
            let slot = self.slot_mut(slot_id)?;
            let hid = slot.hid.as_mut().ok_or("USB HID state missing")?;

            let transfer = Trb {
                parameter: hid.report_buffer.physical().as_u64(),
                status: hid.report_size as u32,
                control: ((TRB_NORMAL as u32) << 10) | IOC | ISP,
            };
            let trb_phys = hid.ring.enqueue(transfer)?;
            hid.in_flight_trb = Some(trb_phys);
            hid.endpoint_id
        };

        if XHCI_HID_INPUT_TRACE_ENABLED {
            let arm_n = HID_ARM_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
            if arm_n < 8 || arm_n & 0x7F == 0 {
                crate::serial_println!(
                    "[kbd] usb hid endpoint armed slot={} ep={} n={}",
                    slot_id,
                    endpoint_id,
                    arm_n
                );
            }
        }
        self.ring_doorbell(slot_id, endpoint_id);
        Ok(())
    }

    fn handle_transfer_event(&mut self, event: Trb) {
        let slot_id = event.slot_id();
        let endpoint_id = event.endpoint_id();
        if endpoint_id == 1 {
            return;
        }

        // Check network RX first
        let is_net_rx = self
            .slot(slot_id)
            .ok()
            .and_then(|s| s.network.as_ref())
            .map_or(false, |n| n.bulk_in_id == endpoint_id && n.rx_armed);
        if is_net_rx {
            self.process_network_rx_event(&event, slot_id);
            let _ = self.arm_network_rx(slot_id);
            return;
        }

        let should_rearm;
        {
            let Ok(slot) = self.slot_mut(slot_id) else {
                return;
            };
            let Some(hid) = slot.hid.as_mut() else {
                return;
            };
            if hid.endpoint_id != endpoint_id {
                return;
            }

            let cc = event.completion_code();
            let residue = event.transfer_residue();
            if cc == COMPLETION_SUCCESS || cc == COMPLETION_SHORT_PACKET {
                let actual = hid
                    .report_size
                    .saturating_sub((residue as usize).min(hid.report_size));
                let data = &hid.report_buffer.slice()[..actual];
                match hid.hid_kind {
                    HidKind::Keyboard => {
                        let trace_id = crate::hal::input::next_trace_id();
                        if XHCI_HID_INPUT_TRACE_ENABLED {
                            let detail = alloc::format!(
                                "slot={} ep={} cc={} residue={} report_len={} first={:02X} second={:02X}",
                                slot_id,
                                endpoint_id,
                                cc,
                                residue,
                                actual,
                                data.first().copied().unwrap_or(0),
                                data.get(1).copied().unwrap_or(0)
                            );
                            crate::hal::input::trace_stage_hw(
                                trace_id,
                                crate::hal::input::INPUT_SOURCE_USB,
                                detail.as_str(),
                            );
                        }
                        let emitted = hid::process_keyboard_boot_report(
                            &mut hid.keyboard_state,
                            data,
                            hid::HidTraceContext {
                                trace_id,
                                slot_id,
                                endpoint_id,
                                completion_code: cc,
                                report_len: actual,
                            },
                        );
                        if XHCI_HID_INPUT_TRACE_ENABLED {
                            let transfer_n =
                                HID_TRANSFER_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
                            if emitted > 0 || transfer_n < 8 || transfer_n & 0x3F == 0 {
                                crate::serial_println!(
                                    "[kbd] usb hid transfer slot={} ep={} cc={} residue={} data_len={} emitted={} kind={:?} n={}",
                                    slot_id,
                                    endpoint_id,
                                    cc,
                                    residue,
                                    actual,
                                    emitted,
                                    hid.hid_kind,
                                    transfer_n
                                );
                            }
                        }
                    }
                    HidKind::Mouse => {
                        let _ = hid::process_mouse_boot_report(data);
                    }
                    HidKind::Combined | HidKind::Unknown => {}
                }
            } else {
                if matches!(hid.hid_kind, HidKind::Keyboard) {
                    hid::reset_keyboard_boot_state(&mut hid.keyboard_state);
                }
                let fail_n = HID_TRANSFER_FAIL_TRACE_COUNT.fetch_add(1, Ordering::Relaxed);
                if fail_n < 16 || fail_n & 0xFF == 0 {
                    crate::serial_println!(
                        "[kbd] usb hid transfer FAILED cc={} slot={} ep={} n={}",
                        cc,
                        slot_id,
                        endpoint_id,
                        fail_n
                    );
                }
            }

            hid.in_flight_trb = None;
            should_rearm = true;
        }

        if should_rearm {
            if self.arm_hid_endpoint(slot_id).is_err() {
                if let Ok(slot) = self.slot_mut(slot_id) {
                    if let Some(hid) = slot.hid.as_mut() {
                        if matches!(hid.hid_kind, HidKind::Keyboard) {
                            hid::reset_keyboard_boot_state(&mut hid.keyboard_state);
                        }
                    }
                }
            }
        }
    }

    fn scsi_data_in(
        &mut self,
        slot_id: u8,
        command: &[u8; 16],
        command_length: u8,
        data_length: u32,
    ) -> Result<Vec<u8>, &'static str> {
        let tag = self.next_storage_tag(slot_id)?;
        let cbw = Cbw::new(tag, data_length, true, command, command_length);
        self.storage_bulk_out(slot_id, cbw.as_bytes())?;
        let data = self.storage_bulk_in(slot_id, data_length as usize)?;
        let csw_bytes = self.storage_bulk_in(slot_id, 13)?;
        let csw = Csw::from_bytes(&csw_bytes)?;
        csw.validate(tag)?;
        Ok(data)
    }

    fn scsi_data_out(
        &mut self,
        slot_id: u8,
        command: &[u8; 16],
        command_length: u8,
        payload: &[u8],
    ) -> Result<(), &'static str> {
        let tag = self.next_storage_tag(slot_id)?;
        let cbw = Cbw::new(tag, payload.len() as u32, false, command, command_length);
        self.storage_bulk_out(slot_id, cbw.as_bytes())?;
        self.storage_bulk_out(slot_id, payload)?;
        let csw_bytes = self.storage_bulk_in(slot_id, 13)?;
        let csw = Csw::from_bytes(&csw_bytes)?;
        csw.validate(tag)?;
        Ok(())
    }

    pub fn storage_read_sectors(
        &mut self,
        slot_id: u8,
        lba: u32,
        sectors: u16,
    ) -> Result<Vec<u8>, &'static str> {
        let sector_size = self
            .slot(slot_id)?
            .storage
            .as_ref()
            .ok_or("USB storage state missing")?
            .info
            .sector_size;
        let command = mass_storage::scsi_read_10_command(lba, sectors);
        self.scsi_data_in(slot_id, &command, 10, u32::from(sectors) * sector_size)
    }

    pub fn storage_write_sectors(
        &mut self,
        slot_id: u8,
        lba: u32,
        sectors: u16,
        payload: &[u8],
    ) -> Result<(), &'static str> {
        let sector_size = self
            .slot(slot_id)?
            .storage
            .as_ref()
            .ok_or("USB storage state missing")?
            .info
            .sector_size;
        if payload.len() < (u32::from(sectors) * sector_size) as usize {
            return Err("USB storage write payload too small");
        }
        let command = mass_storage::scsi_write_10_command(lba, sectors);
        self.scsi_data_out(slot_id, &command, 10, payload)
    }

    fn storage_bulk_out(&mut self, slot_id: u8, data: &[u8]) -> Result<(), &'static str> {
        let mut region = DmaRegion::allocate(data.len().max(1))
            .map_err(|_| "xHCI bulk OUT allocation failed")?;
        region.slice_mut()[..data.len()].copy_from_slice(data);

        let (endpoint_id, trb_phys) = {
            let slot = self.slot_mut(slot_id)?;
            let storage = slot.storage.as_mut().ok_or("USB storage state missing")?;
            let transfer = Trb {
                parameter: region.physical().as_u64(),
                status: data.len() as u32,
                control: ((TRB_NORMAL as u32) << 10) | IOC,
            };
            let trb_phys = storage.bulk_out_ring.enqueue(transfer)?;
            (storage.bulk_out_id, trb_phys)
        };

        self.ring_doorbell(slot_id, endpoint_id);
        let event = self.wait_transfer_event(slot_id, endpoint_id, trb_phys)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI bulk OUT transfer failed");
        }
        Ok(())
    }

    fn storage_bulk_in(&mut self, slot_id: u8, length: usize) -> Result<Vec<u8>, &'static str> {
        let region =
            DmaRegion::allocate(length.max(1)).map_err(|_| "xHCI bulk IN allocation failed")?;

        let (endpoint_id, trb_phys) = {
            let slot = self.slot_mut(slot_id)?;
            let storage = slot.storage.as_mut().ok_or("USB storage state missing")?;
            let transfer = Trb {
                parameter: region.physical().as_u64(),
                status: length as u32,
                control: ((TRB_NORMAL as u32) << 10) | IOC | ISP,
            };
            let trb_phys = storage.bulk_in_ring.enqueue(transfer)?;
            (storage.bulk_in_id, trb_phys)
        };

        self.ring_doorbell(slot_id, endpoint_id);
        let event = self.wait_transfer_event(slot_id, endpoint_id, trb_phys)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI bulk IN transfer failed");
        }

        let actual = length.saturating_sub((event.transfer_residue() as usize).min(length));
        Ok(region.slice()[..actual].to_vec())
    }

    fn next_storage_tag(&mut self, slot_id: u8) -> Result<u32, &'static str> {
        let slot = self.slot_mut(slot_id)?;
        let storage = slot.storage.as_mut().ok_or("USB storage state missing")?;
        let tag = storage.tag;
        storage.tag = storage.tag.wrapping_add(1).max(1);
        Ok(tag)
    }

    fn allocate_slot(
        &mut self,
        slot_id: u8,
        port: u8,
        speed: UsbSpeed,
    ) -> Result<(), &'static str> {
        let input_context = DmaRegion::allocate(self.context_size * 33)
            .map_err(|_| "xHCI input context allocation failed")?;
        let output_context = DmaRegion::allocate(self.context_size * 32)
            .map_err(|_| "xHCI output context allocation failed")?;
        let ep0_ring = XhciRing::new(32, true)?;
        let max_packet_size0 = default_max_packet_size0(speed);

        self.dcbaa_entries()[slot_id as usize] = output_context.physical().as_u64();
        fence(Ordering::SeqCst);
        self.slots[slot_id as usize] = Some(UsbSlotState {
            port,
            speed,
            input_context,
            output_context,
            ep0_ring,
            max_packet_size0,
            addressed: false,
            hid: None,
            storage: None,
            network: None,
        });
        crate::serial_println!(
            "[xHCI] slot {} assigned port={} speed={:?} ctx={} input=0x{:X} output=0x{:X} ep0=0x{:X} mps0={}",
            slot_id,
            port,
            speed,
            self.context_size,
            self.slots[slot_id as usize]
                .as_ref()
                .map(|slot| slot.input_context.physical().as_u64())
                .unwrap_or(0),
            self.slots[slot_id as usize]
                .as_ref()
                .map(|slot| slot.output_context.physical().as_u64())
                .unwrap_or(0),
            self.slots[slot_id as usize]
                .as_ref()
                .map(|slot| slot.ep0_ring.phys_addr)
                .unwrap_or(0),
            max_packet_size0
        );
        Ok(())
    }

    fn address_device(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let (input_phys, add_flags, slot_ctx_a, slot_ctx_b, ep0_ctx_a, ep0_ctx_b, ep0_ctx_ptr, ep0_ctx_c) = {
            let context_size = self.context_size;
            let slot = self.slot_mut(slot_id)?;
            slot.input_context.zero();

            write_ctx32(&mut slot.input_context, 4, 0b11);
            write_ctx32(
                &mut slot.input_context,
                context_size,
                (1 << 27) | (speed_code(slot.speed) << 20),
            );
            write_ctx32(
                &mut slot.input_context,
                context_size + 4,
                u32::from(slot.port) << 16,
            );

            let ep0_offset = input_ep_context_offset(context_size, 1);
            write_ctx32(&mut slot.input_context, ep0_offset, 0);
            write_ctx32(
                &mut slot.input_context,
                ep0_offset + 4,
                (3 << 1) | (EP_TYPE_CONTROL << 3) | (u32::from(slot.max_packet_size0) << 16),
            );
            write_ctx64(
                &mut slot.input_context,
                ep0_offset + 8,
                slot.ep0_ring.phys_addr | 1,
            );
            write_ctx32(&mut slot.input_context, ep0_offset + 16, 8);

            (
                slot.input_context.physical().as_u64(),
                read_ctx32(&slot.input_context, 4),
                read_ctx32(&slot.input_context, context_size),
                read_ctx32(&slot.input_context, context_size + 4),
                read_ctx32(&slot.input_context, ep0_offset),
                read_ctx32(&slot.input_context, ep0_offset + 4),
                read_ctx64(&slot.input_context, ep0_offset + 8),
                read_ctx32(&slot.input_context, ep0_offset + 16),
            )
        };

        crate::serial_println!(
            "[xHCI] address command slot={} input=0x{:X} add=0x{:08X} slot=[0x{:08X} 0x{:08X}] ep0=[0x{:08X} 0x{:08X} 0x{:016X} 0x{:08X}]",
            slot_id,
            input_phys,
            add_flags,
            slot_ctx_a,
            slot_ctx_b,
            ep0_ctx_a,
            ep0_ctx_b,
            ep0_ctx_ptr,
            ep0_ctx_c
        );

        // Retry loop: first try BSR=1 (just move slot to Default state without USB SET_ADDRESS),
        // then BSR=0 (full addressing). On Context State Error (cc=19) or USB Transaction Error
        // (cc=4), retry after a delay. This handles Intel 11th gen xHCI timing issues.
        let mut last_cc = 0u8;
        for attempt in 0..3u8 {
            if attempt > 0 {
                crate::serial_println!(
                    "[xHCI] address-device retry {} slot={} last_cc={}({})",
                    attempt, slot_id, last_cc, completion_code_name(last_cc)
                );
                // Re-populate input context (may have been corrupted by failed attempt)
                {
                    let context_size = self.context_size;
                    let slot = self.slot_mut(slot_id)?;
                    slot.input_context.zero();
                    write_ctx32(&mut slot.input_context, 4, 0b11);
                    write_ctx32(
                        &mut slot.input_context,
                        context_size,
                        (1 << 27) | (speed_code(slot.speed) << 20),
                    );
                    write_ctx32(
                        &mut slot.input_context,
                        context_size + 4,
                        u32::from(slot.port) << 16,
                    );
                    let ep0_offset = input_ep_context_offset(context_size, 1);
                    write_ctx32(&mut slot.input_context, ep0_offset, 0);
                    write_ctx32(
                        &mut slot.input_context,
                        ep0_offset + 4,
                        (3 << 1) | (EP_TYPE_CONTROL << 3) | (u32::from(slot.max_packet_size0) << 16),
                    );
                    write_ctx64(
                        &mut slot.input_context,
                        ep0_offset + 8,
                        slot.ep0_ring.phys_addr | 1,
                    );
                    write_ctx32(&mut slot.input_context, ep0_offset + 16, 8);
                }
                fence(Ordering::SeqCst);
                delay_ms(20 * u64::from(attempt));
            }

            let command = Trb {
                parameter: input_phys,
                status: 0,
                control: ((TRB_ADDRESS_DEVICE as u32) << 10) | ((slot_id as u32) << 24),
            };
            let event = match self.submit_command(command) {
                Ok(e) => e,
                Err(_) if attempt < 2 => {
                    last_cc = 0;
                    continue;
                }
                Err(e) => return Err(e),
            };
            last_cc = event.completion_code();

            if event.completion_code() != COMPLETION_SUCCESS {
                let portsc = self
                    .slot(slot_id)
                    .map(|slot| self.read_portsc(slot.port))
                    .unwrap_or(0);
                let (slot_state, usb_addr, ep0_state, ep0_mps) =
                    self.output_slot_ep0_summary(slot_id).unwrap_or((0, 0, 0, 0));
                crate::serial_println!(
                    "[xHCI] Address Device failed slot={} attempt={} cc={}({}) portsc=0x{:08X} out_slot_state={} usb_addr={} ep0_state={} ep0_mps={}",
                    slot_id,
                    attempt,
                    event.completion_code(),
                    completion_code_name(event.completion_code()),
                    portsc,
                    slot_state_name(slot_state),
                    usb_addr,
                    endpoint_state_name(ep0_state),
                    ep0_mps
                );
                // Retryable errors: Context State Error (19), USB Transaction Error (4), Parameter Error (17)
                if attempt < 2 && matches!(event.completion_code(), 4 | 17 | 19) {
                    continue;
                }
                return Err("xHCI Address Device failed");
            }

            let (slot_state, usb_addr, ep0_state, ep0_mps) =
                self.output_slot_ep0_summary(slot_id).unwrap_or((0, 0, 0, 0));
            crate::serial_println!(
                "[xHCI] address complete slot={} cc={}({}) state={} usb_addr={} ep0_state={} ep0_mps={} attempt={}",
                slot_id,
                event.completion_code(),
                completion_code_name(event.completion_code()),
                slot_state_name(slot_state),
                usb_addr,
                endpoint_state_name(ep0_state),
                ep0_mps,
                attempt
            );
            self.slot_mut(slot_id)?.addressed = true;
            return Ok(());
        }
        Err("xHCI Address Device failed after retries")
    }

    fn evaluate_ep0(&mut self, slot_id: u8, max_packet_size0: u16) -> Result<(), &'static str> {
        let input_phys = {
            let context_size = self.context_size;
            let slot = self.slot_mut(slot_id)?;
            slot.input_context.zero();

            write_ctx32(&mut slot.input_context, 4, 1 << 1);
            let ep0_offset = input_ep_context_offset(context_size, 1);
            write_ctx32(&mut slot.input_context, ep0_offset, 0);
            write_ctx32(
                &mut slot.input_context,
                ep0_offset + 4,
                (3 << 1) | (EP_TYPE_CONTROL << 3) | (u32::from(max_packet_size0) << 16),
            );
            write_ctx64(
                &mut slot.input_context,
                ep0_offset + 8,
                slot.ep0_ring.phys_addr | 1,
            );
            write_ctx32(&mut slot.input_context, ep0_offset + 16, 8);
            slot.input_context.physical().as_u64()
        };

        crate::serial_println!(
            "[xHCI] evaluate-context slot={} ep0-mps={}",
            slot_id,
            max_packet_size0
        );
        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_EVALUATE_CONTEXT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            crate::serial_println!(
                "[xHCI] Evaluate Context failed slot={} cc={}({})",
                slot_id,
                event.completion_code(),
                completion_code_name(event.completion_code())
            );
            return Err("xHCI Evaluate Context failed");
        }
        Ok(())
    }

    fn read_descriptor(
        &mut self,
        slot_id: u8,
        descriptor_type: u8,
        descriptor_index: u8,
        language_id: u16,
        length: usize,
    ) -> Result<Vec<u8>, &'static str> {
        let mut buffer = DmaRegion::allocate(length.max(1))
            .map_err(|_| "xHCI descriptor buffer allocation failed")?;
        let setup = SetupPacket {
            request_type: 0x80,
            request: 0x06,
            value: (u16::from(descriptor_type) << 8) | u16::from(descriptor_index),
            index: language_id,
            length: length as u16,
        };
        self.control_transfer(slot_id, setup, Some(&mut buffer), None)?;
        crate::serial_println!(
            "[xHCI] descriptor read slot={} type=0x{:02X} index={} lang=0x{:04X} len={}",
            slot_id,
            descriptor_type,
            descriptor_index,
            language_id,
            length
        );
        Ok(buffer.slice()[..length].to_vec())
    }

    fn set_configuration(&mut self, slot_id: u8, configuration: u8) -> Result<(), &'static str> {
        let setup = SetupPacket {
            request_type: 0x00,
            request: 0x09,
            value: u16::from(configuration),
            index: 0,
            length: 0,
        };
        self.control_transfer(slot_id, setup, None, None)
    }

    fn set_interface(
        &mut self,
        slot_id: u8,
        interface_number: u8,
        alternate_setting: u8,
    ) -> Result<(), &'static str> {
        let setup = SetupPacket {
            request_type: 0x01,
            request: 0x0B,
            value: u16::from(alternate_setting),
            index: u16::from(interface_number),
            length: 0,
        };
        self.control_transfer(slot_id, setup, None, None)
    }

    fn set_boot_protocol(&mut self, slot_id: u8, interface_number: u8) -> Result<(), &'static str> {
        let setup = SetupPacket {
            request_type: 0x21,
            request: 0x0B,
            value: 0,
            index: u16::from(interface_number),
            length: 0,
        };
        self.control_transfer(slot_id, setup, None, None)
    }

    fn control_transfer(
        &mut self,
        slot_id: u8,
        setup: SetupPacket,
        mut in_buffer: Option<&mut DmaRegion>,
        out_data: Option<&[u8]>,
    ) -> Result<(), &'static str> {
        crate::serial_println!(
            "[xHCI] ep0 slot={} bmRequestType=0x{:02X} bRequest=0x{:02X} wValue=0x{:04X} wIndex=0x{:04X} wLength={}",
            slot_id,
            setup.request_type,
            setup.request,
            setup.value,
            setup.index,
            setup.length
        );
        let mut out_region = None;
        let status_trb_phys = {
            let slot = self.slot_mut(slot_id)?;

            let setup_trb = Trb {
                parameter: setup.as_u64(),
                status: 8,
                control: ((TRB_SETUP_STAGE as u32) << 10) | (setup.transfer_type() << 16) | IDT,
            };
            let _ = slot.ep0_ring.enqueue(setup_trb)?;

            if setup.length > 0 {
                let data_length = u32::from(setup.length);
                if let Some(buffer) = in_buffer.as_deref_mut() {
                    let data_trb = Trb {
                        parameter: buffer.physical().as_u64(),
                        status: data_length,
                        control: ((TRB_DATA_STAGE as u32) << 10) | (1 << 16),
                    };
                    let _ = slot.ep0_ring.enqueue(data_trb)?;
                } else if let Some(out_data) = out_data {
                    if out_data.len() < data_length as usize {
                        return Err("xHCI control OUT buffer too short");
                    }
                    let mut data_region = DmaRegion::allocate(out_data.len())
                        .map_err(|_| "xHCI OUT data buffer allocation failed")?;
                    data_region.slice_mut()[..out_data.len()].copy_from_slice(out_data);
                    let data_trb = Trb {
                        parameter: data_region.physical().as_u64(),
                        status: data_length,
                        control: (TRB_DATA_STAGE as u32) << 10,
                    };
                    let _ = slot.ep0_ring.enqueue(data_trb)?;
                    out_region = Some(data_region);
                }
            }

            let status_direction = if setup.length == 0 || !setup.direction_in() {
                1u32
            } else {
                0u32
            };
            let status_trb = Trb {
                parameter: 0,
                status: 0,
                control: ((TRB_STATUS_STAGE as u32) << 10) | (status_direction << 16) | IOC,
            };
            slot.ep0_ring.enqueue(status_trb)?
        };

        self.ring_doorbell(slot_id, 1);
        let event = self.wait_transfer_event(slot_id, 1, status_trb_phys)?;
        let _ = out_region.as_ref();
        if !matches!(
            event.completion_code(),
            COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET
        ) {
            crate::serial_println!(
                "[xHCI] control transfer failed slot={} cc={}({})",
                slot_id,
                event.completion_code(),
                completion_code_name(event.completion_code())
            );
            return Err("xHCI control transfer failed");
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), &'static str> {
        let usbcmd = read_mmio32(self.op_base + USBCMD);
        write_mmio32(self.op_base + USBCMD, usbcmd | (1 << 1));

        for _ in 0..2_000_000 {
            let cmd = read_mmio32(self.op_base + USBCMD);
            let sts = read_mmio32(self.op_base + USBSTS);
            if (cmd & (1 << 1)) == 0 && (sts & (1 << 11)) == 0 {
                return Ok(());
            }
            spin_loop();
        }

        Err("xHCI reset timed out")
    }

    fn configure_runtime(&mut self) -> Result<(), &'static str> {
        self.initialize_dcbaa();
        write_mmio32(self.op_base + CONFIG, self.max_slots as u32);
        write_mmio64(self.op_base + DCBAAP, self.dcbaa.physical().as_u64());
        write_mmio64(self.op_base + CRCR, self.command_ring.phys_addr | 1);

        let erst_words = self.erst.slice_mut();
        erst_words.fill(0);
        erst_words[..8].copy_from_slice(&self.event_ring.phys_addr.to_le_bytes());
        erst_words[8..12].copy_from_slice(&(self.event_ring.size as u32).to_le_bytes());

        let ir0_base = self.rt_base + 0x20;
        write_mmio32(ir0_base + ERSTSZ, 1);
        write_mmio64(ir0_base + ERSTBA, self.erst.physical().as_u64());
        write_mmio64(ir0_base + ERDP, self.event_ring.dequeue_pointer());
        write_mmio32(ir0_base + IMAN, 1 << 1);
        fence(Ordering::SeqCst);
        Ok(())
    }

    fn start(&mut self) -> Result<(), &'static str> {
        let usbcmd = read_mmio32(self.op_base + USBCMD);
        write_mmio32(self.op_base + USBCMD, usbcmd | 0x1 | (1 << 2));

        for _ in 0..2_000_000 {
            if read_mmio32(self.op_base + USBSTS) & 0x1 == 0 {
                return Ok(());
            }
            spin_loop();
        }

        Err("xHCI start timed out")
    }

    fn reset_port(&mut self, port: u8) -> Result<u32, &'static str> {
        self.check_runtime_budget(port, "port-reset")?;
        let address = self.port_base(port) + PORTSC;
        let mut portsc = self.power_port_if_needed(port);
        log_portsc("before-reset", port, portsc);
        self.note_portsc_snapshot(port, "before-reset", portsc, 0);
        if portsc & PORTSC_CCS == 0 {
            return Err("xHCI port disconnected before reset");
        }

        let speed = decode_speed(((portsc >> 10) & 0xF) as u8);
        let is_super_speed =
            matches!(speed, UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2);

        if !is_super_speed {
            self.check_runtime_budget(port, "usb2-connect-debounce")?;
            delay_ms(USB2_CONNECT_DEBOUNCE_MS);
            self.check_runtime_budget(port, "usb2-connect-debounce")?;
            portsc = read_mmio32(address);
            self.note_portsc_snapshot(port, "usb2-post-debounce", portsc, 0);
            log_portsc("usb2-post-debounce", port, portsc);
            if portsc & PORTSC_CCS == 0 {
                return Err("xHCI USB2/full-speed port disconnected during debounce");
            }
            if portsc & PORTSC_PED != 0 {
                return Ok(portsc);
            }
        }

        if is_super_speed {
            self.note_port_stage(port, "warm-reset");
            crate::serial_println!("[xHCI] port {} attempting warm reset first (SuperSpeed)", port);
            match self.do_port_reset(port, address, portsc, true, false) {
                Ok(value) => return Ok(value),
                Err(err) => {
                    crate::serial_println!(
                        "[xHCI] port {} warm reset failed: {} speed={:?}",
                        port,
                        err,
                        speed
                    );
                }
            }
        }

        self.note_port_stage(port, "port-reset");
        match self.do_port_reset(port, address, self.read_portsc(port), false, !is_super_speed) {
            Ok(value) => return Ok(value),
            Err(err) => {
                crate::serial_println!(
                    "[xHCI] port {} standard reset failed: {} speed={:?} is_ss={}",
                    port,
                    err,
                    speed,
                    is_super_speed
                );
                if is_super_speed || self.runtime_catch_up_deadline_ms != 0 {
                    return Err(err);
                }
            }
        }

        delay_ms(50);
        let portsc_retry = read_mmio32(address);
        if portsc_retry & PORTSC_CCS == 0 {
            return Err("xHCI port disconnected before reset retry");
        }
        crate::serial_println!("[xHCI] port {} retrying standard reset after settle", port);
        self.do_port_reset(port, address, portsc_retry, false, true)
    }
    fn do_port_reset(
        &mut self,
        port: u8,
        address: u64,
        portsc: u32,
        warm: bool,
        usb2_port: bool,
    ) -> Result<u32, &'static str> {
        let change_mask = PORTSC_CHANGE_BITS;
        let reset_bit = if warm { PORTSC_WPR } else { PORTSC_PR };

        write_portsc(address, portsc, reset_bit, change_mask);
        fence(Ordering::SeqCst);
        self.note_portsc_snapshot(
            port,
            if warm {
                "warm-reset-issued"
            } else {
                "port-reset-issued"
            },
            portsc,
            change_mask,
        );

        let check_bit = if warm { PORTSC_WPR } else { PORTSC_PR };
        let reset_deadline = current_time_ms().saturating_add(PORT_RESET_TIMEOUT_MS);
        loop {
            self.check_runtime_budget(port, if warm { "warm-reset" } else { "port-reset" })?;
            let value = read_mmio32(address);
            if value & check_bit == 0 {
                let reset_complete_seen = if usb2_port {
                    value & (PORTSC_PRC | PORTSC_PED) != 0
                } else {
                    true
                };
                if !reset_complete_seen {
                    self.note_portsc_snapshot(
                        port,
                        if warm {
                            "warm-reset-pr-cleared-wait"
                        } else {
                            "reset-pr-cleared-wait"
                        },
                        value,
                        0,
                    );
                    if current_time_ms() >= reset_deadline {
                        return Err(
                            "xHCI USB2/full-speed reset cleared PR but never asserted PRC/PED",
                        );
                    }
                    spin_loop();
                    continue;
                }
                if usb2_port && value & PORTSC_PRC != 0 && value & PORTSC_PED == 0 {
                    self.note_portsc_snapshot(
                        port,
                        if warm {
                            "warm-reset-complete-no-ped"
                        } else {
                            "reset-complete-no-ped"
                        },
                        value,
                        0,
                    );
                    self.last_port_progress.reset_completion_source = "prc-no-ped";
                    log_portsc(
                        if warm {
                            "after-warm-reset-no-ped"
                        } else {
                            "after-reset-no-ped"
                        },
                        port,
                        value,
                    );
                    delay_ms(PORT_RESET_SETTLE_DELAY_MS);
                    self.check_runtime_budget(port, "post-reset-enable-check")?;
                    let settled = read_mmio32(address);
                    self.note_portsc_snapshot(port, "post-reset-enable-check", settled, 0);
                    if settled & PORTSC_CCS == 0 {
                        return Err(
                            "xHCI USB2/full-speed port disconnected after reset completion",
                        );
                    }
                    if settled & PORTSC_PED == 0 {
                        let settled_ack = settled & PORTSC_CHANGE_BITS;
                        if settled_ack != 0 {
                            write_portsc(address, settled, 0, settled_ack);
                        }
                        self.note_portsc_snapshot(
                            port,
                            "post-reset-enable-timeout",
                            settled,
                            settled_ack,
                        );
                        return Err(
                            "xHCI USB2/full-speed reset completed (PRC) but PED never asserted",
                        );
                    }
                    let settled_ack = settled & PORTSC_CHANGE_BITS;
                    if settled_ack != 0 {
                        write_portsc(address, settled, 0, settled_ack);
                    }
                    self.note_portsc_snapshot(port, "post-reset-enable-ok", settled, settled_ack);
                    log_portsc("post-reset-enable-ok", port, settled);
                    return Ok(settled);
                }

                let ack = value & PORTSC_CHANGE_BITS;
                let reset_source = if usb2_port {
                    if value & PORTSC_PRC != 0 && value & PORTSC_PED != 0 {
                        "prc+ped"
                    } else if value & PORTSC_PRC != 0 {
                        "prc"
                    } else {
                        "ped"
                    }
                } else {
                    "pr-cleared"
                };
                if usb2_port && value & PORTSC_PED != 0 {
                    if ack != 0 {
                        write_portsc(address, value, 0, ack);
                    }
                    self.note_portsc_snapshot(
                        port,
                        if warm {
                            "warm-reset-complete-enabled"
                        } else {
                            "reset-complete-enabled"
                        },
                        value,
                        ack,
                    );
                    self.last_port_progress.reset_completion_source = reset_source;
                    log_portsc(
                        if warm {
                            "after-warm-reset-enabled"
                        } else {
                            "after-reset-enabled"
                        },
                        port,
                        value,
                    );
                    return Ok(value);
                }
                if ack != 0 {
                    write_portsc(address, value, 0, ack);
                }
                self.note_portsc_snapshot(
                    port,
                    if warm {
                        "warm-reset-complete"
                    } else {
                        "reset-complete"
                    },
                    value,
                    ack,
                );
                if usb2_port {
                    self.last_port_progress.reset_completion_source = reset_source;
                }
                log_portsc(if warm { "after-warm-reset" } else { "after-reset" }, port, value);
                delay_ms(PORT_RESET_SETTLE_DELAY_MS);
                break;
            }
            if current_time_ms() >= reset_deadline {
                self.note_portsc_snapshot(
                    port,
                    if warm {
                        "warm-reset-timeout"
                    } else {
                        "reset-timeout"
                    },
                    value,
                    0,
                );
                log_portsc(if warm { "warm-reset-timeout" } else { "reset-timeout" }, port, value);
                return Err(if warm {
                    "xHCI warm port reset timed out"
                } else {
                    "xHCI port reset timed out"
                });
            }
            spin_loop();
        }

        let enable_deadline = current_time_ms().saturating_add(PORT_ENABLE_TIMEOUT_MS);
        loop {
            self.check_runtime_budget(port, if warm { "warm-reset-enable" } else { "port-enable" })?;
            let value = read_mmio32(address);
            let ack = value & PORTSC_CHANGE_BITS;
            if ack != 0 {
                write_portsc(address, value, 0, ack);
            }
            self.note_portsc_snapshot(
                port,
                if warm {
                    "warm-reset-enable"
                } else {
                    "port-enable"
                },
                value,
                ack,
            );
            if value & PORTSC_CCS == 0 {
                log_portsc(if warm { "warm-reset-lost-connect" } else { "reset-lost-connect" }, port, value);
                return Err("xHCI port disconnected while waiting for enable");
            }
            if value & PORTSC_PED != 0 {
                log_portsc(if warm { "warm-reset-enabled" } else { "reset-enabled" }, port, value);
                return Ok(value);
            }
            if current_time_ms() >= enable_deadline {
                self.note_portsc_snapshot(
                    port,
                    if warm {
                        "warm-reset-enable-timeout"
                    } else {
                        "reset-enable-timeout"
                    },
                    value,
                    ack,
                );
                log_portsc(if warm { "warm-reset-enable-timeout" } else { "reset-enable-timeout" }, port, value);
                return Err(if usb2_port {
                    if self.last_port_progress.ped_observed {
                        "xHCI USB2/full-speed PED was observed, but port enable did not remain stable long enough to advance"
                    } else {
                        "xHCI USB2/full-speed port reset completed but PED never asserted"
                    }
                } else {
                    "xHCI port never reached enabled state"
                });
            }
            spin_loop();
        }
    }
    fn enable_slot(&mut self) -> Result<u8, &'static str> {
        let mut last_cc = 0u8;
        for attempt in 0..3u8 {
            if attempt > 0 {
                crate::serial_println!(
                    "[xHCI] enable-slot retry {} (last cc={}({}))",
                    attempt,
                    last_cc,
                    completion_code_name(last_cc)
                );
                delay_ms(10);
            }
            let command = Trb {
                parameter: 0,
                status: 0,
                control: (TRB_ENABLE_SLOT as u32) << 10,
            };
            let event = match self.submit_command(command) {
                Ok(e) => e,
                Err(_) if attempt < 2 => {
                    last_cc = 0;
                    continue;
                }
                Err(e) => return Err(e),
            };
            last_cc = event.completion_code();
            if event.completion_code() != COMPLETION_SUCCESS {
                if attempt < 2 {
                    continue;
                }
                return Err("xHCI Enable Slot failed");
            }
            if event.slot_id() == 0 {
                crate::serial_println!(
                    "[xHCI] enable-slot invalid slot=0 cc={}({})",
                    event.completion_code(),
                    completion_code_name(event.completion_code())
                );
                if attempt < 2 {
                    continue;
                }
                return Err("xHCI Enable Slot returned slot 0");
            }
            crate::serial_println!(
                "[xHCI] enable-slot cc={}({}) slot={} attempt={}",
                event.completion_code(),
                completion_code_name(event.completion_code()),
                event.slot_id(),
                attempt
            );
            return Ok(event.slot_id());
        }
        Err("xHCI Enable Slot failed after retries")
    }

    fn disable_slot(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let command = Trb {
            parameter: 0,
            status: 0,
            control: ((TRB_DISABLE_SLOT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Disable Slot failed");
        }
        Ok(())
    }

    fn teardown_slot(&mut self, slot_id: u8, log: bool) {
        if slot_id == 0 {
            return;
        }
        let addressed = if let Some(slot) = self
            .slots
            .get(slot_id as usize)
            .and_then(Option::as_ref)
        {
            slot.addressed
        } else {
            return;
        };

        if !addressed {
            if log {
                crate::serial_println!(
                    "[xHCI] slot {} dropped without Disable Slot because Address Device never succeeded",
                    slot_id
                );
            }
        } else if let Err(error) = self.disable_slot(slot_id) {
            if log {
                crate::serial_println!("[xHCI] slot {} disable failed: {}", slot_id, error);
            }
        } else if log {
            crate::serial_println!("[xHCI] slot {} detached", slot_id);
        }
        if let Some(slot) = self.slots.get_mut(slot_id as usize) {
            *slot = None;
        }
    }

    fn submit_command(&mut self, trb: Trb) -> Result<Trb, &'static str> {
        let trb_phys = self.command_ring.enqueue(trb)?;
        let command_name = trb_type_name(trb.trb_type());
        self.last_command_name = command_name;
        self.last_command_trb_phys = trb_phys;
        self.last_command_trb = Some(trb);
        crate::serial_println!(
            "[xHCI] cmd submit {} phys=0x{:X} param=0x{:016X} status=0x{:08X} control=0x{:08X}",
            command_name,
            trb_phys,
            trb.parameter,
            trb.status,
            trb.control
        );
        fence(Ordering::SeqCst);
        write_mmio32(self.db_base, 0);

        let deadline = if self.runtime_catch_up_deadline_ms != 0 {
            self.runtime_catch_up_deadline_ms
        } else {
            0
        };
        for _ in 0..2_000_000 {
            if deadline != 0 && current_time_ms() >= deadline {
                self.runtime_catch_up_aborted = true;
                self.last_topology_service_result = "budget-exhausted";
                self.last_topology_service_ms = current_time_ms();
                if let Some(port) = self.last_port_progress.port {
                    self.note_enumeration_failure(
                        port,
                        self.last_port_progress.last_stage,
                        "runtime catch-up budget exhausted while waiting for xHCI command completion",
                    );
                }
                return Err("runtime catch-up budget exhausted");
            }
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_COMMAND_COMPLETION {
                    self.last_command_completion = Some(event);
                    crate::serial_println!(
                        "[xHCI] cmd event {} phys=0x{:X} cc={}({}) slot={} ep={}",
                        command_name,
                        event.parameter,
                        event.completion_code(),
                        completion_code_name(event.completion_code()),
                        event.slot_id(),
                        event.endpoint_id()
                    );
                    if event.parameter == trb_phys {
                        return Ok(event);
                    }
                    crate::serial_println!(
                        "[xHCI] cmd event ignored expected=0x{:X} actual=0x{:X}",
                        trb_phys,
                        event.parameter
                    );
                } else if event.trb_type() == TRB_TRANSFER_EVENT {
                    self.last_transfer_completion = Some(event);
                    self.handle_transfer_event(event);
                } else if event.trb_type() == TRB_PORT_STATUS_CHANGE {
                    crate::serial_println!(
                        "[xHCI] cmd wait saw port status change slot={} ep={}",
                        event.slot_id(),
                        event.endpoint_id()
                    );
                }
            }
            spin_loop();
        }

        Err("xHCI command timed out")
    }

    fn wait_transfer_event(
        &mut self,
        slot_id: u8,
        endpoint_id: u8,
        trb_pointer: u64,
    ) -> Result<Trb, &'static str> {
        let deadline = if self.runtime_catch_up_deadline_ms != 0 {
            self.runtime_catch_up_deadline_ms
        } else {
            0
        };
        for _ in 0..2_000_000 {
            if deadline != 0 && current_time_ms() >= deadline {
                self.runtime_catch_up_aborted = true;
                self.last_topology_service_result = "budget-exhausted";
                self.last_topology_service_ms = current_time_ms();
                if let Ok(slot) = self.slot(slot_id) {
                    self.note_enumeration_failure(
                        slot.port,
                        self.last_port_progress.last_stage,
                        "runtime catch-up budget exhausted while waiting for USB transfer completion",
                    );
                }
                return Err("runtime catch-up budget exhausted");
            }
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_TRANSFER_EVENT
                    && event.slot_id() == slot_id
                    && event.endpoint_id() == endpoint_id
                    && event.parameter == trb_pointer
                {
                    self.last_transfer_completion = Some(event);
                    crate::serial_println!(
                        "[xHCI] transfer event slot={} ep={} ptr=0x{:X} cc={}({}) residue={}",
                        slot_id,
                        endpoint_id,
                        trb_pointer,
                        event.completion_code(),
                        completion_code_name(event.completion_code()),
                        event.transfer_residue()
                    );
                    return Ok(event);
                } else if event.trb_type() == TRB_COMMAND_COMPLETION {
                    self.last_command_completion = Some(event);
                } else if event.trb_type() == TRB_PORT_STATUS_CHANGE {
                    crate::serial_println!(
                        "[xHCI] transfer wait saw port status change slot={} ep={}",
                        event.slot_id(),
                        event.endpoint_id()
                    );
                }
            }
            spin_loop();
        }

        Err("xHCI transfer timed out")
    }

    fn next_event(&mut self) -> Option<Trb> {
        let event = self.event_ring.dequeue()?;
        let ir0_base = self.rt_base + 0x20;
        // Memory fence ensures the event TRB data is fully read before we advance ERDP,
        // preventing the controller from overwriting the TRB while we still read it.
        fence(Ordering::SeqCst);
        write_mmio64(
            ir0_base + ERDP,
            self.event_ring.dequeue_pointer() | (1 << 3),
        );
        Some(event)
    }

    fn slot(&self, slot_id: u8) -> Result<&UsbSlotState, &'static str> {
        self.slots
            .get(slot_id as usize)
            .and_then(Option::as_ref)
            .ok_or("xHCI slot state missing")
    }

    fn slot_mut(&mut self, slot_id: u8) -> Result<&mut UsbSlotState, &'static str> {
        self.slots
            .get_mut(slot_id as usize)
            .and_then(Option::as_mut)
            .ok_or("xHCI slot state missing")
    }

    fn dcbaa_entries(&mut self) -> &mut [u64] {
        unsafe { slice::from_raw_parts_mut(self.dcbaa.as_mut_ptr().cast::<u64>(), 256) }
    }

    fn initialize_dcbaa(&mut self) {
        let scratchpad_phys = self
            .scratchpad_array
            .as_ref()
            .map(|array| array.physical().as_u64())
            .unwrap_or(0);
        {
            let entries = self.dcbaa_entries();
            entries.fill(0);
            entries[0] = scratchpad_phys;
        }
        crate::serial_println!(
            "[xHCI] DCBAA base=0x{:X} scratchpad_array=0x{:X} scratchpads={}",
            self.dcbaa.physical().as_u64(),
            scratchpad_phys,
            self.scratchpad_buffers.len()
        );
    }

    fn output_slot_ep0_summary(&self, slot_id: u8) -> Option<(u8, u8, u8, u16)> {
        let slot = self.slot(slot_id).ok()?;
        let slot_state_raw = read_ctx32(&slot.output_context, 12);
        let ep0_state_raw = read_ctx32(&slot.output_context, output_ep_context_offset(self.context_size, 1));
        let ep0_info = read_ctx32(
            &slot.output_context,
            output_ep_context_offset(self.context_size, 1) + 4,
        );
        Some((
            ((slot_state_raw >> 27) & 0x1F) as u8,
            (slot_state_raw & 0xFF) as u8,
            (ep0_state_raw & 0x7) as u8,
            ((ep0_info >> 16) & 0xFFFF) as u16,
        ))
    }

    pub fn diagnostic_report(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let active_attempt_port = self.last_port_progress.port.filter(|port| {
            self.pending_port_rescan || self.ports.iter().any(|status| status.port == *port)
        });
        lines.push(format!(
            "controller pci={:02X}:{:02X}.{} mmio=0x{:X} ctx={} dcbaa=0x{:X} command_ring=0x{:X} event_ring=0x{:X} scratchpads={}",
            self.pci.bus,
            self.pci.device,
            self.pci.function,
            self.mmio_phys,
            self.context_size,
            self.dcbaa.physical().as_u64(),
            self.command_ring.phys_addr,
            self.event_ring.phys_addr,
            self.scratchpad_buffers.len()
        ));
        if let Some(command) = self.last_command_trb {
            lines.push(format!(
                "last-command name={} phys=0x{:X} param=0x{:016X} status=0x{:08X} control=0x{:08X}",
                self.last_command_name,
                self.last_command_trb_phys,
                command.parameter,
                command.status,
                command.control
            ));
        }
        if let Some(event) = self.last_command_completion {
            lines.push(format!(
                "last-command-completion cc={}({}) slot={} ep={} ptr=0x{:X}",
                event.completion_code(),
                completion_code_name(event.completion_code()),
                event.slot_id(),
                event.endpoint_id(),
                event.parameter
            ));
        }
        if let Some(event) = self.last_transfer_completion {
            lines.push(format!(
                "last-transfer-completion cc={}({}) slot={} ep={} ptr=0x{:X} residue={}",
                event.completion_code(),
                completion_code_name(event.completion_code()),
                event.slot_id(),
                event.endpoint_id(),
                event.parameter,
                event.transfer_residue()
            ));
        }
        lines.push(format!(
            "runtime pending_topology={} last_event={} port={} event_ms={} topo_result={} topo_ms={} inventory_sync_ms={} inventory_devices={} active_attempt_port={} progress_port={} progress_stage={} connect={} reset={} reset_done={} reset_src={} warm_reset={} enabled={} ped_seen={} ped_first_ms={} ped_last_ms={} ped_lost={} slot={} addressed={} desc={} cfg_try={} configured={} budget_after_reset={} portsc=0x{:08X} link={} chg=0x{:08X} ack=0x{:08X} seen=0x{:08X} acked=0x{:08X} fail_step={} last_enum_failure_port={} last_enum_failure_reason={}",
            self.pending_port_rescan,
            self.last_runtime_event,
            self.last_runtime_event_port.unwrap_or(0),
            self.last_runtime_event_ms,
            self.last_topology_service_result,
            self.last_topology_service_ms,
            self.last_inventory_sync_ms,
            self.last_inventory_sync_devices,
            active_attempt_port.unwrap_or(0),
            self.last_port_progress.port.unwrap_or(0),
            self.last_port_progress.last_stage,
            self.last_port_progress.connect_seen,
            self.last_port_progress.reset_attempted,
            self.last_port_progress.reset_completed,
            self.last_port_progress.reset_completion_source,
            self.last_port_progress.warm_reset_attempted,
            self.last_port_progress.port_enabled,
            self.last_port_progress.ped_observed,
            self.last_port_progress.ped_first_seen_ms,
            self.last_port_progress.ped_last_seen_ms,
            self.last_port_progress.ped_lost_after_observed,
            self.last_port_progress.slot_allocated,
            self.last_port_progress.address_assigned,
            self.last_port_progress.descriptors_fetched,
            self.last_port_progress.configuration_attempted,
            self.last_port_progress.configured,
            self.last_port_progress.budget_exhausted_after_reset,
            self.last_port_progress.last_portsc,
            self.last_port_progress.last_link_state,
            self.last_port_progress.last_change_bits,
            self.last_port_progress.last_ack_bits,
            self.last_port_progress.observed_change_bits,
            self.last_port_progress.observed_ack_bits,
            self.last_port_progress.failure_step,
            self.last_enumeration_failure_port.unwrap_or(0),
            self.last_enumeration_failure_reason
        ));
        for stale in self.stale_failed_ports.iter().flatten() {
            lines.push(format!(
                "stale-failure port={} stage={} reason={} age_ms={}",
                stale.port,
                stale.stage,
                stale.reason,
                current_time_ms().saturating_sub(stale.noted_ms)
            ));
        }
        for port in 1..=self.max_ports {
            let portsc = self.read_portsc(port);
            let attached_slot = self
                .ports
                .iter()
                .find(|status| status.port == port)
                .and_then(|status| status.slot_id);
            lines.push(format!(
                "port={} portsc=0x{:08X} connected={} enabled={} slot={}",
                port,
                portsc,
                portsc & PORTSC_CCS != 0,
                portsc & PORTSC_PED != 0,
                attached_slot.unwrap_or(0)
            ));
            if let Some(slot_id) = attached_slot {
                if let Some((slot_state, usb_addr, ep0_state, ep0_mps)) =
                    self.output_slot_ep0_summary(slot_id)
                {
                    lines.push(format!(
                        "slot={} slot-state={} usb-address={} ep0-state={} ep0-mps={}",
                        slot_id,
                        slot_state_name(slot_state),
                        usb_addr,
                        endpoint_state_name(ep0_state),
                        ep0_mps
                    ));
                }
            }
        }
        lines
    }

    fn note_runtime_event(&mut self, label: &'static str, port: Option<u8>) {
        self.prune_stale_failed_ports();
        self.last_runtime_event = label;
        self.last_runtime_event_port = port;
        self.last_runtime_event_ms = current_time_ms();
    }

    fn note_port_stage(&mut self, port: u8, stage: &'static str) {
        if self.last_port_progress.port != Some(port) {
            self.retire_last_port_progress("superseded-by-new-port");
            self.last_port_progress = UsbRuntimePortProgress {
                port: Some(port),
                ..UsbRuntimePortProgress::default()
            };
        }
        self.clear_stale_failed_port(port);
        self.last_port_progress.last_stage = stage;
        self.last_port_progress.last_stage_ms = current_time_ms();
        match stage {
            "connect-seen" => self.last_port_progress.connect_seen = true,
            "port-reset" => self.last_port_progress.reset_attempted = true,
            "warm-reset" => {
                self.last_port_progress.reset_attempted = true;
                self.last_port_progress.warm_reset_attempted = true;
            }
            "reset-complete"
            | "warm-reset-complete"
            | "reset-complete-enabled"
            | "warm-reset-complete-enabled"
            | "reset-complete-no-ped"
            | "warm-reset-complete-no-ped" => {
                self.last_port_progress.reset_attempted = true;
                self.last_port_progress.reset_completed = true;
                if matches!(stage, "reset-complete-enabled" | "warm-reset-complete-enabled") {
                    self.last_port_progress.port_enabled = true;
                    self.last_port_progress.ped_observed = true;
                }
                self.last_port_progress.reset_completion_source = match stage {
                    "reset-complete-enabled" => "ped-latched",
                    "warm-reset-complete-enabled" => "warm-ped-latched",
                    "warm-reset-complete" => "warm-pr-cleared",
                    "warm-reset-complete-no-ped" => "warm-prc-no-ped",
                    "reset-complete-no-ped" => "prc-no-ped",
                    _ => "pr-cleared",
                };
            }
            "port-enabled" => {
                self.last_port_progress.reset_attempted = true;
                self.last_port_progress.reset_completed = true;
                self.last_port_progress.port_enabled = true;
                self.last_port_progress.ped_observed = true;
            }
            "post-reset-enable-ok" => {
                self.last_port_progress.reset_attempted = true;
                self.last_port_progress.reset_completed = true;
                self.last_port_progress.port_enabled = true;
                self.last_port_progress.ped_observed = true;
            }
            "enable-slot" => {
                self.last_port_progress.port_enabled = true;
                self.last_port_progress.ped_observed = true;
            }
            "slot-allocated" => self.last_port_progress.slot_allocated = true,
            "address-device" => self.last_port_progress.slot_allocated = true,
            "address-assigned" => self.last_port_progress.address_assigned = true,
            "read-device-header" | "read-device-descriptor" | "read-config-header"
            | "read-config-descriptors" | "descriptors-fetched" => {
                self.last_port_progress.address_assigned = true;
                if matches!(stage, "descriptors-fetched") {
                    self.last_port_progress.descriptors_fetched = true;
                }
            }
            "configuration-attempted" => {
                self.last_port_progress.descriptors_fetched = true;
                self.last_port_progress.configuration_attempted = true;
            }
            "configured" => {
                self.last_port_progress.descriptors_fetched = true;
                self.last_port_progress.configuration_attempted = true;
                self.last_port_progress.configured = true;
            }
            _ => {}
        }
    }

    fn note_portsc_snapshot(&mut self, port: u8, stage: &'static str, portsc: u32, ack_bits: u32) {
        self.note_port_stage(port, stage);
        let change_bits = portsc & PORTSC_CHANGE_BITS;
        let acked_bits = ack_bits & PORTSC_CHANGE_BITS;
        let now = current_time_ms();
        self.last_port_progress.last_portsc = portsc;
        self.last_port_progress.last_portsc_ms = now;
        self.last_port_progress.last_change_bits = change_bits;
        self.last_port_progress.last_ack_bits = acked_bits;
        if change_bits != 0 {
            self.last_port_progress.observed_change_bits |= change_bits;
        }
        if acked_bits != 0 {
            self.last_port_progress.observed_ack_bits |= acked_bits;
        }
        if portsc & PORTSC_PED != 0 {
            if !self.last_port_progress.ped_observed {
                self.last_port_progress.ped_first_seen_ms = now;
            }
            self.last_port_progress.ped_observed = true;
            self.last_port_progress.ped_last_seen_ms = now;
        } else if self.last_port_progress.ped_observed && !self.last_port_progress.slot_allocated {
            self.last_port_progress.ped_lost_after_observed = true;
        }
        self.last_port_progress.last_link_state = port_link_state_name(portsc);
    }

    fn check_runtime_budget(&mut self, port: u8, step: &'static str) -> Result<(), &'static str> {
        self.note_port_stage(port, step);
        if self.runtime_catch_up_deadline_ms == 0 {
            return Ok(());
        }
        if current_time_ms() < self.runtime_catch_up_deadline_ms {
            return Ok(());
        }
        self.runtime_catch_up_aborted = true;
        self.last_topology_service_result = "budget-exhausted";
        self.last_topology_service_ms = current_time_ms();
        self.last_port_progress.budget_exhausted_after_reset = self.last_port_progress.reset_completed;
        let reason = match step {
            "port-enable" | "warm-reset-enable" | "post-reset-enable-check"
                if self.last_port_progress.reset_completed
                    && self.last_port_progress.ped_observed
                    && !self.last_port_progress.port_enabled =>
            {
                "runtime catch-up budget exhausted after PED was observed, but port enable did not stabilize for slot allocation"
            }
            "port-enable" | "warm-reset-enable" | "post-reset-enable-check"
                if self.last_port_progress.reset_completed =>
            {
                "runtime catch-up budget exhausted while waiting for port enable after reset completion"
            }
            "port-reset" | "warm-reset" if !self.last_port_progress.reset_completed => {
                "runtime catch-up budget exhausted before reset completion"
            }
            _ => "runtime catch-up budget exhausted",
        };
        self.note_enumeration_failure(port, step, reason);
        Err(reason)
    }

    fn note_enumeration_failure(&mut self, port: u8, step: &'static str, reason: &'static str) {
        self.note_port_stage(port, step);
        self.last_port_progress.failure_step = step;
        self.last_enumeration_failure_port = Some(port);
        self.last_enumeration_failure_reason = reason;
        self.note_runtime_event("enum-failure", Some(port));
    }

    fn clear_stale_failed_port(&mut self, port: u8) {
        for entry in &mut self.stale_failed_ports {
            if matches!(entry, Some(stale) if stale.port == port) {
                *entry = None;
            }
        }
    }

    pub(super) fn prune_stale_failed_ports(&mut self) {
        let now = current_time_ms();
        let mut retained = [None; 4];
        let mut next = 0usize;
        for stale in self.stale_failed_ports.iter().flatten() {
            if now.saturating_sub(stale.noted_ms) > RETIRED_PORT_FAILURE_RETENTION_MS {
                continue;
            }
            if next >= retained.len() {
                break;
            }
            retained[next] = Some(*stale);
            next += 1;
        }
        self.stale_failed_ports = retained;
    }

    fn retire_last_port_progress(&mut self, reason: &'static str) {
        let Some(port) = self.last_port_progress.port else {
            return;
        };
        let meaningful = self.last_port_progress.connect_seen
            || self.last_port_progress.reset_attempted
            || self.last_port_progress.port_enabled
            || self.last_port_progress.slot_allocated
            || self.last_port_progress.address_assigned
            || self.last_port_progress.descriptors_fetched
            || self.last_port_progress.failure_step != "none";
        if !meaningful {
            return;
        }

        let stage = if self.last_port_progress.failure_step != "none" {
            self.last_port_progress.failure_step
        } else {
            self.last_port_progress.last_stage
        };
        let stale = UsbRetiredPortFailure {
            port,
            stage,
            reason: if self.last_enumeration_failure_port == Some(port) {
                self.last_enumeration_failure_reason
            } else {
                reason
            },
            noted_ms: current_time_ms(),
        };
        if self.stale_failed_ports.iter().any(|entry| {
            matches!(
                entry,
                Some(existing)
                    if existing.port == stale.port
                        && existing.stage == stale.stage
                        && existing.reason == stale.reason
            )
        }) {
            return;
        }
        for index in (1..self.stale_failed_ports.len()).rev() {
            self.stale_failed_ports[index] = self.stale_failed_ports[index - 1];
        }
        self.stale_failed_ports[0] = Some(stale);
    }

    fn ring_doorbell(&self, slot_id: u8, endpoint_id: u8) {
        fence(Ordering::SeqCst);
        write_mmio32(
            self.db_base + (u64::from(slot_id) * 4),
            u32::from(endpoint_id),
        );
    }

    fn port_base(&self, port: u8) -> u64 {
        self.op_base + PORT_REGS_BASE + (u64::from(port) - 1) * PORT_REGS_STRIDE
    }

    fn read_portsc(&self, port: u8) -> u32 {
        read_mmio32(self.port_base(port) + PORTSC)
    }

    fn power_port_if_needed(&self, port: u8) -> u32 {
        let address = self.port_base(port) + PORTSC;
        let portsc = read_mmio32(address);
        if portsc & PORTSC_PP != 0 {
            return portsc;
        }

        let ack = PORTSC_CSC | PORTSC_PEC | PORTSC_WRC | PORTSC_PRC | PORTSC_PLC | PORTSC_CEC;
        write_portsc(address, portsc, PORTSC_PP, ack);
        fence(Ordering::SeqCst);
        delay_ms(20);
        let powered = read_mmio32(address);
        if powered & PORTSC_PP != 0 {
            log_portsc("powered", port, powered);
        }
        powered
    }
}

fn map_xhci_mmio(mmio_phys: u64, length: usize) -> Result<u64, &'static str> {
    let virtual_address = memory::map_mmio(PhysAddr::new(mmio_phys), length)
        .map_err(|_| "xHCI MMIO mapping failed")?;
    trace::record_mmio_map(
        BreadcrumbTag::XhciMmioMap,
        mmio_phys,
        virtual_address.as_u64(),
        length as u64,
    );
    crate::serial_println!(
        "[xHCI] mmio map phys=0x{:X} virt=0x{:X} len=0x{:X}",
        mmio_phys,
        virtual_address.as_u64(),
        length
    );
    Ok(virtual_address.as_u64())
}

fn required_mmio_span(
    cap_length: u64,
    db_offset: u64,
    rt_offset: u64,
    max_slots: u8,
    max_ports: u8,
) -> usize {
    let operational_end = cap_length + 0x40;
    let runtime_end = rt_offset + 0x20 + ERDP + 8;
    let doorbell_end = db_offset + (u64::from(max_slots) + 1) * 4;
    let ports_end = cap_length + PORT_REGS_BASE + u64::from(max_ports) * PORT_REGS_STRIDE + 4;
    operational_end
        .max(runtime_end)
        .max(doorbell_end)
        .max(ports_end)
        .max(INITIAL_MMIO_MAP_SIZE as u64) as usize
}

fn current_time_ms() -> u64 {
    let ticks = interrupts::tick_count();
    let elapsed = pit::elapsed_millis(ticks);
    if elapsed == 0 {
        ticks.saturating_mul(1_000) / u64::from(pit::PIT_FREQUENCY_HZ)
    } else {
        elapsed
    }
}

pub fn runtime_catch_up_budget_ms() -> u64 {
    RUNTIME_TOPOLOGY_CATCH_UP_BUDGET_MS
}

fn delay_ms(duration_ms: u64) {
    if duration_ms == 0 {
        return;
    }
    let deadline = current_time_ms().saturating_add(duration_ms);
    while current_time_ms() < deadline {
        spin_loop();
    }
}

fn default_max_packet_size0(speed: UsbSpeed) -> u16 {
    match speed {
        UsbSpeed::Low | UsbSpeed::Full => 8,
        UsbSpeed::High => 64,
        UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2 => 512,
    }
}

fn actual_max_packet_size0(speed: UsbSpeed, raw: u8) -> u16 {
    match speed {
        UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2 => 1u16 << raw,
        _ => u16::from(raw),
    }
}

fn decode_speed(bits: u8) -> UsbSpeed {
    match bits {
        2 => UsbSpeed::Low,
        1 => UsbSpeed::Full,
        3 => UsbSpeed::High,
        4 => UsbSpeed::Super,
        5 => UsbSpeed::SuperPlus,
        _ => UsbSpeed::Full,
    }
}

fn speed_code(speed: UsbSpeed) -> u32 {
    match speed {
        UsbSpeed::Full => 1,
        UsbSpeed::Low => 2,
        UsbSpeed::High => 3,
        UsbSpeed::Super => 4,
        UsbSpeed::SuperPlus => 5,
        UsbSpeed::Super2x2 => 6,
    }
}

fn log_portsc(stage: &str, port: u8, portsc: u32) {
    crate::serial_println!(
        "[xHCI] port {} {} portsc=0x{:08X} ccs={} ped={} pr={} link={} chg=0x{:08X} speed={:?}",
        port,
        stage,
        portsc,
        portsc & PORTSC_CCS != 0,
        portsc & PORTSC_PED != 0,
        portsc & PORTSC_PR != 0,
        port_link_state_name(portsc),
        portsc & PORTSC_CHANGE_BITS,
        decode_speed(((portsc >> 10) & 0xF) as u8)
    );
}

fn port_link_state_name(portsc: u32) -> &'static str {
    let code = (portsc & PORTSC_PLS_MASK) >> 5;
    let speed = decode_speed(((portsc >> 10) & 0xF) as u8);
    if matches!(speed, UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2) {
        return match code {
            0 => "u0",
            1 => "u1",
            2 => "u2",
            3 => "u3",
            4 => "disabled",
            5 => "rx-detect",
            6 => "inactive",
            7 => "polling",
            8 => "recovery",
            9 => "hot-reset",
            10 => "compliance",
            11 => "test",
            15 => "resume",
            _ => "reserved",
        };
    }

    match code {
        0 => "usb2-u0",
        1 => "usb2-pls-1",
        2 => "usb2-pls-2",
        3 => "usb2-pls-3",
        4 => "usb2-disabled",
        5 => "usb2-pls-5",
        6 => "usb2-pls-6",
        7 => "usb2-pls-7",
        8 => "usb2-pls-8",
        9 => "usb2-hot-reset",
        10 => "usb2-pls-10",
        11 => "usb2-test",
        12 => "usb2-pls-12",
        13 => "usb2-pls-13",
        14 => "usb2-pls-14",
        15 => "usb2-resume",
        _ => "usb2-reserved",
    }
}

fn scratchpad_buffer_count(hcs_params2: u32) -> u16 {
    let low = (hcs_params2 & 0x1F) as u16;
    let high = ((hcs_params2 >> 27) & 0x1F) as u16;
    (high << 5) | low
}

fn allocate_scratchpads(
    count: usize,
) -> Result<(Option<DmaRegion>, Vec<DmaRegion>), &'static str> {
    if count == 0 {
        return Ok((None, Vec::new()));
    }

    let mut array = DmaRegion::allocate(count * size_of::<u64>())
        .map_err(|_| "xHCI scratchpad array allocation failed")?;
    let entries = unsafe {
        slice::from_raw_parts_mut(array.as_mut_ptr().cast::<u64>(), array.len() / size_of::<u64>())
    };
    entries.fill(0);

    let mut buffers = Vec::with_capacity(count);
    for index in 0..count {
        let buffer =
            DmaRegion::allocate(4096).map_err(|_| "xHCI scratchpad buffer allocation failed")?;
        entries[index] = buffer.physical().as_u64();
        buffers.push(buffer);
    }
    fence(Ordering::SeqCst);
    Ok((Some(array), buffers))
}

fn endpoint_id_from_address(address: u8) -> u8 {
    let endpoint_number = address & 0x0F;
    if endpoint_number == 0 {
        1
    } else if address & 0x80 != 0 {
        endpoint_number * 2 + 1
    } else {
        endpoint_number * 2
    }
}

fn endpoint_type(endpoint: &UsbEndpoint) -> u32 {
    match (endpoint.transfer_type, endpoint.direction) {
        (TransferType::Bulk, EndpointDirection::Out) => EP_TYPE_BULK_OUT,
        (TransferType::Bulk, EndpointDirection::In) => EP_TYPE_BULK_IN,
        (TransferType::Interrupt, EndpointDirection::In) => EP_TYPE_INTERRUPT_IN,
        (TransferType::Interrupt, EndpointDirection::Out) => EP_TYPE_INTERRUPT_OUT,
        _ => EP_TYPE_CONTROL,
    }
}

fn endpoint_interval(speed: UsbSpeed, b_interval: u8) -> u32 {
    match speed {
        UsbSpeed::High | UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2 => {
            u32::from(b_interval.saturating_sub(1))
        }
        UsbSpeed::Low | UsbSpeed::Full => {
            let mut value = u32::from(b_interval.max(1)) * 8;
            let mut shift = 0u32;
            while value > 1 {
                value >>= 1;
                shift += 1;
            }
            shift.clamp(3, 10)
        }
    }
}

fn extract_rndis_frame(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() < RNDIS_PACKET_HEADER_SIZE {
        return None;
    }
    let msg_type = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    if msg_type != RNDIS_MSG_PACKET {
        return None;
    }
    let data_offset = u32::from_le_bytes([raw[8], raw[9], raw[10], raw[11]]) as usize;
    let data_length = u32::from_le_bytes([raw[12], raw[13], raw[14], raw[15]]) as usize;
    // DataOffset is relative to byte 8 (start of DataOffset field)
    let start = 8 + data_offset;
    let end = start + data_length;
    if end > raw.len() || data_length == 0 {
        return None;
    }
    Some(raw[start..end].to_vec())
}

fn write_non_control_endpoint_context(
    input_context: &mut DmaRegion,
    offset: usize,
    endpoint: &UsbEndpoint,
    speed: UsbSpeed,
    ep_type: u32,
    ring_phys: u64,
) {
    let interval = endpoint_interval(speed, endpoint.interval);
    let max_packet = u32::from(endpoint.max_packet_size);
    write_ctx32(input_context, offset, interval << 16);
    write_ctx32(
        input_context,
        offset + 4,
        (3 << 1) | (ep_type << 3) | (max_packet << 16),
    );
    write_ctx64(input_context, offset + 8, ring_phys | 1);
    write_ctx32(input_context, offset + 16, max_packet << 16);
}

fn input_ep_context_offset(context_size: usize, endpoint_id: u8) -> usize {
    (usize::from(endpoint_id) + 1) * context_size
}

fn output_ep_context_offset(context_size: usize, endpoint_id: u8) -> usize {
    usize::from(endpoint_id) * context_size
}

fn read_ctx32(region: &DmaRegion, offset: usize) -> u32 {
    let bytes = region.slice();
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

fn read_ctx64(region: &DmaRegion, offset: usize) -> u64 {
    let bytes = region.slice();
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap_or([0; 8]))
}

fn write_ctx32(region: &mut DmaRegion, offset: usize, value: u32) {
    let bytes = region.slice_mut();
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_ctx64(region: &mut DmaRegion, offset: usize, value: u64) {
    let bytes = region.slice_mut();
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_mmio8(address: u64) -> u8 {
    unsafe { read_volatile(address as *const u8) }
}

fn read_mmio32(address: u64) -> u32 {
    unsafe { read_volatile(address as *const u32) }
}

fn write_mmio32(address: u64, value: u32) {
    unsafe {
        write_volatile(address as *mut u32, value);
    }
}

fn write_portsc(address: u64, current: u32, set_bits: u32, ack_bits: u32) {
    let value = (current & PORTSC_PRESERVE_WRITE_BITS) | set_bits | (ack_bits & PORTSC_CHANGE_BITS);
    write_mmio32(address, value);
}

fn write_mmio64(address: u64, value: u64) {
    unsafe {
        write_volatile(address as *mut u64, value);
    }
}

fn trb_type_name(trb_type: u8) -> &'static str {
    match trb_type {
        TRB_NORMAL => "Normal",
        TRB_SETUP_STAGE => "Setup Stage",
        TRB_DATA_STAGE => "Data Stage",
        TRB_STATUS_STAGE => "Status Stage",
        TRB_LINK => "Link",
        TRB_ENABLE_SLOT => "Enable Slot",
        TRB_DISABLE_SLOT => "Disable Slot",
        TRB_ADDRESS_DEVICE => "Address Device",
        TRB_CONFIGURE_ENDPOINT => "Configure Endpoint",
        TRB_EVALUATE_CONTEXT => "Evaluate Context",
        TRB_TRANSFER_EVENT => "Transfer Event",
        TRB_COMMAND_COMPLETION => "Command Completion",
        TRB_PORT_STATUS_CHANGE => "Port Status Change",
        _ => "Unknown",
    }
}

fn summarize_interface(interface: &UsbInterface) -> UsbInterfaceStatus {
    UsbInterfaceStatus {
        number: interface.number,
        alternate_setting: interface.alternate_setting,
        class: interface.class,
        subclass: interface.subclass,
        protocol: interface.protocol,
        endpoint_count: interface.endpoints.len().min(u8::MAX as usize) as u8,
        has_bulk_in: interface.has_bulk_in(),
        has_bulk_out: interface.has_bulk_out(),
        has_interrupt_in: interface.has_interrupt_in(),
        has_interrupt_out: interface.has_interrupt_out(),
    }
}

fn classify_port_visibility(
    category: DeviceCategory,
    interfaces: &[UsbInterface],
    network_info: Option<UsbNetworkInfo>,
) -> UsbPortVisibility {
    if network_info.is_some() {
        return UsbPortVisibility::NetworkReady;
    }
    if interfaces.iter().any(UsbInterface::is_network)
        || interfaces.iter().any(UsbInterface::is_cdc_data)
    {
        return UsbPortVisibility::NetworkCandidate;
    }
    if interfaces.iter().any(UsbInterface::is_adb) {
        return UsbPortVisibility::PhoneAdb;
    }
    if interfaces.iter().any(UsbInterface::is_ptp_mtp_like) {
        return UsbPortVisibility::PhoneMedia;
    }
    if interfaces.iter().any(UsbInterface::is_vendor_specific) {
        return UsbPortVisibility::VendorSpecific;
    }
    match category {
        DeviceCategory::Input => UsbPortVisibility::Input,
        DeviceCategory::Storage => UsbPortVisibility::Storage,
        _ => UsbPortVisibility::Generic,
    }
}

fn infer_attach_state(
    interfaces: &[UsbInterface],
    network_info: Option<UsbNetworkInfo>,
    probe_error: Option<&'static str>,
) -> (UsbAttachState, &'static str) {
    if network_info.is_some() {
        return (
            UsbAttachState::Ready,
            "supported USB network interface detected; manual attach is available",
        );
    }
    if let Some(error) = probe_error {
        let state = if interfaces.iter().any(UsbInterface::is_cdc_eem) {
            UsbAttachState::Unsupported
        } else {
            UsbAttachState::Candidate
        };
        return (state, error);
    }
    if interfaces.iter().any(UsbInterface::is_cdc_eem) {
        return (
            UsbAttachState::Unsupported,
            "CDC-EEM interface detected, but attach is not implemented",
        );
    }
    if interfaces.iter().any(UsbInterface::is_network) {
        return (
            UsbAttachState::Candidate,
            "supported USB network control interfaces are visible, but the data layout is incomplete",
        );
    }
    if interfaces.iter().any(UsbInterface::is_cdc_data) {
        return (
            UsbAttachState::Candidate,
            "CDC data interface is visible without a matching supported control interface",
        );
    }
    if interfaces.iter().any(UsbInterface::is_adb) {
        return (
            UsbAttachState::Unsupported,
            "phone/ADB interface detected, but no USB network interface is exposed",
        );
    }
    if interfaces.iter().any(UsbInterface::is_ptp_mtp_like) {
        return (
            UsbAttachState::Unsupported,
            "phone/media USB interface detected, but no tethering network interface is exposed",
        );
    }
    if interfaces.iter().any(UsbInterface::is_vendor_specific) {
        return (
            UsbAttachState::Unsupported,
            "vendor-specific USB interfaces detected; no supported network layout recognized",
        );
    }
    (
        UsbAttachState::NotApplicable,
        "no supported USB network interfaces visible",
    )
}

fn completion_code_name(code: u8) -> &'static str {
    match code {
        0 => "Invalid",
        1 => "Success",
        2 => "Data Buffer Error",
        3 => "Babble Detected Error",
        4 => "USB Transaction Error",
        5 => "TRB Error",
        6 => "Stall Error",
        7 => "Resource Error",
        8 => "Bandwidth Error",
        9 => "No Slots Available Error",
        10 => "Invalid Stream Type Error",
        11 => "Slot Not Enabled Error",
        12 => "Endpoint Not Enabled Error",
        13 => "Short Packet",
        14 => "Ring Underrun",
        15 => "Ring Overrun",
        16 => "VF Event Ring Full Error",
        17 => "Parameter Error",
        18 => "Bandwidth Overrun Error",
        19 => "Context State Error",
        20 => "No Ping Response Error",
        21 => "Event Ring Full Error",
        22 => "Incompatible Device Error",
        23 => "Missed Service Error",
        24 => "Command Ring Stopped",
        25 => "Command Aborted",
        26 => "Stopped",
        27 => "Stopped - Length Invalid",
        28 => "Stopped - Short Packet",
        29 => "Max Exit Latency Too Large Error",
        31 => "Isoch Buffer Overrun",
        32 => "Event Lost Error",
        33 => "Undefined Error",
        34 => "Invalid Stream ID Error",
        35 => "Secondary Bandwidth Error",
        36 => "Split Transaction Error",
        _ => "Unknown",
    }
}

fn endpoint_state_name(state: u8) -> &'static str {
    match state {
        0 => "Disabled",
        1 => "Running",
        2 => "Halted",
        3 => "Stopped",
        4 => "Error",
        _ => "Unknown",
    }
}

fn slot_state_name(state: u8) -> &'static str {
    match state {
        0 => "Disabled/Reserved",
        1 => "Default",
        2 => "Addressed",
        3 => "Configured",
        _ => "Unknown",
    }
}

