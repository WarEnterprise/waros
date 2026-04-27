use smoltcp::socket::dhcpv4;

use super::ipv4::{mask_from_prefix_len, prefix_len_from_mask, Ipv4Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DhcpConfig {
    pub ip: Ipv4Addr,
    pub prefix_len: u8,
    pub subnet_mask: Ipv4Addr,
    pub gateway: Option<Ipv4Addr>,
    pub dns_server: Option<Ipv4Addr>,
}

impl DhcpConfig {
    #[must_use]
    pub fn cidr_string(&self) -> alloc::string::String {
        alloc::format!("{}/{}", self.ip, self.prefix_len)
    }

    #[must_use]
    pub fn is_on_link(&self, address: Ipv4Addr) -> bool {
        let mask = u32::from_be_bytes(self.subnet_mask.0);
        let local = u32::from_be_bytes(self.ip.0);
        let remote = u32::from_be_bytes(address.0);
        (local & mask) == (remote & mask)
    }

    #[must_use]
    pub fn can_reach(&self, address: Ipv4Addr) -> bool {
        self.is_on_link(address) || self.gateway.is_some()
    }
}

impl<'a> From<&dhcpv4::Config<'a>> for DhcpConfig {
    fn from(config: &dhcpv4::Config<'a>) -> Self {
        let prefix_len = config.address.prefix_len();
        let ip = Ipv4Addr::from_smoltcp(config.address.address());
        let subnet_mask = mask_from_prefix_len(prefix_len);
        let gateway = config.router.map(Ipv4Addr::from_smoltcp);
        let dns_server = config
            .dns_servers
            .first()
            .copied()
            .map(Ipv4Addr::from_smoltcp);

        Self {
            ip,
            prefix_len,
            subnet_mask,
            gateway,
            dns_server,
        }
    }
}

#[must_use]
pub fn prefix_len_from_config(config: &DhcpConfig) -> u8 {
    prefix_len_from_mask(config.subnet_mask)
}
