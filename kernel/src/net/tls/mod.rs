use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::arch::x86_64::{__cpuid, _rdrand64_step};
use core::marker::PhantomData;

use embedded_io::{ErrorKind, ErrorType, Read, Write};
use embedded_tls::blocking::{
    Aes128GcmSha256, Certificate, CertificateEntryRef, CertificateRef, CryptoProvider, Sha256,
    TlsClock, TlsConfig, TlsConnection as EmbeddedTlsConnection, TlsContext,
    TlsError as EmbeddedTlsError, TlsVerifier,
};
use embedded_tls::pki::CertVerifier;
use rand_core::{CryptoRng, CryptoRngCore, RngCore};
use sha2::Digest;
use spin::Lazy;

use crate::{serial_println, KERNEL_VERSION};

use super::http::{parse_response, HttpResponse, UrlParts};
use super::tcp::TcpConnection;
use super::{ipv4::Ipv4Addr, NetError, NetworkSubsystem};

const TLS_RECORD_BUFFER_SIZE: usize = 16_640;
const TLS_CERTIFICATE_STORE_SIZE: usize = 4_096;
const TLS_IO_TIMEOUT_MS: u64 = 15_000;
const TLS_TRUST_ROOT_COUNT: usize = 2;
const SUPPORTED_TLS_HOSTS: [&str; 3] = [
    "iam.cloud.ibm.com",
    "quantum.cloud.ibm.com",
    "warenterprise.com",
];

const DIGICERT_GLOBAL_ROOT_G3_HEX: &str = include_str!("anchors/digicert_global_root_g3.hex");
const GLOBALSIGN_ECC_ROOT_R4_HEX: &str = include_str!("anchors/globalsign_ecc_root_r4.hex");
const PROOF_WARENTERPRISE_LEAF_HEX: &str = include_str!("proof/warenterprise_leaf.hex");
const PROOF_WE1_INTERMEDIATE_HEX: &str = include_str!("proof/we1_intermediate.hex");

static DIGICERT_GLOBAL_ROOT_G3: Lazy<Vec<u8>> =
    Lazy::new(|| decode_hex_bytes(DIGICERT_GLOBAL_ROOT_G3_HEX));
static GLOBALSIGN_ECC_ROOT_R4: Lazy<Vec<u8>> =
    Lazy::new(|| decode_hex_bytes(GLOBALSIGN_ECC_ROOT_R4_HEX));
static PROOF_WARENTERPRISE_LEAF: Lazy<Vec<u8>> =
    Lazy::new(|| decode_hex_bytes(PROOF_WARENTERPRISE_LEAF_HEX));
static PROOF_WE1_INTERMEDIATE: Lazy<Vec<u8>> =
    Lazy::new(|| decode_hex_bytes(PROOF_WE1_INTERMEDIATE_HEX));

#[derive(Clone, Copy)]
struct HostTrustAnchor {
    label: &'static str,
    der: &'static [u8],
}

struct KernelTlsClock;

impl TlsClock for KernelTlsClock {
    fn now() -> Option<u64> {
        None
    }
}

struct KernelTlsSignature;

impl AsRef<[u8]> for KernelTlsSignature {
    fn as_ref(&self) -> &[u8] {
        &[]
    }
}

struct KernelTlsProvider<CipherSuite>
where
    CipherSuite: embedded_tls::blocking::TlsCipherSuite,
{
    rng: KernelRng,
    verifier: CertVerifier<CipherSuite, KernelTlsClock, TLS_CERTIFICATE_STORE_SIZE>,
    _marker: PhantomData<CipherSuite>,
}

impl<CipherSuite> KernelTlsProvider<CipherSuite>
where
    CipherSuite: embedded_tls::blocking::TlsCipherSuite,
{
    fn new(rng: KernelRng) -> Self {
        Self {
            rng,
            verifier: CertVerifier::new(),
            _marker: PhantomData,
        }
    }
}

impl<CipherSuite> CryptoProvider for KernelTlsProvider<CipherSuite>
where
    CipherSuite: embedded_tls::blocking::TlsCipherSuite,
{
    type CipherSuite = CipherSuite;
    type Signature = KernelTlsSignature;

    fn rng(&mut self) -> impl CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Self::CipherSuite>, EmbeddedTlsError> {
        Ok(&mut self.verifier)
    }
}

pub struct TlsConnection;

