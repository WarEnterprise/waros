use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::tcp::TcpConnection;
use super::{ipv4::Ipv4Addr, NetError, NetworkSubsystem};

pub struct HttpResponse {
    pub status_code: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct UrlParts {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub path: String,
}

pub fn parse_url(url: &str) -> Result<UrlParts, NetError> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or(NetError::InitializationFailed("URL is missing a scheme"))?;
    let path_start = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..path_start];
    let path = if path_start < rest.len() {
        &rest[path_start..]
    } else {
        "/"
    };

    let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|_| NetError::InitializationFailed("invalid URL port"))?;
        (host.to_string(), port)
    } else {
        let default_port = match scheme {
            "http" => 80,
            "https" => 443,
            _ => return Err(NetError::InitializationFailed("unsupported URL scheme")),
        };
        (authority.to_string(), default_port)
    };

    Ok(UrlParts {
        scheme: scheme.to_string(),
        host,
        port,
        path: path.to_string(),
    })
}

pub fn http_get(stack: &mut NetworkSubsystem, url: &str) -> Result<HttpResponse, NetError> {
    http_get_with_headers(stack, url, &[])
}

pub fn http_get_with_headers(
    stack: &mut NetworkSubsystem,
    url: &str,
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse, NetError> {
    let parts = parse_url(url)?;
    match parts.scheme.as_str() {
        "http" => plain_request(stack, "GET", &parts, extra_headers, None),
        "https" => {
            super::tls::TlsConnection::https_request(stack, "GET", &parts, extra_headers, None)
        }
        _ => Err(NetError::InitializationFailed("unsupported URL scheme")),
    }
}

pub fn http_post(
    stack: &mut NetworkSubsystem,
    url: &str,
    content_type: &str,
    body: &[u8],
) -> Result<HttpResponse, NetError> {
    http_post_with_headers(stack, url, content_type, body, &[])
}

pub fn http_post_with_headers(
    stack: &mut NetworkSubsystem,
    url: &str,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> Result<HttpResponse, NetError> {
    let parts = parse_url(url)?;
    match parts.scheme.as_str() {
        "http" => plain_request(stack, "POST", &parts, extra_headers, Some((content_type, body))),
        "https" => super::tls::TlsConnection::https_request(
            stack,
            "POST",
            &parts,
            extra_headers,
            Some((content_type, body)),
        ),
        _ => Err(NetError::InitializationFailed("unsupported URL scheme")),
    }
}

fn plain_request(
    stack: &mut NetworkSubsystem,
    method: &str,
    parts: &UrlParts,
    extra_headers: &[(&str, &str)],
    body: Option<(&str, &[u8])>,
) -> Result<HttpResponse, NetError> {
    let remote_ip = stack.resolve_host(&parts.host)?;
    let mut connection = TcpConnection::connect(stack, remote_ip, parts.port, 5_000)?;

    let mut request = alloc::format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: WarOS/{}\r\nConnection: close\r\n",
        parts.path, parts.host, crate::KERNEL_VERSION
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

    let _ = connection.send(stack, request.as_bytes(), 5_000)?;
    if let Some((_, body_bytes)) = body {
        let _ = connection.send(stack, body_bytes, 5_000)?;
    }

    let response = connection.read_to_end(stack, 10_000)?;
    connection.close(stack, 2_000)?;
    parse_response(&response)
}

pub(crate) fn parse_response(bytes: &[u8]) -> Result<HttpResponse, NetError> {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .ok_or(NetError::InitializationFailed("HTTP response missing headers"))?;

    let header_text = core::str::from_utf8(&bytes[..header_end])
        .map_err(|_| NetError::InitializationFailed("HTTP headers are not UTF-8"))?;
    let mut lines = header_text.split("\r\n").filter(|line| !line.is_empty());
    let status_line = lines
        .next()
        .ok_or(NetError::InitializationFailed("HTTP response missing status line"))?;
    let status_code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(NetError::InitializationFailed("HTTP status line is invalid"))?;

    let mut headers = Vec::new();
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_string();
            let value = value.trim().to_string();
            if name.eq_ignore_ascii_case("Transfer-Encoding")
                && value.eq_ignore_ascii_case("chunked")
            {
                chunked = true;
            }
            headers.push((name, value));
        }
    }

    let body = if chunked {
        decode_chunked_body(&bytes[header_end..])?
    } else {
        bytes[header_end..].to_vec()
    };

    Ok(HttpResponse {
        status_code,
        headers,
        body,
    })
}

fn decode_chunked_body(bytes: &[u8]) -> Result<Vec<u8>, NetError> {
    let mut cursor = 0usize;
    let mut body = Vec::new();
    while cursor < bytes.len() {
        let size_end = bytes[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| cursor + offset)
            .ok_or(NetError::InitializationFailed("invalid chunked HTTP body"))?;
        let size_str = core::str::from_utf8(&bytes[cursor..size_end])
            .map_err(|_| NetError::InitializationFailed("invalid chunk size"))?;
        let size = usize::from_str_radix(size_str.trim(), 16)
            .map_err(|_| NetError::InitializationFailed("invalid chunk size"))?;
        cursor = size_end + 2;
        if size == 0 {
            break;
        }
        let end = cursor.saturating_add(size);
        if end > bytes.len() {
            return Err(NetError::InitializationFailed("chunk exceeds response length"));
        }
        body.extend_from_slice(&bytes[cursor..end]);
        cursor = end.saturating_add(2);
    }
    Ok(body)
}

#[allow(dead_code)]
fn _ipv4_to_string(ip: Ipv4Addr) -> String {
    ip.to_string()
}
