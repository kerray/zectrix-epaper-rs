# Rust firmware for NOTE4 / NOTE4C (ESP32-S3 N16R8): research, Sept 2026

> **Snapshot from 2026-09-24.** The versions actually used are in `Cargo.toml` (and
> `Cargo.lock`). Some versions recommended below have since changed or been yanked (for
> example `esp-radio` 1.0.0-beta.2); do not copy them from here.

Researched 2026-09-24. Target hardware: ZECTRIX NOTE4 (mono) and NOTE4C (BWRY), both ESP32-S3 N16R8
(16 MiB quad flash, 8 MiB octal PSRAM), native USB-Serial-JTAG (GPIO19/20), 4.2" 400x300 e-paper
with an **SSD2683** controller (see `docs/HARDWARE.md`). EPD pins: PWR 6, BUSY 8, RST 9, DC 10,
CS 11, SCK 12, MOSI 13. Battery ADC is on GPIO4.

**Legend**
- **[V]** Verified: I checked it directly against crates.io API data, the crate source or README, a GitHub repo or release, or the CLI source code.
- **[U]** Uncertain: an inference or recollection that was not checked against a primary source, or behaviour not tested on hardware.

---

## 1. TL;DR

| Topic | Recommendation |
|---|---|
| Stack | **no_std: esp-hal 1.2 + esp-rtos 0.4 (Embassy) + esp-radio 1.0.0-beta.2 + embassy-net 0.9**. Scaffold with `esp-generate` 1.4.0. |
| Fallback | std with **esp-idf-svc 0.53 / esp-idf-hal 0.47 / esp-idf-sys 0.38**, which supports ESP-IDF v5.3 to v6.1 (default v5.5.5). Use it if you need ESP-IDF C components such as esp_http_client with the CA bundle, esp_pm auto light sleep or managed components. |
| Display | No ready-made SSD2683 crate exists. Write a small in-house driver on `embedded-hal` 1.0 `SpiDevice` and implement `embedded-graphics` `DrawTarget`: 1 bpp for mono, 2 bpp BWRY for 4C. Port the init and LUT sequences from the MIT C drivers (zectrix-note4-epd-demo, LazyYoun, emini-home). |
| TLS | **mbedtls-rs 0.3** (the renamed esp-mbedtls): TLS 1.2 and 1.3, hardware acceleration on S3, needs `alloc`. `embedded-tls` 0.19 is TLS 1.3 only. |
| Storage | `esp-storage` 0.10 + **`esp-nvs` 0.5** (ESP-IDF-compatible NVS format), or `sequential-storage` 8 for a custom key-value store. |
| Toolchain | `espup` 0.17.1, which installs the Xtensa Rust fork `esp` channel v1.98.1.0. `espflash` 4.6.0, `probe-rs` 0.32.0, esptool 5.4.0. |
| Flashing | Do **not** use a plain `espflash flash`, because it writes its own bootloader to 0x0 and a partition table to 0x8000. Build an app-only image with `espflash save-image`, write it with `write-bin 0x20000`, and erase `otadata` so the factory bootloader boots `ota_0`. |

---

## 2. Current versions (crates.io API, 2026-09-24)

All entries in this section are **[V]**. The versions come from `https://crates.io/api/v1/crates/<name>`.