impl TlsConnection {
    pub fn https_request(
        stack: &mut NetworkSubsystem,
        method: &str,
        parts: &UrlParts,
        extra_headers: &[(&str, &str)],
        body: Option<(&str, &[u8])>,
    ) -> Result<HttpResponse, NetError> {
        let Some(anchor) = trust_anchor_for_host(&parts.host) else {
            log_tls_validation_event(
                &parts.host,
                "none",
                "deny",
                "unsupported-host-under-embedded-policy",
            );
            return Err(NetError::ProtocolError(alloc::format!(
                "TLS validation unavailable for host {} under current embedded trust policy",
                parts.host
            )));
        };

        serial_println!("[TLS] Resolving DNS for {}...", parts.host);
        let remote_ip = stack.resolve_host(&parts.host)?;
        serial_println!("[TLS] DNS resolved: {} -> {}", parts.host, remote_ip);
        serial_println!("[TLS] TCP connecting to {}:{}...", remote_ip, parts.port);

        let stream = KernelTcpStream::connect(stack, &parts.host, remote_ip, parts.port)?;
        serial_println!("[TLS] TCP connected");
        let mut read_record_buffer = vec![0u8; TLS_RECORD_BUFFER_SIZE];
        let mut write_record_buffer = vec![0u8; TLS_RECORD_BUFFER_SIZE];
        let config = TlsConfig::new()
            .with_ca(Certificate::X509(anchor.der))
            .with_server_name(&parts.host);
        let rng = KernelRng::new();
        serial_println!("[TLS] Preparing ClientHello...");
        serial_println!("[TLS]   SNI: {}", parts.host);
        serial_println!("[TLS]   Cipher suites: TLS_AES_128_GCM_SHA256");
        serial_println!("[TLS]   Key exchange: secp256r1 (embedded-tls)");
        serial_println!("[TLS]   Trust anchor: {}", anchor.label);
        serial_println!("[TLS]   Hostname check: SAN/CN");
        serial_println!("[TLS]   Time validity: no RTC-backed expiry check");
        serial_println!(
            "[TLS]   Read buffer: {} bytes | Write buffer: {} bytes",
            read_record_buffer.len(),
            write_record_buffer.len()
        );
        serial_println!(
            "[TLS]   RNG: {}",
            if rng.using_rdrand() {
                "RDRAND + xorshift mix"
            } else {
                "PIT-seeded xorshift fallback"
            }
        );
        let provider = KernelTlsProvider::<Aes128GcmSha256>::new(rng);

        let mut tls = EmbeddedTlsConnection::<_, Aes128GcmSha256>::new(
            stream,
            &mut read_record_buffer,
            &mut write_record_buffer,
        );
        serial_println!("[TLS] Starting embedded-tls handshake...");
        if let Err(error) = tls.open(TlsContext::new(&config, provider)) {
            serial_println!("[TLS] Handshake failed: {}", error);
            log_tls_validation_event(
                &parts.host,
                anchor.label,
                "deny",
                &alloc::format!("handshake-failed:{error}"),
            );
            return Err(tls_error("TLS handshake failed", error));
        }
        log_tls_validation_event(&parts.host, anchor.label, "allow", "handshake-certificate-accepted");
        serial_println!("[TLS] TLS 1.3 handshake complete");

        let mut request = alloc::format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: WarOS/{}\r\nConnection: close\r\n",
            parts.path, parts.host, KERNEL_VERSION
        );
        if let Some((content_type, body_bytes)) = body {
            request.push_str(&alloc::format!(
                "Content-Type: {content_type}\r\nContent-Length: {}\r\n",
                body_bytes.len()
            ));
        }
        for &(name, value) in extra_headers {
            request.push_str(&alloc::format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");

        serial_println!("[TLS] Sending HTTPS request headers ({} bytes)...", request.len());
        write_all(&mut tls, request.as_bytes())?;
        if let Some((_, body_bytes)) = body {
            serial_println!("[TLS] Sending HTTPS request body ({} bytes)...", body_bytes.len());
            write_all(&mut tls, body_bytes)?;
        }
        tls.flush()
            .map_err(|error| tls_error("TLS flush failed", error))?;
        serial_println!("[TLS] HTTPS request flushed");

        let mut response = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            match tls.read(&mut buffer) {
                Ok(0) | Err(EmbeddedTlsError::ConnectionClosed) => break,
                Ok(size) => {
                    serial_println!("[TLS] Application data read: {} bytes", size);
                    response.extend_from_slice(&buffer[..size])
                }
                Err(EmbeddedTlsError::Io(kind)) => {
                    serial_println!("[TLS] Transport closed during HTTPS read: {:?}", kind);
                    break;
                }
                Err(error) => return Err(tls_error("TLS read failed", error)),
            }
        }

        let _ = tls.close();
        parse_response(&response)
    }
}

