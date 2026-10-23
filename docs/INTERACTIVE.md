# NOTE4 interactive screens (buttons + partial refresh)

The mono NOTE4 becomes a small remote: Up/Down move through a list, OK acts on the selected
item. Example screens: a calendar, home lights, weather, and the last voice answer as a QR
code. This only works because the mono panel can redraw part of the screen. The four-colour
NOTE4C stays a frame board with full refreshes only.

Status, 2026-10-23: I1 (driver) is done and host-tested. I2 (lab screen in `apps/note4`) is
built but not yet run on hardware, so nothing below is measured yet; the numbers to collect
are in "Measurements".

## What the panel can do

From the official driver (`itopinion/zectrix-note4-epd-demo`, MIT,
`components/zectrix_epd/zectrix_epd.cc`, `zectrix_epd_refresh_partial_1bpp`). The command
sequence is in [vendor/RESEARCH_NOTES.md](vendor/RESEARCH_NOTES.md) §3.2.

- **Partial refresh uses the OTP waveforms.** No external LUT is needed.
  - Reset and select OTP first, as for every NOTE4 refresh.
  - Then send `50 77`, `E0 00`, `A5`, and the window `83 x0 x1 y0 y1 01`. x is aligned to
    8 px: x0 is rounded down, the end is rounded up.
  - Then `10` with one 2-bit **transition code** per pixel, `(old << 1) | new`
    (`00` B→B, `01` B→W, `10` W→B, `11` W→W), followed by `04`, `12 00`, `02 00`.
- **The driver must know the old image.** We keep a 15,000 B 1 bpp copy of what is on the
  panel (the "shadow"). Without a valid shadow, a partial refresh would draw garbage, so the
  caller does a full refresh instead. A failed or timed-out refresh invalidates the shadow.
- **Ghosting:** the vendor UI does a full refresh after **8** partial refreshes, and uses full
  refreshes for page changes. We follow the same rule.
- **The "0.3 s" fast black/white waveform** in `ssd2683_waveform.h` is a full-screen external
  LUT. The vendor found it ghosts, so we do not use it for now.
- **Speed is not published.** Measuring it is the first task (see "Measurements").

## Driver API (I1)

`crates/ssd2683`, no allocation; the caller owns both frames (2 × 15,000 B, e.g. in
`static` cells):

```rust
// After a full refresh the shadow is a copy of the frame that was shown.
epd.show_mono(&frame, None)?;
shadow.copy_from(&frame);

// Draw into `frame`, then refresh only the part that changed.
if let Some(rect) = shadow.dirty_rect(&frame) {
    match epd.show_mono_partial(&mut shadow, &frame, rect) {
        Ok(_window) => {}              // `shadow` now matches the panel again
        Err(_) => { /* shadow invalid: next update must be epd.show_mono(...) */ }
    }
}
```

- `show_mono_partial(&mut shadow, &new, rect) -> Result<Rect, Error>`: sends the window
  `rect` widened to whole bytes (x down to a multiple of 8, the right edge up, at most 400)
  and returns that window. The widened pixels also come from `new`. On success it copies the
  window from `new` into `shadow`; on an error it leaves `shadow` alone, but the panel may
  have changed, so the caller must treat the shadow as invalid.
- It does the reset + OTP selection itself (the NOTE4 rule) and uses the same `04`/`12`/`02`
  trigger and 120 s BUSY timeouts as a full refresh.
- Errors: `WrongPanel` on the NOTE4C (nothing is sent), `InvalidRect` for an empty or
  off-panel rectangle (nothing is sent), `NotPowered`, `BusyTimeout(stage)`.
- Helpers: `Rect` (`aligned`, `union`, `is_valid`, `full`), `MonoFrame::dirty_rect`,
  `copy_rect`, `copy_from`.
- Host tests: `crates/ssd2683/tests/partial.rs` (exact sequence of a small window, all four
  transition codes, alignment incl. the right edge, a full-screen window, refusal on NOTE4C,
  invalid rectangles, shadow handling on success and on a timeout).

## Design: the server renders, the device moves the cursor

Waking from deep sleep and fetching over HTTPS takes about 2–4 s. That is fine for opening a
screen but too slow for every Up/Down press. So:

