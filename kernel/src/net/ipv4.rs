use alloc::string::String;
use core::fmt;

use smoltcp::wire::{IpAddress, IpCidr, Ipv4Address, Ipv4Cidr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    pub const ZERO: Self = Self([0, 0, 0, 0]);
    pub const BROADCAST: Self = Self([255, 255, 255, 255]);

    #[must_use]
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Self([a, b, c, d])
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let mut octets = [0u8; 4];
        let mut parts = value.split('.');
        for octet in &mut octets {
            *octet = parts.next()?.parse().ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Self(octets))
    }

    #[must_use]
    pub fn as_smoltcp(self) -> Ipv4Address {
        Ipv4Address::new(self.0[0], self.0[1], self.0[2], self.0[3])
    }

    #[must_use]
    pub fn from_smoltcp(address: Ipv4Address) -> Self {
        Self(address.0)
    }

    #[must_use]
    pub fn to_ip_address(self) -> IpAddress {
        IpAddress::Ipv4(self.as_smoltcp())
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}",
            self.0[0], self.0[1], self.0[2], self.0[3]
        )
    }
}

#[must_use]
pub fn ip_cidr(address: Ipv4Addr, prefix_len: u8) -> IpCidr {
    IpCidr::Ipv4(Ipv4Cidr::new(address.as_smoltcp(), prefix_len))
}

#[must_use]
pub fn prefix_len_from_mask(mask: Ipv4Addr) -> u8 {
    mask.0.iter().map(|octet| octet.count_ones() as u8).sum()
}

#[must_use]
pub fn mask_from_prefix_len(prefix_len: u8) -> Ipv4Addr {
    let mut mask = [0u8; 4];
    for (index, octet) in mask.iter_mut().enumerate() {
        let bits_remaining = prefix_len.saturating_sub((index as u8) * 8);
        *octet = match bits_remaining {
            0 => 0,
            1..=7 => (!0u8) << (8 - bits_remaining),
            _ => 0xFF,
        };
    }
    Ipv4Addr(mask)
}

#[must_use]
pub fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in header.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]])
        } else {
            u16::from_be_bytes([chunk[0], 0])
        };
        sum = sum.wrapping_add(u32::from(word));
    }

    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

#[must_use]
pub fn format_cidr(address: Ipv4Addr, prefix_len: u8) -> String {
    alloc::format!("{address}/{prefix_len}")
}