#[must_use]
pub fn supported_hosts_summary() -> String {
    let mut summary = String::new();
    for host in SUPPORTED_TLS_HOSTS {
        if !summary.is_empty() {
            summary.push_str(", ");
        }
        summary.push_str(host);
    }
    summary
}

#[must_use]
pub fn trust_policy_summary() -> String {
    alloc::format!(
        "{} embedded root(s); supported hosts: {}; SAN/CN hostname checks; no RTC-backed expiry check",
        TLS_TRUST_ROOT_COUNT,
        supported_hosts_summary()
    )
}

pub fn run_validation_proof() -> Result<(), &'static str> {
    let anchor = trust_anchor_for_host("warenterprise.com").ok_or("TLS proof root missing")?;

    let mut verifier =
        CertVerifier::<Aes128GcmSha256, KernelTlsClock, TLS_CERTIFICATE_STORE_SIZE>::new();
    verifier
        .set_hostname_verification("warenterprise.com")
        .map_err(|_| "TLS proof hostname setup failed")?;
    let trusted_chain =
        build_certificate_chain(PROOF_WARENTERPRISE_LEAF.as_slice(), PROOF_WE1_INTERMEDIATE.as_slice())
            .map_err(|_| "TLS proof chain build failed")?;
    verifier
        .verify_certificate(
            &Sha256::new(),
            &Some(Certificate::X509(anchor.der)),
            trusted_chain,
        )
        .map_err(|error| {
            serial_println!(
                "[INFO] WarShield TLS proof: trusted certificate chain rejected ({})",
                tls_error_label(error)
            );
            "trusted certificate chain rejected"
        })?;
    log_tls_validation_event("warenterprise.com", anchor.label, "allow", "proof-trusted-chain");
    crate::serial_println!("[PROOF] TLS: trusted certificate chain accepted");

    let mut verifier =
        CertVerifier::<Aes128GcmSha256, KernelTlsClock, TLS_CERTIFICATE_STORE_SIZE>::new();
    verifier
        .set_hostname_verification("wrong.example")
        .map_err(|_| "TLS proof reject-host setup failed")?;
    let rejected_chain =
        build_certificate_chain(PROOF_WARENTERPRISE_LEAF.as_slice(), PROOF_WE1_INTERMEDIATE.as_slice())
            .map_err(|_| "TLS proof reject chain build failed")?;
    match verifier.verify_certificate(
        &Sha256::new(),
        &Some(Certificate::X509(anchor.der)),
        rejected_chain,
    ) {
        Err(EmbeddedTlsError::InvalidCertificate) => {
            log_tls_validation_event(
                "wrong.example",
                anchor.label,
                "deny",
                "proof-hostname-mismatch",
            );
            crate::serial_println!("[PROOF] TLS: invalid host rejected");
            Ok(())
        }
        Err(_) => Err("invalid certificate path rejected with wrong error"),
        Ok(()) => Err("invalid certificate path unexpectedly accepted"),
    }
}

fn tls_error_label(error: EmbeddedTlsError) -> &'static str {
    match error {
        EmbeddedTlsError::InvalidCertificate => "invalid-certificate",
        EmbeddedTlsError::DecodeError => "decode-error",
        EmbeddedTlsError::InvalidSignature => "invalid-signature",
        EmbeddedTlsError::InvalidSignatureScheme => "invalid-signature-scheme",
        EmbeddedTlsError::ParseError(_) => "parse-error",
        EmbeddedTlsError::InsufficientSpace => "insufficient-space",
        _ => "other",
    }
}

fn trust_anchor_for_host(host: &str) -> Option<HostTrustAnchor> {
    if host.eq_ignore_ascii_case("iam.cloud.ibm.com") {
        return (!DIGICERT_GLOBAL_ROOT_G3.is_empty()).then_some(HostTrustAnchor {
            label: "DigiCert Global Root G3",
            der: DIGICERT_GLOBAL_ROOT_G3.as_slice(),
        });
    }
    if host.eq_ignore_ascii_case("quantum.cloud.ibm.com")
        || host.eq_ignore_ascii_case("warenterprise.com")
    {
        return (!GLOBALSIGN_ECC_ROOT_R4.is_empty()).then_some(HostTrustAnchor {
            label: "GlobalSign ECC Root CA - R4",
            der: GLOBALSIGN_ECC_ROOT_R4.as_slice(),
        });
    }
    None
}

fn build_certificate_chain<'a>(
    leaf: &'a [u8],
    intermediate: &'a [u8],
) -> Result<CertificateRef<'a>, EmbeddedTlsError> {
    let mut chain = CertificateRef::with_context(&[]);
    chain.add(CertificateEntryRef::X509(leaf))?;
    chain.add(CertificateEntryRef::X509(intermediate))?;
    Ok(chain)
}

