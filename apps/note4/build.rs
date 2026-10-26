// This build script is the esp-generate 1.4.0 project template's build.rs,
// Copyright (c) 2024 esp-rs, MIT OR Apache-2.0 (see THIRD_PARTY_NOTICES.md).

use std::path::PathBuf;

use zectrix_net::build_config::{self, Source, Url, Wifi};

fn main() {
    config();
    linker_be_nice();
    check_xtensa_linker_available();
    // make sure linkall.x is the last linker script (otherwise might cause problems with flip-link)
    println!("cargo:rustc-link-arg=-Tlinkall.x");
}

/// Longest screen id the firmware appends to `SCREEN_URL` (keep in sync with
/// `src/remote.rs`, `SCREEN_ID_MAX`), plus ETag and key headers, must fit the request buffer.
const SCREEN_ID_MAX: usize = 24;

/// Optional compile-time configuration for server screens. Where values come from (the
/// environment, `$ZECTRIX_ENV` or the nearest `.env`) is described in
/// `zectrix_net::build_config`.
///
/// - `SCREEN_URL`: `https://host[:port]/path?query` of the screen endpoint
///   (docs/INTERACTIVE.md, "Protocol"). The firmware appends `screen=<id>` with `?` or `&`.
///   Without it the firmware builds without networking and server screens say so.
/// - `WIFI_SSID`, `WIFI_PASSWORD`: required when `SCREEN_URL` is set.
/// - `SCREEN_KEY`: optional bearer token.
/// - `SCREEN_CA_PEM`: optional path to a PEM trust anchor (default ISRG Root X1).
///
/// Sets `cfg(screen_server)` when configured.
fn config() {
    println!("cargo::rustc-check-cfg=cfg(screen_server)");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = Source::locate(&manifest);
    let Some(url) = src.get("SCREEN_URL") else {
        build_config::warn("SCREEN_URL is not set: building without server screens (see build.rs)");
        return;
    };
    Wifi::required(&src).emit();
    let url = Url::parse("SCREEN_URL", &url, src.origin());
    let key = src.get("SCREEN_KEY").unwrap_or_default();
    let sep = if url.path.contains('?') { "&" } else { "?" };

    let auth = format!("Bearer {key}");
    let id = format!("{sep}screen={}", "x".repeat(SCREEN_ID_MAX));
    let etag = "x".repeat(zectrix_net::http::ETAG_MAX);
    let mut headers = vec![("If-None-Match", etag.as_str())];
    if !key.is_empty() {
        headers.insert(0, ("Authorization", auth.as_str()));
    }
    url.check_request_fits(&id, &headers, "SCREEN_URL and SCREEN_KEY");

    url.emit("SCREEN");
    println!("cargo:rustc-env=SCREEN_SEP={sep}");
    println!("cargo:rustc-env=SCREEN_KEY={key}");
    build_config::ca_pem(&src, "SCREEN_CA_PEM", "SCREEN_CA_FILE", "screen_custom_ca");
    println!("cargo:rustc-cfg=screen_server");
}

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
