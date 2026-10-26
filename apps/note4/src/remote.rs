//! Server screens: fetching screen bundles over HTTPS (docs/INTERACTIVE.md, "Protocol").
//!
//! Built only with `cfg(screen_server)` (build.rs: `SCREEN_URL` is set). Wi-Fi and TLS
//! start the first time a server screen opens and stay up for the rest of the session;
//! the device sleeps soon after anyway.

use core::fmt::Write as _;

use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, with_timeout};
use esp_hal::peripherals::{AES, RSA, SHA, WIFI};
use esp_hal::rng::Trng;
use log::info;
use mbedtls_rs::Tls;
use mbedtls_rs::sys::hook::backend::esp::{EspAccel, EspAccelQueue, EspHooksGuard};
use static_cell::StaticCell;
use zectrix_net::{https, wifi};
use zectrix_screen::view::Label;

// Set by build.rs from the environment or `.env`.
const WIFI_SSID: &str = env!("WIFI_SSID");
const WIFI_PASSWORD: &str = env!("WIFI_PASSWORD");
const SCREEN_KEY: &str = env!("SCREEN_KEY");
const SCREEN_AUTH: &str = concat!("Bearer ", env!("SCREEN_KEY"));
const SCREEN_HOST: &core::ffi::CStr =
    match core::ffi::CStr::from_bytes_with_nul(concat!(env!("SCREEN_HOST"), "\0").as_bytes()) {
        Ok(host) => host,
        Err(_) => panic!("SCREEN_HOST contains a NUL"),
    };
const SCREEN_PORT: u16 = match u16::from_str_radix(env!("SCREEN_PORT"), 10) {
    Ok(port) => port,
    Err(_) => panic!("SCREEN_PORT is not a port number"),
};
/// `SCREEN_URL`'s path and query, then `?` or `&` before `screen=<id>`.
const SCREEN_PATH: &str = env!("SCREEN_PATH");
const SCREEN_SEP: &str = env!("SCREEN_SEP");

#[cfg(screen_custom_ca)]
const SCREEN_CA_PEM: &core::ffi::CStr = match core::ffi::CStr::from_bytes_with_nul(
    concat!(include_str!(env!("SCREEN_CA_FILE")), "\0").as_bytes(),
) {
    Ok(pem) => pem,
    Err(_) => panic!("SCREEN_CA_PEM contains a NUL"),
};
#[cfg(not(screen_custom_ca))]
const SCREEN_CA_PEM: &core::ffi::CStr = zectrix_net::ISRG_ROOT_X1_PEM;

/// Longest screen id; build.rs sizes the request for it (keep in sync).
pub const SCREEN_ID_MAX: usize = 24;

const WIFI_TIMEOUT: Duration = Duration::from_secs(20);
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

static TRNG: StaticCell<Trng> = StaticCell::new();
static ACCEL: StaticCell<EspAccel<'static>> = StaticCell::new();
static QUEUE: StaticCell<EspAccelQueue<'static, 'static>> = StaticCell::new();

/// The peripherals networking needs, kept until the first fetch.
pub struct Parts {
    pub wifi: WIFI<'static>,
    pub sha: SHA<'static>,
    pub rsa: RSA<'static>,
    pub aes: AES<'static>,
}

struct Net {
    stack: embassy_net::Stack<'static>,
    tls: Tls<'static>,
    _hooks: EspHooksGuard<'static>,
}

pub struct Remote {
    spawner: Spawner,
    parts: Option<Parts>,
    net: Option<Net>,
    /// Why networking failed for good (Wi-Fi can be started only once per boot).
    failed: Option<Label>,
}

impl Remote {
    pub fn new(spawner: Spawner, parts: Parts) -> Self {
        Self {
            spawner,
            parts: Some(parts),
            net: None,
            failed: None,
        }
    }

    /// Wi-Fi is up (so the battery ADC may be read again, see docs/ROADMAP.md).
    pub fn connected(&self) -> bool {
        self.net.is_some()
    }