fn log_tls_validation_event(host: &str, anchor: &str, outcome: &str, detail: &str) {
    crate::security::audit::log_event(
        crate::security::audit::events::AuditEvent::TlsValidation {
            host: host.into(),
            anchor: anchor.into(),
            outcome: outcome.into(),
            detail: detail.into(),
        },
    );
}

fn decode_hex_bytes(text: &str) -> Vec<u8> {
    let mut filtered = Vec::new();
    for byte in text.bytes() {
        if !byte.is_ascii_whitespace() {
            filtered.push(byte);
        }
    }

    if !filtered.len().is_multiple_of(2) {
        return Vec::new();
    }

    let mut output = Vec::with_capacity(filtered.len() / 2);
    let mut index = 0usize;
    while index < filtered.len() {
        let Some(high) = hex_nibble(filtered[index]) else {
            return Vec::new();
        };
        let Some(low) = hex_nibble(filtered[index + 1]) else {
            return Vec::new();
        };
        output.push((high << 4) | low);
        index += 2;
    }
    output
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn tls_error(context: &str, error: EmbeddedTlsError) -> NetError {
    NetError::ProtocolError(alloc::format!("{context}: {error}"))
}

fn write_all(
    tls: &mut EmbeddedTlsConnection<'_, KernelTcpStream<'_>, Aes128GcmSha256>,
    mut data: &[u8],
) -> Result<(), NetError> {
    while !data.is_empty() {
        let written = tls
            .write(data)
            .map_err(|error| tls_error("TLS write failed", error))?;
        if written == 0 {
            return Err(NetError::InitializationFailed(
                "TLS write returned zero bytes",
            ));
        }
        data = &data[written..];
    }
    Ok(())
}

struct KernelTcpStream<'a> {
    server_name: &'a str,
    stack: &'a mut NetworkSubsystem,
    connection: Option<TcpConnection>,
    pending_rx: Vec<u8>,
    pending_tx: Vec<u8>,
}

impl<'a> KernelTcpStream<'a> {
    fn connect(
        stack: &'a mut NetworkSubsystem,
        host: &'a str,
        remote_ip: Ipv4Addr,
        port: u16,
    ) -> Result<Self, NetError> {
        let connection = TcpConnection::connect(stack, remote_ip, port, TLS_IO_TIMEOUT_MS)?;
        Ok(Self {
            server_name: host,
            stack,
            connection: Some(connection),
            pending_rx: Vec::new(),
            pending_tx: Vec::new(),
        })
    }
}

impl ErrorType for KernelTcpStream<'_> {
    type Error = ErrorKind;
}

impl Read for KernelTcpStream<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let size = self
            .connection
            .as_mut()
            .ok_or(ErrorKind::NotConnected)?
            .recv(self.stack, buf, TLS_IO_TIMEOUT_MS)
            .map_err(|_| ErrorKind::Other)?;
        if size > 0 {
            self.pending_rx.extend_from_slice(&buf[..size]);
            log_tls_records("RX", self.server_name, &mut self.pending_rx);
        } else {
            serial_println!("[TLS] RX stream closed by peer");
        }
        Ok(size)
    }
}

impl Write for KernelTcpStream<'_> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let written = self
            .connection
            .as_mut()
            .ok_or(ErrorKind::NotConnected)?
            .send(self.stack, buf, TLS_IO_TIMEOUT_MS)
            .map_err(|_| ErrorKind::Other)?;
        if written > 0 {
            self.pending_tx.extend_from_slice(&buf[..written]);
            log_tls_records("TX", self.server_name, &mut self.pending_tx);
        }
        Ok(written)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        serial_println!("[TLS] Waiting for TCP flush...");
        self.connection
            .as_mut()
            .ok_or(ErrorKind::NotConnected)?
            .flush(self.stack, TLS_IO_TIMEOUT_MS)
            .map_err(|_| ErrorKind::Other)?;
        serial_println!("[TLS] TCP flush complete");
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

struct KernelRng {
    state: u64,
    use_rdrand: bool,
}

impl KernelRng {
    fn new() -> Self {
        Self {
            state: seed_entropy(),
            use_rdrand: cpu_has_rdrand(),
        }
    }

    fn using_rdrand(&self) -> bool {
        self.use_rdrand
    }

