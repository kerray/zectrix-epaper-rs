//! State kept across deep sleep in RTC fast memory: the last frame's ETag, whether the
//! panel shows a frame, the number of failed fetches and failed panel refreshes in a row,
//! and what the previous wake measured and drew (reported to the server on the next fetch).

const MAGIC: u32 = 0x5754_4832; // "WTH2"; bump whenever the layout below changes
const ETAG_CAP: usize = zectrix_net::http::ETAG_MAX;
pub const DISPLAY_CAP: usize = 40;

// Persistent memory starts with arbitrary contents after power-up; `MAGIC` tells real
// state apart from noise. Plain integers and byte arrays only (esp-hal `Persistable`).
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_MAGIC: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_FAILS: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_DISPLAY_FAILS: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_HAVE_FRAME: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_BATTERY_MV: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_ETAG_LEN: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_ETAG: [u8; ETAG_CAP] = [0; ETAG_CAP];
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_DISPLAY_LEN: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut S_DISPLAY: [u8; DISPLAY_CAP] = [0; DISPLAY_CAP];

#[derive(Default)]
pub struct State {
    pub fails: u32,
    /// Panel refreshes that returned an error, in a row.
    pub display_fails: u32,
    pub have_frame: bool,
    /// Battery reading of the previous wake (0 = none).
    pub battery_mv: u32,
    /// Result of the last panel refresh, e.g. `frame ok 19.4s` (empty = none since power-up).
    pub display: heapless::String<DISPLAY_CAP>,
    etag: heapless::String<ETAG_CAP>,
}

impl State {
    pub fn etag(&self) -> Option<heapless::String<ETAG_CAP>> {
        (!self.etag.is_empty()).then(|| self.etag.clone())
    }

    pub fn set_etag(&mut self, tag: &str) {
        self.etag = heapless::String::try_from(tag).unwrap_or_default();
    }

    pub fn clear_etag(&mut self) {
        self.etag.clear();
    }
}

/// Read `len` bytes of `buf` back as a string; garbage gives an empty string.
fn load_str<const N: usize>(buf: &[u8; N], len: u32) -> heapless::String<N> {
    let bytes = &buf[..(len as usize).min(N)];
    core::str::from_utf8(bytes)
        .ok()
        .and_then(|s| heapless::String::try_from(s).ok())
        .unwrap_or_default()
}

/// The state saved by the previous run, or the default after power-up.
pub fn load() -> State {
    // SAFETY: single-threaded; only `load` and `store` touch these statics.
    unsafe {
        if *core::ptr::addr_of!(S_MAGIC) != MAGIC {
            return State::default();
        }
        State {
            fails: *core::ptr::addr_of!(S_FAILS),
            display_fails: *core::ptr::addr_of!(S_DISPLAY_FAILS),
            have_frame: *core::ptr::addr_of!(S_HAVE_FRAME) == 1,
            battery_mv: *core::ptr::addr_of!(S_BATTERY_MV),
            display: load_str(
                &*core::ptr::addr_of!(S_DISPLAY),
                *core::ptr::addr_of!(S_DISPLAY_LEN),
            ),
            etag: load_str(
                &*core::ptr::addr_of!(S_ETAG),
                *core::ptr::addr_of!(S_ETAG_LEN),
            ),
        }
    }
}

pub fn store(st: &State) {
    // SAFETY: as in `load`.
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(S_ETAG);
        buf[..st.etag.len()].copy_from_slice(st.etag.as_bytes());
        *core::ptr::addr_of_mut!(S_ETAG_LEN) = st.etag.len() as u32;
        let buf = &mut *core::ptr::addr_of_mut!(S_DISPLAY);
        buf[..st.display.len()].copy_from_slice(st.display.as_bytes());
        *core::ptr::addr_of_mut!(S_DISPLAY_LEN) = st.display.len() as u32;
        *core::ptr::addr_of_mut!(S_FAILS) = st.fails;
        *core::ptr::addr_of_mut!(S_DISPLAY_FAILS) = st.display_fails;
        *core::ptr::addr_of_mut!(S_HAVE_FRAME) = u32::from(st.have_frame);
        *core::ptr::addr_of_mut!(S_BATTERY_MV) = st.battery_mv;
        *core::ptr::addr_of_mut!(S_MAGIC) = MAGIC;
    }
}