    /// Fetch screen `id` into `body`. Returns the bundle length, or a one-line reason.
    pub async fn fetch(&mut self, id: &str, body: &mut [u8]) -> Result<usize, Label> {
        let net = self.connect().await?;
        let mut path: heapless::String<{ zectrix_net::http::REQUEST_MAX }> =
            heapless::String::new();
        if write!(path, "{SCREEN_PATH}{SCREEN_SEP}screen={id}").is_err() || id.len() > SCREEN_ID_MAX
        {
            return Err(reason(format_args!("screen id too long")));
        }
        let ep = https::Endpoint {
            host: SCREEN_HOST,
            port: SCREEN_PORT,
            path: &path,
            ca_pem: SCREEN_CA_PEM,
        };
        let auth = [("Authorization", SCREEN_AUTH)];
        let headers: &[(&str, &str)] = if SCREEN_KEY.is_empty() { &[] } else { &auth };
        let t = Instant::now();
        let resp = match with_timeout(
            FETCH_TIMEOUT,
            https::get(net.stack, net.tls.reference(), ep, headers, body),
        )
        .await
        {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(reason(format_args!("fetch: {e:?}"))),
            Err(_) => return Err(reason(format_args!("fetch: timeout"))),
        };
        info!(
            "screen {id}: {} {} B in {} ms",
            resp.head.status,
            resp.body_len,
            t.elapsed().as_millis()
        );
        match resp.head.status {
            200 => Ok(resp.body_len),
            401 => Err(reason(format_args!("server: key rejected (401)"))),
            status => Err(reason(format_args!("server: HTTP {status}"))),
        }
    }

    async fn connect(&mut self) -> Result<&mut Net, Label> {
        if let Some(why) = &self.failed {
            return Err(why.clone());
        }
        if self.net.is_none() {
            match self.start().await {
                Ok(net) => self.net = Some(net),
                Err(why) => {
                    self.failed = Some(why.clone());
                    return Err(why);
                }
            }
        }
        Ok(self.net.as_mut().expect("just connected"))
    }

    async fn start(&mut self) -> Result<Net, Label> {
        let parts = self
            .parts
            .take()
            .ok_or_else(|| reason(format_args!("no radio")))?;
        let link = wifi::connect(
            self.spawner,
            parts.wifi,
            WIFI_SSID,
            WIFI_PASSWORD,
            WIFI_TIMEOUT,
        )
        .await
        .map_err(|e| match e {
            wifi::WifiError::Associate(sv) => match (sv.best, sv.last_reason) {
                (Some((ch, rssi, _)), Some(why)) => {
                    reason(format_args!("Wi-Fi ch{ch} {rssi}dBm: {why:?}"))
                }
                (Some((ch, rssi, _)), None) => reason(format_args!(
                    "Wi-Fi ch{ch} {rssi}dBm: no join x{}",
                    sv.attempts
                )),
                (None, _) => reason(format_args!("Wi-Fi: SSID not seen ({} APs)", sv.total)),
            },
            wifi::WifiError::Dhcp(sv) => reason(format_args!(
                "Wi-Fi: no DHCP ({} dBm, link {})",
                sv.rssi.unwrap_or(0),
                sv.link_up
            )),
            e => reason(format_args!("Wi-Fi: {e:?}")),
        })?;
        info!(
            "wifi: {} ch{} {} dBm, assoc {} ms, dhcp {} ms",
            link.ip,
            link.channel,
            link.rssi.unwrap_or(0),
            link.associate_ms,
            link.dhcp_ms
        );
        // The running radio is the entropy source (no ADC TrngSource, see wifi::connect).
        let trng = Trng::try_new().map_err(|_| reason(format_args!("TRNG: no entropy")))?;
        let tls = Tls::new(TRNG.init(trng)).map_err(|e| reason(format_args!("TLS: {e:?}")))?;
        // Hardware SHA/RSA/AES, as on the NOTE4C (software RSA-4096 stalls TCP).
        let accel = ACCEL.init(
            EspAccel::new()
                .with_sha(parts.sha)
                .with_rsa(parts.rsa)
                .with_aes(parts.aes),
        );
        let queue = QUEUE.init(accel.start());
        // SAFETY: the queue is 'static, so it outlives every TLS session.
        let hooks = unsafe { queue.hook() };
        Ok(Net {
            stack: link.stack,
            tls,
            _hooks: hooks,
        })
    }
}

fn reason(args: core::fmt::Arguments<'_>) -> Label {
    let mut l = Label::new();
    let _ = l.write_fmt(args);
    l
}
