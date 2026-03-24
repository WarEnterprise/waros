#![allow(dead_code)]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::hint::spin_loop;
use core::mem::size_of;
use core::ptr::{read_volatile, write_volatile};
use core::slice;

use x86_64::PhysAddr;

use crate::memory;
use crate::net::buffer::DmaRegion;
use crate::net::pci::{self, PciBar, PciDevice};

use super::descriptors::{self, EndpointDirection, TransferType, UsbEndpoint, UsbInterface};
use super::hid::{self, HidKind, KeyboardBootState};
use super::mass_storage::{self, Cbw, Csw, UsbMassStorageInfo};
use super::super::device::{DeviceCategory, UsbSpeed};

const CAPLENGTH: u64 = 0x00;
const HCSPARAMS1: u64 = 0x04;
const HCCPARAMS1: u64 = 0x10;
const DBOFF: u64 = 0x14;
const RTSOFF: u64 = 0x18;

const USBCMD: u64 = 0x00;
const USBSTS: u64 = 0x04;
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
const TRB_ADDRESS_DEVICE: u8 = 11;
const TRB_CONFIGURE_ENDPOINT: u8 = 12;
const TRB_EVALUATE_CONTEXT: u8 = 13;
const TRB_TRANSFER_EVENT: u8 = 32;
const TRB_COMMAND_COMPLETION: u8 = 33;
const TRB_PORT_STATUS_CHANGE: u8 = 34;

const PORTSC_CCS: u32 = 1 << 0;
const PORTSC_PED: u32 = 1 << 1;
const PORTSC_PR: u32 = 1 << 4;
const PORTSC_CSC: u32 = 1 << 17;
const PORTSC_PRC: u32 = 1 << 21;

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

#[derive(Clone)]
pub struct UsbPortStatus {
    pub port: u8,
    pub connected: bool,
    pub enabled: bool,
    pub speed: UsbSpeed,
    pub slot_id: Option<u8>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub category: DeviceCategory,
    pub driver: &'static str,
    pub name: String,
    pub configured: bool,
    pub hid_kind: Option<HidKind>,
    pub storage: Option<UsbMassStorageInfo>,
}

struct HidEndpoint {
    endpoint_address: u8,
    endpoint_id: u8,
    ring: XhciRing,
    report_buffer: DmaRegion,
    report_size: usize,
    in_flight_trb: Option<u64>,
    hid_kind: HidKind,
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

struct UsbSlotState {
    port: u8,
    speed: UsbSpeed,
    input_context: DmaRegion,
    output_context: DmaRegion,
    ep0_ring: XhciRing,
    max_packet_size0: u16,
    hid: Option<HidEndpoint>,
    storage: Option<StorageEndpoint>,
}

pub struct XhciController {
    pub pci: PciDevice,
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
    slots: Vec<Option<UsbSlotState>>,
    pub ports: Vec<UsbPortStatus>,
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
        let mut region =
            DmaRegion::allocate(size * size_of::<Trb>()).map_err(|_| "xHCI ring allocation failed")?;
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
        let mmio_base = memory::phys_to_virt(PhysAddr::new(mmio_phys))
            .ok_or("xHCI MMIO mapping missing")?
            .as_u64();

        let cap_length = read_mmio8(mmio_base + CAPLENGTH) as u64;
        let hcs_params1 = read_mmio32(mmio_base + HCSPARAMS1);
        let hcc_params1 = read_mmio32(mmio_base + HCCPARAMS1);
        let db_offset = read_mmio32(mmio_base + DBOFF);
        let rt_offset = read_mmio32(mmio_base + RTSOFF);

        let op_base = mmio_base + cap_length;
        let rt_base = mmio_base + (rt_offset as u64);
        let db_base = mmio_base + (db_offset as u64);
        let max_slots = (hcs_params1 & 0xFF) as u8;
        let max_ports = ((hcs_params1 >> 24) & 0xFF) as u8;
        let context_size = if hcc_params1 & (1 << 2) != 0 { 64 } else { 32 };

