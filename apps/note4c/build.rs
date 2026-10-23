use std::path::{Path, PathBuf};

use zectrix_net::{dotenv, http};

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

/// Compile-time configuration, see README "Configuration".
///
/// Each value comes from the environment or, failing that, from a dotenv file: `$ZECTRIX_ENV`
/// if set (a relative path is taken relative to `apps/note4c`), otherwise the nearest `.env`
/// from `apps/note4c` up to the workspace root or the first directory with a `.git` entry,
/// whichever comes first. The dotenv syntax is described in `zectrix_net::dotenv`.
/// A newly created `.env` is picked up after `cargo clean -p note4c` (or `touch build.rs`).
/// Values are not printed at the default verbosity (`cargo -vv` shows the `rustc-env`
/// lines, and `target/` contains them).
///
/// - `WIFI_SSID` (1–32 bytes), `WIFI_PASSWORD` (8–64 bytes, WPA2): required.
/// - `FRAME_URL`: required, `https://host[:port]/path?query`, host made of `A-Z a-z 0-9 . -`.
///   Passed on as `FRAME_HOST`, `FRAME_PORT` and `FRAME_PATH`.
/// - `FRAME_KEY`: optional bearer token (`NOTE4_FRAME_KEY` is accepted as an old name).
/// - `FRAME_CA_PEM`: optional path to a PEM trust anchor; relative paths are resolved against
///   the dotenv file's directory. Default: ISRG Root X1 (Let's Encrypt).
///
/// No value may contain control characters (CR, LF, tab, …). The URL and key together must
/// leave room in the request buffer (`zectrix_net::http::REQUEST_MAX`) for the headers the
/// firmware adds.
fn config() {
    println!("cargo:rerun-if-env-changed=ZECTRIX_ENV");
    println!("cargo::rustc-check-cfg=cfg(frame_custom_ca)");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dotenv_file = dotenv_path(&manifest);
    let base = dotenv_file
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest.clone());
    let file = match &dotenv_file {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
        }
        None => String::new(),
    };
    let where_ = match &dotenv_file {
        Some(path) => format!("the environment or {}", path.display()),
        None => "the environment or a .env file (none found; see ZECTRIX_ENV)".to_string(),
    };
    let get = |key: &str| -> Option<String> {
        println!("cargo:rerun-if-env-changed={key}");
        let value = std::env::var(key)
            .ok()
            .or_else(|| dotenv::get(&file, key).map(str::to_string))
            .filter(|v| !v.is_empty())?;
        // Also stops header injection and truncated `cargo:` lines.
        assert!(
            !value.chars().any(char::is_control),
            "{key} contains a control character (CR, LF, tab, …); check {where_}."
        );
        Some(value)
    };
    let required = |key: &str| -> String {
        get(key).unwrap_or_else(|| panic!("{key} is not set. Put it into {where_}."))
    };

    let ssid = required("WIFI_SSID");
    assert!(
        ssid.len() <= 32,
        "WIFI_SSID is {} bytes; Wi-Fi allows at most 32.",
        ssid.len()
    );
    // The firmware joins WPA2 / WPA2-WPA3 networks only (zectrix_net::wifi), so no open ones.
    let password = required("WIFI_PASSWORD");
    assert!(
        (8..=64).contains(&password.len()),
        "WIFI_PASSWORD is {} bytes; WPA2 needs 8-63 characters (or 64 hex digits).",
        password.len()
    );
    println!("cargo:rustc-env=WIFI_SSID={ssid}");
    println!("cargo:rustc-env=WIFI_PASSWORD={password}");

    let url = required("FRAME_URL");
    assert!(
        url.is_ascii(),
        "FRAME_URL must be ASCII; percent-encode other characters (from {where_})."
    );
    let Some(parts) = http::parse_https_url(&url) else {
        panic!("FRAME_URL must look like https://host[:port]/path?query (from {where_}).");
    };
    assert!(
        parts
            .host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
        "FRAME_URL host may contain only A-Z, a-z, 0-9, '.' and '-' (a name or an IPv4 address)."
    );

    let key = get("FRAME_KEY")
        .or_else(|| {
            let old = get("NOTE4_FRAME_KEY");
            if old.is_some() {
                println!("cargo:warning=NOTE4_FRAME_KEY is deprecated, rename it to FRAME_KEY");
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
    let mut request: heapless::String<16384> = heapless::String::new();
    let fits = http::write_get(&mut request, parts.host, parts.port, parts.path, &headers).is_ok()
        && request.len() <= http::REQUEST_MAX;
    assert!(
        fits,
        "FRAME_URL and FRAME_KEY are too long: the longest request would be {} bytes, the \
         firmware's buffer is {} (zectrix_net::http::REQUEST_MAX). Shorten the path, query or key.",
        request.len(),
        http::REQUEST_MAX
    );

    println!("cargo:rustc-env=FRAME_HOST={}", parts.host);
    println!("cargo:rustc-env=FRAME_PORT={}", parts.port);
    println!("cargo:rustc-env=FRAME_PATH={}", parts.path);
    println!("cargo:rustc-env=FRAME_KEY={key}");

    if let Some(ca) = get("FRAME_CA_PEM") {
        // include_str! resolves relative paths against src/, so pass an absolute one.
        let path = std::path::absolute(base.join(ca)).expect("FRAME_CA_PEM path");
        let pem = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("FRAME_CA_PEM {}: {e}", path.display()));
        assert!(
            pem.contains("-----BEGIN CERTIFICATE-----") && !pem.contains('\0'),
            "FRAME_CA_PEM {} is not a PEM certificate",
            path.display()
        );
        let shown = path.display().to_string();
        assert!(
            !shown.chars().any(char::is_control),
            "FRAME_CA_PEM path contains a control character"
        );
        println!("cargo:rerun-if-changed={shown}");
        println!("cargo:rustc-env=FRAME_CA_FILE={shown}");
        println!("cargo:rustc-cfg=frame_custom_ca");
    }
}

/// `$ZECTRIX_ENV` (made absolute against `apps/note4c`), else the nearest `.env` from this
/// crate's directory up to the workspace root (`apps/note4c/../..`, which holds the
/// workspace `Cargo.toml`) or the first directory with a `.git` entry, whichever comes
/// first. So a `.env` above the checkout (e.g. `~/.env` next to an unpacked ZIP) is never used.
fn dotenv_path(manifest: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ZECTRIX_ENV") {
        let path = std::path::absolute(PathBuf::from(path)).expect("ZECTRIX_ENV path");
        assert!(
            path.is_file(),
            "ZECTRIX_ENV={} is not a file",
            path.display()
        );
        return Some(path);
    }
    let workspace = manifest.join("../..");
    let workspace = workspace.canonicalize().unwrap_or(workspace);
    let manifest = manifest
        .canonicalize()
        .unwrap_or_else(|_| manifest.to_path_buf());
    for dir in manifest.ancestors() {
        let candidate = dir.join(".env");
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == workspace || dir.join(".git").exists() {
            break;
        }
    }
    None
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
