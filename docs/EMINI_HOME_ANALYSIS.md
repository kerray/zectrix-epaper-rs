# Analysis of fiedoruk/emini-home 0.6.0

Source: <https://github.com/fiedoruk/emini-home> (MIT, © 2026 Tomasz Fiedoruk), analysed at
commit `afd1239` ("emini Home 0.6.0"). The author says 0.6.0 is the **final release**. The code is
ESP-IDF v6.0 C and supports only the four-colour **NOTE4C**. Line references below are to that
commit.

## Why it matters to us

- It is the best-documented third-party firmware for the NOTE4C. Its panel driver, pin map,
  battery curve and flash-layout handling were all checked on a real unit.
- It keeps the factory bootloader and writes only two areas: the partition table at `0x8000`
  and the app at `0x20000`. We copy the same approach (see [FLASHING.md](FLASHING.md)).
- It does **not** support the monochrome NOTE4.

## Architecture (`firmware/main`)

| Module | Role |
| --- | --- |
| `home_main.c` | `app_main` (:831), button gestures, display state machine (`home_loop_step` :554) |
| `home_panel.c` | 4-colour display driver (adapted from LazyYoun, MIT) |
| `home_power.c` | automatic light sleep, wake-up pins, power-management locks |
| `home_battery.c` | battery ADC and charger pins |
| `home_network.c` / `home_wake.c` | Wi-Fi, SNTP, radio on/off policy (the "Breath" and "Open" modes), data-fetch task |
| `home_fetch.c` | its own HTTP/1.1 client on top of esp-tls (TLS 1.2, CA bundle, ETag, 128 KB cap) |
| `home_parse*.c` | parsers for MET Norway, RSS/Atom and Open-Meteo air data |
| `home_render.c` | pure-C renderer (2753 lines, no heap use, dithering) |
| `home_sky.c` | sun and moon positions (Meeus/NOAA) |
| `home_server.c` | local web panel, 6-digit pairing that issues a bearer token |
| `home_store.c` | NVS records written to two alternating slots with a CRC |
| `home_discovery.c` | mDNS (`home-xxxx`, `_http._tcp`) |
| `home_usb.c` | JSON-lines maintenance protocol over USB-Serial-JTAG |

**Tasks:**

- main loop
- `home_sources` (24 KB stack, priority 4)
- `home_battery` (3 KB, priority 1)
- `home_net_control` (6 KB, priority 5)
- `home_usb` (4 KB, priority 3)
- httpd

Shared state sits in one struct behind a mutex. The firmware hashes each frame with SHA-256 and
skips the refresh when nothing changed.

## Display driver: NOTE4C, 4-colour (`home_panel.c`)

**Pins (:43-50):**

| Function | GPIO |
| --- | --- |
| Rail | 6 |
| BUSY | 8 |
| RST | 9 |
| DC | 10 |
| CS | 11 |
| SCK | 12 |
| MOSI | 13 |

The driver uses host SPI3 at 40 MHz, mode 0, MOSI only, and drives CS in software. It sends one
row (100 bytes) per SPI transaction.

**BUSY** is **low while busy**. The driver polls every 50 ms, with a 120 s timeout per stage.
After the refresh command, BUSY must go low within 1 s, or the driver reports a fault.

**Frame:** 400×300 at 2 bits per pixel, MSB first, 30,000 bytes. Colour values: 0 black,
1 white, 2 yellow, 3 red.

**`home_panel_show` (:291-325):**

1. Rail GPIO6 high (with `gpio_hold_en`), wait 10 ms.
2. RST high, wait 10 ms; RST low, wait 20 ms; RST high, wait 10 ms; wait for BUSY.
3. `0xE9` ← `0x01` (load from OTP).
4. `0x10`, wait for BUSY, then 300 × 100 bytes of pixel data.
5. `0x04` (power on), wait for BUSY, wait 10 ms.
6. `0x12` ← `0x00` (refresh). Check that BUSY went low, then wait for BUSY to go high again.

**Power-off (:271-277):**

1. `0x02` ← `0x00`, wait for BUSY, wait 20 ms.
2. `0x07` ← `0xA5` (deep sleep).
3. Rail low.

The monochrome `00 2F 2E` and the temperature/LUT commands were deliberately left out (:301). A
NOTE4 driver needs them, plus a 1 bpp frame (15,000 bytes) and probably partial refresh. Take
those from the vendor/LazyYoun sources; this repo does not have them.

## Power

- **At boot:** the firmware drives GPIO17 (power latch) high and holds it. It drives GPIO21, 42
  and 46 low. Buttons 39, 18 and 0 are inputs with pull-ups.
- **Sleep:** automatic light sleep only, at 160/80 MHz. Buttons wake the chip on a low level.
  PM locks keep it awake while a USB cable is connected, while charging, and during the setup
  and maintenance windows.
- **No shutdown path:** there is no deep sleep, and GPIO17 is never released. Any real off or
  deep-sleep behaviour is new work for us.
- **Button gestures:**

  | Gesture | Action |
  | --- | --- |
  | OK held 2 s | opens or closes setup |
  | Up held 2 s | holds the current picture |
  | Down held 2–5 s | refreshes data |
  | Down held 5 s | switches display language |

  GPIO18 is shared with the power key.
- **Battery:** GPIO4 (ADC1 ch3) at 12 dB with curve-fitting calibration and a ×2 divider. It
  averages 10 samples every 30 s and accepts 2800–4350 mV. Charger pins: CHG GPIO2 (low =
  charging), FULL GPIO1 (high = full).
  Percent = `(-v² + 9016v - 19189000) / 10000`.

## Networking and storage

- **Setup:** SoftAP plus station, WPA2 with a random password, 5-minute window. Wi-Fi
  credentials are not kept in the IDF's own NVS (`nvs_enable=0`); Home keeps them itself.
  Power save is MAX_MODEM, listen interval 3.
- **Settings:** the `home_nvs` partition at `0x10000` (64 KB), which Home creates in an erased
  gap of the factory layout. The factory `nvs`, `ota_1` and `assets` areas are never touched.

## `tools/preflight.py`

An offline check of two 16 MiB flash backups:

- the backups are identical;
- the bootloader and otadata hashes match the tested unit;
- the partition table is either the factory one or Home's;
- optionally, the app image is valid.

It cannot tell a NOTE4 from a NOTE4C.

## Licensing notes for reuse

- The panel driver and battery curve come from LazyYoun/youn-ink-fourcolor-firmware@51812e4
  (MIT). If you copy them, keep the notice at `home_panel.c:7-28`.
- Bundled components: cJSON (MIT), Nayuki QR (MIT), mDNS (Apache-2.0), the Mozilla CA bundle
  (MPL-2.0), and fonts (OFL).
- Porting logic into Rust counts as a derivative work, so keep MIT attribution in
  `THIRD_PARTY_NOTICES.md`.