        let command_ring = XhciRing::new(256, true)?;
        let event_ring = XhciRing::new(256, false)?;
        let dcbaa =
            DmaRegion::allocate(256 * size_of::<u64>()).map_err(|_| "xHCI DCBAA allocation failed")?;
        let erst = DmaRegion::allocate(16).map_err(|_| "xHCI ERST allocation failed")?;

        let mut slots = Vec::new();
        slots.resize_with(max_slots as usize + 1, || None);

        let mut controller = Self {
            pci,
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
            slots,
            ports: Vec::new(),
        };

        crate::serial_println!(
            "[xHCI] controller {:02X}:{:02X}.{} mmio=0x{:X} slots={} ports={}",
            controller.pci.bus,
            controller.pci.device,
            controller.pci.function,
            controller.mmio_phys,
            controller.max_slots,
            controller.max_ports
        );
        controller.reset()?;
        controller.configure_runtime()?;
        controller.start()?;
        controller.scan_ports();
        Ok(controller)
    }

    pub fn scan_ports(&mut self) {
        self.ports.clear();

        for index in 0..self.max_ports {
            let port = index + 1;
            let portsc = self.read_portsc(port);
            if portsc & PORTSC_CCS == 0 {
                continue;
            }

            let speed = decode_speed(((portsc >> 10) & 0xF) as u8);
            crate::serial_println!("[xHCI] port {} connected speed={:?}", port, speed);
            let mut status = UsbPortStatus {
                port,
                connected: true,
                enabled: false,
                speed,
                slot_id: None,
                vendor_id: None,
                product_id: None,
                category: DeviceCategory::UsbDevice,
                driver: "xhci-port",
                name: format!("USB device on port {} ({:?})", port, speed),
                configured: false,
                hid_kind: None,
                storage: None,
            };

            let _ = self.reset_port(port, speed);
            status.enabled = self.read_portsc(port) & PORTSC_PED != 0;

            if let Ok(slot_id) = self.enable_slot() {
                status.slot_id = Some(slot_id);
                if let Ok(enumerated) = self.enumerate_device(port, speed, slot_id) {
                    crate::serial_println!(
                        "[xHCI] port {} slot {} -> {} ({:04X}:{:04X}) driver={}",
                        enumerated.port,
                        enumerated.slot_id.unwrap_or(0),
                        enumerated.name,
                        enumerated.vendor_id.unwrap_or(0),
                        enumerated.product_id.unwrap_or(0),
                        enumerated.driver
                    );
                    status = enumerated;
                }
            }

            self.ports.push(status);
        }
    }

    pub fn poll(&mut self) {
        while let Some(event) = self.next_event() {
            match event.trb_type() {
                TRB_TRANSFER_EVENT => self.handle_transfer_event(event),
                TRB_PORT_STATUS_CHANGE => self.scan_ports(),
                _ => {}
            }
        }
    }

    fn enumerate_device(
        &mut self,
        port: u8,
        speed: UsbSpeed,
        slot_id: u8,
    ) -> Result<UsbPortStatus, &'static str> {
        self.allocate_slot(slot_id, port, speed)?;
        self.address_device(slot_id)?;

        let header = self.read_descriptor(slot_id, 0x01, 0, 0, 8)?;
        if header.len() < 8 {
            return Err("xHCI short device descriptor header");
        }

        let max_packet_size0 = actual_max_packet_size0(speed, header[7]);
        if max_packet_size0 != 0 && max_packet_size0 != self.slot(slot_id)?.max_packet_size0 {
            self.evaluate_ep0(slot_id, max_packet_size0)?;
            self.slot_mut(slot_id)?.max_packet_size0 = max_packet_size0;
        }

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
        let mut hid_kind = None;
        let mut storage_info = None;
        let mut name = format!("USB {:04X}:{:04X}", vendor_id, product_id);

