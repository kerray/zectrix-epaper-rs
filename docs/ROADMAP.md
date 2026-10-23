# Roadmap and architecture

## Goals

| Device | First milestone | Endgame |
| --- | --- | --- |
| **NOTE4C** (4-colour) | A frame board: show a server-rendered 400×300 four-colour image (for example a weather forecast), sleep until the server says otherwise | Same, polished; optional push-to-talk voice assistant (audio only) |
| **NOTE4** (mono) | A frame board in 1 bpp (for example a calendar) | Interactive screens with partial refresh ([INTERACTIVE.md](INTERACTIVE.md)): lists moved with Up/Down, actions on OK (for example lights), the last voice answer as a QR code; push-to-talk voice assistant |

## Architecture: thin device, the server does the work

Data sources need OAuth (calendars), API keys (home automation, LLMs) or heavy parsing
(weather). The display needs careful dithering. Both jobs are easier and safer on a server
than in firmware: the layout can be tuned without reflashing, fonts and any script's
diacritics are plain Python (Pillow), and the firmware stays small and stable (Wi-Fi, HTTPS
GET, panel, sleep, buttons). Any HTTPS server works: a small Python service, a workflow
engine such as Windmill or Node-RED, a Home Assistant add-on.

```
┌────────────── server ────────────────┐           ┌──────── device ────────┐
│ schedule / webhook                   │           │ wake (timer / button)  │
│  e.g. weather → rules →              │  HTTPS    │ Wi-Fi up               │
│       render 400×300 2bpp BWRY ──────┼──────────▶│ GET FRAME_URL + ETag   │
│  e.g. calendars → render 1bpp        │  304 if   │ show frame, panel off  │
│                                      │ unchanged │ deep sleep until next  │
│  act on POSTed events (planned)  ◀───┼───────────│ POST /event {button}   │
└──────────────────────────────────────┘           └────────────────────────┘
```

- **Frame protocol v1:** [FRAME_PROTOCOL.md](FRAME_PROTOCOL.md). The device sends
  `If-None-Match`, a bearer key and device-health headers; the server answers with a raw
  controller frame (`application/octet-stream`), `ETag` and `X-Next-Refresh`.
  - NOTE4C: 30,000 B (2 bpp, codes 00 black, 01 white, 10 yellow, 11 red).
  - NOTE4 (planned): 15,000 B (1 bpp, 1 = white).
- **Configuration** is compiled in for now (Wi-Fi, `FRAME_URL`, `FRAME_KEY`, optional
  `FRAME_CA_PEM`; see the README). Settings in NVS with a provisioning mode would allow a
  prebuilt release image.
- **Fallback:** if a fetch fails, the device keeps the last image and retries. Rendering on
  the device (as emini-home does) remains possible later.
- **Interactive screens** (NOTE4 endgame, [INTERACTIVE.md](INTERACTIVE.md)): the server sends
  a list screen as a base frame plus one highlight strip per row. Up/Down move the selection
  locally with **partial refresh** and no network. OK sends `POST /event`, and the response is
  a new screen or a patch. The NOTE4C does full refreshes only (10–25 s).
- **Reachability:** the device must reach the server from its Wi-Fi. A server that only
  listens on a VPN needs a public HTTPS route (reverse proxy) for the frame and event
  endpoints.
- **Voice assistant** (planned, both devices): hold a button and speak; the answer comes from
  the speaker. The device streams PCM over WSS to a small relay service, which talks to a
  speech model or an LLM with tools. The endpoint and device key are build-time settings, like
  the frame URL. The display is not used, except that the NOTE4 may show the last answer's
  summary and QR code with one partial refresh.

## Milestones

### M0: bring-up ✅

- [x] `ssd2683` driver: 4-colour full refresh, mono full refresh; host tests of the command
  sequences
- [x] Board crate: pins, power latch with pad hold, battery ADC and percentage, deep sleep on
  OK or a timer
- [x] `apps/note4c` and `apps/note4` test cards
- [x] Flash recipes that keep the factory bootloader, with per-device partition-table checks
- [x] NOTE4C on hardware: backup, factory partition table confirmed, colours, refresh
  (~20 s), latch, wake from deep sleep on OK
- [x] NOTE4 on hardware: partition table read (`partitions/note4-devkit-16m.csv`); test card
  flashed with `just flash note4` and shown (2026-10-23)
- [ ] Photograph the panel FPC labels (the part numbers are still open questions)

### M1: networking ✅

