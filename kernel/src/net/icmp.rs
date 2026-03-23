use smoltcp::socket::icmp;
use smoltcp::wire::{Icmpv4Packet, Icmpv4Repr, IpAddress};

use super::ipv4::Ipv4Addr;
use super::{NetError, NetworkSubsystem};

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
    let socket = icmp::Socket::new(
        icmp::PacketBuffer::new(alloc::vec![icmp::PacketMetadata::EMPTY], alloc::vec![0; 256]),
        icmp::PacketBuffer::new(alloc::vec![icmp::PacketMetadata::EMPTY], alloc::vec![0; 256]),
    );
    let handle = stack.sockets.add(socket);
    let ident = 0x574F;
    let target_ip = IpAddress::Ipv4(target.as_smoltcp());

    {
        let socket = stack.sockets.get_mut::<icmp::Socket>(handle);
        if !socket.is_open() {
            socket
                .bind(icmp::Endpoint::Ident(ident))
                .map_err(|_| NetError::InitializationFailed("ICMP bind failed"))?;
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
        let payload = socket
            .send(request.buffer_len(), target_ip)
            .map_err(|_| NetError::InitializationFailed("ICMP send failed"))?;
        let mut packet = Icmpv4Packet::new_unchecked(payload);
        request.emit(&mut packet, &checksum_caps);
    }

    let deadline = stack.now_ms().saturating_add(timeout_ms);
    loop {
        stack.poll_network();
        let reply = {
            let checksum_caps = smoltcp::phy::ChecksumCapabilities::default();
            let socket = stack.sockets.get_mut::<icmp::Socket>(handle);
            if socket.can_recv() {
                let (payload, endpoint) = socket
                    .recv()
                    .map_err(|_| NetError::InitializationFailed("ICMP receive failed"))?;
                let packet = Icmpv4Packet::new_checked(&payload)
                    .map_err(|_| NetError::InitializationFailed("invalid ICMP packet"))?;
                match Icmpv4Repr::parse(&packet, &checksum_caps)
                    .map_err(|_| NetError::InitializationFailed("invalid ICMP reply"))?
                {
                    Icmpv4Repr::EchoReply {
                        ident: reply_ident,
                        seq_no: reply_seq,
                        data,
                    } if reply_ident == ident => {
                        let source = match endpoint {
                            IpAddress::Ipv4(ip) => Ipv4Addr::from_smoltcp(ip),
                        };
                        Some(PingReply {
                            source,
                            seq_no: reply_seq,
                            payload_len: data.len(),
                        })
                    }
                    _ => None,
                }
            } else {
                None
            }
        };

        if let Some(reply) = reply {
            let _ = stack.sockets.remove(handle);
            return Ok(reply);
        }
        if stack.now_ms() >= deadline {
            let _ = stack.sockets.remove(handle);
            return Err(NetError::InitializationFailed("ICMP request timed out"));
        }
    }
}
