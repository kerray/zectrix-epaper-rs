# Guide for coding agents

Rust `no_std` firmware for the ZECTRIX NOTE4 (mono) and NOTE4C (4-colour) e-paper devkits
(ESP32-S3 N16R8, Solomon SSD2683, 4.2" 400×300). This file is for an agent helping someone
build their own thing on these boards. Read `README.md` first, then `docs/HARDWARE.md`,
`docs/FLASHING.md` and `docs/FRAME_PROTOCOL.md` as the task needs.

## Layout

| Path | What |
| --- | --- |
| `crates/ssd2683` | panel driver: full refresh, mono + 4-colour frames, embedded-graphics (host-tested) |
| `crates/zectrix-board` | pin map, battery maths; `hal` feature: power latch, ADC, deep sleep |
| `crates/zectrix-net` | Wi-Fi, HTTPS GET, HTTP parser, dotenv reader (parser parts host-tested) |
| `apps/note4c` | NOTE4C frame client (fetches a finished frame, shows it, sleeps) |
| `apps/note4` | NOTE4 test card; the simplest template for a new app |
| `partitions/` | factory partition tables of both devices |

## Build facts

- **Toolchain:** `just setup` (needs rustup with stable) installs espup, espflash and the
  Xtensa toolchain `esp` (version pinned in the justfile and CI). Source `~/export-esp.sh`
  in every shell. Build scripts also need a host C compiler (`cc`).
- **Host tests:** `cargo test` at the root. `default-members` covers only `crates/*`.
- **Firmware:** `just build <app>` (= `cd apps/<app> && cargo build --release`). The Xtensa
  target and `build-std` live in `apps/<app>/.cargo/config.toml`; do not build apps from
  the root.
- **Everything CI runs:** `just check` (tests, host clippy, fmt, both apps, app clippy).
- **Configuration:** `apps/note4c` needs `WIFI_SSID`, `WIFI_PASSWORD` and `FRAME_URL` at build
  time (environment or a git-ignored `.env`, see README "Configuration"; `cp .env.example
  .env`). Dummy values are fine for a build check.
- **esp-hal ~1.2.2** with the `unstable` feature. Deep sleep: `rtc_cntl::sleep::LowPower`
  plus `gpio::WakeupConfig` and `Input::listen(Event::LowLevel)`. Pad hold:
  `Output::set_pad_hold`. These APIs are recent: check them against the source in
  `~/.cargo/registry`, not against older examples or memory.
- Keep personal data out of commits: no real host names, keys, SSIDs or locations. Use
  `example.com` in tests and docs.

## Hardware rules that must not be broken

- Drive GPIO17 (power latch) high **first** in `main` and keep it held, or the device turns
  off on battery. Flash and test on USB power.
- Panel BUSY (GPIO8) is **low while busy**. A four-colour refresh takes 10–25 s; keep the
  120 s timeouts.
- **NOTE4C:** full refresh only. Never send the mono PSR, temperature or LUT sequences.
- **NOTE4:** reset and select OTP again before every refresh.
- Never send a frame for one panel type to the other (the driver refuses; keep `Panel` right).

## Flashing: safe workflow

Flashing, erasing and restoring are hard to undo. **Ask the human before each of them**,
and say which device and app.

1. `just backup <app>`: two full 16 MiB reads, compared. Backups go to the git-ignored
   `backups/` and contain Wi-Fi credentials; never commit or upload them.
2. `just partitions backups/<app>-<stamp>-a.bin`: must say NOTE4C or NOTE4 table. If it
   says "unknown", stop.
3. `just flash <app>`: writes only `ota_0` at `0x20000`, verifies it, blanks `otadata`,
   keeps the factory bootloader. It refuses without a backup or with the wrong table.
4. `just monitor <app>` for logs (on WSL/USB-IP an attached monitor resets the chip; the
   firmware draws diagnostics on the panel instead).

Never use `espflash flash`, `espflash erase-flash` or `cargo run` on a device: they
overwrite or erase the factory bootloader. If the port is gone (deep sleep), press OK, or hold
BOOT/OK and press the RESET pinhole for download mode. `just restore <backup>` puts a full
backup back.

## Adding your own app

Copy `apps/note4` (mono) or `apps/note4c` (4-colour, with networking) to `apps/<name>`:

- `Cargo.toml`: change `name` and the `[[bin]] name`; keep the dependencies you use.
- Root `Cargo.toml`: add `apps/<name>` to `members` (not `default-members`).
- Keep `.cargo/config.toml` and `esp-config.toml` (target, `build-std`, disabled `cargo run`)
  and `build.rs` (linker helpers; note4c's also reads the `.env`).
- In `main`: latch GPIO17 first, keep `esp_app_desc!()` (the factory bootloader needs it),
  create `Ssd2683::new(…, Panel::Mono)` or `Panel::Quad` to match the device.
- `justfile`: `image` and `flash` choose the partition table by app name (`note4` → NOTE4
  table, anything else → NOTE4C). For a NOTE4 app, extend those `if app == "note4"`
  expressions. Add the app to `check` and to the CI matrix.
- Backups are looked up by app name: `just backup <name>` before `just flash <name>`.

## Writing a frame server

`apps/note4c` fetches `FRAME_URL` over HTTPS and expects `200` with exactly 30,000 bytes
(`application/octet-stream`, `Content-Length`, optional `ETag` and `X-Next-Refresh`), or
`304`. The contract is `docs/FRAME_PROTOCOL.md`. Frame layouts (rows top-down, MSB =
leftmost pixel):

| Device | Bytes | Pixel |
| --- | --- | --- |
| NOTE4C | 30,000 (100 per row) | 2 bit: `00` black, `01` white, `10` yellow, `11` red |
| NOTE4 (frame client planned) | 15,000 (50 per row) | 1 bit: `1` white, `0` black |

Minimal server (Python, Pillow + Flask):

```python
import hashlib
from flask import Flask, Response
from PIL import Image, ImageDraw

app = Flask(__name__)
PALETTE = Image.new("P", (1, 1))  # index = 2-bit code: black, white, yellow, red
PALETTE.putpalette([0, 0, 0, 255, 255, 255, 255, 255, 0, 255, 0, 0] + [0, 0, 0] * 252)

def quad_frame(img):  # NOTE4C, 400x300 -> 30,000 bytes
    px = img.convert("RGB").quantize(palette=PALETTE, dither=Image.Dither.NONE).tobytes()
    return bytes(a << 6 | b << 4 | c << 2 | d for a, b, c, d in zip(*[iter(px)] * 4))

def mono_frame(img):  # NOTE4, 400x300 -> 15,000 bytes (mode "1": MSB first, 1 = white)
    return img.convert("1", dither=Image.Dither.NONE).tobytes()

@app.get("/frame")
def frame():
    img = Image.new("RGB", (400, 300), "white")
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, 399, 40), fill=(255, 0, 0))
    d.text((10, 60), "Hello NOTE4C", fill=(0, 0, 0))
    body = quad_frame(img)
    return Response(body, mimetype="application/octet-stream", headers={
        "ETag": '"%s"' % hashlib.sha1(body).hexdigest()[:16],
        "X-Next-Refresh": "1800",  # seconds; clamped to 5 min .. 12 h
    })
```

This answers `200` every time; to save refreshes, return `304` when the request's
`If-None-Match` equals the ETag. Serve it behind HTTPS (a Let's Encrypt certificate works
with the default trust anchor; otherwise set `FRAME_CA_PEM`). Check
`Authorization: Bearer <FRAME_KEY>` if you set a key. Test with the `curl` line in
`docs/FRAME_PROTOCOL.md`.

## Licensing

- This repo is `MIT OR Apache-2.0`. Ported logic comes from MIT projects; when you port or
  copy more, add it to `THIRD_PARTY_NOTICES.md` with the full licence text.
- Do not copy code from `trmnl-firmware` (GPL-3.0), `transitink-os` (PolyForm-NC) or
  `zectrix-rs` (no licence). Reading them for facts is fine.
- Reference clones go in `docs/vendor/_refs/` (git-ignored; `just fetch-refs`, list in
  `tools/refs.txt`).
- Vendor documents, datasheets, images and firmware images are not redistributed. `just
  fetch-vendor` downloads them into the git-ignored `docs/vendor/` (list with SHA-256 in
  `tools/vendor-files.txt`). Only `docs/vendor/RESEARCH_NOTES.md` is tracked.
