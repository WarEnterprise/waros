use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::dns::{self, GetQueryResultError};
use smoltcp::wire::{DnsQueryType, IpAddress};

use super::ipv4::Ipv4Addr;
use super::{NetError, NetworkSubsystem};

#[derive(Debug, Clone)]
pub struct DnsCacheEntry {
    pub domain: String,
    pub ip: Ipv4Addr,
    pub expires_at_ms: u64,
}

#[derive(Debug, Default, Clone)]
pub struct DnsResolver {
    cache: Vec<DnsCacheEntry>,
}

impl DnsResolver {
    #[must_use]
    pub fn new() -> Self {
        Self { cache: Vec::new() }
    }

    pub fn resolve(
        &mut self,
        stack: &mut NetworkSubsystem,
        domain: &str,
        timeout_ms: u64,
    ) -> Result<Ipv4Addr, NetError> {
        let now = stack.now_ms();
        self.cache.retain(|entry| entry.expires_at_ms > now);
        if let Some(entry) = self
            .cache
            .iter()
            .find(|entry| entry.domain.eq_ignore_ascii_case(domain))
        {
            return Ok(entry.ip);
        }

        let server = stack
            .network_config
            .and_then(|config| config.dns_server)
            .ok_or(NetError::InitializationFailed("no DNS server configured"))?;
        let servers = [IpAddress::Ipv4(server.as_smoltcp())];
        let query_handle = stack
            .sockets
            .add(dns::Socket::new(&servers, vec![None, None, None, None]));

        let query = {
            let (iface, sockets) = (&mut stack.iface, &mut stack.sockets);
            let iface = iface
                .as_mut()
                .ok_or(NetError::InitializationFailed("network interface not ready"))?;
            let socket = sockets.get_mut::<dns::Socket>(query_handle);
            socket
                .start_query(iface.context(), domain, DnsQueryType::A)
                .map_err(|_| NetError::InitializationFailed("DNS query could not be started"))?
        };

        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();

            let result = {
                let socket = stack.sockets.get_mut::<dns::Socket>(query_handle);
                socket.get_query_result(query)
            };

            match result {
                Ok(addresses) => {
                    if let Some(IpAddress::Ipv4(address)) =
                        addresses.into_iter().find(|address| matches!(address, IpAddress::Ipv4(_)))
                    {
                        let resolved = Ipv4Addr::from_smoltcp(address);
                        self.cache.push(DnsCacheEntry {
                            domain: domain.to_string(),
                            ip: resolved,
                            expires_at_ms: deadline.saturating_add(60_000),
                        });
                        remove_socket(&mut stack.sockets, query_handle);
                        return Ok(resolved);
                    }
                    remove_socket(&mut stack.sockets, query_handle);
                    return Err(NetError::InitializationFailed("DNS response had no IPv4 records"));
                }
                Err(GetQueryResultError::Pending) => {}
                Err(GetQueryResultError::Failed) => {
                    remove_socket(&mut stack.sockets, query_handle);
                    return Err(NetError::InitializationFailed("DNS query failed"));
                }
            }

            if stack.now_ms() >= deadline {
                remove_socket(&mut stack.sockets, query_handle);
                return Err(NetError::InitializationFailed("DNS query timed out"));
            }
        }
    }

    #[must_use]
    pub fn entries(&self) -> &[DnsCacheEntry] {
        &self.cache
    }
}

fn remove_socket(sockets: &mut smoltcp::iface::SocketSet<'static>, handle: SocketHandle) {
    let _ = sockets.remove(handle);
}
