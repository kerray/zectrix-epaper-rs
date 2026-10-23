# zectrix-epaper-rs

Rust `no_std` firmware and drivers for two ZECTRIX e-paper devkits built on the ESP32-S3:

| App | Device | Panel | What it does |
| --- | --- | --- | --- |
| [`apps/note4c`](apps/note4c) | NOTE4C Devkit | 4.2" 400×300, black/white/yellow/red | **frame client**: wakes, fetches a server-rendered frame over HTTPS, shows it, deep-sleeps until the server's `X-Next-Refresh` |
| [`apps/note4`](apps/note4) | NOTE4 Developer Kit | 4.2" 400×300, black/white | test card, OK opens a partial-refresh lab screen; deep sleep, wake on OK / 30 min (frame client and interactive screens planned) |

Both boards are the same V1.0 hardware: ESP32-S3 N16R8 and a Solomon **SSD2683** panel
controller. Only the panel differs. The devices stay thin: rendering happens on a server you
run, following the small [frame protocol](docs/FRAME_PROTOCOL.md).

**Status:** an earlier build of this client has run on a NOTE4C since early October 2026;
the current `.env`-configurable build has not been flashed yet. The NOTE4 test card
runs on hardware (since 2026-10-23); the driver does partial refresh (not yet measured on
hardware) and the interactive screens are planned
([ROADMAP](docs/ROADMAP.md)).

Not affiliated with ZECTRIX. "ZECTRIX", "NOTE4" and "NOTE4C" name the hardware this runs on.

## Layout

```
crates/ssd2683        SSD2683 driver: 4-colour + mono full refresh, embedded-graphics frames (host-tested)
crates/zectrix-board  pin map, battery maths (host-tested); `hal` feature: power latch, ADC, deep sleep
crates/zectrix-net    Wi-Fi station, HTTPS GET (mbedtls-rs), HTTP/1.x head + URL parser, dotenv reader (host-tested)
crates/zectrix-screen NOTE4 interactive screens: bundle/patch parser, cursor, refresh policy, buttons, session (host-tested)
apps/note4c           NOTE4C frame client
apps/note4            NOTE4 test card
partitions/           factory 16 MB partition tables of the NOTE4C and the NOTE4 devkit
docs/                 hardware, flashing, frame protocol, roadmap, research notes
tools/                reference-repo list, vendor file list (URLs + SHA-256), fetch scripts, screen-bundle.py
```

## Quick start

