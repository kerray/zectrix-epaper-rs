use std::path::PathBuf;

use zectrix_net::build_config::{self, Source, Url, Wifi};
use zectrix_net::http;

fn main() {
    config();
    linker_be_nice();
    check_xtensa_linker_available();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

// Longest values of the headers src/main.rs adds to every request (keep in sync):
// `wake` (heapless::String<48>), `battery` (String<10>), `state::DISPLAY_CAP` and the ETag.
const WAKE_MAX: usize = 48;
const BATTERY_MAX: usize = 10;
const DISPLAY_MAX: usize = 40;

/// Compile-time configuration, see README "Configuration". Where values come from (the
/// environment, `$ZECTRIX_ENV` or the nearest `.env`) is described in
/// `zectrix_net::build_config`.
///
/// - `WIFI_SSID` (1–32 bytes), `WIFI_PASSWORD` (8–64 bytes, WPA2): required.
/// - `FRAME_URL`: required, `https://host[:port]/path?query`, host made of `A-Z a-z 0-9 . -`.
///   Passed on as `FRAME_HOST`, `FRAME_PORT` and `FRAME_PATH`.
/// - `FRAME_KEY`: optional bearer token (`NOTE4_FRAME_KEY` is accepted as an old name).
/// - `FRAME_CA_PEM`: optional path to a PEM trust anchor; relative paths are resolved against
///   the dotenv file's directory. Default: ISRG Root X1 (Let's Encrypt).
///
/// The URL and key together must leave room in the request buffer
/// (`zectrix_net::http::REQUEST_MAX`) for the headers the firmware adds.
fn config() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = Source::locate(&manifest);

    Wifi::required(&src).emit();

    let url = Url::parse("FRAME_URL", &src.required("FRAME_URL"), src.origin());
    let key = src
        .get("FRAME_KEY")
        .or_else(|| {
            let old = src.get("NOTE4_FRAME_KEY");
            if old.is_some() {
                build_config::warn("NOTE4_FRAME_KEY is deprecated, rename it to FRAME_KEY");
            }
            old
        })
        .unwrap_or_default();

    // The longest request the firmware can send with this host, path and key.
    let auth = format!("Bearer {key}");
    let filler = |n: usize| "x".repeat(n);
    let (wake, battery, display, etag) = (
        filler(WAKE_MAX),
        filler(BATTERY_MAX),
        filler(DISPLAY_MAX),
        filler(http::ETAG_MAX),
    );
    let mut headers = vec![
        ("X-Wake", wake.as_str()),
        ("X-Battery-mV", battery.as_str()),
        ("X-Last-Display", display.as_str()),
        ("If-None-Match", etag.as_str()),
    ];
    if !key.is_empty() {
        headers.insert(0, ("Authorization", auth.as_str()));
    }
    url.check_request_fits("", &headers, "FRAME_URL and FRAME_KEY");

    url.emit("FRAME");
    println!("cargo:rustc-env=FRAME_KEY={key}");
    build_config::ca_pem(&src, "FRAME_CA_PEM", "FRAME_CA_FILE", "frame_custom_ca");
}

// The linker helpers below (check_xtensa_linker_available, xtensa_linker,
// cargo_configured_linker, cargo_linker_env_var, linker_be_nice) and the linker lines in
// main() come from the esp-generate 1.4.0 project template, Copyright (c) 2024 esp-rs, MIT OR Apache-2.0
// (see THIRD_PARTY_NOTICES.md).
#[cfg(unix)]
fn check_xtensa_linker_available() {
    println!("cargo:rerun-if-env-changed=PATH");

    let target = std::env::var("TARGET").unwrap_or_default();
    println!(
        "cargo:rerun-if-env-changed={}",
        cargo_linker_env_var(&target)
    );

    let linker = xtensa_linker(&target);

    let error = match std::process::Command::new(&linker)
        .arg("--version")
        .output()
    {
        Ok(_) => return,
        Err(error) => error,
    };

    let export_file = std::env::var("HOME")
        .map(|home| format!("{home}/export-esp.sh"))
        .unwrap_or_else(|_| "$HOME/export-esp.sh".to_string());

    panic!(
        "Xtensa linker `{linker}` was not found in PATH or could not be executed: {error}.\n\n\
         Xtensa targets on Unix require espup's environment export file to be sourced before building.\n\
         Try: `source {export_file}`\n\n\
         If the export file does not exist, run `espup install` first.\n\
         For more details, see:\n\
         https://github.com/esp-rs/espup?tab=readme-ov-file#environment-variables-setup"
    );
}

#[cfg(not(unix))]
fn check_xtensa_linker_available() {}

#[cfg(unix)]
fn xtensa_linker(target: &str) -> String {
    if let Some(linker) = cargo_configured_linker(target) {
        return linker;
    }

    target
        .strip_prefix("xtensa-")
        .and_then(|target| target.strip_suffix("-none-elf"))
        .map(|chip| format!("xtensa-{chip}-elf-gcc"))
        .unwrap_or_else(|| "xtensa-esp32-elf-gcc".to_string())
}

#[cfg(unix)]
fn cargo_configured_linker(target: &str) -> Option<String> {
    std::env::var(cargo_linker_env_var(target)).ok()
}

#[cfg(unix)]
fn cargo_linker_env_var(target: &str) -> String {
    let target = target.replace('-', "_").to_ascii_uppercase();
    format!("CARGO_TARGET_{target}_LINKER")
}

fn linker_be_nice() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let kind = &args[1];
        let what = &args[2];

        match kind.as_str() {
            "undefined-symbol" => match what.as_str() {
                what if what.starts_with("_defmt_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `defmt` not found - make sure `defmt.x` is added as a linker script and you have included `use defmt_rtt as _;`"
                    );
                    eprintln!();
                }
                "_stack_start" => {
                    eprintln!();
                    eprintln!("💡 Is the linker script `linkall.x` missing?");
                    eprintln!();
                }
                what if what.starts_with("esp_rtos_") => {
                    eprintln!();
                    eprintln!(
                        "💡 `esp-radio` has no scheduler enabled. Make sure you have initialized `esp-rtos` or provided an external scheduler."
                    );
                    eprintln!();
                }
                "embedded_test_linker_file_not_added_to_rustflags" => {
                    eprintln!();
                    eprintln!(
                        "💡 `embedded-test` not found - make sure `embedded-test.x` is added as a linker script for tests"
                    );
                    eprintln!();
                }
                "free"
                | "malloc"
                | "calloc"
                | "get_free_internal_heap_size"
                | "malloc_internal"
                | "realloc_internal"
                | "calloc_internal"
                | "free_internal" => {
                    eprintln!();
                    eprintln!(
                        "💡 Did you forget the `esp-alloc` dependency or didn't enable the `compat` feature on it?"
                    );
                    eprintln!();
                }
                _ => (),
            },
            // we don't have anything helpful for "missing-lib" yet
            _ => {
                std::process::exit(1);
            }
        }

        std::process::exit(0);
    }

    println!(
        "cargo:rustc-link-arg=-Wl,--error-handling-script={}",
        std::env::current_exe().unwrap().display()
    );
}