- **The server still renders everything** (e.g. Pillow: fonts, any script and diacritics,
  icons, QR codes), as in [ROADMAP.md](ROADMAP.md) "thin device".
- **A list screen arrives as a bundle:** the base frame (every row drawn unselected) plus one
  small **highlight strip** per selectable row.
  - Up/Down: the device redraws the old row from the base frame and the new row from its
    strip. That is two small partial refreshes with **no network**.
  - OK sends an event. The reply is either a new screen or a patch (for example a light's row
    changing to "on").
- **An interactive session** keeps Wi-Fi up and the shadow in RAM for about 60 s after the last
  press, then the device goes back to deep sleep. The first press after sleep is slow
  (wake + Wi-Fi + fetch); the presses after it are immediate.
- **Before sleeping**, if the session did any partial refreshes, the device does one full
  refresh of the final screen. This leaves a clean image for the hours until the next wake.
- **Keeping the shadow through deep sleep (optional):** RTC memory is about 16 KB in total,
  so 15,000 B does not fit beside our other state. Frames are mostly white, so a simple
  RLE usually compresses them to a few KB. If the shadow fits, keep it; if not, the first
  update after wake is a full refresh. Without this, the first change after every wake is a
  full refresh, which is acceptable.

### Buttons

| Input | Action |
| --- | --- |
| Up / Down short | move the selection (wraps) |
| OK short | act on the selected item |
| OK held (> 0.4 s) | push-to-talk (planned voice assistant, see [ROADMAP.md](ROADMAP.md)) |
| Up held (> 1 s) | back to the screen list |
| Down held 3 s | off (the vendor convention) |

- Only **OK (GPIO0)** and **Down (GPIO18)** are RTC pins, so only they can wake the chip from
  deep sleep. Up (GPIO39) works only while the device is awake. Verify this on hardware.
- **Open:** how to choose between voice modes (for example a fast speech-to-speech model and
  a slower assistant with tools). Options are a settings row, or holding Up while pressing OK.

## Screens

| Screen | Content | Rows / OK does |
| --- | --- | --- |
| Screen list | the screens below | opens the screen |
| Calendar | today + next days from one or more calendars | scroll by day; OK = event detail |
| Lights | lights and groups from an allow-list on the server | OK cycles `off → scene 1 → scene 2 …`; the row shows the new state |
| Weather | a forecast laid out for 1 bpp | none (static) |
| Last answer | the summary of the last voice answer + a QR code of its link | OK = previous answers |

### Lights (example of an action screen)

- The server owns the integration with the home automation system (Home Assistant,
  openHAB, a Hue bridge, …). The device only sends "OK on row n".
- Keep an **allow-list** of entities on the server, with each entity's cycle of states (off,
  scenes, colours). The device can never address anything outside it.
- Row labels and states come from the home automation system, so a light that someone
  switches by hand shows its real state the next time the screen opens.

### Last answer (voice → display)

- After each voice exchange, the voice backend stores a record for the device: question,
  short answer, and any links.
- If the record has a link, the backend tells the device. The NOTE4 then does one partial
  refresh of the "Last answer" area (summary + QR). The QR code is rendered on the server.
- The NOTE4C keeps voice audio-only. At most, it shows a small marker on its next scheduled
  refresh.

## Protocol (draft v2)

Transport, auth and the device-health headers are the same as the v1
[frame protocol](FRAME_PROTOCOL.md). `<base>` is a configured server URL.

- `GET <base>/screen?device=note4&screen=<id>` →
  `application/x-note4-screen`, with `ETag` and `X-Next-Refresh` as in v1:

  ```
  "N4S1"  u8 version=1  u8 flags  u8 n_rows  u8 selected
  15000 B base frame (1 bpp, 1 = white, MSB = left)
  n_rows × { u16 x, u16 y, u16 w, u16 h  (x and w multiples of 8)
             w/8 × h B highlight strip }
  ```

  A full-width 40 px row strip is 2,000 B, so a 10-row list is about 35 KB in total.
- `POST <base>/event` with `{"screen":…, "row":…, "action":"ok"}` → one of:
  - `200` + a new screen bundle;
  - `200` + `application/x-note4-patch`: `u16 x, y, w, h` + 1 bpp pixels. This patch replaces
    that area in the base frame and in the row's strip, so a later cursor move does not
    restore the stale row.
  - `204`: nothing to show.
