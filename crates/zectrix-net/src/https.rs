//! HTTPS GET of one resource per connection, over mbedtls-rs and an embassy-net socket.

use core::ffi::CStr;

use embassy_net::{Stack, dns::DnsQueryType, tcp::TcpSocket};
use embassy_time::{Duration, Instant};
use log::info;
use mbedtls_rs::io::Write;
use mbedtls_rs::{
    Certificate, ClientSessionConfig, Session, SessionConfig, SessionError, TlsReference, X509,
};

use crate::http::{self, HeadError, ResponseHead};

/// Socket idle timeout. The whole fetch is bounded by the caller.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(30);

/// What to fetch and whom to trust.
#[derive(Clone, Copy, Debug)]
pub struct Endpoint<'a> {
    /// Host name, used for DNS, SNI, certificate verification and the `Host` header.
    pub host: &'a CStr,
    pub port: u16,
    /// Path plus query, starting with `/`.
    pub path: &'a str,
    /// PEM trust anchor(s) for the server certificate, e.g. [`crate::ISRG_ROOT_X1_PEM`].
    pub ca_pem: &'a CStr,
}

/// A completed fetch. The body is in the caller's buffer, `body[..body_len]`.
#[derive(Debug)]
pub struct Response {
    pub head: ResponseHead,
    pub body_len: usize,
    pub dns_ms: u32,
    pub connect_ms: u32,
    pub tls_ms: u32,
    pub total_ms: u32,
}

/// Where a fetch failed.
#[derive(Debug)]
pub enum FetchError {
    BadHost,
    Dns,
    Connect,
    /// Building the CA chain or the TLS session failed (out of memory, bad certificate).
    TlsSetup(SessionError),
    /// The TLS handshake failed: certificate not trusted, name mismatch, protocol error.
    Handshake(SessionError),
    Io(SessionError),
    /// The request did not fit the request buffer.
    Request,
    Head(HeadError),
    /// The connection closed before the response head was complete.
    Truncated,
    /// The body is larger than the buffer.
    BodyTooLarge,
}

/// `GET https://{host}:{port}{path}`. The server is verified against `ep.ca_pem`.
///
/// `extra` headers are sent as is (e.g. `If-None-Match`, `Authorization`); they must not
/// contain CR or LF (see [`http::write_get`]). The whole request must fit
/// [`http::REQUEST_MAX`] bytes, or the fetch fails with [`FetchError::Request`].
pub async fn get(
    stack: Stack<'_>,
    tls: TlsReference<'_>,
    ep: Endpoint<'_>,
    extra: &[(&str, &str)],
    body: &mut [u8],
) -> Result<Response, FetchError> {
    let t0 = Instant::now();
    let host = ep.host;
    let host_str = host.to_str().map_err(|_| FetchError::BadHost)?;

    let addrs = stack
        .dns_query(host_str, DnsQueryType::A)
        .await
        .map_err(|_| FetchError::Dns)?;
    let ip = *addrs.first().ok_or(FetchError::Dns)?;
    let dns_ms = t0.elapsed().as_millis() as u32;

    let mut rx = [0u8; 4096];
    let mut tx = [0u8; 1024];
    let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
    socket.set_timeout(Some(SOCKET_TIMEOUT));
    socket
        .connect((ip, ep.port))
        .await
        .map_err(|_| FetchError::Connect)?;
    let connect_ms = t0.elapsed().as_millis() as u32 - dns_ms;

    let ca = Certificate::new(X509::PEM(ep.ca_pem)).map_err(|e| FetchError::TlsSetup(e.into()))?;
    let config = SessionConfig::Client(ClientSessionConfig {
        ca_chain: Some(ca),
        server_name: Some(host),
        ..ClientSessionConfig::new()
    });
    let mut session = Session::new(tls, &mut socket, &config).map_err(FetchError::TlsSetup)?;
    let t_tls = Instant::now();
    session.connect().await.map_err(FetchError::Handshake)?;
    let tls_ms = t_tls.elapsed().as_millis() as u32;

    let mut request: heapless::String<{ http::REQUEST_MAX }> = heapless::String::new();
    http::write_get(&mut request, host_str, ep.port, ep.path, extra)
        .map_err(|_| FetchError::Request)?;
    session
        .write_all(request.as_bytes())
        .await
        .map_err(FetchError::Io)?;

    // Read until the end of the head; whatever follows it is the start of the body.
    let mut head_buf = [0u8; 1024];
    let mut filled = 0;
    let head_end = loop {
        if filled == head_buf.len() {
            return Err(FetchError::Head(HeadError::TooLarge));
        }
        let n = session
            .read(&mut head_buf[filled..])
            .await
            .map_err(FetchError::Io)?;
        if n == 0 {
            return Err(FetchError::Truncated);
        }
        filled += n;
        if let Some(end) = http::head_len(&head_buf[..filled]) {
            break end;
        }
    };
    let head = http::parse_head(&head_buf[..head_end]).map_err(FetchError::Head)?;

    let early = filled - head_end;
    if early > body.len() {
        return Err(FetchError::BodyTooLarge);
    }
    body[..early].copy_from_slice(&head_buf[head_end..filled]);
    let mut body_len = early;
    let want = head.content_length;
    if want.is_some_and(|w| w > body.len()) {
        return Err(FetchError::BodyTooLarge);
    }
    loop {
        if want.is_some_and(|w| body_len >= w) {
            break;
        }
        if body_len == body.len() {
            // Buffer full with no length given: anything more is too much.
            let mut probe = [0u8; 1];
            if session.read(&mut probe).await.map_err(FetchError::Io)? != 0 {
                return Err(FetchError::BodyTooLarge);
            }
            break;
        }
        let n = session
            .read(&mut body[body_len..])
            .await
            .map_err(FetchError::Io)?;
        if n == 0 {
            break;
        }
        body_len += n;
    }
    // Best effort: the server closes the connection anyway.
    let _ = session.close().await;

    let total_ms = t0.elapsed().as_millis() as u32;
    info!(
        "https: {host_str}{} -> {} ({body_len} B) dns {dns_ms} tcp {connect_ms} tls {tls_ms} total {total_ms} ms",
        ep.path, head.status
    );
    Ok(Response {
        head,
        body_len,
        dns_ms,
        connect_ms,
        tls_ms,
        total_ms,
    })
}
