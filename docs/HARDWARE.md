# Hardware: ZECTRIX NOTE4 and NOTE4C Devkits

This is a short reference. The full research with sources, confidence markers and open
questions is in [vendor/RESEARCH_NOTES.md](vendor/RESEARCH_NOTES.md). The schematic
([wiki.zectrix.com](https://wiki.zectrix.com/sch_zectrix_note4_developer_kit_v1.0_en.pdf)), the
SSD2683 datasheet and the other vendor files are not redistributed here: `just fetch-vendor`
downloads them into `docs/vendor/` and checks their hashes (`tools/vendor-files.txt`).

## The two devkits

Both are sold by Zectrix Lab:

| Unit | Panel | App in this repo |
| --- | --- | --- |
| **NOTE4 Developer Kit** | 4.2" 400×300 black/white, partial refresh, 16 grey levels possible | `apps/note4` |
| **NOTE4C Devkit** | 4.2" 400×300 black/white/yellow/red (Spectra-class), full refresh only, 10–25 s | `apps/note4c` |

Both have the **same V1.0 main board** and **the same pin map**: an ESP32-S3 **N16R8**
(16 MiB flash, 8 MiB octal PSRAM, native USB-Serial-JTAG), a **Solomon SSD2683** e-paper
controller, a PCF8563 RTC, an ES8311 audio codec with mic and speaker, NFC, and a
2000+ mAh Li-ion battery. Only the panel on the 24-pin FPC differs. Factory images are
**not interchangeable**, because each image initialises its own panel.

## Pin map (both devices)

| Function | GPIO | Notes |
| --- | --- | --- |
| EPD power | 6 | high = panel rail on; hold low in deep sleep |
| EPD BUSY | 8 | **low = busy** |
| EPD RST / DC / CS | 9 / 10 / 11 | CS driven in software |
| EPD SCK / MOSI | 12 / 13 | SPI mode 0; MOSI is bidirectional for the temperature read (≤ 8 MHz) |
| **Power latch** | 17 | **drive high at once on boot and hold through deep sleep**, or the device turns off on battery |
| Button OK / BOOT | 0 | active low; strapping pin; wake source |
| Button Down / Power | 18 | active low; also the power key |
| Button Up | 39 | active low |
| LED | 3 | active low |
| Charge full / charging | 1 / 2 | full = high; charging = **low** |
| Battery | 4 | ADC1 ch3, 2:1 divider, 12 dB attenuation |
| I2C SDA / SCL | 47 / 48 | ES8311 `0x18`, PCF8563 `0x51`, NFC `0x55`; pull-ups sit on the audio rail (GPIO42) |
| RTC INT | 5 | active low |
| NFC FD / power | 7 / 21 | |
| I2S MCLK / BCLK / WS / DIN / DOUT | 14 / 15 / 38 / 16 / 45 | |
| Audio rail / speaker amp | 42 / 46 | active high; 46 is a strapping pin |
| Expansion J3 | 40, 41, TX 43, RX 44 | |

The same values are in `crates/zectrix-board/src/pins.rs`.

## Display controller: SSD2683

The controller RAM is always 2 bpp: 4 px per byte, MSB = leftmost pixel, 100 B per row,
30,000 B per frame.

| | NOTE4C (4-colour) | NOTE4 (mono) |
| --- | --- | --- |
| Codes | `00` black, `01` white, `10` yellow, `11` red | `00` black, `01` white |
| Init | reset, `E9 01` | reset, `00 2F 0E`, `E9 01`; **reset and repeat before every refresh** |
| Before data | – | temperature code → `E0 02`, `E6 <232..244>`, `A5`. The sensor read (`0x40`) is not implemented yet, so the driver uses the official 25 °C fallback (code 241). |
| Refresh | `10` + data, `04`, `12 00`, `02 00` | same |
| Sleep | `07 A5`, rail off | rail off (our driver also sends `07 A5`; the official mono driver only cuts the rail) |
| Partial | none known | `83` window + transition codes (roadmap) |
| Grey | – | 16 levels via external LUT (roadmap) |

`crates/ssd2683` implements both full-refresh paths. Host tests check the command
sequences against the reference C drivers.

## Flash layout (NOTE4C factory, verified by hash)

`nvs 0x9000 16K · otadata 0xd000 8K · phy_init 0xf000 4K · (gap 0x10000 64K) · ota_0 0x20000
4032K · ota_1 0x410000 4032K · assets 0x800000 8M`

The NOTE4 devkit has the same slots with 6080 K apps and 4 M assets (`partitions/note4-devkit-16m.csv`). See [FLASHING.md](FLASHING.md).