        if configuration_count > 0 {
            let config_header = self.read_descriptor(slot_id, 0x02, 0, 0, 9)?;
            if config_header.len() >= 9 {
                let total_length =
                    u16::from_le_bytes([config_header[2], config_header[3]]) as usize;
                let config_blob = self.read_descriptor(slot_id, 0x02, 0, 0, total_length)?;
                if let Ok(configuration) = descriptors::parse_configuration_descriptors(&config_blob) {
                    category = descriptors::classify_device(device_class, &configuration.interfaces);
                    configured =
                        self.set_configuration(slot_id, configuration.configuration_value).is_ok();
                    if configured {
                        if let Ok(kind) =
                            self.configure_hid(slot_id, speed, &configuration.interfaces)
                        {
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
                        } else if category == DeviceCategory::Storage {
                            if let Ok(info) =
                                self.configure_mass_storage(slot_id, speed, &configuration.interfaces)
                            {
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
                                name =
                                    format!("USB Storage {:04X}:{:04X}", vendor_id, product_id);
                            }
                        }
                    }
                }
            }
        }

        Ok(UsbPortStatus {
            port,
            connected: true,
            enabled: self.read_portsc(port) & PORTSC_PED != 0,
            speed,
            slot_id: Some(slot_id),
            vendor_id: Some(vendor_id),
            product_id: Some(product_id),
            category,
            driver,
            name,
            configured,
            hid_kind,
            storage: storage_info,
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

            self.set_boot_protocol(slot_id, interface.number)?;
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

    fn configure_interrupt_in(
        &mut self,
        slot_id: u8,
        speed: UsbSpeed,
        endpoint: &UsbEndpoint,
        hid_kind: HidKind,
    ) -> Result<(), &'static str> {
        let endpoint_id = endpoint_id_from_address(endpoint.address);
        let report_size = match hid_kind {
            HidKind::Keyboard => 8usize,
            HidKind::Mouse => 4usize,
            HidKind::Combined | HidKind::Unknown => usize::from(endpoint.max_packet_size.max(8)),
        };
        let ring = XhciRing::new(32, true)?;
        let report_buffer =
            DmaRegion::allocate(report_size.max(8)).map_err(|_| "xHCI HID buffer allocation failed")?;
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

        self.ring_doorbell(slot_id, endpoint_id);
        Ok(())
    }

    fn handle_transfer_event(&mut self, event: Trb) {
        let slot_id = event.slot_id();
        let endpoint_id = event.endpoint_id();
        if endpoint_id == 1 {
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

            if event.completion_code() == COMPLETION_SUCCESS {
                let actual = hid
                    .report_size
                    .saturating_sub((event.transfer_residue() as usize).min(hid.report_size));
                let data = &hid.report_buffer.slice()[..actual];
                match hid.hid_kind {
                    HidKind::Keyboard => {
                        let _ = hid::process_keyboard_boot_report(&mut hid.keyboard_state, data);
                    }
                    HidKind::Mouse => {
                        let _ = hid::process_mouse_boot_report(data);
                    }
                    HidKind::Combined | HidKind::Unknown => {}
                }
            }

            hid.in_flight_trb = None;
            should_rearm = true;
        }

        if should_rearm {
            let _ = self.arm_hid_endpoint(slot_id);
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
        let mut region =
            DmaRegion::allocate(data.len().max(1)).map_err(|_| "xHCI bulk OUT allocation failed")?;
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
        self.slots[slot_id as usize] = Some(UsbSlotState {
            port,
            speed,
            input_context,
            output_context,
            ep0_ring,
            max_packet_size0,
            hid: None,
            storage: None,
        });
        Ok(())
    }

    fn address_device(&mut self, slot_id: u8) -> Result<(), &'static str> {
        let input_phys = {
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
            write_ctx64(&mut slot.input_context, ep0_offset + 8, slot.ep0_ring.phys_addr | 1);
            write_ctx32(&mut slot.input_context, ep0_offset + 16, 8);

            slot.input_context.physical().as_u64()
        };

        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_ADDRESS_DEVICE as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Address Device failed");
        }
        Ok(())
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
            write_ctx64(&mut slot.input_context, ep0_offset + 8, slot.ep0_ring.phys_addr | 1);
            write_ctx32(&mut slot.input_context, ep0_offset + 16, 8);
            slot.input_context.physical().as_u64()
        };

