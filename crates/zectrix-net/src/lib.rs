//! Networking for the ZECTRIX NOTE4 and NOTE4C: Wi-Fi station, HTTPS GET of one resource
//! per connection, and the small HTTP/1.1 parser behind it.
//!
//! The parser in [`http`] and the build-script helper [`dotenv`] always build and are tested
//! on the host. The `hal` feature adds the ESP32-S3 parts: `wifi` (esp-radio +
//! embassy-net), `https` (mbedtls-rs) and `counting`.

#![cfg_attr(not(test), no_std)]

pub mod dotenv;
pub mod http;

#[cfg(feature = "hal")]
pub mod counting;
#[cfg(feature = "hal")]
pub mod https;
#[cfg(feature = "hal")]
pub mod wifi;

/// ISRG Root X1 (Let's Encrypt), a sensible default trust anchor for `https::Endpoint`:
/// every Let's Encrypt certificate served with the default chain (and e.g.
/// api.open-meteo.com) chains to it. A server that serves the alternate ISRG Root X2
/// (ECDSA) chain does not verify against it; give that server's root as the trust anchor.
/// Valid until 2035-06-04.
/// SHA-256 96:BC:EC:06:26:49:76:F3:74:60:77:9A:CF:28:C5:A7:CF:E8:A3:C0:AA:E1:1A:8F:FC:EE:05:C0:BD:DF:08:C6
pub const ISRG_ROOT_X1_PEM: &core::ffi::CStr = match core::ffi::CStr::from_bytes_with_nul(
    concat!(include_str!("../certs/isrgrootx1.pem"), "\0").as_bytes(),
) {
    Ok(pem) => pem,
    Err(_) => panic!("certificate is not a NUL-free text file"),
};
