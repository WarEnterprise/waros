use alloc::vec::Vec;

use super::ethernet::{EthernetFrame, ETHERTYPE_ARP};
use super::ipv4::Ipv4Addr;

const ARP_HEADER_LEN: usize = 28;
const ARP_ETHERNET: u16 = 1;
const ARP_IPV4: u16 = 0x0800;
const ARP_REQUEST: u16 = 1;
const ARP_REPLY: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArpEntry {
    pub ip: Ipv4Addr,
    pub mac: [u8; 6],
    pub timestamp_ms: u64,
}

#[derive(Debug, Default, Clone)]
pub struct ArpCache {
    entries: Vec<ArpEntry>,
}

impl ArpCache {
    #[must_use]
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    #[must_use]
    pub fn entries(&self) -> &[ArpEntry] {
        &self.entries
    }

    #[must_use]
    pub fn lookup(&self, ip: Ipv4Addr) -> Option<[u8; 6]> {
        self.entries.iter().find(|entry| entry.ip == ip).map(|entry| entry.mac)
    }

    pub fn insert(&mut self, ip: Ipv4Addr, mac: [u8; 6], timestamp_ms: u64) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.ip == ip) {
            entry.mac = mac;
            entry.timestamp_ms = timestamp_ms;
            return;
        }
        self.entries.push(ArpEntry {
            ip,
            mac,
            timestamp_ms,
        });
    }

    pub fn observe_frame(&mut self, frame: &[u8], timestamp_ms: u64) {
        let Ok(ethernet) = EthernetFrame::parse(frame) else {
            return;
        };
        if ethernet.ethertype() != ETHERTYPE_ARP || ethernet.payload.len() < ARP_HEADER_LEN {
            return;
        }

        let payload = &ethernet.payload;
        let hardware_type = u16::from_be_bytes([payload[0], payload[1]]);
        let protocol_type = u16::from_be_bytes([payload[2], payload[3]]);
        let operation = u16::from_be_bytes([payload[6], payload[7]]);
        if hardware_type != ARP_ETHERNET || protocol_type != ARP_IPV4 {
            return;
        }
        if operation != ARP_REQUEST && operation != ARP_REPLY {
            return;
        }

        let mut sender_mac = [0u8; 6];
        sender_mac.copy_from_slice(&payload[8..14]);
        let mut sender_ip = [0u8; 4];
        sender_ip.copy_from_slice(&payload[14..18]);
        self.insert(Ipv4Addr(sender_ip), sender_mac, timestamp_ms);
    }
}

#[must_use]
pub fn build_request_frame(sender_mac: [u8; 6], sender_ip: Ipv4Addr, target_ip: Ipv4Addr) -> Vec<u8> {
    let mut payload = Vec::with_capacity(ARP_HEADER_LEN);
    payload.extend_from_slice(&ARP_ETHERNET.to_be_bytes());
    payload.extend_from_slice(&ARP_IPV4.to_be_bytes());
    payload.push(6);
    payload.push(4);
    payload.extend_from_slice(&ARP_REQUEST.to_be_bytes());
    payload.extend_from_slice(&sender_mac);
    payload.extend_from_slice(&sender_ip.0);
    payload.extend_from_slice(&[0u8; 6]);
    payload.extend_from_slice(&target_ip.0);
    EthernetFrame::new([0xFF; 6], sender_mac, ETHERTYPE_ARP, payload).serialize()
}
