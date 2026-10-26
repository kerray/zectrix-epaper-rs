//! Compile-time configuration for the firmware build scripts (feature `build`, host only).
//!
//! Each value comes from the environment or, failing that, from a dotenv file:
//! `$ZECTRIX_ENV` if set (a relative path is taken relative to the app's directory),
//! otherwise the nearest `.env` from the app's directory up to the workspace root or the
//! first directory with a `.git` entry, whichever comes first. The dotenv syntax is
//! described in [`crate::dotenv`]. Values are never printed at the default verbosity
//! (`cargo -vv` shows the `rustc-env` lines, and `target/` contains them).
//!
//! No value may contain control characters (CR, LF, tab, …): that also stops header
//! injection and truncated `cargo:` lines.

extern crate std;

use std::format;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::vec::Vec;
use std::{print, println};

use crate::{dotenv, http};

/// Where configuration values come from.
pub struct Source {
    file: String,
    /// Directory of the dotenv file (or the app's), for relative paths in values.
    base: PathBuf,
    /// "the environment or <file>", for messages.
    origin: String,
}

impl Source {
    /// Find the dotenv file for the app whose `Cargo.toml` is in `manifest_dir` (normally
    /// `CARGO_MANIFEST_DIR`), and tell cargo to rerun when it changes.
    pub fn locate(manifest_dir: &Path) -> Self {
        println!("cargo:rerun-if-env-changed=ZECTRIX_ENV");
        let dotenv_file = dotenv_path(manifest_dir);
        let base = dotenv_file
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_dir.to_path_buf());
        let file = match &dotenv_file {
            Some(path) => {
                println!("cargo:rerun-if-changed={}", path.display());
                std::fs::read_to_string(path)
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
            }
            None => String::new(),
        };
        let origin = match &dotenv_file {
            Some(path) => format!("the environment or {}", path.display()),
            None => "the environment or a .env file (none found; see ZECTRIX_ENV)".to_string(),
        };
        Self { file, base, origin }
    }

    /// A non-empty value, or `None`.
    pub fn get(&self, key: &str) -> Option<String> {
        println!("cargo:rerun-if-env-changed={key}");
        let value = std::env::var(key)
            .ok()
            .or_else(|| dotenv::get(&self.file, key).map(str::to_string))
            .filter(|v| !v.is_empty())?;
        assert!(
            !value.chars().any(char::is_control),
            "{key} contains a control character (CR, LF, tab, …); check {}.",
            self.origin
        );
        Some(value)
    }

    pub fn required(&self, key: &str) -> String {
        self.get(key)
            .unwrap_or_else(|| panic!("{key} is not set. Put it into {}.", self.origin))
    }

    /// Where values come from, for messages.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Resolve a path given in a value against the dotenv file's directory.
    pub fn path(&self, value: &str) -> PathBuf {
        std::path::absolute(self.base.join(value)).expect("absolute path")
    }
}

/// Validated Wi-Fi credentials.
pub struct Wifi {
    pub ssid: String,
    pub password: String,
}

impl Wifi {
    /// Check `ssid` and `password` (WPA2 only: the firmware joins no open networks).
    pub fn new(ssid: String, password: String) -> Self {
        assert!(
            ssid.len() <= 32,
            "WIFI_SSID is {} bytes; Wi-Fi allows at most 32.",
            ssid.len()
        );
        assert!(
            (8..=64).contains(&password.len()),
            "WIFI_PASSWORD is {} bytes; WPA2 needs 8-63 characters (or 64 hex digits).",
            password.len()
        );
        Self { ssid, password }
    }

    /// `WIFI_SSID` and `WIFI_PASSWORD`, both required.
    pub fn required(src: &Source) -> Self {
        Self::new(src.required("WIFI_SSID"), src.required("WIFI_PASSWORD"))
    }