| Crate / tool | Latest | Date | Notes |
|---|---|---|---|
| esp-hal | **1.2.2** | 2026-09-18 | 1.2.0 was released on 2026-09-01. `unstable` feature needed for ADC, sleep (`rtc_cntl`), PSRAM, flash, DMA and USB. |
| esp-rtos | **0.4.0** | 2026-08-26 | Replaces `esp-hal-embassy` (last release 0.9.1 in Oct 2025, now obsolete). Provides the scheduler and the Embassy executor/time driver. |
| esp-radio | **1.0.0-beta.2** (pre-release) / 0.18.0 (max stable) | 2026-09-17 | Formerly esp-wifi, whose last release was 0.15.1. **0.18.0 depends on esp-hal ~1.1**, and **1.0.0-beta.x depends on esp-hal ~1.2**. The README says it requires esp-hal `unstable`. |
| esp-bootloader-esp-idf | **0.6.0** | 2026-08-26 | App descriptor, partition table, OTA and `OtaUpdater`. |
| esp-storage | **0.10.0** | 2026-08-26 | Implements `embedded-storage`. |
| esp-alloc | 0.11.0 | 2026-08-26 | `psram_allocator!` macro. |
| esp-println | 0.18.0 | 2026-08-26 | |
| esp-backtrace | 0.20.0 | 2026-08-26 | |
| esp-phy | 0.3.0 | 2026-09-16 | |
| embassy-executor | 0.10.0 | 2026-03-20 | |
| embassy-time | 0.5.1 | 2026-03-20 | |
| embassy-net | 0.9.1 | 2026-04-16 | |
| embedded-graphics | 0.8.2 | 2026-02-15 | |
| embedded-storage | 0.3.2 | 2026-09-20 | |
| esp-nvs | 0.5.0 | 2026-07-10 | "ESP-IDF compatible, bare metal NVS" (lhemala/esp-nvs) |
| sequential-storage | 8.0.1 | 2026-07-19 | |
| mbedtls-rs / mbedtls-rs-sys | 0.3.0 / 0.3.1 | 2026-09-14/15 | The esp-rs/esp-mbedtls repo now hosts these crates. There is no `esp-mbedtls` crate on crates.io. |
| embedded-tls | 0.19.0 | 2026-06-01 | TLS 1.3 only. Features: `rustpki`, `webpki`, `rsa`, `p384`, `ed25519`. |
| reqwless | 0.14.0 | 2026-01-12 | Depends on **embedded-tls ^0.18**, not 0.19. |
| edge-http / edge-net / edge-nal-embassy | 0.8.0 / 0.16.0 / 0.9.0 | 2026 | |
| picoserve | 0.20.1 | 2026-09-21 | Small HTTP server, for example for provisioning. |
| epd-waveshare | 0.6.0 | **2024-10-28** | Effectively dormant (last commit Feb 2026, "preparation for ownership transfer"). |
| epdsi | 0.4.2 | 2026-09-19 | New multi-controller EPD framework with 2 bpp BWRY support, but no SSD2683. |
| ssd1683 | 0.3.1 | 2026-03-19 | Tested on GDEY042T81 (400x300). Might be close to SSD2683 in mono mode **[U]**. |
| esp-idf-svc | **0.53.0** | 2026-09-15 | |
| esp-idf-hal | **0.47.0** | 2026-09-15 | |
| esp-idf-sys | **0.38.1** | 2026-09-16 | |
| embedded-svc | 0.29.0 | 2026-03-09 | |
| espflash / cargo-espflash | **4.6.0** | 2026-09-10 | MSRV 1.95 |
| espup | **0.17.1** | 2026-04-17 | |
| esp-generate | **1.4.0** | 2026-09-18 | |
| probe-rs-tools | **0.32.0** | 2026-07-22 | |
| Xtensa Rust toolchain (esp-rs/rust-build) | **v1.98.1.0** | 2026-09-08 | GitHub releases |
| esptool (PyPI) | **5.4.0** | 2026-09-02 | |
| ESP-IDF | **v6.1** (latest, 2026-08-27). Other current releases: v6.0.3 (2026-09-02) and v5.5.5 (2026-07-17). | | GitHub releases **[V]** |

**ESP-IDF v6.0 is no longer the newest line.** v6.1 is the latest feature release, and v6.0.x is still maintained (v6.0.3). **[V]**

---

## 3. Option (a): no_std with esp-hal and Embassy