    fn next_word(&mut self) -> u64 {
        if self.use_rdrand {
            for _ in 0..8 {
                let mut value = 0u64;
                let success = unsafe {
                    // SAFETY: RDRAND is only attempted after CPUID reports support.
                    _rdrand64_step(&mut value)
                };
                if success == 1 {
                    self.state ^= value.rotate_left(17);
                    self.state ^= self.state >> 11;
                    return value ^ self.state;
                }
            }
        }

        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state = self
            .state
            .wrapping_add(crate::arch::x86_64::interrupts::tick_count().rotate_left(9));
        self.state
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

fn seed_entropy() -> u64 {
    let ticks = crate::arch::x86_64::interrupts::tick_count();
    let address_mix = (&ticks as *const u64 as usize as u64).rotate_left(17);
    let boot_mix = crate::boot_complete_ms().rotate_left(29);
    ticks ^ address_mix ^ boot_mix ^ 0x57_41_52_4F_53_54_4C_53
}

fn cpu_has_rdrand() -> bool {
    (__cpuid(1).ecx & (1 << 30)) != 0
}

fn log_tls_records(direction: &str, host: &str, pending: &mut Vec<u8>) {
    loop {
        if pending.len() < 5 {
            return;
        }

        let record_len = u16::from_be_bytes([pending[3], pending[4]]) as usize;
        if pending.len() < 5 + record_len {
            return;
        }

        let content_type = pending[0];
        let version = u16::from_be_bytes([pending[1], pending[2]]);
        let fragment = pending[5..5 + record_len].to_vec();
        log_tls_record(direction, host, content_type, version, &fragment);
        pending.drain(..5 + record_len);
    }
}

fn log_tls_record(direction: &str, host: &str, content_type: u8, version: u16, fragment: &[u8]) {
    serial_println!(
        "[TLS] {} record from {}: type={} ({}) version=0x{:04X} len={}",
        direction,
        host,
        content_type,
        tls_content_type_name(content_type),
        version,
        fragment.len()
    );

    match content_type {
        21 if fragment.len() >= 2 => {
            let level = fragment[0];
            let description = fragment[1];
            serial_println!(
                "[TLS] ALERT received: level={} ({}) desc={} ({})",
                level,
                tls_alert_level_name(level),
                description,
                tls_alert_description_name(description)
            );
        }
        22 if fragment.len() >= 4 => {
            let handshake_type = fragment[0];
            let handshake_len =
                ((fragment[1] as usize) << 16) | ((fragment[2] as usize) << 8) | fragment[3] as usize;
            serial_println!(
                "[TLS] Handshake msg: type={} ({}) len={}",
                handshake_type,
                tls_handshake_type_name(handshake_type),
                handshake_len
            );
            if direction == "TX" && handshake_type == 1 {
                dump_tls_fragment_hex("[TLS] ClientHello", fragment);
            }
        }
        _ => {}
    }
}

fn dump_tls_fragment_hex(label: &str, bytes: &[u8]) {
    for (index, chunk) in bytes.chunks(16).enumerate() {
        let mut line = alloc::string::String::new();
        for byte in chunk {
            line.push_str(&alloc::format!("{:02X} ", byte));
        }
        serial_println!("{} {:03}: {}", label, index, line);
    }
}

fn tls_content_type_name(content_type: u8) -> &'static str {
    match content_type {
        20 => "ChangeCipherSpec",
        21 => "Alert",
        22 => "Handshake",
        23 => "ApplicationData",
        _ => "Unknown",
    }
}

fn tls_handshake_type_name(handshake_type: u8) -> &'static str {
    match handshake_type {
        1 => "ClientHello",
        2 => "ServerHello",
        4 => "NewSessionTicket",
        8 => "EncryptedExtensions",
        11 => "Certificate",
        13 => "CertificateRequest",
        15 => "CertificateVerify",
        20 => "Finished",
        _ => "Unknown",
    }
}

fn tls_alert_level_name(level: u8) -> &'static str {
    match level {
        1 => "Warning",
        2 => "Fatal",
        _ => "Unknown",
    }
}

fn tls_alert_description_name(description: u8) -> &'static str {
    match description {
        0 => "close_notify",
        10 => "unexpected_message",
        20 => "bad_record_mac",
        22 => "record_overflow",
        40 => "handshake_failure",
        42 => "bad_certificate",
        43 => "unsupported_certificate",
        45 => "certificate_expired",
        46 => "certificate_unknown",
        47 => "illegal_parameter",
        48 => "unknown_ca",
        70 => "protocol_version",
        71 => "insufficient_security",
        80 => "internal_error",
        86 => "inappropriate_fallback",
        109 => "missing_extension",
        110 => "unsupported_extension",
        112 => "unrecognized_name",
        116 => "certificate_required",
        120 => "no_application_protocol",
        _ => "unknown_alert",
    }
}
