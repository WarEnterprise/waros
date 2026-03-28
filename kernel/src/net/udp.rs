use alloc::vec;

use smoltcp::socket::udp;

use super::ipv4::Ipv4Addr;
use super::{NetError, NetworkSubsystem};

pub struct UdpResponse {
    pub source: Ipv4Addr,
    pub source_port: u16,
    pub payload: alloc::vec::Vec<u8>,
}

pub fn send_udp(
    stack: &mut NetworkSubsystem,
    dst_ip: Ipv4Addr,
    dst_port: u16,
    src_port: u16,
    payload: &[u8],
    timeout_ms: u64,
) -> Result<Option<UdpResponse>, NetError> {
    {
        use crate::security::firewall;
        use crate::security::firewall::rules::{Action, Direction, Protocol};

        let action = firewall::process_packet(
            Direction::Outbound,
            Protocol::Udp,
            local_ipv4_u32(stack),
            u32::from_be_bytes(dst_ip.0),
            src_port,
            dst_port,
        );
        if action == Action::Deny {
            crate::serial_println!("[WarGuard] DENY outbound UDP to {}:{}", dst_ip, dst_port);
            return Err(NetError::ProtocolError(alloc::string::String::from(
                "firewall: outbound UDP denied",
            )));
        }
    }

    let socket = udp::Socket::new(
        udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 4], vec![0; 2048]),
        udp::PacketBuffer::new(vec![udp::PacketMetadata::EMPTY; 4], vec![0; 2048]),
    );
    let handle = stack.sockets.add(socket);

    {
        let socket = stack.sockets.get_mut::<udp::Socket>(handle);
        socket
            .bind(src_port)
            .map_err(|_| NetError::InitializationFailed("UDP bind failed"))?;
        socket
            .send_slice(payload, (dst_ip.to_ip_address(), dst_port))
            .map_err(|_| NetError::InitializationFailed("UDP send failed"))?;
    }

    let deadline = stack.now_ms().saturating_add(timeout_ms);
    loop {
        stack.poll_network();
        let recv = {
            let socket = stack.sockets.get_mut::<udp::Socket>(handle);
            if socket.can_recv() {
                let mut buffer = vec![0u8; 2048];
                match socket.recv_slice(&mut buffer) {
                    Ok((size, metadata)) => Some((buffer, size, metadata)),
                    Err(_) => None,
                }
            } else {
                None
            }
        };

        if let Some((buffer, size, metadata)) = recv {
            let source = match metadata.endpoint.addr {
                smoltcp::wire::IpAddress::Ipv4(ip) => Ipv4Addr::from_smoltcp(ip),
            };
            {
                use crate::security::firewall;
                use crate::security::firewall::rules::{Action, Direction, Protocol};

                let action = firewall::process_packet(
                    Direction::Inbound,
                    Protocol::Udp,
                    u32::from_be_bytes(source.0),
                    local_ipv4_u32(stack),
                    metadata.endpoint.port,
                    src_port,
                );
                if action == Action::Deny {
                    let _ = stack.sockets.remove(handle);
                    crate::serial_println!(
                        "[WarGuard] DENY inbound UDP from {}:{} to local {}",
                        source,
                        metadata.endpoint.port,
                        src_port
                    );
                    return Err(NetError::ProtocolError(alloc::string::String::from(
                        "firewall: inbound UDP denied",
                    )));
                }
            }
            let mut payload = vec![0u8; size];
            payload.copy_from_slice(&buffer[..size]);
            let _ = stack.sockets.remove(handle);
            return Ok(Some(UdpResponse {
                source,
                source_port: metadata.endpoint.port,
                payload,
            }));
        }

        if stack.now_ms() >= deadline {
            let _ = stack.sockets.remove(handle);
            return Ok(None);
        }
    }
}

fn local_ipv4_u32(stack: &NetworkSubsystem) -> u32 {
    stack
        .network_config
        .map(|config| u32::from_be_bytes(config.ip.0))
        .unwrap_or(0)
}