    /// Pass both to the firmware as `env!("WIFI_SSID")` / `env!("WIFI_PASSWORD")`.
    pub fn emit(&self) {
        println!("cargo:rustc-env=WIFI_SSID={}", self.ssid);
        println!("cargo:rustc-env=WIFI_PASSWORD={}", self.password);
    }
}

/// A validated `https://host[:port]/path?query` URL.
pub struct Url {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Url {
    /// Parse the value of `var` (named in messages).
    pub fn parse(var: &str, url: &str, origin: &str) -> Self {
        assert!(
            url.is_ascii(),
            "{var} must be ASCII; percent-encode other characters (from {origin})."
        );
        let Some(parts) = http::parse_https_url(url) else {
            panic!("{var} must look like https://host[:port]/path?query (from {origin}).");
        };
        assert!(
            parts
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'),
            "{var} host may contain only A-Z, a-z, 0-9, '.' and '-' (a name or an IPv4 address)."
        );
        Self {
            host: parts.host.to_string(),
            port: parts.port,
            path: parts.path.to_string(),
        }
    }

    /// Pass the parts on as `{prefix}_HOST`, `{prefix}_PORT` and `{prefix}_PATH`.
    pub fn emit(&self, prefix: &str) {
        println!("cargo:rustc-env={prefix}_HOST={}", self.host);
        println!("cargo:rustc-env={prefix}_PORT={}", self.port);
        println!("cargo:rustc-env={prefix}_PATH={}", self.path);
    }

    /// Panic unless a GET of this URL plus `path_extra` (the longest suffix the firmware
    /// appends) with `headers` fits [`http::REQUEST_MAX`]. `vars` names the settings to
    /// shorten.
    pub fn check_request_fits(&self, path_extra: &str, headers: &[(&str, &str)], vars: &str) {
        let path = format!("{}{path_extra}", self.path);
        let mut request: heapless::String<16384> = heapless::String::new();
        let fits = http::write_get(&mut request, &self.host, self.port, &path, headers).is_ok()
            && request.len() <= http::REQUEST_MAX;
        assert!(
            fits,
            "{vars} are too long: the longest request would be {} bytes, the firmware's buffer \
             is {} (zectrix_net::http::REQUEST_MAX). Shorten the path, query or key.",
            request.len(),
            http::REQUEST_MAX
        );
    }
}

/// An optional PEM trust anchor in `var` (a path, relative to the dotenv file). When set,
/// its absolute path goes to the firmware as `{file_env}` (for `include_str!`) and
/// `cfg({cfg})` is set. Declares the cfg either way.
pub fn ca_pem(src: &Source, var: &str, file_env: &str, cfg: &str) {
    println!("cargo::rustc-check-cfg=cfg({cfg})");
    let Some(ca) = src.get(var) else { return };
    let path = src.path(&ca);
    let pem =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{var} {}: {e}", path.display()));
    assert!(
        pem.contains("-----BEGIN CERTIFICATE-----") && !pem.contains('\0'),
        "{var} {} is not a PEM certificate",
        path.display()
    );
    let shown = path.display().to_string();
    assert!(
        !shown.chars().any(char::is_control),
        "{var} path contains a control character"
    );
    println!("cargo:rerun-if-changed={shown}");
    println!("cargo:rustc-env={file_env}={shown}");
    println!("cargo:rustc-cfg={cfg}");
}

/// A cargo warning (one line).
pub fn warn(msg: &str) {
    let line: Vec<&str> = msg.lines().collect();
    print!("cargo:warning=");
    println!("{}", line.join(" "));
}

/// `$ZECTRIX_ENV` (made absolute against the app's directory), else the nearest `.env`
/// from the app's directory up to the workspace root (`<app>/../..`, which holds the
/// workspace `Cargo.toml`) or the first directory with a `.git` entry, whichever comes
/// first. So a `.env` above the checkout (e.g. `~/.env` next to an unpacked ZIP) is never
/// used.
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
