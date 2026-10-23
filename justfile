# ZECTRIX NOTE4 / NOTE4C firmware tasks.   `just` lists them.
# Apps: note4 (mono test card) · note4c (four-colour frame client) · audio (mic/speaker
# loopback, panel untouched, runs on either device).
# See docs/FLASHING.md before touching a device.
#
# Another repo can reuse these recipes with `import 'path/to/zectrix-epaper-rs/justfile'`:
# paths below are relative to this file (source_directory()), backups/ to the caller's root.

set shell := ["bash", "-euo", "pipefail", "-c"]
# Comment lines inside recipes are for readers; do not echo them.
set ignore-comments := true

port := env("ESPFLASH_PORT", "/dev/ttyACM0")
root := source_directory()
target_dir := env("CARGO_TARGET_DIR", root / "target")
elf_dir := target_dir / "xtensa-esp32s3-none-elf" / "release"
# sha256 of the 4 KiB page at 0x8000 on a factory NOTE4C (emini-home preflight.py, vendor image)
note4c_table_sha256 := "ce40cfe75056ef74bc052942f8a9ee3dce8e5e14ba17a6f63685a8fa0d11a23d"
# same page on a NOTE4 devkit (read 2026-10-23, partitions/note4-devkit-16m.csv)
note4_table_sha256 := "a82133fa4cd77c180d65fa75ca3b5c27bcebfb8cc4d419362838852e995ba9e5"
# Xtensa Rust toolchain installed by `just setup` (same as .github/workflows/ci.yml)
esp_toolchain := "1.97.0.0"

default:
    @just --list

# One-time toolchain install (Xtensa Rust via espup, espflash). Needs rustup with the stable
# toolchain: the `esp` channel in rust-toolchain.toml only exists after this. Then `. ~/export-esp.sh`.
setup:
    cargo +stable install espup@0.17.1 espflash@4.6.0 --locked
    espup install --targets esp32s3 --toolchain-version {{esp_toolchain}}
    @echo "Now run: . ~/export-esp.sh   (add it to your shell profile)"
    @echo "Optional, for 'esptool get-security-info' (docs/FLASHING.md): pipx install esptool"

# Host tests of the driver and board logic, host-crate clippy, format check.
test:
    cd {{root}} && cargo test --locked
    cd {{root}} && cargo clippy --locked --workspace --exclude note4 --exclude note4c --exclude audio --all-targets -- -D warnings
    cd {{root}} && cargo fmt --all -- --check

# Build one app (note4 | note4c | audio).
build app:
    cd {{root}}/apps/{{app}} && cargo build --locked --release

# Clippy for one app, warnings are errors.
clippy app:
    cd {{root}}/apps/{{app}} && cargo clippy --locked --release -- -D warnings

# Build all apps, run tests and clippy (what CI does).
check: test (build "note4") (build "note4c") (build "audio") (clippy "note4") (clippy "note4c") (clippy "audio")

# App image for the ota_0 slot of the factory partition table → target/<app>-app.bin
# (audio uses the NOTE4C table: its 4032 K ota_0 is the smaller of the two, so the image
# fits either device; ota_0 sits at 0x20000 in both).
image app: (build app)
    espflash save-image --chip esp32s3 --flash-size 16mb \
      --partition-table {{root}}/partitions/{{ if app == "note4" { "note4-devkit-16m.csv" } else { "factory-16m.csv" } }} \
      --target-app-partition ota_0 \
      {{elf_dir}}/{{app}} {{target_dir}}/{{app}}-app.bin

# Two full 16 MiB flash backups of the connected device, compared byte for byte.
backup name:
    mkdir -p backups
    stamp=$(date +%Y%m%d-%H%M%S); \
    for s in a b; do espflash read-flash -p {{port}} -c esp32s3 0 0x1000000 backups/{{name}}-$stamp-$s.bin; done; \
    cmp backups/{{name}}-$stamp-a.bin backups/{{name}}-$stamp-b.bin && echo "OK: backups/{{name}}-$stamp-{a,b}.bin identical" \
      || { echo "MISMATCH: read again before flashing anything"; exit 1; }; \
    sha256sum backups/{{name}}-$stamp-a.bin

# Print the partition table stored in a backup image and compare it with the known tables.
partitions backup:
    size=$(stat -c %s {{backup}}); [ "$size" -ge 36864 ] || { echo "{{backup}} is $size bytes: too short to hold the partition table at 0x8000 (needs 0x9000)."; exit 1; }
    tmp=$(mktemp); dd if={{backup}} of=$tmp bs=4096 skip=8 count=1 status=none; \
    h=$(sha256sum $tmp | cut -d' ' -f1); echo "0x8000 page sha256 $h"; \
    case "$h" in \
      {{note4c_table_sha256}}) echo "= NOTE4C factory table (partitions/factory-16m.csv)";; \
      {{note4_table_sha256}}) echo "= NOTE4 devkit table (partitions/note4-devkit-16m.csv)";; \
      *) echo "unknown table: check docs/FLASHING.md step 2";; \
    esac; \
    espflash partition-table $tmp; rm $tmp

