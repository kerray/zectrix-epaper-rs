# Audio: ES8311 codec, mic and speaker

Both devices have the same audio parts on the V1.0 main board: an Everest **ES8311** mono
codec, an analog microphone on MIC1P/N (the small pinhole on the front of the case) and a
speaker amplifier (8-pin, pinout matches an NS4150B). This page covers the driver
(`crates/es8311`), the board helpers (`zectrix_board::audio`, `power::AudioPower`) and the
loopback test app (`apps/audio`, voice milestone V1 in [ROADMAP.md](ROADMAP.md)).

Status: built and host-tested; **not yet run on hardware**.

## Pins and power

| Signal | GPIO | Notes |
| --- | --- | --- |
| Audio rail `PA_PWR_EN` | 42 | active high. Also powers the I2C pull-ups R45/R46 |
| Speaker amp `PA_CTRL` | 46 | active high. **Strapping pin**: low at reset, driven only after boot |
| I2C SDA / SCL | 47 / 48 | ES8311 at `0x18` (7-bit); 100 kHz; esp-hal also enables the internal pull-ups |
| I2S MCLK | 14 | 256 × fs = 4.096 MHz at 16 kHz |
| I2S BCLK / WS | 15 / 38 | 512 kHz / 16 kHz, driven by the ESP32-S3 (codec is slave) |
| I2S DIN | 16 | codec → ESP (mic) |
| I2S DOUT | 45 | ESP → codec (speaker) |

Order on the way up: audio rail on, amplifier off → 20 ms → I2S running (MCLK, BCLK, WS) →
codec init over I2C → amplifier on when something plays. On the way down: amplifier off →
codec power-down over I2C → rail off; both pads held low through deep sleep
(`AudioPower::off_for_sleep`). The panic hook (`power::audio_off_after_panic`) does the same
before the usual `halt_after_panic`.

## Codec settings

The register sequences are ported from Espressif `esp_codec_dev` (`device/es8311/es8311.c`,
Apache-2.0). The values are what the official Zectrix board demo (MIT) uses with that driver;
provenance in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

| Setting | Value | Register |
| --- | --- | --- |
| Mode | I2S slave, MCLK from the MCLK pin | `00` = `80`, `01` = `3F` |
| Format | standard (Philips) I2S, 16-bit words, both ports unmuted | `09`, `0A` = `0C` |
| Clocks | MCLK 4.096 MHz / 16 kHz row of the coefficient table | `02`–`08` |
| Mic input | analog MIC1P/N, analog PGA 30 dB | `14` = `1A` |
| Mic gain | digital ADC scale +30 dB (demo: `set_in_gain(30)`) | `16` = `05` |
| ADC volume | 0 dB | `17` = `BF` |
| DAC reference | DAC output looped into the right ADC slot (for echo cancelling later) | `44` = `58` |
| Volume | 0–100 % on the reference curve (`98 + v`); demo default 80 % = `B2` ≈ −6.5 dB. The app starts at 70 %. | `32` |
| Mute | DAC soft mute | `31` bits 6:5 |
| Power down | `es8311_suspend` sequence | `0D` = `FC` at the end |

`Config` makes the sample rate, mic gain, ADC volume, volume and DAC reference configurable.
The clock table covers 8–48 kHz with the usual MCLK values; for 256 × fs, 8, 16, 24, 32 and
48 kHz all have a row.

### I2S on the ESP32-S3 (esp-hal 1.2.2)

- One `I2s::new(I2S0, DMA_CH0, TdmConfig::new_tdm_philips()…)`, 16-bit data in 16-bit
  slots, two slots (stereo), `with_signal_loopback(true)`, `with_mclk(GPIO14)`.
- TX owns BCLK/WS (GPIO15/38) and DOUT (GPIO45). RX gets only DIN (GPIO16); with signal
  loopback its BCLK/WS come from TX internally, so **TX must run for RX to receive**. The app
  therefore streams both directions continuously and sends silence when nothing plays.
- Both directions use `DmaRxStreamBuf` / `DmaTxStreamBuf` rings of 8 KiB (128 ms) in internal
  RAM. The recording (up to 30 s of stereo frames, 1.9 MB) lives in the 8 MB octal PSRAM
  (`Psram::new` with `PsramMode::OctalSpi`, `esp_alloc::psram_allocator!`).
- Frames are 4 bytes: slot 0 then slot 1, little endian. The ES8311 is mono: the app sends
  the same sample in both slots, and on receive it plays back whichever slot was louder
  during the recording (the mic slot; with the DAC reference on, the other slot is the
  silent playback). The log line after each recording shows both peaks, which tells us the
  real slot order.

### 16 kHz up, 24 kHz down?

