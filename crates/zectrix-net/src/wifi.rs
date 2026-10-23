//! Wi-Fi station bring-up: associate, start embassy-net, wait for a DHCP lease.
//!
//! This connects once (with retries) and reports timings, which is what a device that
//! wakes, fetches and sleeps again needs. There is no reconnect task: the next wake
//! simply connects again.

use embassy_executor::Spawner;
use embassy_net::{Ipv4Address, Runner, Stack, StackResources};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use esp_hal::peripherals::WIFI;
use esp_radio::wifi::{
    AuthenticationMethod, AuthenticationMethodConfig, Config, ConnectionError, ControllerConfig,
    DisconnectReason, Interface, WifiController, scan::ScanConfig, sta::StationConfig,
};
use log::{info, warn};
use static_cell::StaticCell;

use crate::counting::Counting;

/// Maximum transmit power in 0.25 dBm steps. esp-radio defaults to 20 (5 dBm), which
/// access points often do not hear: on the NOTE4C, 15 dBm (60) gave runs with an empty
/// scan and `AuthenticationExpired`. 80 = 20 dBm is ESP-IDF's default and what the
/// mbedtls-rs ESP example uses. (esp-radio's docs mention auth failures above ~65 on
/// some hardware; if that shows up here, step down.)
const MAX_TX_POWER: i8 = 80;

/// A connected station. Keep it alive while the network is used: dropping the
/// controller stops Wi-Fi.
pub struct Link {
    pub stack: Stack<'static>,
    pub controller: &'static mut WifiController<'static>,
    pub ip: Ipv4Address,
    /// Received signal strength of the access point, in dBm.
    pub rssi: Option<i32>,
    pub channel: u8,
    /// Time from start until association succeeded.
    pub associate_ms: u32,
    /// Time from association until the DHCP lease.
    pub dhcp_ms: u32,
}

/// What the station saw while trying to associate, for diagnostics on the panel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Survey {
    /// Access points with our SSID in the scan.
    pub matches: u8,
    /// The strongest of them: channel, signal in dBm, advertised security.
    pub best: Option<(u8, i8, Option<AuthenticationMethod>)>,
    /// Access points of any SSID in the scan.
    pub total: u8,
    pub attempts: u8,
    /// Reason the access point gave for the last failed attempt.
    pub last_reason: Option<DisconnectReason>,
    /// After association: signal in dBm, whether embassy-net saw the link come up, and
    /// whether the station was still associated when DHCP gave up.
    pub rssi: Option<i32>,
    pub link_up: bool,
    pub still_associated: bool,
}

/// Why the station did not come up.
#[derive(Clone, Copy, Debug)]
pub enum WifiError {
    /// The SSID or password does not fit esp-radio's limits.
    BadCredentials,
    /// The Wi-Fi driver did not start.
    Driver,
    /// No association within the timeout (wrong password, AP out of range, …).
    Associate(Survey),
    /// Associated, but no DHCP lease within the timeout.
    Dhcp(Survey),
}

static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
// Static so that an early return never drops the controller (which would stop Wi-Fi)
// while the embassy-net runner still drives the interface.
static CONTROLLER: StaticCell<WifiController<'static>> = StaticCell::new();