- Row 0 can be a non-selectable header (`flags` bit 0).

## Measurements (first hardware task)

A test app on the NOTE4 does each refresh and draws its time on the panel. USB/IP logs are
unreliable, see [FLASHING.md](FLASHING.md).

### Lab screen (I2, `apps/note4`)

1. Flash `apps/note4` (`just flash note4`). After a reset or an OK wake the test card
   appears; its last line says "OK within 60 s = lab". (After a 30-minute timer wake it
   sleeps at once.)
2. Press **OK** within 60 s. The lab screen runs the benchmark at once:
   1. full refresh → row 1 "Full refresh";
   2. one 400×40 row (row 3 turns black) → row 2 "Partial 400x40";
   3. rows 1 and 6 (160 px apart) as two calls → row 3 "2 rows, 2 calls";
   4. the same two rows back as one 400×240 window → row 4 "2 rows, 1 window";
   5. the whole screen as a partial (draws the results) → row 5 "Full-scr partial";
   6. one more window (row 5 to the status band) to show that last value.
3. Then:

   | Button | Action |
   | --- | --- |
   | Up / Down | move the black highlight (wraps): two partial refreshes (old row, new row), then one of the status band. Row 6 "Cursor move (2x)" shows the time of the two row refreshes (updated when row 6 is redrawn). |
   | OK | full refresh; row 1 gets the new time |
   | OK held 1 s | run the benchmark again |
   | nothing for 60 s | one full refresh ("Asleep. OK wakes."), then deep sleep |

4. The status band shows the last refresh ("Last: cursor 2x 612 ms", "Last: full 2950 ms",
   or "FAILED: …"), the number of partial refreshes since the last full refresh, and the
   time of the status band's own partial refresh (400×60). There is **no automatic full
   refresh**, so ghosting can be judged after 8, 16 and 32 partials (count them on the
   status band). After a failed refresh the next update is a full refresh.

Times are measured with `esp_hal::time::Instant` around the driver call, so they include the
reset + OTP selection, the SPI transfer (20 MHz) and BUSY polling at 1 ms. Each value is the
total of the calls listed.

What to record after flashing (fill in the table below and take photos):

- the five benchmark values (run the benchmark two or three times with OK held, they
  should repeat);
- the cursor-move time, and whether a move looks like one event or two visible steps;
- ghosting after 8, 16 and 32 moves: photo of the screen in daylight, then press OK and
  compare with the clean full refresh;
- whether the unchanged rows flicker during a partial refresh (they should not);
- whether Up (GPIO39) works while awake (it cannot wake the chip).

| Case | Time | Ghosting |
| --- | --- | --- |
| Full 1 bpp refresh (OTP) | ? | – |
| Partial, one 400×40 row | ? | ? |
| Partial, two rows far apart (two calls) | ? | ? |
| Partial, both rows in one window | ? | ? |
| Partial, full screen | ? | ? |
| 8, 16 and 32 partials in a row, then a photo | – | ? |
| Wake from deep sleep → first partial refresh visible | ? | – |
| Wake → Wi-Fi → screen bundle shown | ? | – |

If a partial refresh is too slow (more than about 1 s), the fallbacks are the fast waveform
with a periodic full refresh, or moving a small marker in a margin instead of inverting
whole rows.

## Milestones

- **I0, NOTE4 bring-up:** backup, partition table, flash recipe for the NOTE4 layout, test
  card (ROADMAP M0). Done 2026-10-23.
- **I1, driver:** `Ssd2683::show_mono_partial` with the shadow and transition encoding;
  host tests of the command sequence and the encoding against the official driver.
  **Done** (2026-10-23).
- **I2, measurement app:** the table above, filled in. The lab screen is built (see "Lab
  screen"); the hardware run is open.
- **I3, session + protocol:** button handling, the 60 s session, and the bundle/patch parser
  (host-tested); an example server with a dummy list.
- **I4, screens:** an action screen (lights), then a calendar and the screen list.
- **I5, last answer:** after the voice track, the answer record and the QR screen.