### 3.1 Maturity
- **esp-hal is 1.x.** Its stable API covers GPIO, SPI, I2C, UART, `init` and the clocks. Everything this project needs beyond that sits behind the `unstable` feature, including ADC, `rtc_cntl` (sleep), `psram`, `timer`, `dma` and `usb` (`esp-hal/src/lib.rs`, `unstable_module!` / `unstable_driver!`). Unstable APIs may change in minor releases. **[V]**
- **esp-radio is still in beta for 1.0 (1.0.0-beta.2).** The official `esp-generate` template already uses `esp-radio = "1.0.0-beta.1"` with esp-hal `~1.2.2`. **[V]**
- **New in esp-hal 1.2 for our use case [V]** (CHANGELOG v1.2.0-rc.0):
  - "ADC: async ADC API for ESP32-S2 and ESP32-S3" (#6167)
  - The `LowPower` sleep API (`set_wakeup_deadline`, `sleep_deep`, `sleep_deep_with_rejection`, `sleep_light`) and GPIO `apply_wakeup_config` / `caused_wakeup`
  - "Sleep: auto lightsleep for ... ESP32-S3" (#5768) and `WakeLock`
  - `Input::wait_for_with_options()`, which lets a GPIO wait wake the chip from light sleep
  - GPIO `set_pad_hold`
- Deep sleep example (`examples/qa/src/bin/sleep-timer-gpio.rs`) **[V]**:
  ```rust
  let mut lpwr = LowPower::new(peripherals.LPWR);
  let mut btn = Input::new(peripherals.GPIO18, InputConfig::default().with_pull(Pull::Up));
  btn.apply_wakeup_config(&WakeupConfig::default().with_low_power_path(true)).unwrap();
  btn.listen(Event::LowLevel);
  lpwr.set_wakeup_deadline(Instant::now() + Duration::from_secs(1800));
  lpwr.sleep_deep(RtcSleepConfig::deep());
  ```
  Use `#[esp_hal::ram(persistent)]` for variables that must survive deep sleep. On S3, GPIO wake from deep sleep needs an RTC-capable pad. NOTE4's buttons are GPIO0, 18 and 39, and GPIO0 and GPIO18 are RTC pads. GPIO39 is not an RTC GPIO on S3 (RTC GPIOs are 0 to 21), so it can wake the chip only from light sleep. **[U]** Check this against the TRM.

### 3.2 PSRAM (octal, 8 MiB)
- esp-hal has `psram::PsramConfig { mode: PsramMode::{Auto, QuadSpi, OctalSpi}, size, core_clock, flash_frequency, ram_frequency }` for S3. The docs say auto-detection is "not entirely reliable", so **set `OctalSpi` explicitly**. **[V]** (`esp-hal/src/psram/esp32s3.rs`)
- Add PSRAM to the heap with `esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram);`. The docs also say "The PSRAM example *must* be built in release mode". **[V]**
- Octal PSRAM occupies GPIO33 to 37, so don't use those pins. **[V]** (ESP32-S3 datasheet; not relevant for NOTE4's pin map)
- A 400x300 BWRY framebuffer at 2 bpp is only 30 KB, so internal RAM is enough. PSRAM is useful for HTTP bodies, images and fonts. Wi-Fi and DMA buffers must stay in internal RAM. esp-alloc has capability-aware regions, and the esp-radio buffers use the internal heap. **[U]** Check the esp-radio docs.
- `esp_hal::Config` does not seem to take a PSRAM config field in 1.2 any more. Configuration may go through `esp-config` keys (`.cargo/esp-config.toml`) or the `psram_allocator!` path. **[U]** Look up `ESP_HAL_CONFIG_PSRAM_*` in `esp-hal/esp_config.yml` for the exact release.

### 3.3 Wi-Fi, HTTP and TLS
- The template wiring is `esp_rtos::start(timg0.timer0, FROM_CPU_INTR0)`, then `esp_radio::wifi::WifiController::new(peripherals.WIFI, Default::default())` and `Interface::station()` → embassy-net `Stack`. **[V]** (template `async_main.rs`)
- **HTTP client choices:**
  1. **reqwless 0.14** with embedded-tls 0.18: TLS 1.3 only, and certificate verification is weak in no_std. Fine for your own server if it speaks TLS 1.3. **[V]** for versions, **[U]** for verification quality.
  2. **mbedtls-rs 0.3** (MbedTLS 3.x) over an embassy-net `TcpSocket`, with a small HTTP/1.1 layer (hand-written, or `edge-http` 0.8). It supports TLS 1.2 and 1.3, SNI, CA-chain verification and hardware acceleration on S3 (feature `esp32s3`, plus `hook-wall-clock` for notBefore/notAfter checks). Without a wall clock, certificate dates are **not** checked. The official ESP example uses `esp-hal ~1.2`, `esp-radio 1.0.0-beta.0`, `embassy-net 0.9`, `esp-rtos 0.4` and `tinyrlibc`. **[V]** (`esp-rs/esp-mbedtls` repo, README and `examples/esp/Cargo.toml`)
  - mbedtls-rs-sys appears to ship prebuilt static libraries for the esp targets, so no C toolchain is needed. **[U]**
- **Recommendation:** use mbedtls-rs for general HTTPS (weather APIs and similar, many of which are TLS 1.2). Pin the root CAs you need; there is no automatic `esp_crt_bundle` like in ESP-IDF.

### 3.4 Storage
- `esp_storage::FlashStorage::new(peripherals.FLASH)` implements `embedded_storage::{ReadStorage, Storage}` / `NorFlash`. **[V]**
- `esp_bootloader_esp_idf::partitions::read_partition_table(...)` finds `nvs` / `otadata` / `ota_0` / `spiffs` by type and label and returns a bounded `FlashRegion`, with `as_nor_flash()` new in 0.6. `OtaUpdater` handles OTA slot switching and `reset_data()`. **[V]**
- **esp-nvs 0.5** reads and writes the real ESP-IDF NVS format. Use it on the factory `nvs` partition (0x9000, 16K, but check whether factory data lives there first), or on a new data partition. **[V]** (README)
- `sequential-storage` 8 is a simpler option (map/queue on NorFlash) if you don't need IDF-compatible NVS.
- The 8 MiB `spiffs` partition at 0x800000 can hold raw assets such as fonts and images through `FlashRegion`, without a filesystem.

### 3.5 ADC and buttons
- `esp_hal::analog::adc::{Adc, AdcConfig, Attenuation, AdcCalCurve}` is unstable, and S3 now has an async API. GPIO4 is ADC1_CH3. Read it with 11/12 dB attenuation and multiply by the divider ratio. The emini-home `home_battery.c` has the divider and battery curve. **[V]** for the API's existence, **[U]** for exact type names in 1.2.
- Buttons: `Input` + `wait_for_falling_edge().await` in Embassy tasks. For deep sleep wake, see 3.1.

### 3.6 E-paper driver
- **Controller: SSD2683** on both devices (vendor wiki). No Rust crate supports it. **[V]** (crates.io searches for epaper, e-paper, epd, eink, ssd1683, jd79668)
- Candidates to borrow from:
  - `epd-waveshare` 0.6: dormant, has only `epd4in2` (the UC8176 V1 panel), and no SSD2683 or BWRY 4.2". **[V]**
  - `ssd1683` 0.3.1: 400x300 GDEY042T81. A community PlatformIO firmware drives NOTE4 mono with "an SSD1683-style 400 × 300 display profile", so mono NOTE4 may largely work with SSD1683 command semantics. **[U]**
  - `epdsi` 0.4.2: a clean controller/panel split with 2 bpp BWRY (JD7966x, Pervasive Spectra-4 at 400x300), `embedded-graphics` `DrawTarget` and paged rendering. It is a good template for an SSD2683 BWRY controller module, or a candidate for an upstream PR. **[V]** (README)
  - `weact-studio-epd` 0.1.3: its 4.2" 400x300 B/W panel is SSD1683-based, async. **[V]**
- **Plan:** write a `Ssd2683<SPI, BUSY, DC, RST, PWR>` driver in about 300 lines on `embedded-hal` 1.0 / `embedded-hal-async`. It needs a 1 bpp framebuffer (15,000 B) for mono and a 2 bpp framebuffer (30,000 B) for BWRY, and `DrawTarget<Color = BinaryColor>` / a custom `QuadColor`. Port the init sequence and refresh commands from the MIT C sources: `itopinion/zectrix-note4-epd-demo`, `LazyYoun/youn-ink-fourcolor-firmware`, and `fiedoruk/emini-home` `home_panel.c`.

### 3.7 Cargo.toml (no_std, ESP32-S3)
Based on the `esp-generate` 1.4.0 template (**[V]**), with the S3 chip and extra crates added:

```toml
[package]
name = "note4-fw"
version = "0.1.0"
edition = "2024"
rust-version = "1.95"

[dependencies]
esp-hal = { version = "~1.2.2", features = ["esp32s3", "unstable", "log-04"] }
esp-rtos = { version = "0.4.0", features = ["esp32s3", "embassy", "esp-radio", "esp-alloc", "log-04"] }
esp-bootloader-esp-idf = { version = "0.6.0", features = ["esp32s3", "log-04"] }
esp-radio = { version = "1.0.0-beta.2", features = ["esp32s3", "wifi", "esp-alloc", "unstable", "log-04"] }
esp-alloc = { version = "0.11.0", features = ["esp32s3"] }
esp-println = { version = "0.18.0", features = ["esp32s3", "log-04"] }
esp-backtrace = { version = "0.20.0", features = ["esp32s3", "panic-handler", "println"] }
esp-storage = { version = "0.10.0", features = ["esp32s3"] }
log = "0.4.27"

embassy-executor = { version = "0.10.0", features = ["log"] }
embassy-time = { version = "0.5.0", features = ["log"] }
embassy-net = { version = "0.9.1", features = ["tcp", "udp", "dns", "dhcpv4", "medium-ethernet", "log"] }
embassy-sync = "0.8"
embedded-io = "0.7.1"
embedded-io-async = "0.7.0"
static_cell = "2.1.1"
critical-section = "1.2.0"
heapless = "0.9"

embedded-hal = "1.0"
embedded-hal-async = "1.0"
embedded-graphics = "0.8.2"
embedded-storage = "0.3.2"
esp-nvs = "0.5.0"                    # or sequential-storage = "8.0.1"

mbedtls-rs = { version = "0.3.0", features = ["esp32s3", "embassy-time", "log", "hook-wall-clock"] }  # feature names per examples/esp [V]
tinyrlibc = { version = "0.5", default-features = false, features = ["memchr"] }
# alternative, TLS1.3-only:
# reqwless = { version = "0.14.0", features = ["log"] }

[profile.dev]
opt-level = "s"

[profile.release]
codegen-units = 1
debug = 2
lto = "fat"
opt-level = "s"
```

`.cargo/config.toml` for Xtensa, as generated **[V]**:
```toml
include = ["esp-config.toml"]
[target.xtensa-esp32s3-none-elf]
runner = "espflash flash --monitor --chip esp32s3"   # CHANGE, see §6
[build]
rustflags = ["-C", "link-arg=-nostartfiles"]
target = "xtensa-esp32s3-none-elf"
[unstable]
build-std = ["alloc", "core"]
```
`rust-toolchain.toml`: `[toolchain] channel = "esp"` **[V]**.

Main skeleton (template) **[V]**:
```rust
esp_bootloader_esp_idf::esp_app_desc!();   // REQUIRED for the ESP-IDF 2nd-stage bootloader

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);
    esp_alloc::psram_allocator!(p.PSRAM, esp_hal::psram);
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0, p.FROM_CPU_INTR0);
    let wifi = esp_radio::wifi::WifiController::new(p.WIFI, Default::default()).unwrap();
    ...
}
```

---

## 4. Option (b): std with esp-idf-svc, esp-idf-hal and esp-idf-sys

- **Status [V]:**
  - The three crates moved into the **`esp-rs/esp-idf` monorepo** on 2026-09-19, and the old repos are now read-only. The monorepo is active (commits on 2026-09-23).
  - Releases: esp-idf-sys 0.38.1, esp-idf-hal 0.47.0 and esp-idf-svc 0.53.0 (Sept 2026).
  - The README still says they are a **community effort** and that "Espressif puts little to no paid developer time in these". They "might be a bit lagging behind the latest stable ESP-IDF".
- **Supported ESP-IDF [V]:**
  - esp-idf-sys 0.38.0 added "Basic compatibility for ESP-IDF release 6.0", and esp-idf-svc 0.53 adds "Compatibility with ESP-IDF V6.0".
  - Monorepo CI builds **v5.3.6, v5.4.4, v5.5.5, v6.0.3 and v6.1**. The CI also runs a release/v6.2 "next" job, currently commented out.
  - **The default is `DEFAULT_ESP_IDF_VERSION = "v5.5.5"`**. The `esp-idf-template` offers the choices `v6.1, v6.0.3, v5.5.5, v5.4.4, v5.3.6, master`, with v5.5.5 as the default.
  - Versions below 5.3 are deprecated for hal and svc.
  - In IDF 6.0, some components (MQTT, Ethernet PHY/SPI drivers) moved to managed components and need `extra_components`.
- **Pros:**
  - Mature Wi-Fi, esp_tls/esp_http_client (`EspHttpConnection`) with the **CA certificate bundle**, NVS (`EspNvs`) in the factory `nvs` partition, and `esp_pm` automatic light sleep with DTIM Wi-Fi modem sleep.
  - OTA (`EspOta`) and SNTP.
  - You can pull any ESP-IDF or managed C component (`[package.metadata.esp-idf-sys] extra_components`), including a C port of the SSD2683 driver from the vendor demo.
  - The same model as the C reference firmware (emini-home, which targets IDF v6.0).
- **Cons:**
  - A slow first build that downloads ESP-IDF and its GCC.
  - Larger binaries, although the 3.9 MiB OTA slots are plenty.
  - Community-maintained bindings.
  - More complex CMake/Kconfig. Needs `sdkconfig.defaults` with PSRAM (`CONFIG_SPIRAM=y`, `CONFIG_SPIRAM_MODE_OCT=y`) and a 16 MB flash setting.
- Cargo.toml (std) **[V versions]**:
  ```toml
  [dependencies]
  esp-idf-svc = { version = "0.53", features = ["critical-section", "embassy-time-driver", "embassy-sync"] }
  log = "0.4"
  embedded-graphics = "0.8.2"
  [build-dependencies]
  embuild = "0.33"            # [U] check the current embuild version
  [package.metadata.esp-idf-sys]
  esp_idf_version = "v6.0.3"   # or v5.5.5 (default) / v6.1
  esp_idf_sdkconfig_defaults = ["sdkconfig.defaults"]
  ```
  In `.cargo/config.toml`: `[env] ESP_IDF_VERSION = "v6.0.3"`, target `xtensa-esp32s3-espidf`, toolchain `esp`, and `build-std = ["std", "panic_abort"]`. **[U]** for exact template contents; generate with `cargo generate esp-rs/esp-idf-template cargo`.
- A std app also needs its app descriptor. esp-idf-sys provides it automatically through the IDF build (`esp_app_desc`). You can keep the factory bootloader as long as its IDF major version is compatible. An app from IDF 5.x or 6.x with a bootloader from 5.x or 6.x is normally fine, but check the factory bootloader's version string in the first-stage log. **[U]**

---

## 5. Toolchain installation

```bash
# Xtensa toolchain (ESP32-S3 still needs the esp-rs LLVM/Rust fork) [V]
cargo install espup --locked          # 0.17.1
espup install                          # installs the "esp" channel (rust-build v1.98.1.0) + xtensa GCC/ld
. ~/export-esp.sh                      # add to shell profile; esp-generate 1.4 now checks for the linker

cargo install esp-generate --locked    # 1.4.0
esp-generate -o esp32s3 note4-fw       # TUI: enable unstable-hal, alloc, wifi, embassy, log, esp-backtrace
# CLI syntax changed in 1.4: chip is `-o <chip>` (was --chip)            [V] CHANGELOG

cargo install espflash --locked        # 4.6.0 (or cargo binstall espflash); also cargo-espflash
cargo install probe-rs-tools --locked  # 0.32.0 (optional; ESP32-S3 built-in USB-JTAG works with probe-rs)
pipx install esptool                   # 5.4.0 (useful for backups and raw writes)
# std path additionally: cargo install ldproxy; pip deps are handled by embuild
```

---

## 6. Flashing without touching the factory bootloader or partition table

### 6.1 Facts
- `espflash flash` / `cargo espflash flash` (ESP-IDF format) always builds three segments [V]:
  - the **bootloader at 0x0**: the bundled one, built from IDF `release/v6.1` per `espflash/resources/README.md`, or the file given with `--bootloader`
  - the **partition table at 0x8000**: the default table, or `--partition-table` (CSV or binary)
  - the **app** in the `factory` partition, or the first app partition, or `--target-app-partition <label>`
- It **skips regions whose MD5 already matches** unless you pass `--no-skip`. If you give it byte-identical copies of the factory bootloader and partition table, those regions are not rewritten. **[V]** (flag semantics)
- **espflash never touches `otadata`.** A grep of `espflash/src` finds no otadata handling. **[V]**
- `espflash save-image` **without `--merge`** writes only the app image (the OTA segment). It honours `--partition-table`, `--target-app-partition` and `-s/--flash-size`. **[V]** (`save_elf_as_image`, `SaveImageArgs` in `src/bin/espflash.rs`)
- `espflash write-bin <addr> <file>` writes raw data at an address. `espflash erase-region <addr> <size>` works on 4 KiB-aligned ranges, and `erase-parts <labels>` erases partitions by label. **[V]**
- `espflash.toml` supports `[idf] bootloader = "..."`, `partition_table = "..."` and `[flash] size = "16MB"`. **[V]**
- probe-rs has matching `--idf-bootloader`, `--idf-partition-table` and `--idf-target-app-partition` options. **[V]** (`probe-rs-tools/src/bin/probe-rs/util/flash.rs`)

### 6.2 Is an esp-hal app compatible with the factory ESP-IDF bootloader?
- **Yes**, as long as the app contains the **app descriptor** from `esp_bootloader_esp_idf::esp_app_desc!()`. espflash refuses images without one unless you pass `--ignore-app-descriptor`. The descriptor fills in the magic word, version, `idf_ver` (default "0.0.0", which the bootloader does not check), `min/max_efuse_blk_rev_full`, `mmu_page_size` and `secure_version`. **[V]**
- The partition table offset defaults to 0x8000 (`ESP_BOOTLOADER_ESP_IDF_CONFIG_PARTITION_TABLE_OFFSET`), which matches the factory layout. **[V]**
- `esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] ...)` reuses the RAM the 2nd-stage bootloader occupied. The bootloader used at link time is irrelevant here: the region is simply free after the bootloader hands over. **[V]** (crate docs) plus **[U]** (no test with this particular factory bootloader)
- Things to watch **[U]**:
  - If the factory bootloader was built with `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` and otadata marks a slot "pending verify", call `OtaUpdater::set_current_ota_state(Valid)` on the first successful boot.
  - If secure boot or flash encryption is enabled on the device (unlikely on a dev kit), none of this applies.

### 6.3 Recommended procedure (NOTE4/NOTE4C factory layout)
Factory layout: `nvs 0x9000/16K`, `otadata 0xd000/8K`, `phy_init 0xf000/4K`, `ota_0 0x20000/0x3F0000`, `ota_1 0x410000/0x3F0000`, `spiffs 0x800000/8M`.

```bash
# 0. Full backup (once, keep it safe; vendor docs insist on this)
espflash read-flash 0x0 0x1000000 note4_factory_full.bin      # or: esptool --chip esp32s3 read-flash 0 0x1000000 ...
espflash read-flash 0x8000 0x1000 factory_pt.bin
espflash partition-table --to-csv factory_pt.bin > factory_pt.csv

# 1. Build and produce an app-only image (no bootloader, no table)
cargo build --release
espflash save-image --chip esp32s3 -s 16mb \
    --partition-table factory_pt.csv --target-app-partition ota_0 \
    target/xtensa-esp32s3-none-elf/release/note4-fw note4-fw.bin

# 2. Write it into ota_0 only
espflash write-bin 0x20000 note4-fw.bin
#   (equivalent: esptool --chip esp32s3 write-flash 0x20000 note4-fw.bin)

# 3. Make the factory bootloader pick ota_0: erase otadata
espflash erase-region 0xd000 0x2000
#   With blank otadata and no "factory" app partition, the IDF bootloader boots the first OTA slot (ota_0). [U — standard IDF behaviour, verify on device]

# 4. Monitor over USB-Serial-JTAG
espflash monitor --chip esp32s3
```

The runner in `.cargo/config.toml` can be a small script that does steps 1 to 4. Another option is `espflash flash --bootloader factory_bl.bin --partition-table factory_pt.csv --target-app-partition ota_0` plus `[idf]` in `espflash.toml`. Checksum skipping then leaves the identical bootloader and table alone, but a byte mismatch **would overwrite them**, so the `write-bin` route is safer. Dump `factory_bl.bin` with `read-flash 0x0 0x8000`.

For A/B development, keep the factory or third-party firmware in `ota_1` and switch between slots with `OtaUpdater` (esp-bootloader-esp-idf 0.6), or with the dual-firmware button gesture used by some community firmware.

---

## 7. Comparison

| Criterion | no_std (esp-hal 1.2) | std (esp-idf-svc 0.53) |
|---|---|---|
| Vendor backing | Espressif-maintained, 1.x stable core | Community-maintained, Espressif time is minimal [V] |
| Stability | Core is stable. ADC, sleep, PSRAM and Wi-Fi are behind `unstable`, and esp-radio is in beta. | Mature C underneath; the Rust wrappers change their API every minor release |
| Build | Fast, pure cargo | Slow first build (downloads IDF and toolchains), CMake |
| Binary / RAM | Small | Larger, but that doesn't matter with 3.9 MiB slots and 8 MiB PSRAM |
| HTTPS | mbedtls-rs 0.3 (young), manual root CAs | esp-tls with the CA bundle, esp_http_client |
| Sleep | Deep sleep plus new auto light sleep in 1.2 (new code) | esp_pm auto light sleep, proven |
| NVS | esp-nvs (third party, format-compatible) | Native NVS |
| C component reuse | No (except porting code by hand) | Yes (managed components, the vendor's C EPD driver) |
| Factory bootloader | Works with `esp_app_desc!` | Works |

**Verdict:**
- Use **no_std** for a lean, deep-sleep-centric "wake, fetch, render, sleep" device written in Rust from the ground up.
- Switch to **std with IDF v6.0.3 or v5.5.5** if you need robust generic HTTPS to arbitrary public servers with a CA bundle, if you rely on automatic light sleep while Wi-Fi stays associated, or if you want to link the vendor's C display code as-is. IDF v6.1 is also in CI, but it is newer and "basic" support only.

## 8. Sources
- crates.io API: `https://crates.io/api/v1/crates/{esp-hal,esp-rtos,esp-radio,esp-bootloader-esp-idf,esp-storage,esp-idf-svc,esp-idf-hal,esp-idf-sys,espflash,espup,esp-generate,probe-rs-tools,mbedtls-rs,embedded-tls,reqwless,epd-waveshare,epdsi,ssd1683,esp-nvs,...}`
- esp-hal monorepo, including CHANGELOG, psram, rtc_cntl/sleep, esp-radio README and esp-bootloader-esp-idf: https://github.com/esp-rs/esp-hal
- esp-hal release tags: https://github.com/esp-rs/esp-hal/releases
- esp-generate template (Cargo.toml, async_main.rs, .cargo/config.toml, CHANGELOG 1.4.0): https://github.com/esp-rs/esp-generate
- espflash source (cli/mod.rs, bin/espflash.rs, image_format/idf.rs, resources/README.md, README): https://github.com/esp-rs/espflash
- mbedtls-rs (formerly esp-mbedtls): https://github.com/esp-rs/esp-mbedtls
- esp-idf monorepo (CI matrix, moved crates): https://github.com/esp-rs/esp-idf
- esp-idf-sys CHANGELOG, DEFAULT_ESP_IDF_VERSION: https://github.com/esp-rs/esp-idf-sys
- esp-idf-svc CHANGELOG: https://github.com/esp-rs/esp-idf-svc
- esp-idf-template: https://github.com/esp-rs/esp-idf-template
- ESP-IDF releases: https://github.com/espressif/esp-idf/releases
- Xtensa Rust builds: https://github.com/esp-rs/rust-build/releases
- probe-rs: https://github.com/probe-rs/probe-rs
- epd-waveshare: https://github.com/rust-embedded-community/epd-waveshare
- epdsi: https://github.com/melastmohican/epdsi
- esp-nvs: https://github.com/lhemala/esp-nvs
- ESP-IDF app image format / app descriptor: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/system/app_image_format.html
- ESP-IDF OTA and otadata behaviour: https://docs.espressif.com/projects/esp-idf/en/stable/esp32s3/api-reference/system/ota.html
- Vendor docs: <https://wiki.zectrix.com/en/software/note4-development-guide>, <https://wiki.zectrix.com/en/hardware/note4c/quick-start> (all pages: `tools/vendor-pages.txt`), `docs/EMINI_HOME_ANALYSIS.md`
