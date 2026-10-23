//! Minimal HTTP/1.1 client pieces for fetching one resource per connection.
//!
//! The device only ever does `GET` with `Connection: close`, so this module only needs to
//! write a request and parse a response head. The body is either `Content-Length` bytes or
//! everything up to the close. The request line says HTTP/1.0 (with a `Host` header), so a
//! conforming server never answers with chunked transfer encoding; if one does anyway,
//! [`parse_head`] rejects it rather than decoding it.

use core::fmt::Write as _;

/// Size of the request buffer in `https::get` (request line, `Host`, the
/// fixed headers and the caller's extra headers). `apps/note4c/build.rs` checks at build time
/// that the configured URL and key leave room for the headers the firmware adds.
pub const REQUEST_MAX: usize = 768;

/// Longest ETag we keep. Longer ones are treated as absent (the frame is then always shown).
pub const ETAG_MAX: usize = 64;

/// Things that can go wrong while parsing a response head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadError {
    /// The status line is not `HTTP/1.x NNN ...`.
    BadStatusLine,
    /// A header line has no colon, or a header value is not valid UTF-8.
    BadHeader,
    /// `Content-Length` is not a decimal number.
    BadContentLength,
    /// The server used `Transfer-Encoding: chunked`, which this client does not decode.
    Chunked,
    /// The head did not fit into the buffer.
    TooLarge,
}

/// The parts of a response head the firmware uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResponseHead {
    pub status: u16,
    pub content_length: Option<usize>,
    /// Entity tag including its quotes, as the server sent it (to echo in `If-None-Match`).
    pub etag: Option<heapless::String<ETAG_MAX>>,
    /// `X-Next-Refresh: <seconds>`, the server's choice of when to wake next.
    pub next_refresh_s: Option<u32>,
    /// `Content-Type` starts with `application/octet-stream`.
    pub octet_stream: bool,
}

/// The parts of an `https://` URL, as [`parse_https_url`] returns them.
#[derive(Debug, PartialEq, Eq)]
pub struct HttpsUrl<'a> {
    pub host: &'a str,
    pub port: u16,
    /// Path plus query, always starting with `/`.
    pub path: &'a str,
}

/// Split `https://host[:port][/path][?query]` into its parts. Anything else (another
/// scheme, user info, a fragment, an IPv6 literal, an empty host, a bad port, or a query
/// without a path such as `https://host?q`) is `None`.
///
/// The host is not otherwise checked. An IPv4 literal parses, but `https::get` passes the
/// host to mbedtls as the server name (SNI and certificate check), so the server
/// certificate must be issued for that IP address; most public CAs do not issue those, and
/// a host name is the normal case.
pub fn parse_https_url(url: &str) -> Option<HttpsUrl<'_>> {
    let rest = url.strip_prefix("https://")?;
    if rest.contains('#') || rest.contains(char::is_whitespace) {
        return None;
    }
    let split = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(split);
    if authority.contains('@') || authority.starts_with('[') {
        return None;
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok().filter(|p| *p != 0)?),
        None => (authority, 443),
    };
    if host.is_empty() {
        return None;
    }
    let path = match path {
        "" => "/",
        p if p.starts_with('?') => return None,
        p => p,
    };
    Some(HttpsUrl { host, port, path })
}

/// Write a `GET` request into `out`. `extra` headers are written verbatim, one per entry,
/// for example `("If-None-Match", "\"abc\"")` or `("Authorization", "Bearer …")`.
/// The `Host` header carries the port only when it is not 443.
///
/// Nothing is escaped: callers must make sure `host`, `path` and the header names and
/// values contain no CR or LF (or other control characters), or they could inject headers.
pub fn write_get<const N: usize>(
    out: &mut heapless::String<N>,
    host: &str,
    port: u16,
    path: &str,
    extra: &[(&str, &str)],
) -> core::fmt::Result {
    write!(out, "GET {path} HTTP/1.0\r\nHost: {host}")?;
    if port != 443 {
        write!(out, ":{port}")?;
    }
    out.push_str("\r\n").map_err(|_| core::fmt::Error)?;
    write!(
        out,
        "User-Agent: zectrix-epaper-rs/{}\r\n",
        env!("CARGO_PKG_VERSION")
    )?;
    out.push_str("Accept-Encoding: identity\r\nConnection: close\r\n")
        .map_err(|_| core::fmt::Error)?;
    for (name, value) in extra {
        write!(out, "{name}: {value}\r\n")?;
    }
    out.push_str("\r\n").map_err(|_| core::fmt::Error)
}

