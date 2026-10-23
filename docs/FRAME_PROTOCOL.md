# Frame protocol (v1)

`apps/note4c` is a thin client. All rendering happens on a server you run; the device fetches
a finished frame, shows it and sleeps. This page is the contract between the two. The server
can be anything that answers HTTPS: a small Python service, a workflow engine such as
Windmill or Node-RED, a Home Assistant add-on, a static file behind nginx.

## Request

On every wake (timer, OK button, power-on), the device sends one request to the configured
`FRAME_URL`:

```
GET <path and query of FRAME_URL> HTTP/1.0
Host: <host>[:<port>]
User-Agent: zectrix-epaper-rs/<version>
Accept-Encoding: identity
Connection: close
Authorization: Bearer <FRAME_KEY>         (only when FRAME_KEY is set)
X-Wake: Timer Some(CoreDeepSleep)
X-Battery-mV: 3912                         (absent after power-on or without a reading)
X-Last-Display: frame ok 19.4s             (absent after power-on)
If-None-Match: "<etag of the frame shown>" (absent after a press of OK or a failed refresh)
```

- **TLS** is required. The certificate is checked against one trust anchor: ISRG Root X1
  (Let's Encrypt) by default, or the PEM given in `FRAME_CA_PEM` at build time. A
  self-signed server certificate works if you put that certificate (or your private CA) in
  `FRAME_CA_PEM`.
- **Which screen** the device wants is up to you: put it in the URL, e.g.
  `https://frames.example.com/frame?device=note4c&screen=weather`.
- **Device health headers.** The serial port is rarely attached to a device on the wall, so
  every request tells the server how the device is doing. Log them.

  | Header | Example | Meaning |
  | --- | --- | --- |
  | `X-Wake` | `Timer Some(CoreDeepSleep)` | wake-up sources (`Timer` = scheduled, `Gpio` = OK button, `none` = reset or power-on), then the chip's reset reason |
  | `X-Battery-mV` | `3912` | battery voltage measured on the previous wake |
  | `X-Last-Display` | `frame ok 19.4s`, `frame BusyTimeout(Refresh) 120.0s` | result of the last panel refresh (`frame` or the diagnostic `card`) and how long it took |

- A press of OK always fetches **without** `If-None-Match`, so the user can force a redraw.

## Response

| Status | Body | Device does |
| --- | --- | --- |
| `200` | exactly **30 000 bytes**, `Content-Type: application/octet-stream` | shows the frame (10–25 s full refresh on the four-colour panel), stores the `ETag` |
| `304` | none | leaves the panel alone (no refresh, saves battery) |
| anything else | optional short text, e.g. a JSON error | counts a failure, keeps the last frame, retries in 15 min; the first 40 bytes of the body appear on the diagnostic card |

Headers the device reads:

- `Content-Length`: required. Chunked transfer encoding is not supported.
- `ETag`: up to 64 bytes; sent back as `If-None-Match` next time. Without an ETag, every
  fetch redraws the panel.
- `X-Next-Refresh: <seconds>`: how long to deep-sleep before the next fetch. The device
  clamps it to 5 min … 12 h; without the header it sleeps 2 h. Use it to skip the night or
  to wake right after an event changes.

The device allows the server up to 90 s to answer, so the server may render on demand.

## Frame format (NOTE4C, 400 × 300, four colours)

The body is the SSD2683's native RAM layout, sent to the panel as is:

- 300 rows of 100 bytes, top row first; landscape, 400 px wide.
- 2 bits per pixel, 4 pixels per byte, **most significant bits = leftmost pixel**.
- Codes: `00` black, `01` white, `10` yellow, `11` red.

A white frame is 30 000 bytes of `0x55`. Converting a 400 × 300 palette image in Python:

```python
from PIL import Image
PALETTE = Image.new("P", (1, 1))  # palette index = 2-bit code: black, white, yellow, red
PALETTE.putpalette([0, 0, 0, 255, 255, 255, 255, 255, 0, 255, 0, 0] + [0, 0, 0] * 252)
def to_frame(img: Image.Image) -> bytes:
    px = img.convert("RGB").quantize(palette=PALETTE, dither=Image.Dither.NONE).tobytes()
    return bytes((a << 6) | (b << 4) | (c << 2) | d
                 for a, b, c, d in zip(*[iter(px)] * 4))
```

This maps every pixel to the nearest pigment, so anti-aliased text works; pass
`dither=Image.Dither.FLOYDSTEINBERG` for photos. Yellow and red need features at least 2 px wide
to show reliably; thin coloured lines and small coloured text wash out.

## Behaviour on errors

- The device never replaces the shown frame with something that looks current. On a failed
  fetch it keeps the panel as it is and retries in 15 minutes.
- Only before the first frame, or after 8 failures in a row (about 2 h), does it draw its own
  diagnostic card (Wi-Fi, IP, fetch result, battery, last panic).
- A failed panel refresh drops the stored ETag, so the next fetch gets a `200` and redraws.
  The device retries once after 15 minutes, then follows `X-Next-Refresh` again.
- The server cannot know that the device is offline. If stale data matters, draw a
  "rendered at 06:02" time into the frame.

## Testing a server

```sh
curl -sS -D - -o frame.bin -H "Authorization: Bearer $FRAME_KEY" "$FRAME_URL"
stat -c %s frame.bin          # 30000
```

A planned v2 for the mono NOTE4 (screen bundles with highlight strips, events, partial
refresh) is described in [INTERACTIVE.md](INTERACTIVE.md).