# Write the app to ota_0 (0x20000), verify it, blank otadata. Bootloader and ota_1 are kept.
# Refuses unless the device's partition table is one known for this app and a backup of that
# device exists. The NOTE4 and NOTE4C tables differ, so this also stops one panel's app
# reaching the other; `audio` never drives the panel and accepts either table.
flash app: (image app)
    tmp=$(mktemp); espflash read-flash -p {{port}} -c esp32s3 --after no-reset 0x8000 0x1000 $tmp >/dev/null; \
    h=$(sha256sum $tmp | cut -d' ' -f1); rm $tmp; \
    case "$h" in {{note4_table_sha256}}) dev=note4;; {{note4c_table_sha256}}) dev=note4c;; *) dev=unknown;; esac; \
    case "{{app}}" in audio) fits=$([ "$dev" != unknown ] && echo yes || true);; *) fits=$([ "$dev" = "{{app}}" ] && echo yes || true);; esac; \
    [ "$fits" = yes ] || { echo "Partition table on the device ($h, $dev) is not a {{app}} table. Wrong device? See docs/FLASHING.md step 2."; exit 1; }; \
    ls backups/$dev-*-a.bin >/dev/null 2>&1 || { echo "No backups/$dev-*-a.bin. Run: just backup $dev"; exit 1; }; \
    echo "Partition table on the device = $dev table; backup found."
    @echo "About to write {{app}} to 0x20000 and erase otadata (0xd000) on {{port}}."
    @echo "Anything else stops here; the chip then stays in the bootloader until you press RESET."
    @read -p "Type the app name to continue: " ans && [ "$ans" = "{{app}}" ]
    # --after no-reset keeps the chip in the bootloader between steps; only the last step resets it.
    espflash write-bin -p {{port}} -c esp32s3 --after no-reset 0x20000 {{target_dir}}/{{app}}-app.bin
    # espflash prints the md5 as a hex number (0x…, leading zeros dropped), so pad it back to 32 digits.
    size=$(stat -c %s {{target_dir}}/{{app}}-app.bin); \
    want=$(md5sum {{target_dir}}/{{app}}-app.bin | cut -d' ' -f1); \
    got=$(espflash checksum-md5 -p {{port}} -c esp32s3 --after no-reset 0x20000 $size 2>/dev/null | grep -oE '0x[0-9a-f]+' | tail -1 | sed 's/^0x//'); \
    got=$(printf '%32s' "$got" | tr ' ' 0); \
    [ "$want" = "$got" ] && echo "verified md5 $got" || { echo "VERIFY FAILED ($want vs $got)"; exit 1; }
    espflash erase-region -p {{port}} -c esp32s3 --after hard-reset 0xd000 0x2000

# Serial log from the device (USB-Serial-JTAG); the ELF decodes backtraces.
monitor app="note4c":
    espflash monitor -p {{port}} --elf {{elf_dir}}/{{app}}

# Write a full 16 MiB backup image back to the device at 0x0 and verify it.
# Refuses unless the file is exactly 16 MiB; shows its sha256 and partition table first.
restore backup:
    size=$(stat -c %s {{backup}}); [ "$size" = 16777216 ] || { echo "{{backup}} is $size bytes, not a full 16 MiB backup (16777216). Refusing."; exit 1; }
    sha256sum {{backup}}
    {{just_executable()}} -f {{justfile()}} partitions {{backup}}
    h=$(dd if={{backup}} bs=4096 skip=8 count=1 status=none | sha256sum | cut -d' ' -f1); \
    [ "$h" = "{{note4_table_sha256}}" ] || [ "$h" = "{{note4c_table_sha256}}" ] || { echo "Unknown partition table in {{backup}}. Refusing; see docs/FLASHING.md."; exit 1; }
    @echo "About to overwrite the WHOLE flash of {{port}} with {{backup}}."
    @read -p "Type RESTORE to continue: " ans && [ "$ans" = "RESTORE" ]
    espflash write-bin -p {{port}} -c esp32s3 --after no-reset 0x0 {{backup}}
    # Same md5 parsing as in `flash` (espflash drops leading zeros of the hex number).
    want=$(md5sum {{backup}} | cut -d' ' -f1); \
    got=$(espflash checksum-md5 -p {{port}} -c esp32s3 --after hard-reset 0x0 0x1000000 2>/dev/null | grep -oE '0x[0-9a-f]+' | tail -1 | sed 's/^0x//'); \
    got=$(printf '%32s' "$got" | tr ' ' 0); \
    [ "$want" = "$got" ] && echo "verified md5 $got" || { echo "VERIFY FAILED ($want vs $got): restore again before unplugging"; exit 1; }

# Clone the reference firmwares into docs/vendor/_refs (git-ignored).
fetch-refs:
    {{root}}/tools/fetch-refs.sh

# Download the vendor documents (wiki, schematic, datasheets, firmware) into docs/vendor (git-ignored).
fetch-vendor:
    {{root}}/tools/fetch-vendor.sh