/// Find the end of the head (`\r\n\r\n`) in `buf`. Returns the number of bytes in the head
/// including the blank line, or `None` if more data is needed.
pub fn head_len(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Parse a complete response head (as delimited by [`head_len`]).
pub fn parse_head(head: &[u8]) -> Result<ResponseHead, HeadError> {
    let text = core::str::from_utf8(head).map_err(|_| HeadError::BadHeader)?;
    let mut lines = text.split("\r\n");

    let status_line = lines.next().ok_or(HeadError::BadStatusLine)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    if !version.starts_with("HTTP/1.") {
        return Err(HeadError::BadStatusLine);
    }
    let status = parts
        .next()
        .filter(|s| s.len() == 3)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or(HeadError::BadStatusLine)?;

    let mut head = ResponseHead {
        status,
        ..ResponseHead::default()
    };
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or(HeadError::BadHeader)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            head.content_length = Some(value.parse().map_err(|_| HeadError::BadContentLength)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if value.to_ascii_lowercase_contains("chunked") {
                return Err(HeadError::Chunked);
            }
        } else if name.eq_ignore_ascii_case("etag") {
            head.etag = heapless::String::try_from(value).ok();
        } else if name.eq_ignore_ascii_case("x-next-refresh") {
            head.next_refresh_s = value.parse().ok();
        } else if name.eq_ignore_ascii_case("content-type") {
            head.octet_stream = value
                .get(..24)
                .is_some_and(|v| v.eq_ignore_ascii_case("application/octet-stream"));
        }
    }
    Ok(head)
}

trait ContainsIgnoreCase {
    fn to_ascii_lowercase_contains(&self, needle: &str) -> bool;
}

impl ContainsIgnoreCase for str {
    fn to_ascii_lowercase_contains(&self, needle: &str) -> bool {
        self.as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_get_with_extra_headers() {
        let mut s: heapless::String<256> = heapless::String::new();
        write_get(
            &mut s,
            "example.com",
            443,
            "/api/version",
            &[("If-None-Match", "\"x1\"")],
        )
        .unwrap();
        assert!(s.starts_with("GET /api/version HTTP/1.0\r\nHost: example.com\r\n"));
        assert!(s.contains("Connection: close\r\n"));
        assert!(s.contains("If-None-Match: \"x1\"\r\n"));
        assert!(s.ends_with("\r\n\r\n"));
    }

    #[test]
    fn host_header_carries_non_default_port() {
        let mut s: heapless::String<256> = heapless::String::new();
        write_get(&mut s, "example.com", 8443, "/", &[]).unwrap();
        assert!(s.starts_with("GET / HTTP/1.0\r\nHost: example.com:8443\r\n"));
    }

    #[test]
    fn parses_https_urls() {
        assert_eq!(
            parse_https_url("https://example.com/frame?device=note4c&screen=weather"),
            Some(HttpsUrl {
                host: "example.com",
                port: 443,
                path: "/frame?device=note4c&screen=weather"
            })
        );
        assert_eq!(
            parse_https_url("https://frames.example.org:8443"),
            Some(HttpsUrl {
                host: "frames.example.org",
                port: 8443,
                path: "/"
            })
        );
        for bad in [
            "http://example.com/",
            "https://",
            "https://:443/",
            "https://example.com:0/",
            "https://example.com:99999/",
            "https://user@example.com/",
            "https://example.com/#frag",
            "https://example.com?q",
            "https://[::1]/",
            "https://exa mple.com/",
        ] {
            assert_eq!(parse_https_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn finds_head_end() {
        assert_eq!(head_len(b"HTTP/1.1 200 OK\r\nA: b\r\n\r\nbody"), Some(25));
        assert_eq!(head_len(b"HTTP/1.1 200 OK\r\nA: b\r\n"), None);
    }

    #[test]
    fn parses_frame_response() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
            content-length: 30000\r\nETag: \"9f2c\"\r\nX-Next-Refresh: 7200\r\n\r\n";
        let h = parse_head(raw).unwrap();
        assert_eq!(h.status, 200);
        assert_eq!(h.content_length, Some(30_000));
        assert_eq!(h.etag.as_deref(), Some("\"9f2c\""));
        assert_eq!(h.next_refresh_s, Some(7200));
        assert!(h.octet_stream);
    }

    #[test]
    fn parses_not_modified_without_body() {
        let h = parse_head(b"HTTP/1.1 304 Not Modified\r\nETag: W/\"a\"\r\n\r\n").unwrap();
        assert_eq!(h.status, 304);
        assert_eq!(h.content_length, None);
        assert_eq!(h.etag.as_deref(), Some("W/\"a\""));
    }

    #[test]
    fn rejects_chunked_and_garbage() {
        assert_eq!(
            parse_head(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: Chunked\r\n\r\n"),
            Err(HeadError::Chunked)
        );
        assert_eq!(
            parse_head(b"FTP 200 OK\r\n\r\n"),
            Err(HeadError::BadStatusLine)
        );
        assert_eq!(
            parse_head(b"HTTP/1.1 2000 OK\r\n\r\n"),
            Err(HeadError::BadStatusLine)
        );
        assert_eq!(
            parse_head(b"HTTP/1.1 200 OK\r\nContent-Length: lots\r\n\r\n"),
            Err(HeadError::BadContentLength)
        );
        assert_eq!(
            parse_head(b"HTTP/1.1 200 OK\r\nnocolon\r\n\r\n"),
            Err(HeadError::BadHeader)
        );
    }

    #[test]
    fn overlong_etag_is_dropped() {
        let mut raw = std::string::String::from("HTTP/1.1 200 OK\r\nETag: \"");
        raw.push_str(&"a".repeat(100));
        raw.push_str("\"\r\n\r\n");
        assert_eq!(parse_head(raw.as_bytes()).unwrap().etag, None);
    }
}