        let command = Trb {
            parameter: input_phys,
            status: 0,
            control: ((TRB_EVALUATE_CONTEXT as u32) << 10) | ((slot_id as u32) << 24),
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
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
        let mut buffer =
            DmaRegion::allocate(length.max(1)).map_err(|_| "xHCI descriptor buffer allocation failed")?;
        let setup = SetupPacket {
            request_type: 0x80,
            request: 0x06,
            value: (u16::from(descriptor_type) << 8) | u16::from(descriptor_index),
            index: language_id,
            length: length as u16,
        };
        self.control_transfer(slot_id, setup, Some(&mut buffer), None)?;
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
        if event.completion_code() != COMPLETION_SUCCESS {
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

    fn reset_port(&mut self, port: u8, speed: UsbSpeed) -> Result<(), &'static str> {
        if matches!(speed, UsbSpeed::Super | UsbSpeed::SuperPlus | UsbSpeed::Super2x2) {
            return Ok(());
        }

        let address = self.port_base(port) + PORTSC;
        let portsc = read_mmio32(address);
        let write_mask = PORTSC_CSC | PORTSC_PRC;
        write_mmio32(address, (portsc & !write_mask) | PORTSC_PR | write_mask);

        for _ in 0..2_000_000 {
            let value = read_mmio32(address);
            if value & PORTSC_PR == 0 {
                if value & PORTSC_PRC != 0 {
                    write_mmio32(address, value | PORTSC_PRC | PORTSC_CSC);
                }
                return Ok(());
            }
            spin_loop();
        }

        Err("xHCI port reset timed out")
    }

    fn enable_slot(&mut self) -> Result<u8, &'static str> {
        let command = Trb {
            parameter: 0,
            status: 0,
            control: (TRB_ENABLE_SLOT as u32) << 10,
        };
        let event = self.submit_command(command)?;
        if event.completion_code() != COMPLETION_SUCCESS {
            return Err("xHCI Enable Slot failed");
        }
        Ok(event.slot_id())
    }

    fn submit_command(&mut self, trb: Trb) -> Result<Trb, &'static str> {
        self.command_ring.enqueue(trb)?;
        write_mmio32(self.db_base, 0);

        for _ in 0..2_000_000 {
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_COMMAND_COMPLETION {
                    return Ok(event);
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
        for _ in 0..2_000_000 {
            if let Some(event) = self.next_event() {
                if event.trb_type() == TRB_TRANSFER_EVENT
                    && event.slot_id() == slot_id
                    && event.endpoint_id() == endpoint_id
                    && event.parameter == trb_pointer
                {
                    return Ok(event);
                }
            }
            spin_loop();
        }

        Err("xHCI transfer timed out")
    }

    fn next_event(&mut self) -> Option<Trb> {
        let event = self.event_ring.dequeue()?;
        let ir0_base = self.rt_base + 0x20;
        write_mmio64(ir0_base + ERDP, self.event_ring.dequeue_pointer() | (1 << 3));
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

    fn ring_doorbell(&self, slot_id: u8, endpoint_id: u8) {
        write_mmio32(self.db_base + (u64::from(slot_id) * 4), u32::from(endpoint_id));
    }

    fn port_base(&self, port: u8) -> u64 {
        self.op_base + PORT_REGS_BASE + (u64::from(port) - 1) * PORT_REGS_STRIDE
    }

    fn read_portsc(&self, port: u8) -> u32 {
        read_mmio32(self.port_base(port) + PORTSC)
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

fn write_mmio64(address: u64, value: u64) {
    unsafe {
        write_volatile(address as *mut u64, value);
    }
}
