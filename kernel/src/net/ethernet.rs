use alloc::vec::Vec;

use super::NetError;

pub const ETH_ALEN: usize = 6;
pub const ETH_HEADER_LEN: usize = 14;
#[allow(dead_code)]
pub const ETHERTYPE_IPV4: u16 = 0x0800;
#[allow(dead_code)]
pub const ETHERTYPE_ARP: u16 = 0x0806;

#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EthernetHeader {
    pub dst_mac: [u8; ETH_ALEN],
    pub src_mac: [u8; ETH_ALEN],
    pub ethertype: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EthernetFrame {
    pub header: EthernetHeader,
    pub payload: Vec<u8>,
}

impl EthernetFrame {
    #[must_use]
    pub fn new(dst_mac: [u8; ETH_ALEN], src_mac: [u8; ETH_ALEN], ethertype: u16, payload: Vec<u8>) -> Self {
        Self {
            header: EthernetHeader {
                dst_mac,
                src_mac,
                ethertype: ethertype.to_be(),
            },
            payload,
        }
    }

    pub fn parse(data: &[u8]) -> Result<Self, NetError> {
        if data.len() < ETH_HEADER_LEN {
            return Err(NetError::FrameTooShort);
        }

        let mut dst_mac = [0u8; ETH_ALEN];
        dst_mac.copy_from_slice(&data[..ETH_ALEN]);
        let mut src_mac = [0u8; ETH_ALEN];
        src_mac.copy_from_slice(&data[ETH_ALEN..(2 * ETH_ALEN)]);
        let ethertype = u16::from_be_bytes([data[12], data[13]]);

        Ok(Self {
            header: EthernetHeader {
                dst_mac,
                src_mac,
                ethertype: ethertype.to_be(),
            },
            payload: data[ETH_HEADER_LEN..].to_vec(),
        })
    }

    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut frame = Vec::with_capacity(ETH_HEADER_LEN + self.payload.len());
        frame.extend_from_slice(&self.header.dst_mac);
        frame.extend_from_slice(&self.header.src_mac);
        frame.extend_from_slice(&self.ethertype().to_be_bytes());
        frame.extend_from_slice(&self.payload);
        frame
    }

    #[must_use]
    pub fn ethertype(&self) -> u16 {
        u16::from_be(self.header.ethertype)
    }

    #[must_use]
    pub fn src_mac(&self) -> [u8; ETH_ALEN] {
        self.header.src_mac
    }

    #[must_use]
    pub fn dst_mac(&self) -> [u8; ETH_ALEN] {
        self.header.dst_mac
    }
}
