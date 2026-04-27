use smoltcp::socket::icmp;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr, IpAddress};

use super::ipv4::Ipv4Addr;
use super::{NetError, NetworkSubsystem};

const ICMP_ARP_PROBE_TIMEOUT_MS: u64 = 750;

pub struct PingReply {
    pub source: Ipv4Addr,
    pub seq_no: u16,
    pub payload_len: usize,
}

pub fn ping(
    stack: &mut NetworkSubsystem,
    target: Ipv4Addr,
    seq_no: u16,
    timeout_ms: u64,
) -> Result<PingReply, NetError> {
    let (_, config) = stack.require_route_to(target, "ICMP ping")?;
    let on_link = config.is_on_link(target);
    if on_link && stack.arp_cache.lookup(target).is_none() {
        stack.send_arp_probe(target)?;
        let deadline = stack
            .now_ms()
            .saturating_add(ICMP_ARP_PROBE_TIMEOUT_MS.min(timeout_ms));
        while stack.now_ms() < deadline {
            stack.poll_network();
            if stack.arp_cache.lookup(target).is_some() {
                break;
            }
            super::wait_for_runtime_progress();
        }
        if stack.arp_cache.lookup(target).is_none() {
            return Err(NetError::ProtocolError(alloc::format!(
                "ICMP ping: ARP unresolved for on-link target {target}"
            )));
        }
    }
    {
        use crate::security::firewall;
        use crate::security::firewall::rules::{Action, Direction, Protocol};

        let action = firewall::process_packet(
            Direction::Outbound,
            Protocol::Icmp,
            local_ipv4_u32(stack),
            u32::from_be_bytes(target.0),
            0,
            0,
        );
        if action == Action::Deny {
            crate::serial_println!("[WarGuard] DENY outbound ICMP to {}", target);
            return Err(NetError::ProtocolError(alloc::string::String::from(
                "firewall: outbound ICMP denied",
            )));
        }
    }

    let socket = icmp::Socket::new(
        icmp::PacketBuffer::new(
            alloc::vec![icmp::PacketMetadata::EMPTY],
            alloc::vec![0; 256],
        ),
        icmp::PacketBuffer::new(
            alloc::vec![icmp::PacketMetadata::EMPTY],
            alloc::vec![0; 256],
        ),
    );
    let handle = stack.sockets.add(socket);
    let ident = 0x574F;
    let target_ip = IpAddress::Ipv4(target.as_smoltcp());

    {
        let socket = stack.sockets.get_mut::<icmp::Socket>(handle);
        if !socket.is_open() {
            if socket.bind(icmp::Endpoint::Ident(ident)).is_err() {
                let _ = stack.sockets.remove(handle);
                return Err(NetError::InitializationFailed("ICMP bind failed"));
            }
        }
    }

    {
        let checksum_caps = smoltcp::phy::ChecksumCapabilities::default();
        let socket = stack.sockets.get_mut::<icmp::Socket>(handle);
        let request = Icmpv4Repr::EchoRequest {
            ident,
            seq_no,
            data: b"waros-ping",
        };
        let payload = match socket.send(request.buffer_len(), target_ip) {
            Ok(payload) => payload,
            Err(_) => {
                let _ = stack.sockets.remove(handle);
                return Err(NetError::InitializationFailed("ICMP send failed"));
            }
        };
        let mut packet = Icmpv4Packet::new_unchecked(payload);
        request.emit(&mut packet, &checksum_caps);
    }

    let deadline = stack.now_ms().saturating_add(timeout_ms);
    loop {
        stack.poll_network();
        let local_ip = local_ipv4_u32(stack);
        let reply = {
            let checksum_caps = smoltcp::phy::ChecksumCapabilities::default();
            let socket = stack.sockets.get_mut::<icmp::Socket>(handle);
            if socket.can_recv() {
                let (payload, endpoint) = match socket.recv() {
                    Ok(result) => result,
                    Err(_) => {
                        let _ = stack.sockets.remove(handle);
                        return Err(NetError::InitializationFailed("ICMP receive failed"));
                    }
                };
                let packet = match Icmpv4Packet::new_checked(&payload) {
                    Ok(packet) => packet,
                    Err(_) => {
                        let _ = stack.sockets.remove(handle);
                        return Err(NetError::InitializationFailed("invalid ICMP packet"));
                    }
                };
                match Icmpv4Repr::parse(&packet, &checksum_caps) {
                    Ok(repr) => match repr {
                        Icmpv4Repr::EchoReply {
                            ident: reply_ident,
                            seq_no: reply_seq,
                            data,
                        } if reply_ident == ident && endpoint == target_ip => {
                            let source = match endpoint {
                                IpAddress::Ipv4(ip) => Ipv4Addr::from_smoltcp(ip),
                            };
                            use crate::security::firewall;
                            use crate::security::firewall::rules::{Action, Direction, Protocol};

                            let action = firewall::process_packet(
                                Direction::Inbound,
                                Protocol::Icmp,
                                u32::from_be_bytes(source.0),
                                local_ip,
                                0,
                                0,
                            );
                            if action == Action::Deny {
                                Some(Err(NetError::ProtocolError(alloc::string::String::from(
                                    "firewall: inbound ICMP denied",
                                ))))
                            } else {
                                Some(Ok(PingReply {
                                    source,
                                    seq_no: reply_seq,
                                    payload_len: data.len(),
                                }))
                            }
                        }
                        _ => None,
                    },
                    Err(_) => {
                        let _ = stack.sockets.remove(handle);
                        return Err(NetError::InitializationFailed("invalid ICMP reply"));
                    }
                }
            } else {
                None
            }
        };

        if let Some(reply) = reply {
            let _ = stack.sockets.remove(handle);
            return reply;
        }
        if stack.now_ms() >= deadline {
            let _ = stack.sockets.remove(handle);
            let message = if on_link {
                alloc::format!(
                    "ICMP ping: ARP resolved and echo request was sent to {target}, but no reply was received"
                )
            } else {
                alloc::format!(
                    "ICMP ping: echo request was sent to {target} via the configured route, but no reply was received"
                )
            };
            return Err(NetError::ProtocolError(message));
        }

        super::wait_for_runtime_progress();
    }
}

fn local_ipv4_u32(stack: &NetworkSubsystem) -> u32 {
    stack
        .network_config
        .map(|config| u32::from_be_bytes(config.ip.0))
        .unwrap_or(0)
}
