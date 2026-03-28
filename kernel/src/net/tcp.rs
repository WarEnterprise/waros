use alloc::vec;
use alloc::vec::Vec;

use smoltcp::iface::SocketHandle;
use smoltcp::socket::tcp;

use super::ipv4::Ipv4Addr;
use super::{NetError, NetworkSubsystem};

const TCP_SOCKET_BUFFER_SIZE: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpState {
    Closed,
    Connecting,
    Established,
    Closing,
}

pub struct TcpConnection {
    handle: SocketHandle,
    pub remote_ip: Ipv4Addr,
    pub remote_port: u16,
    pub local_port: u16,
    pub state: TcpState,
    inbound_firewall_verified: bool,
}

impl TcpConnection {
    pub fn connect(
        stack: &mut NetworkSubsystem,
        remote_ip: Ipv4Addr,
        remote_port: u16,
        timeout_ms: u64,
    ) -> Result<Self, NetError> {
        let local_port = stack.allocate_local_port();

        // WarGuard firewall check: outbound TCP
        {
            use crate::security::firewall;
            use crate::security::firewall::rules::{Action, Direction, Protocol};

            let src_ip = local_ipv4_u32(stack);
            let action = firewall::process_packet(
                Direction::Outbound,
                Protocol::Tcp,
                src_ip,
                u32::from_be_bytes(remote_ip.0),
                local_port,
                remote_port,
            );
            if action == Action::Deny {
                crate::serial_println!(
                    "[WarGuard] DENY outbound TCP to {}:{}",
                    remote_ip,
                    remote_port
                );
                return Err(NetError::ProtocolError(
                    alloc::string::String::from("firewall: outbound TCP denied"),
                ));
            }
        }

        let socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; TCP_SOCKET_BUFFER_SIZE]),
            tcp::SocketBuffer::new(vec![0; TCP_SOCKET_BUFFER_SIZE]),
        );
        let handle = stack.sockets.add(socket);

        {
            let (iface, sockets) = (&mut stack.iface, &mut stack.sockets);
            let iface = iface
                .as_mut()
                .ok_or(NetError::InitializationFailed("network interface not ready"))?;
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            socket
                .connect(
                    iface.context(),
                    (remote_ip.to_ip_address(), remote_port),
                    local_port,
                )
                .map_err(|_| NetError::InitializationFailed("TCP connect failed"))?;
        }

        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();
            let established = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
                socket.may_send()
            };
            if established {
                stack
                    .sockets
                    .get_mut::<tcp::Socket>(handle)
                    .set_nagle_enabled(false);
                crate::security::audit::log_event(
                    crate::security::audit::events::AuditEvent::NetworkConnection {
                        src_ip: local_ipv4_u32(stack),
                        src_port: local_port,
                        dst_ip: u32::from_be_bytes(remote_ip.0),
                        dst_port: remote_port,
                        protocol: alloc::string::String::from("TCP"),
                    },
                );
                return Ok(Self {
                    handle,
                    remote_ip,
                    remote_port,
                    local_port,
                    state: TcpState::Established,
                    inbound_firewall_verified: false,
                });
            }
            if stack.now_ms() >= deadline {
                let _ = stack.sockets.remove(handle);
                return Err(NetError::InitializationFailed("TCP connection timed out"));
            }
        }
    }

    pub fn send(
        &mut self,
        stack: &mut NetworkSubsystem,
        data: &[u8],
        timeout_ms: u64,
    ) -> Result<usize, NetError> {
        let deadline = stack.now_ms().saturating_add(timeout_ms);
        let mut written = 0usize;

        while written < data.len() {
            stack.poll_network();
            let sent = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                if socket.can_send() {
                    socket
                        .send_slice(&data[written..])
                        .map_err(|_| NetError::InitializationFailed("TCP send failed"))?
                } else {
                    0
                }
            };

            written += sent;
            if written == data.len() {
                break;
            }
            if stack.now_ms() >= deadline {
                return Err(NetError::InitializationFailed("TCP send timed out"));
            }
        }

        Ok(written)
    }

    pub fn flush(
        &mut self,
        stack: &mut NetworkSubsystem,
        timeout_ms: u64,
    ) -> Result<(), NetError> {
        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();
            let pending = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                socket.send_queue()
            };
            if pending == 0 {
                return Ok(());
            }
            if stack.now_ms() >= deadline {
                return Err(NetError::InitializationFailed("TCP flush timed out"));
            }
        }
    }

    pub fn recv(
        &mut self,
        stack: &mut NetworkSubsystem,
        buffer: &mut [u8],
        timeout_ms: u64,
    ) -> Result<usize, NetError> {
        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();
            let received = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                if socket.can_recv() {
                    socket
                        .recv_slice(buffer)
                        .map_err(|_| NetError::InitializationFailed("TCP receive failed"))?
                } else if !socket.may_recv() {
                    0
                } else {
                    usize::MAX
                }
            };

            if received != usize::MAX {
                if received > 0 {
                    self.verify_inbound_firewall(stack)?;
                }
                return Ok(received);
            }
            if stack.now_ms() >= deadline {
                return Err(NetError::InitializationFailed("TCP receive timed out"));
            }
        }
    }

    pub fn read_to_end(
        &mut self,
        stack: &mut NetworkSubsystem,
        timeout_ms: u64,
    ) -> Result<Vec<u8>, NetError> {
        let mut response = Vec::new();
        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();
            let state = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                if socket.can_recv() {
                    let mut chunk = vec![0u8; 1024];
                    match socket.recv_slice(&mut chunk) {
                        Ok(size) => {
                            chunk.truncate(size);
                            Some(chunk)
                        }
                        Err(_) => None,
                    }
                } else if !socket.may_recv() {
                    return Ok(response);
                } else {
                    None
                }
            };

            if let Some(chunk) = state {
                if !chunk.is_empty() {
                    self.verify_inbound_firewall(stack)?;
                }
                response.extend_from_slice(&chunk);
            } else if stack.now_ms() >= deadline {
                return Err(NetError::InitializationFailed("TCP receive timed out"));
            }
        }
    }

    pub fn close(mut self, stack: &mut NetworkSubsystem, timeout_ms: u64) -> Result<(), NetError> {
        self.state = TcpState::Closing;
        {
            let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
            socket.close();
        }

        let deadline = stack.now_ms().saturating_add(timeout_ms);
        loop {
            stack.poll_network();
            let closed = {
                let socket = stack.sockets.get_mut::<tcp::Socket>(self.handle);
                !socket.is_active()
            };
            if closed || stack.now_ms() >= deadline {
                break;
            }
        }
        let _ = stack.sockets.remove(self.handle);
        Ok(())
    }

    fn verify_inbound_firewall(&mut self, stack: &mut NetworkSubsystem) -> Result<(), NetError> {
        if self.inbound_firewall_verified {
            return Ok(());
        }

        use crate::security::firewall;
        use crate::security::firewall::rules::{Action, Direction, Protocol};

        let action = firewall::process_packet(
            Direction::Inbound,
            Protocol::Tcp,
            u32::from_be_bytes(self.remote_ip.0),
            local_ipv4_u32(stack),
            self.remote_port,
            self.local_port,
        );
        if action == Action::Deny {
            crate::serial_println!(
                "[WarGuard] DENY inbound TCP response from {}:{} to local {}",
                self.remote_ip,
                self.remote_port,
                self.local_port
            );
            return Err(NetError::ProtocolError(alloc::string::String::from(
                "firewall: inbound TCP denied",
            )));
        }

        self.inbound_firewall_verified = true;
        Ok(())
    }
}

fn local_ipv4_u32(stack: &NetworkSubsystem) -> u32 {
    stack
        .network_config
        .map(|config| u32::from_be_bytes(config.ip.0))
        .unwrap_or(0)
}