`crates/zectrix-net`, working on the NOTE4C:

- [x] esp-rtos 0.4 + Embassy + esp-radio `=1.0.0-beta.1` (beta.2 is yanked) station and DHCP.
- [x] HTTPS GET with mbedtls-rs 0.3 (TLS 1.3), one configurable trust anchor (default ISRG
  Root X1), HTTP/1.0 requests (no chunked bodies), host-tested response-head parser with ETag
  and `X-Next-Refresh`.
- [x] Results and the previous run's panic message are drawn on the panel (USB/IP logs are
  unreliable, see [FLASHING.md](FLASHING.md)).
- [x] Hardware SHA/RSA/AES for mbedtls (`EspAccel`).
- [ ] Settings in NVS: `esp-nvs` on the free gap at `0x10000` (as emini-home does) or on the
  factory `nvs` partition, plus a provisioning mode (SoftAP or BLE).
- [ ] SNTP, then mbedtls `hook-wall-clock` so certificate dates are checked. Also consider the
  PCF8563 RTC with its INT on GPIO5 for timed wakes.

**Lessons from bring-up (keep these):**
- No ADC entropy source (`esp_hal::rng::TrngSource`) while Wi-Fi runs: like ESP-IDF's
  `bootloader_random_enable`, it shares the RF/ADC path and the radio hears nothing. The
  running radio is itself an entropy source for `Trng`.
- Read the battery ADC **after** the network work: an ADC read just before Wi-Fi left the
  station associated but without a DHCP lease.
- TX power 20 dBm (`set_max_tx_power(80)`); 15 dBm gave `AuthenticationExpired` at -79 dBm.
- The first scan right after the radio starts can come back empty; wait ~2 s.
- Typical run: associate ~1-3 s, DHCP ~0.1 s.

### M2: NOTE4C frame client ✅

- [x] `apps/note4c`: fetch the frame, show it, sleep until `X-Next-Refresh`; ETag in RTC
  memory (304 = no panel refresh); OK always redraws; on failure keep the last frame and retry
  every 15 min, diagnostic card after 8 failures.
- [x] Device health reported to the server: `X-Wake`, `X-Battery-mV`, `X-Last-Display`. A
  failed panel refresh drops the ETag and retries once after 15 min. (Added after a panel
  stopped redrawing overnight while every fetch succeeded.)
- [ ] Device: draw a "stale since" mark itself on failed wakes.
- [ ] Wi-Fi robustness at weak spots (-79 dBm): all-channel join, longer join timeout.
- [ ] Example frame server in this repository (PNG → frame, ETag, `X-Next-Refresh`).

### M3: NOTE4 frame client and partial refresh

- [ ] `apps/note4`: the same frame client in 1 bpp.
- [ ] Driver: mono **partial refresh** (`0x83` window plus transition codes). The sequence is
  documented in `vendor/RESEARCH_NOTES.md` §3.2. Measure it first (INTERACTIVE.md I1–I2).
- [ ] Interactive session, screen bundles and buttons (INTERACTIVE.md I3).

### M4 and later

- [ ] Action screens and the last voice answer as a QR code (INTERACTIVE.md I4–I5).
- [ ] Optional: 16-grey mode (external LUT) for photos, and OTA via `ota_1`.

### Voice track V1–V4 (both devices)

- [ ] **V1, audio loopback:** ES8311 setup over I2C, full-duplex I2S; record while OK is held
  and play it back; beeps. No network needed. Built (`crates/es8311`, `apps/audio`,
  [AUDIO.md](AUDIO.md)); not yet tried on hardware.
- [ ] **V2, relay transport:** a WSS client on the device and a relay that echoes the audio
  back. Tests the transport, device keys and latency.
- [ ] **V3, fast mode:** the relay connects to a realtime speech model and runs its tool calls.
- [ ] **V4, assistant mode:** speech-to-text → an LLM with server-side tools → TTS; choosing
  the mode with a button.

## Open technical questions

1. **Mono PSR byte:** `0x0E` (official driver) vs `0x2E` (community). The driver uses the
   official value.
2. **Mono temperature read** (3-wire SPI read of `0x40`) is not implemented. The driver uses
   the official fallback of 25 °C. Add the read if cold rooms cause ghosting.
3. **esp-radio** is `1.0.0-beta`, so expect API churn. The std path (esp-idf-svc) is the
   documented fallback in [RUST_RESEARCH.md](RUST_RESEARCH.md).
