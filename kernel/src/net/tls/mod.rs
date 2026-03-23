use embedded_io::{ErrorKind, ErrorType, Read, Write};
use embedded_tls::blocking::{
    Aes128GcmSha256, TlsConfig, TlsConnection as EmbeddedTlsConnection, TlsContext,
    UnsecureProvider,
};
use rand_core::{CryptoRng, RngCore};

use super::http::{parse_response, HttpResponse, UrlParts};
use super::tcp::TcpConnection;
use super::{NetError, NetworkSubsystem};

pub struct TlsConnection;

impl TlsConnection {
    pub fn https_request(
        stack: &mut NetworkSubsystem,
        method: &str,
        parts: &UrlParts,
        body: Option<(&str, &[u8])>,
    ) -> Result<HttpResponse, NetError> {
        let stream = KernelTcpStream::connect(stack, &parts.host, parts.port)?;
        let mut read_record_buffer = [0u8; 16_640];
        let mut write_record_buffer = [0u8; 4_096];
        let config = TlsConfig::new()
            .with_server_name(&parts.host)
            .enable_rsa_signatures();
        let provider = UnsecureProvider::new::<Aes128GcmSha256>(KernelRng::seeded());
        let mut tls = EmbeddedTlsConnection::<_, Aes128GcmSha256>::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );
        tls.open(TlsContext::new(&config, provider))
            .map_err(|_| NetError::InitializationFailed("TLS handshake failed"))?;

        let mut request = alloc::format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: WarOS/0.1.0\r\nConnection: close\r\n",
            parts.path, parts.host
        );
        if let Some((content_type, body_bytes)) = body {
            request.push_str(&alloc::format!(
                "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
                body_bytes.len()
            ));
        }
        request.push_str("\r\n");
        write_all(&mut tls, request.as_bytes())?;
        if let Some((_, body_bytes)) = body {
            write_all(&mut tls, body_bytes)?;
        }
        tls.flush()
            .map_err(|_| NetError::InitializationFailed("TLS flush failed"))?;

        let mut response = alloc::vec::Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let size = tls
                .read(&mut buffer)
                .map_err(|_| NetError::InitializationFailed("TLS read failed"))?;
            if size == 0 {
                break;
            }
            response.extend_from_slice(&buffer[..size]);
        }

        parse_response(&response)
    }
}

fn write_all(
    tls: &mut EmbeddedTlsConnection<'_, KernelTcpStream<'_>, Aes128GcmSha256>,
    mut data: &[u8],
) -> Result<(), NetError> {
    while !data.is_empty() {
        let written = tls
            .write(data)
            .map_err(|_| NetError::InitializationFailed("TLS write failed"))?;
        if written == 0 {
            return Err(NetError::InitializationFailed("TLS write returned zero bytes"));
        }
        data = &data[written..];
    }
    Ok(())
}

struct KernelTcpStream<'a> {
    stack: &'a mut NetworkSubsystem,
    connection: Option<TcpConnection>,
}

impl<'a> KernelTcpStream<'a> {
    fn connect(
        stack: &'a mut NetworkSubsystem,
        host: &str,
        port: u16,
    ) -> Result<Self, NetError> {
        let remote_ip = stack.resolve_host(host)?;
        let connection = TcpConnection::connect(stack, remote_ip, port, 5_000)?;
        Ok(Self {
            stack,
            connection: Some(connection),
        })
    }
}

impl ErrorType for KernelTcpStream<'_> {
    type Error = ErrorKind;
}

impl Read for KernelTcpStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.connection
            .as_mut()
            .ok_or(ErrorKind::NotConnected)?
            .recv(self.stack, buf, 10_000)
            .map_err(|_| ErrorKind::Other)
    }
}

impl Write for KernelTcpStream<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.connection
            .as_mut()
            .ok_or(ErrorKind::NotConnected)?
            .send(self.stack, buf, 10_000)
            .map_err(|_| ErrorKind::Other)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl Drop for KernelTcpStream<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = connection.close(self.stack, 2_000);
        }
    }
}

struct KernelRng(u64);

impl KernelRng {
    fn seeded() -> Self {
        Self(0x57_41_52_4F_53_54_4C_53)
    }

    fn next_word(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

impl RngCore for KernelRng {
    fn next_u32(&mut self) -> u32 {
        self.next_word() as u32
    }

    fn next_u64(&mut self) -> u64 {
        self.next_word()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut offset = 0usize;
        while offset < dest.len() {
            let chunk = self.next_word().to_le_bytes();
            let remaining = core::cmp::min(dest.len() - offset, chunk.len());
            dest[offset..offset + remaining].copy_from_slice(&chunk[..remaining]);
            offset += remaining;
        }
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for KernelRng {}
