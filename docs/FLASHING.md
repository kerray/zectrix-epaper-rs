# Backing up and flashing

Both devices ship with a working factory firmware and a factory bootloader. The rules
here keep a way back.

1. **Take two full backups before you write anything**, and check that they are
   identical.
2. **Never write the bootloader** at `0x0`. `espflash flash` and `cargo run` would write
   it, so the `just` recipes use `write-bin` instead.
3. **Write only the app slot `ota_0` at `0x20000` and clear `otadata`**, so the factory
   bootloader starts our app.

This builds on what emini-home verified on a real NOTE4C, but writes less (never the
partition table at `0x8000`) and adds one step: clearing
`otadata`. That flow (write `ota_0`, blank `otadata`) has run our app on a NOTE4C since
early October 2026 and on a NOTE4 since 2026-10-23 (its partition table was read the same
day, step 2). Check the backup before you flash either (step 2 below).

## 0. Tools

`espflash` 4.x (installed with `just setup`). `esptool` 5.x (`pipx install esptool`) also
works and is needed for the security check in step 1; its commands are listed after each
step. Plug the device in with a USB-C **data** cable. It shows up as
`/dev/ttyACM0` (native USB-Serial-JTAG, no USB-UART chip).

If the port does not appear, enter download mode: hold **OK/BOOT**, press the **RESET**
pinhole, then release OK.

On battery, the board stays on only while firmware holds the power latch (GPIO17) high.
A new image that crashes before it latches will switch the device off. **Flash and test
on USB power.**

## 1. Check security, then back up (twice)

Flashing plain images only works while Secure Boot and Flash Encryption are off. Check once
(espflash cannot show this; use esptool):

```sh
esptool --chip esp32s3 -p /dev/ttyACM0 get-security-info   # expect both "Disabled"
```

If either is enabled, **stop**: nothing in this repo applies to that device.

Then take the two backups:

```sh
just backup note4c        # writes backups/note4c-<date>-{a,b}.bin and compares them
# esptool:
esptool --chip esp32s3 -p /dev/ttyACM0 -b 460800 read-flash 0 ALL backups/note4c-a.bin
```

`backups/` is git-ignored because the images contain Wi-Fi credentials and cloud device
keys. Copy them somewhere safe as well.

## 2. Check the partition table in the backup

```sh
just partitions backups/note4c-<date>-a.bin
```

Expected on the NOTE4C (sha256 of the 0x8000 page `ce40cfe7…`, the same as
`partitions/factory-16m.csv`):

| Name | Offset | Size |
| --- | --- | --- |
| nvs | 0x9000 | 16 K |
| otadata | 0xd000 | 8 K |
| phy_init | 0xf000 | 4 K |
| ota_0 | 0x20000 | 4032 K |
| ota_1 | 0x410000 | 4032 K |
| assets | 0x800000 | 8 M |

A NOTE4 devkit (read 2026-10-23, sha256 `a82133fa…`, `partitions/note4-devkit-16m.csv`)
has the same `otadata` and `ota_0` offsets but larger slots:

| Name | Offset | Size |
| --- | --- | --- |
| nvs | 0x9000 | 16 K |
| otadata | 0xd000 | 8 K |
| phy_init | 0xf000 | 4 K |
| ota_0 | 0x20000 | 6080 K |
| ota_1 | 0x610000 | 6080 K |
| assets | 0xc00000 | 4 M |

`just flash <app>` accepts only the table known for that app, so it also refuses to put the
NOTE4 app on a NOTE4C or the other way round. `just flash audio` accepts either table: the
audio loopback app never drives the panel (see [AUDIO.md](AUDIO.md)). If the table is neither of these (the public
NOTE4 demo, for example, uses a single `factory` app at `0x10000`), **stop**. Adapt
`partitions/` and the flash recipe to that layout first.

## 3. Build and flash the app

```sh
just flash note4c          # build → espflash save-image → write-bin 0x20000 → erase otadata
just monitor note4c        # or: just monitor note4
```

If you answer anything but the app name at the prompt, nothing is written, but the chip
stays in the bootloader (the table check left it there) until you press RESET.

What `just flash` does (it refuses when the device's 0x8000 page is not a table known for
that app, or when there is no `backups/<device>-*-a.bin` for the device it found):

```sh
cargo build --release          # in apps/note4c
espflash save-image --chip esp32s3 --flash-size 16mb \
  --partition-table partitions/factory-16m.csv --target-app-partition ota_0 \   # note4: note4-devkit-16m.csv; audio: factory (fits both)
  <elf> target/note4c-app.bin
espflash read-flash 0x8000 0x1000 …     # compare with the app's table hash
espflash write-bin --after no-reset 0x20000 target/note4c-app.bin
espflash checksum-md5 --after no-reset 0x20000 <size>    # verify
espflash erase-region 0xd000 0x2000     # blank otadata → bootloader picks ota_0, then reset
```

- **Why erase otadata:** if the factory firmware last booted from `ota_1` (after an OTA
  update), the bootloader would keep starting `ota_1` and ignore our app. With `otadata`
  blank, the ESP-IDF bootloader starts the first OTA slot. This is standard ESP-IDF
  behaviour and has worked on the NOTE4C since early October 2026 and on the NOTE4 since
  2026-10-23.
- `ota_1`, `nvs`, `phy_init` and `assets` are left untouched. If the factory image is in
  `ota_1`, it survives as a fallback.
- The esp-hal app carries an ESP-IDF app descriptor (`esp_app_desc!()`), which the factory
  bootloader requires.

## 4. Restore the factory firmware

```sh
just restore backups/note4c-<date>-a.bin
# esptool:
esptool --chip esp32s3 -p /dev/ttyACM0 -b 460800 write-flash 0x0 backups/note4c-a.bin
```

`just restore` refuses a file that is not exactly 16 MiB, prints its sha256 and partition
table before asking, and checks the written flash with `checksum-md5` afterwards.

Official images are also available. Zectrix's browser flasher is at
<https://zectrix.com/en/firmware-updater.html>. `just fetch-vendor` downloads the reference
NOTE4C and NOTE4 images into `docs/vendor/note4c/firmware/` and `docs/vendor/note4/firmware/`
and checks their SHA-256; each is a merged image written at `0x0`.

## WSL2 (usbipd) notes

Flashing from WSL2 works with [usbipd-win](https://github.com/dorssel/usbipd-win): `usbipd bind
--busid <id>` once (admin), then keep `usbipd attach --wsl --auto-attach --busid <id>` running.

- **The device is on USB only while it is awake.** In deep sleep the USB-Serial-JTAG port
  disappears. Press OK to wake it, or use download mode (below), then flash while it is up.
- **Download mode, when nothing else works:** hold the round front BOOT/OK button, press the
  recessed RESET pinhole on the **left side** of the case once, release BOOT. (The small
  pinhole on the front is the microphone.) The ROM bootloader then stays on USB. After
  flashing, press RESET alone to boot the new firmware; a USB reset may leave it in the
  bootloader.
- **Do not leave a serial monitor attached** while testing on the panel: opening the port
  toggles DTR/RTS, which reset or hold the chip (dark, half-refreshed images, OK "dead").
  USB/IP also re-attaches too slowly after a reset to catch a whole run, which is why the
  firmware draws its diagnostics and the previous run's panic message on the panel.