The relay protocol in the voice design sends PCM16 16 kHz up and 24 kHz down. On this board
both directions share one WS line (one full-duplex I2S peripheral, and the ES8311 has a single
LRCK), so **the bus runs at one rate**. V1 runs both directions at 16 kHz. Options for V2:

1. Keep the bus at 16 kHz and have the relay send 16 kHz (simplest; speech loses little).
2. Keep the bus at 16 kHz and resample the 24 kHz downlink on the device (3:2, a short
   polyphase filter; cheap on the S3).
3. Run the bus at 24 kHz (the codec table has the 6.144 MHz row, `Config { sample_rate:
   24_000, .. }`) and decimate the mic 3:2 to 16 kHz before sending.

Changing the rate means a new `TdmConfig` sample rate and codec `init`; the board's beep
helpers assume `SAMPLE_RATE` = 16 kHz.

## The loopback app

`apps/audio` never touches the e-paper panel, so one image runs on the NOTE4 and the NOTE4C
(the panel keeps whatever it showed). Status is by sound and the green LED (GPIO3).

| Action | Result |
| --- | --- |
| Boot / wake | two rising notes |
| Hold **OK** | one high note, LED on, recording (the first ~230 ms, until the note has left the speaker, are dropped) |
| Release **OK** | two short low notes, LED off, the recording plays back |
| Hold OK for 30 s | recording stops at the limit and plays back; release OK before the next one |
| **Up** / **Down** | volume +10 % / −10 % (0–100, start 70) with a tick at the new volume; ignored while recording |
| 60 s without a press | two falling notes, then codec, amplifier and rail off, deep sleep; **OK** wakes |
| Codec does not answer on I2C | LED blinks fast 10 times, then deep sleep |
| No PSRAM buffer, or an I2C error | long falling buzz |

If OK woke the device, the app waits for it to be released before a press starts a recording.

## On-device test (owner)

Use USB power, and do not keep a serial monitor attached through a whole test over WSL
(see [FLASHING.md](FLASHING.md), WSL notes).

1. `just image audio` builds `target/audio-app.bin`.
2. `just flash audio` (needs a backup of the device it finds, `backups/note4-*-a.bin` or
   `backups/note4c-*-a.bin`; type `audio` to confirm). Then press RESET if the chip stays in
   the bootloader.
3. Expect the startup notes within about a second of reset. Silence: run `just monitor audio`
   once and look for `ES8311 up` or `ES8311 init failed` and the PSRAM size line.
4. Hold OK, say something for a few seconds at 20–30 cm from the front, release. Expect the
   stop notes, then your voice. Note the `recorded … ms; peaks slot0 … slot1 …` log line if a
   monitor is attached.
5. Press Up/Down a few times: ticks get louder/quieter; replay to compare.
6. Leave it for 60 s: falling notes, LED off, USB port disappears (deep sleep). OK wakes it.
7. To go back to the device's own app: `just flash note4` or `just flash note4c`.

What to report back: whether the mic level is usable (peaks of a few thousand for normal
speech are fine; near 32767 means clipping, lower `mic_gain`; below a few hundred means too
quiet), hiss or hum while idle (the amplifier is off when nothing plays), clicks when the
amplifier switches, and the slot peaks.

## Known issues and risks

- **Not tested on hardware.** The register sequences match the reference driver step by step
  (host tests), but the I2S side (slot order, signal loopback, MCLK routing on GPIO14) is
  only checked against esp-hal's source and its HIL test.
- **GPIO46 is a strapping pin.** The app drives it only after boot, starts it low and holds it
  low in deep sleep. Something external pulling it high at reset would change the boot mode.
- **I2C needs the audio rail.** The pull-ups for the whole I2C bus (codec, RTC, NFC) sit on
  GPIO42's rail; with the rail off, only the ESP32-S3's weak internal pull-ups remain. Apps
  that use the RTC or NFC must switch the rail on, or accept a weak bus.
- **Back-powering.** While the rail is off, the I2S and I2C pins must not drive the codec. The
  app stops I2S and powers the codec down before cutting the rail; in deep sleep those pads
  (14, 15, 16, 38, 45, 47, 48) are not held.
- **Stream stalls.** If the main loop ever falls behind by more than 128 ms, a DMA ring runs
  dry or full and that direction stops. The app restarts both streams after 500 ms without
  mic data (a short glitch, logged as `I2S stalled`).
- **Beep in the recording.** The start note is dropped by discarding the first frames; if the
  speaker rings longer than expected, the first ~20 ms of speech may still carry it.
- **Amplifier pops** when it switches on and off around each sound are possible; the codec
  ramps (register `37`) are at the reference setting.
