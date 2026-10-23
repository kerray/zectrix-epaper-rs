# ZECTRIX NOTE4 / NOTE4C Devkit — vendor research notes

Compiled 2026-09-24. Sources: wiki.zectrix.com (all 57 pages fetched), zectrix.com product/updater pages,
the V1.0 schematic PDF, the SSD2683 datasheet (rev 0.20 proposal), and source code of the public firmware
repos listed below (shallow clones in `_refs/`, **not to be committed**).

> Paths starting with `_refs/` point into the reference clones, which are not in the repository: run
> `just fetch-refs` to clone them into `docs/vendor/_refs/` (git-ignored). Vendor files (`common/…`, `note4/…`,
> `note4c/…`) come from `just fetch-vendor`.

Legend: **[V]** = verified from a primary source (official wiki, schematic, datasheet, or source code that runs on
the device; source given). **[I]** = inferred / cross-checked from several community sources but not official.
**[?]** = guess or open question.

---

## 1. Device comparison

| Item | NOTE4 Developer Kit (mono) | NOTE4C Devkit (4-colour) | Source |
|---|---|---|---|
| PCB | ZecTrix_Note4 **V1.0** (same board for both) | same board, V1.0 | [V] wiki + schematic title block |
| SoC | ESP32-S3 **N16R8** (16 MB flash, 8 MB octal PSRAM) | same | [V] wiki |
| Panel | 4.2" 400×300 black/white, "custom fast-refresh panel with partial-refresh support" | 4.2" 400×300 **BWRY** (black/white/yellow/red), E Ink Spectra 3100 class | [V] wiki; Spectra 3100 note printed on SSD2683 datasheet |
| Panel part number | **unknown** | **unknown**; best public match: Good Display **GDEM042F86** (4.2" BWRY, 400×300, SSD2683ZA, 12 s fast / 20 s std) | [?] GDEM042F86 match is a guess; not confirmed by Zectrix |
| Display controller | **Solomon Systech SSD2683** | **SSD2683** | [V] wiki both dev guides |
| Controller RAM format | 2 bpp (4 px/byte, MSB first), 100 B/row, 30 000 B/frame — used for both panels | same | [V] datasheet §DTM + drivers |
| Refresh modes (public code) | full 1bpp (OTP waveform), **partial 1bpp** (window 0x83, transition codes), full **4bpp/16-grey** (external LUT uploads, multiple passes) | **full refresh only** (OTP waveform), "normally >10 s"; community measures ~25 s on USB | [V] wiki + itopinion driver; [V] youn-ink driver; 25 s from emini-home HARDWARE.md [I] |
| Framebuffer app-side | 1bpp 15 000 B (0=black,1=white); 4bpp 60 000 B | 2bpp 30 000 B, index 0=black 1=white 2=yellow 3=red | [V] |
| Audio | ES8311 codec (I2C 0x18), mic, speaker amp (8-pin, pinout matches NS4150B [?]) | same | [V] schematic; amp part [?] |
| RTC | PCF8563 @0x51, INT→GPIO5, powered from 3V3 **or VBAT via diodes** (keeps time with latch off) | same | [V] schematic |
| NFC | I2C @0x55. Wiki: "NT3H2111/NT3H2211 family"; xiaozhi/youn-ink config.h: **GT23SC6699** (NT3H-compatible clone) | same | [V] both claims exist; actual part [?] |
| Battery | 2000+ mAh Li-ion | same | [V] |
| Charger | 8-pin switching charger U1 (NTC, ISET, LED2, LED1, BAT, LX, GND, VIN + inductor); ISET: NC=2.0 A, 1.4k=2.4 A, **2.4k=1.5 A (default)**, 4.3k=1.0 A | same | [V] schematic; part number not printed [?] (IP23xx-class guess) |
| 3V3 regulator | SOT-23-5 buck U2 (EN/GND/SW/VIN/FB), Vfb 0.788 V, 3.22 V out, 1.2 A | same | [V] schematic; part number [?] |
| Buttons | Up GPIO39, Down/Power GPIO18, BOOT/OK GPIO0, RESET pinhole = EN | same | [V] |
| Expansion header J3 (6-pin) | 3V3, GND, TX(GPIO43), RX(GPIO44), GPIO, GPIO — the two GPIOs are **GPIO40/GPIO41** | same | [V] wiki order + schematic net names |
| Dimensions | 97×97×8.75 mm incl. magnetic back, ABS | same | [V] |
| Shipped firmware | "beta demonstration firmware" (devkit) | factory xiaozhi-derived image (see §6) | [V] wiki / [I] |
| Firmware images interchangeable? | **No** — only the panel init/format differs, but images are panel-specific | **No** | [V] wiki (repeated warnings) |

**Pin maps are identical between NOTE4 and NOTE4C** [V] — both dev guides publish the same table and the NOTE4C guide
says it "uses the same controller board and peripheral mapping as the NOTE4 Devkit shown in this schematic". Only the
panel on the 24-pin FPC differs. Consequence for our repo: one board-support layer, a compile-time (or runtime) panel
selection.

Note: There is also a **Tuya T5AI variant** "ZECTRIX_T5AI_NOTE_4" in `tuya/TuyaOpen` (boards/T5AI/ZECTRIX_T5AI_NOTE_4) —
different MCU, not our target, but evidence the product exists in a non-ESP32 SKU. [V exists / ? relevance]

---

## 2. Pinout (both devices) — ESP32-S3 GPIO

Verified against: wiki dev guides (both), schematic V1.0 sheet 2 (U4 module net labels), `config.h` in
youn-ink (NOTE4C) and xiaozhi upstream (NOTE4), itopinion `docs/HARDWARE.md`, TRMNL `docs/zectrix.md`. All agree.

### E-paper (SPI, host SPI3_HOST / "HSPI" in Arduino)
| Signal | GPIO | Notes |
|---|---|---|
| EPD_PWR_EN | 6 | **Active HIGH** → Q7 NMOS → Q6 PMOS switches `3V3_EPD`. Drivers use `gpio_hold_en` so level survives deep sleep; hold LOW in sleep. |
| BUSY | 8 | **LOW = busy, HIGH = idle** (all drivers loop `while(gpio==0)`) |
| RST# | 9 | reset pulse: H 10 ms → L 20 ms → H 10 ms, then wait BUSY |
| D/C# | 10 | 0 = command, 1 = data |
| CS# | 11 | driven as plain GPIO by all drivers (`spics_io_num=-1`), toggled per command/data burst |
| SCK | 12 | SPI mode 0, MSB first |
| MOSI/SDA | 13 | 3-wire bidirectional for reads (temperature 0x40) — drivers switch SPI to RX at ≤8 MHz |
| SPI clock | — | itopinion default 20 MHz (read 8 MHz); youn-ink/TRMNL NOTE4C 40 MHz; TRMNL NOTE4 8 MHz |

Panel booster (GDR/RESE, inductor L2, Q8, diodes D5–D7 for PREVGH/PREVGL) is on the main board → standard 24-pin
Good-Display-style FPC interface. [V schematic]

### Power / buttons / LED / charger / battery
| Function | GPIO | Level / notes |
|---|---|---|
| BOOT / OK / "voice" button | 0 | active LOW, external pull-up (R49) + RC; strapping pin; deep-sleep wake source in TRMNL/youn-ink |
| Charge-complete `STDBY_H` | 1 | active **HIGH** = full |
| Charging `CHRG_L` | 2 | active **LOW** = charging (inverted from charger LED pin by Q9); use pull-up |
| Status LED (green) | 3 | **active LOW** (itopinion HARDWARE.md; youn-ink drives 0 = on) |
| Battery ADC `ADC_BAT` | 4 | ADC1_CH3, **2:1 divider** (firmware multiplies by 2), atten 12 dB. youn-ink % curve: `(-V² + 9016·V − 19189000)/10000` (V in mV) |
| RTC INT (PCF8563) | 5 | open-drain, active LOW |
| NFC field detect | 7 | active LOW (youn-ink `NFC_FD_ACTIVE_LEVEL 0`) |
| **Power latch `PWR_ON`** | 17 | **drive HIGH immediately at boot and keep HIGH (gpio_hold through deep sleep)**. LOW/released while button not pressed ⇒ battery rail cut ⇒ device off. USB power keeps board alive regardless. |
| Down / Power `KEY_DET/PGDN` | 18 | active LOW; physically also the power-on switch SW1 (turns on Q1 via D2); sensed through D3 — "less clean" level (emini). youn-ink waits `while(!gpio_get_level(18))` after latching power (wait for release). itopinion: 3-s hold = shutdown. |
| NFC power | 21 | NFC_PWR (enable/power-down) |
| Up button | 39 | active LOW, external pull-up |
| Audio power `PA_PWR_EN` (AVDD_3V3) | 42 | active HIGH (Q3/Q4 switch) |
| Speaker amp `PA_CTRL` | 46 | active HIGH; strapping pin (keep low at reset) |
| Expansion GPIOs | 40, 41 | on J3 |

### I2C (single bus, port 0)
SDA **GPIO47**, SCL **GPIO48**, external pull-ups R45/R46 to AVDD_3V3 (!) — i.e. pull-ups are powered by the audio rail;
youn-ink also enables internal pull-ups and powers audio rail at boot. [V schematic; implication I]
| Device | 7-bit addr |
|---|---|
| ES8311 codec | 0x18 (`0011000`) |
| PCF8563 RTC | 0x51 (`1010001`) |
| NFC (NT3H2x11 / GT23SC6699) | 0x55 (`1010101`) |

### I2S (ES8311)
MCLK 14, BCLK 15, WS/LRCK 38, codec DOUT→ESP **DIN 16**, ESP **DOUT 45**→codec DSDIN. 16 kHz in/out in xiaozhi configs.

### USB / UART
Native USB-Serial/JTAG D− 19, D+ 20 (no USB-UART bridge; download mode via BOOT+RESET). UART0 TX 43 / RX 44 on J3.

### Not present
No buzzer (speaker via codec), no accelerometer/other sensors, no SD card, no fuel gauge. Temperature only via the
SSD2683 internal sensor (cmd 0x40). [V schematic]

---

## 3. SSD2683 controller & panel command sequences

Datasheet: `common/datasheets/SSD2683_0.20_proposal.pdf` (Solomon Systech rev 0.20, May 2024, a public download
from Good Display's "IC driver" page, no confidentiality marking; not redistributed here, see `tools/vendor-files.txt`). 400 gate × 300 source (so the native scan is 400×300 landscape), 2 bpp per pixel, on-chip OTP
waveforms, internal temperature sensor, auto-sequence 0x17, partial window 0x83 (HRST/HRED must be 4-px aligned).
Commands used by the drivers but **not documented** in the public rev 0.20: `0xE9`, `0xA5` (as a command), `0xE6`
semantics partially (TSSET). [V datasheet]

Documented commands of interest: 00 PSR, 01 PWR, 02 POF, 03 POFS, 04 PON, 06 BTST, 07 DSLP (param 0xA5), 10 DTM,
12 DRF, 17 AUTO (0xA5 = PON>DRF>POF, 0xA7 = …>DSLP), 30 PLL, 40 TSC (read temp), 41 TSE, 50 CDI, 51 LPD, 61 TRES,
65 GSST, 70 REV, 82 VDCS, 83 PTLW, E0 CCSET, E3 PWS, E6 TSSET. DTM pixel code KPixel[1:0] → "Gray 0..3" (DDX=1 default).

### 3.1 NOTE4C (BWRY) — full refresh [V: youn-ink `custom_lcd_display.cc` @51812e4, identical in TRMNL "recovered from the NOTE4C factory firmware", emini-home, today-is-friday, vendor MSP430 sample]
```
EPD_PWR_EN(6)=1; delay 10ms
RST=1 10ms; RST=0 20ms; RST=1 10ms; wait BUSY high
CMD 0xE9, DATA 0x01            ; (undocumented; vendor "SSD2683_Init") — no PSR/PWR/LUT needed, OTP waveform
CMD 0x10 (DTM); wait BUSY      ; then 30 000 bytes: 300 rows × 100 bytes, 4 px/byte, MSB = leftmost px
CMD 0x04 (PON); wait BUSY; delay 10ms
CMD 0x12, DATA 0x00 (DRF); delay 10ms; wait BUSY   ; >10 s … ~25 s; use ≥120 s timeout
CMD 0x02, DATA 0x00 (POF); wait BUSY; delay 20ms
CMD 0x07, DATA 0xA5 (deep sleep)
EPD_PWR_EN=0
```
Colour codes (2-bit): **00 = black, 01 = white, 10 = yellow, 11 = red** (solid bytes 0x00 / 0x55 / 0xAA / 0xFF).
[V youn-ink `ssd2683_solid_4color_byte`, emini-home docs, vendor sample's PIC_WHITE=0x55/PIC_Yellow=0xAA/PIC_R=0xFF.
The vendor sample's VLINE comments are inconsistent — ignore them.]
No temperature compensation step for the 4-colour path (controller uses internal sensor / OTP). No public partial or
fast mode for NOTE4C. Busy timeout 120 s used in youn-ink/emini, 180 s in TRMNL.

### 3.2 NOTE4 (mono) — itopinion `components/zectrix_epd` (official, Zectrix Lab, MIT) [V]
OTP select (after every reset; the driver **resets + re-selects OTP before every refresh**, because the SSD2683 does
not reliably accept a second refresh after POF):
```
power on rail, reset as above
CMD 0x00 PSR: 0x2F, 0x0E        ; youn-ink/xiaozhi/zectrix-rs/transitink use 0x2F, 0x2E (bit5 of byte B differs)
CMD 0xE9: 0x01 ; wait BUSY
```
Temperature → OTP (full 1bpp refresh):
```
CMD 0x40 ; wait BUSY ; read 1 byte T (3-wire SPI read, ≤8 MHz) (fallback 25 °C)
enc = T<=5:232, <=10:235, <=20:238, <=30:241, <=127:244 (else 244; youn-ink else 232)
CMD 0xE0: 0x02 ; CMD 0xE6: enc ; CMD 0xA5 ; wait BUSY ; 10 ms
CMD 0x10 + 30 000 bytes: each 1bpp bit b → 2-bit code 0b0b (0=black→00, 1=white→01)
CMD 0x04; wait; CMD 0x12 0x00; wait; CMD 0x02 0x00; wait
```
Partial 1bpp (requires a valid full-frame "shadow" of the previous image):
```
reset + OTP select
CMD 0x50: 0x77 ; CMD 0xE0: 0x00 ; CMD 0xA5 ; wait ; 10 ms
CMD 0x83: x0>>8&3, x0, x1>>8&3, x1, y0>>8&3, y0, y1>>8&3, y1, 0x01   ; x0 = x&~7, x1 = 8-aligned end − 1
CMD 0x10; wait ; per pixel 2-bit transition code = (old<<1)|new   (00 B→B, 01 B→W, 10 W→B, 11 W→W)
CMD 0x04 … 0x12 … 0x02 as above
```
16-grey (4bpp) full refresh: white OTP base frame, then external-waveform session:
`0x00 {0x2F,0x8E}` (LUT from register), `0x01 {0x07, wf[0..4]}`, `0x30 wf[6]`, `0x82 wf[5]`,
`0x06 {0x0F,0x8B,0x9C,0xAA}`, `0xE7 0x98`, `0x50 0x37`, `0x61 {0x01,0x90,0x01,0x2C}`, `0x62 {0x64,0x53}`,
`0x65 {0,0,0,0}`, `0xE9 0x01`, wait, `0x20` + 535-byte waveform (7 analog bytes + 6 tables × 88 B); then N passes of
{0x10 data, 0x04 (first pass), 0x12 0x00} with a different LUT per pass; finally 0x02 and reset+OTP restore.
Waveform tables: `_refs/zectrix-note4-epd-demo/components/zectrix_epd/private_include/ssd2683_waveform.h`
(also "0.3s" fast-BW LUT reconstruction). **Mono panel only** — do not try on NOTE4C.

Upstream xiaozhi (`78/xiaozhi-esp32 main/boards/zectrix/zectrix-s3-epaper-4.2`) also contains SSD1683-style
0x44/0x45/0x4E/0x4F window commands at ~l.1107 — likely dead/legacy code copied from an SSD1683 driver [?]. TransitInk
calls its driver "Ssd1683" but actually sends the SSD2683 sequence above (misnomer) [V].

### 3.3 Refresh times
- NOTE4C full: wiki ">10 s"; emini measured ~25 s on USB. [V/I]
- NOTE4 mono: no published figures. "0.3s" fast-BW LUT name in itopinion header hints at sub-second fast mode [?].
  Tinnci/zectrix-note4-epd-lab is set up to measure this (no results yet).

---

## 4. Power-management facts for firmware
- First thing in `app_main`/bootloader hook: GPIO17 = 1 with `gpio_hold_en` (+ `gpio_deep_sleep_hold_en`). [V]
- Deep sleep: keep GPIO17 held HIGH, hold GPIO6 LOW (panel off), GPIO42/46 LOW (audio off); wake on GPIO0 (ext0,
  level low) and/or RTC INT GPIO5 / timer. [V TRMNL + youn-ink]
- "Power off" = release GPIO17 (only works on battery; on USB the rail stays up, so reference demo clears the panel and
  deep-sleeps instead). [V wiki guide]
- LED pin 3 also held with gpio_hold in youn-ink.
- Charger status: charging = GPIO2 LOW; full = GPIO1 HIGH; "USB present" ≈ either. [V TRMNL zectrix_power.cpp]

---

## 5. Firmware & source landscape

| Repo | URL | License | Device | What it covers / notes |
|---|---|---|---|---|
| itopinion/zectrix-note4-epd-demo | https://github.com/itopinion/zectrix-note4-epd-demo (HEAD ca285c9, rel v1.0.0) | MIT, © Zectrix Lab | NOTE4 | **Official** ESP-IDF 5.4+ reference: `zectrix_epd` SSD2683 driver (full/partial 1bpp, 4bpp grey), board adapter (ES8311, PCF8563, NFC, charger, battery, LED, buttons), self-tests, image tools. Partition: nvs 0x9000/0x6000, phy 0xf000, factory 0x10000/3 MB. Release bin listed in `tools/vendor-files.txt` (SHA-256 a3649af0…, app built with IDF v5.5.2). |
| LazyYoun/youn-ink-fourcolor-firmware (branch `2bp`) | https://github.com/LazyYoun/youn-ink-fourcolor-firmware — cloned full; **51812e4** = 2026-08-22 "docs: remove internal remote reference"; HEAD 8907e59 only adds Wi-Fi provisioning fixes (board/driver files unchanged) | MIT, © 2026 macheng2017 | **NOTE4C** (Kconfig can also build 1bpp) | Wiki's official NOTE4C reference. xiaozhi-esp32 fork reduced to "FT" (factory-test) firmware; ESP-IDF 6.0; `main/boards/zectrix-s3-epaper-4.2/` has config.h, SSD2683 driver, power BSP, PCF8563, charge status; `firmware/4D2_BWRY_400x300_2683_test(2)(1).c` = panel vendor MSP430 sample (not redistributed, see §8). Mirror: itopinion/youn-ink-fourcolor-firmware. Prebuilt merged image `notellm-2bp-v6.5.9-…-mergebin.bin` listed in `tools/vendor-files.txt` (SHA-256 26f209f7… matches wiki). |
| 78/xiaozhi-esp32 (upstream) | https://github.com/78/xiaozhi-esp32/tree/main/main/boards/zectrix/zectrix-s3-epaper-4.2 (added #2154 2026-07-27) | MIT | NOTE4 (1bpp only) | Official xiaozhi board `zectrix-s3-epaper-4.2`. Sparse clone in `_refs/xiaozhi-esp32-upstream`. |
| cattei/xiaozhi-zectrix (branch `zectrix`) | https://github.com/cattei/xiaozhi-zectrix/tree/zectrix | MIT | NOTE4 | xiaozhi port + dual-OTA switch (hold Up+Down ~500 ms at boot → swap to other OTA slot, i.e. keep the ZECTRIX image in the other slot). IDF 6.0.2. |
| LitoMore/trmnl-firmware (branch `zectrix`) | https://github.com/LitoMore/trmnl-firmware/tree/zectrix, release zectrix-v1.8.10 | **GPL-3.0** | NOTE4 + NOTE4C | PlatformIO/Arduino; envs `zectrix_note4`, `zectrix_note4c`; native SSD2683 sequence "recovered from the NOTE4C factory firmware"; good pin/power doc (read online, see §8). Don't copy code into a non-GPL repo. |
| eyaeya/today-is-friday | https://github.com/eyaeya/today-is-friday | MIT | NOTE4C | youn-ink-based; RTC wake/deep sleep; merged 0x0 / app 0x20000 releases. |
| fiedoruk/emini-home | https://github.com/fiedoruk/emini-home | MIT | NOTE4C | Plain-C ESP-IDF 6.0 driver `firmware/main/home_panel.c` adapted from youn-ink 51812e4; best NOTE4C hardware doc (read online, see §8); `tools/preflight.py` with factory hashes; installs by writing only 0x8000 + 0x20000. |
| DennisGaida/fourcolor-eink-firmware | https://github.com/DennisGaida/fourcolor-eink-firmware | MIT | NOTE4C | youn-ink fork (gallery, weather, busy-light). |
| alexclmy/Paperwake | https://github.com/alexclmy/paperwake | MIT | NOTE4C | youn-ink-based firmware + local Node "Control Tower". |
| Tinnci/zectrix-note4-platform | https://github.com/Tinnci/zectrix-note4-platform | MIT (© Zectrix Lab header) | NOTE4 | Platform derived from the demo. |
| Tinnci/zectrix-note4-epd-lab | https://github.com/Tinnci/zectrix-note4-epd-lab | MIT | NOTE4 | Waveform/refresh measurement protocols. |
| zzccppp/zectrix-rs | https://github.com/zzccppp/zectrix-rs | **no license file** (all rights reserved) | NOTE4 | **Rust no_std** (esp-hal 1.1, embassy, esp-radio, Slint). SSD2683 1bpp full refresh port (`src/epd.rs`), holds GPIO17. Reference only. |
| counhopig/inkwash-firmware | https://github.com/counhopig/inkwash-firmware | Apache-2.0 | NOTE4 | **Rust on esp-idf (std)**, wraps the C++ `zectrix_epd` component; PCF8563 alarms, BLE config. |
| Zerie55699/transitink-os | https://github.com/Zerie55699/transitink-os | PolyForm Noncommercial | NOTE4 | PlatformIO; has backup/restore scripts; driver misnamed Ssd1683. |
| shantanugoel/hearth | https://github.com/shantanugoel/hearth | MIT | NOTE4 | IDF 6.1; `docs/FACTORY_FLASH.md` + `tools/note4_flash.py` dump/verify/restore with MAC guard. |
| LazyYoun/zectrix-note4-4bp-gallery | https://github.com/LazyYoun/zectrix-note4-4bp-gallery | MIT | NOTE4 | 16-grey gallery (not 4-colour!). |
| others (NOTE4C) | yewenkai/note4c-codex-quota-dashboard (MIT), siximuzhu/esp32-color-eink-stock-monitor (MIT), rvben/paperplane (MIT, NOTE4+4C) | | | not cloned |
| tuya/TuyaOpen | boards/T5AI/ZECTRIX_T5AI_NOTE_4 | Apache-2.0 | T5AI SKU | different MCU; ignore |

Not found: no official "zectrix" GitHub org (official code lives under **itopinion** — the Zectrix Lab maintainer
account); no ESPHome config; no GxEPD2 support for SSD2683 (TRMNL uses bb_epaper only as a renderer with the
GDEM042F52/EP42YR profile and bypasses its init); no Arduino library other than the TRMNL port.
Many other GitHub "zectrix" repos are cloud-API clients (todo sync, dashboards pushing images via cloud.zectrix.com
Open API) — not firmware.

---

## 6. Factory firmware, flash layout, backup/restore

### 6.1 NOTE4C factory partition table [V]
The partition table in the published NOTE4C merged image (sha256 of 0x8000 page
`ce40cfe75056ef74bc052942f8a9ee3dce8e5e14ba17a6f63685a8fa0d11a23d`) is **byte-identical to the factory table on a
real NOTE4C** (emini-home `preflight.py` `FACTORY_TABLE` has the same hash). It is xiaozhi-esp32's `partitions/v2/16m.csv`:

| Name | Type/Sub | Offset | Size |
|---|---|---|---|
| (bootloader) | — | 0x0 | 0x8000 |
| (partition table) | — | 0x8000 | 0x1000 |
| nvs | data/nvs | 0x9000 | 0x4000 (16 K) |
| otadata | data/ota | 0xD000 | 0x2000 |
| phy_init | data/phy | 0xF000 | 0x1000 |
| (unused gap) | — | 0x10000 | 0x10000 (emini uses it as `home_nvs`) |
| ota_0 | app/ota_0 | 0x20000 | 0x3F0000 (4032 K) |
| ota_1 | app/ota_1 | 0x410000 | 0x3F0000 |
| assets | data/spiffs(0x82) | 0x800000 | 0x800000 (8 M) |

Reference NOTE4C app: esp_app_desc project `xiaozhi`, version `6.5.9`, IDF `v6.0-dirty`; bootloader header
`E9 03 02 4F` → DIO, 16 MB, 80 MHz. **The factory bootloader differs from the reference image's bootloader**
(factory 0x0–0x8000 sha256 `7d914bc1…d025` per emini; reference image `eb5bba25…78af`). Factory otadata sha256
`8ba3b110…91a3`. So the stock NOTE4C firmware is a xiaozhi-esp32-derived OTA image. [V hashes / I lineage]

### 6.2 NOTE4 layouts
- Consumer NOTE4: official xiaozhi-based firmware flashed via ZECTRIX Cloud "Local Upgrade" (Web Serial). Layout not
  published; very likely the same xiaozhi 16m table (cattei's dual-OTA switch relies on ZECTRIX image in the other OTA
  slot) [I].
- NOTE4 devkit "beta demo": layout unknown [?]; the public demo v1.0.0 uses nvs 0x9000/0x6000, phy 0xF000,
  factory 0x10000/0x300000 (no OTA) [V].

### 6.3 Backup / restore procedure (consensus of wiki + TRMNL + emini + hearth)
```
# (if port missing: hold BOOT(GPIO0), tap RESET pinhole, release BOOT)
esptool --chip esp32s3 -p /dev/ttyACM0 get-security-info          # expect Secure Boot / Flash Encryption: Disabled
esptool --chip esp32s3 -p /dev/ttyACM0 -b 460800 read-flash 0 ALL note4c-backup-a.bin   # 16 MiB, do twice & compare
esptool --chip esp32s3 -p /dev/ttyACM0 verify-flash 0x0 note4c-backup-a.bin
# restore:
esptool --chip esp32s3 -p /dev/ttyACM0 -b 460800 write-flash 0x0 note4c-backup-a.bin
```
- Keep the physical Down/Power button pressed during first boot of a new image on battery (TRMNL) — or stay on USB —
  otherwise the latch (GPIO17) may drop before the new firmware asserts it.
- Backups contain NVS with Wi-Fi credentials and possibly cloud device keys; keep private. The ZECTRIX updater reads
  eFuse BLOCK2 as a device "UUID" (cloud identity) — eFuses are not touched by flashing. [V updater JS]
- Minimal-footprint install used by emini: write only partition table (0x8000) + app (0x20000), preserving
  bootloader, factory NVS, otadata, ota_1 and assets. Useful pattern for our repo (keep factory ota_1 as fallback).
  Note: if otadata points at ota_1, the app at 0x20000 won't boot — erase/set otadata accordingly [I].

### 6.4 Web flasher
- https://zectrix.com/en/firmware-updater.html (Chrome/Edge Web Serial; esptool-js 0.5.7). Flashes a local merged
  `.bin` at 0x0. Optional "keep settings": reads partition table at 0x8000 (0xC00 bytes), finds the NVS partition,
  backs it up, flashes, and restores NVS into the new table's NVS; or generates a fresh 16 KB NVS at 0x9000 with
  namespace `wifi`, keys `ssid`/`password` (xiaozhi convention). (read online, see §8).
- https://cloud.zectrix.com → Local Upgrade (consumer NOTE4 only, official images).
- Community: esp32ai.me/install (emini), zerie55699.github.io/transitink-os.

---

## 7. Open questions / uncertainties
1. **Exact panel part numbers** for both devices (FPC label). GDEM042F86 (BWRY, SSD2683ZA) is a plausible NOTE4C match
   but unverified; NOTE4 mono panel unknown (SSD2683 in a B/W panel is unusual; TRMNL uses the GDEM042F52 profile for
   rendering only). → photograph the FPC label.
2. **PSR second byte for mono OTP**: official driver `0x2F,0x0E` vs xiaozhi/youn-ink/zectrix-rs `0x2F,0x2E`. Both
   reportedly work; prefer the official value.
3. Undocumented SSD2683 commands `0xE9`, `0xA5`-as-command, and meaning of `0xE0/0xE6` encodings (232…244) — only
   empirical. Datasheet rev 0.20 is a "proposal"; a newer rev may exist.
4. **NOTE4C partial / fast refresh**: no public implementation; SSD2683 supports PTLW (0x83) in hardware, but the BWRY
   OTP waveform behaviour in partial mode is untested. Temperature compensation for 4-colour (does OTP use the internal
   sensor automatically?) unverified.
5. NFC IC: NT3H2111/2211 (wiki) vs GT23SC6699 (vendor config.h). Probably the Chinese clone on real boards.
6. Charger IC, buck regulator and speaker amp part numbers not printed on the schematic (NS4150B pinout match only).
7. Battery ADC divider resistor values not printed; "2:1" is from firmware (×2). Calibrate on hardware.
8. Stock NOTE4 devkit beta-demo partition layout and NOTE4 consumer layout: not verified — dump before flashing.
9. Whether GPIO18 can be used as a deep-sleep wake source reliably (shared with power switch path).
10. I2C pull-ups are on AVDD_3V3 (audio rail, GPIO42): RTC/NFC I2C likely needs GPIO42 HIGH (or internal pull-ups).
    Verify on hardware.
11. Board revisions: everything is PCB V1.0; wiki warns a future international consumer version may omit NFC.

---

## 8. Sources (not redistributed)

The vendor material behind these notes belongs to its owners (ZECTRIX / Zectrix Lab, Solomon
Systech, Espressif, NXP, and the community authors). This repository does not contain copies.
Paths in the notes above such as `common/datasheets/…` are relative to `docs/vendor/` after
running `just fetch-vendor`.

- `tools/vendor-files.txt`: schematic V1.0 (valid for both devices), datasheets (SSD2683
  rev 0.20, ESP32-S3, PCF8563, NT3H2111/2211), control/port diagrams and device photos, and
  the official NOTE4 demo and NOTE4C merged firmware images, each with its URL and the
  SHA-256 recorded at research time. `just fetch-vendor` downloads and verifies them. (ES8311:
  the official PB URL is now 404; see https://www.lcsc.com/datasheet/C962342.pdf.)
- `tools/vendor-pages.txt`: the wiki pages (en + zh), zectrix.com product and updater pages,
  and the community docs (emini-home HARDWARE/INSTALL, epd-demo HARDWARE/EPD_API/QUICK_START/
  TEST_CRITERIA, TRMNL port notes) that were read. Read them online.
- The panel vendor's BWRY MSP430 sample (`4D2_BWRY_400x300_2683_test(2)(1).c`) is in the
  `firmware/` directory of youn-ink-fourcolor-firmware; its licence is not stated.
- `tools/refs.txt`: reference firmware repositories; `just fetch-refs` shallow-clones them into
  `docs/vendor/_refs/` for reading only. Mind each licence: trmnl-firmware is GPL-3.0,
  transitink-os PolyForm-NC, zectrix-rs has no licence. Do not copy code from them.