Needs rustup with the stable toolchain, [just](https://github.com/casey/just) and a C
compiler (`build-essential`). `just setup` installs espup 0.17.1, espflash 4.6.0 and the
Xtensa Rust toolchain `1.97.0.0` (the version CI uses).

```sh
just setup                  # espup (Xtensa Rust) + espflash, once
. ~/export-esp.sh
just test                   # host tests
cp .env.example .env        # then edit it, see "Configuration"
just build note4c           # or: just check   (everything CI runs)

# with a device on USB — read docs/FLASHING.md first
just backup note4c          # two full 16 MiB backups, compared
just partitions backups/note4c-<stamp>-a.bin
just flash note4c           # writes only ota_0 (0x20000) + blanks otadata; keeps the bootloader
just monitor note4c         # or: just monitor note4
```

Flash and test on USB power: an image that crashes before it latches power switches a
battery-powered unit off.

`cargo run` is disabled on purpose. `espflash flash` would replace the factory bootloader.
`just flash` refuses unless a file `backups/<app>-*-a.bin` exists (it does not check what is
in it, so take it with `just backup`) and the device's partition table is the one known for
that app, so a NOTE4 image cannot land on a NOTE4C or the other way round.

## Configuration

`apps/note4c` compiles its settings in (there is no provisioning mode yet). `build.rs` takes
each value from the environment or, failing that, from a dotenv file: `$ZECTRIX_ENV` if set,
otherwise the nearest `.env` from `apps/note4c` up to the workspace root (or the first
directory with `.git`). Values are not printed at the default verbosity, but `cargo -vv`
shows them and `target/` contains them. Values with control characters (CR, LF, …), an
SSID over 32 bytes, a password outside 8–64 bytes, or a URL and key too long for the
768-byte request fail the build with a message.

| Variable | Required | Meaning |
| --- | --- | --- |
| `WIFI_SSID`, `WIFI_PASSWORD` | yes | WPA2 / WPA2-WPA3 network |
| `FRAME_URL` | yes | `https://host[:port]/path?query` of your frame server |
| `FRAME_KEY` | no | sent as `Authorization: Bearer …`; omitted when empty (old name `NOTE4_FRAME_KEY` still accepted) |
| `FRAME_CA_PEM` | no | PEM file with the trust anchor for the server; relative to the `.env` file. Default: ISRG Root X1 (Let's Encrypt) |
| `ZECTRIX_ENV` | no | path of the dotenv file (useful when this repo is a submodule); a relative path is relative to `apps/note4c` |

Dotenv syntax: `KEY=value` or `export KEY=value`; `#` comment lines; one pair of matching
quotes is stripped; in unquoted values ` #` starts a comment; the last duplicate wins
(details in `crates/zectrix-net/src/dotenv.rs`).

The built image contains these values, so treat `target/` like the `.env` itself. Flash
backups contain them too (and the factory firmware's own Wi-Fi settings); `backups/` is
git-ignored.

## Working with an AI coding agent

The repo was developed with [Claude Code](https://claude.com/claude-code) and is set up for
coding agents: [CLAUDE.md](CLAUDE.md) (also reached through [AGENTS.md](AGENTS.md)) gives an
agent the build facts, the hardware and flashing rules, and how to add your own app or
frame server. Point your agent at it before it touches a device.

## Using this repo from another one

Keep personal configuration and server code in your own (private) repo and add this one as
a git submodule, e.g. at `upstream/`:

- `.env` in your repo, plus `.cargo/config.toml` with
  `[env] ZECTRIX_ENV = { value = ".env", relative = true }` so builds find it;
- `import 'upstream/justfile'` in your justfile reuses all recipes (`build`, `flash`, …);
  backups land in your repo's `backups/`;
- your own apps can depend on `upstream/crates/*` by path.

## Documentation

- [docs/HARDWARE.md](docs/HARDWARE.md): pin map, controller command sequences, flash layout
- [docs/FLASHING.md](docs/FLASHING.md): backup, flashing and restore
- [docs/FRAME_PROTOCOL.md](docs/FRAME_PROTOCOL.md): what the frame server must answer
- [docs/ROADMAP.md](docs/ROADMAP.md): goals, architecture, milestones, open questions
- [docs/INTERACTIVE.md](docs/INTERACTIVE.md): planned NOTE4 button screens with partial refresh
- [docs/RUST_RESEARCH.md](docs/RUST_RESEARCH.md): Rust stack choice (esp-hal 1.2 no_std vs esp-idf std), versions
- [docs/EMINI_HOME_ANALYSIS.md](docs/EMINI_HOME_ANALYSIS.md): what fiedoruk/emini-home (C, NOTE4C) does and what we reuse
- [docs/vendor/RESEARCH_NOTES.md](docs/vendor/RESEARCH_NOTES.md): full vendor research with sources.
  The vendor files themselves (wiki, schematic, datasheets, firmware images) are not
  redistributed; `just fetch-vendor` downloads them and checks their hashes.

## Licence and credits

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your
option. Unless you state otherwise, any contribution you submit for inclusion is dual
licensed as above, without additional terms.

The panel sequences and pin map are ported from MIT-licensed work by Zectrix Lab, LazyYoun
(macheng2017) and Tomasz Fiedoruk (emini-home); the battery curve comes from xiaozhi-esp32
(Shenzhen Xinzhi Future Technology Co., Ltd. and Project Contributors, MIT). The build-script
linker helpers are from the esp-generate template (esp-rs, MIT / Apache-2.0). See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