/// Connect to `ssid` with a WPA2 (or WPA2/WPA3 mixed) `password` and wait for an IPv4 lease.
///
/// `timeout` bounds association and DHCP separately. Can be called once per boot.
///
/// Do not keep an ADC entropy source (`esp_hal::rng::TrngSource`) alive while calling
/// this: like ESP-IDF's `bootloader_random_enable`, it shares the RF/ADC path and the
/// radio then hears nothing (a scan finds zero access points). The running radio is an
/// entropy source itself, so `esp_hal::rng::Trng::try_new()` works after this returns.
pub async fn connect(
    spawner: Spawner,
    wifi: WIFI<'static>,
    ssid: &str,
    password: &str,
    timeout: Duration,
) -> Result<Link, WifiError> {
    let t0 = Instant::now();
    let station = StationConfig::default()
        .with_ssid(ssid.try_into().map_err(|_| WifiError::BadCredentials)?)
        // This is the weakest security the driver accepts. WPA2 is ESP-IDF's recommended
        // threshold; WPA2/WPA3 mixed networks are still joined (PMF is enabled).
        .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
            password.try_into().map_err(|_| WifiError::BadCredentials)?,
        ));
    let controller = WifiController::new(
        wifi,
        ControllerConfig::default().with_initial_config(Config::Station(station)),
    )
    .map_err(|_| WifiError::Driver)?;
    let controller = CONTROLLER.init(controller);
    if controller.set_max_tx_power(MAX_TX_POWER).is_err() {
        warn!("wifi: could not raise TX power");
    }

    // The radio is running now, so the hardware RNG produces true random numbers.
    let rng = esp_hal::rng::Rng::new();
    let seed = (u64::from(rng.random()) << 32) | u64::from(rng.random());
    let (stack, runner) = embassy_net::new(
        Counting(Interface::station()),
        embassy_net::Config::dhcpv4(Default::default()),
        RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(runner).map_err(|_| WifiError::Driver)?);

    // Scan once so a failure can say whether the network was even visible, on which
    // channel and with which security.
    let mut survey = Survey::default();
    // A scan right after the radio starts can come back empty; give it a moment.
    Timer::after(Duration::from_secs(2)).await;
    match controller
        .scan_async(&ScanConfig::default().with_max(30))
        .await
    {
        Ok(aps) => {
            survey.total = aps.len().min(255) as u8;
            for ap in aps.iter().filter(|ap| ap.ssid.as_str() == ssid) {
                survey.matches += 1;
                if survey
                    .best
                    .is_none_or(|(_, rssi, _)| ap.signal_strength > rssi)
                {
                    survey.best = Some((ap.channel, ap.signal_strength, ap.auth_method));
                }
            }
            info!(
                "wifi: scan {} APs, {} match: {:?}",
                survey.total, survey.matches, survey.best
            );
        }
        Err(e) => warn!("wifi: scan failed: {e:?}"),
    }

    let attempt = with_timeout(timeout, async {
        loop {
            survey.attempts = survey.attempts.saturating_add(1);
            match controller.connect_async().await {
                Ok(info) => break info,
                Err(e) => {
                    warn!("wifi: connect failed: {e:?}");
                    if let ConnectionError::Failed(info) = e {
                        survey.last_reason = Some(info.reason);
                    }
                    Timer::after(Duration::from_millis(1000)).await;
                }
            }
        }
    })
    .await;
    let info = attempt.map_err(|_| WifiError::Associate(survey))?;
    let associate_ms = t0.elapsed().as_millis() as u32;
    info!(
        "wifi: associated on channel {} in {associate_ms} ms",
        info.channel
    );

    let t1 = Instant::now();
    survey.rssi = controller.rssi().ok();
    // DHCP gets longer than association: some routers answer the first DISCOVER slowly.
    let lease = with_timeout(timeout + Duration::from_secs(10), async {
        loop {
            survey.link_up |= stack.is_link_up();
            if let Some(cfg) = stack.config_v4() {
                break cfg.address.address();
            }
            Timer::after(Duration::from_millis(100)).await;
        }
    })
    .await;
    let ip = match lease {
        Ok(ip) => ip,
        Err(_) => {
            survey.still_associated = controller.is_connected();
            survey.best = survey
                .best
                .map(|(_, rssi, auth)| (info.channel, rssi, auth));
            warn!("wifi: no DHCP lease: {survey:?}");
            return Err(WifiError::Dhcp(survey));
        }
    };
    let dhcp_ms = t1.elapsed().as_millis() as u32;
    let rssi = controller.rssi().ok();
    info!("wifi: {ip} after {dhcp_ms} ms, rssi {rssi:?}");

    Ok(Link {
        stack,
        controller,
        ip,
        rssi,
        channel: info.channel,
        associate_ms,
        dhcp_ms,
    })
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Counting<Interface>>) -> ! {
    runner.run().await
}
