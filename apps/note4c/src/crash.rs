//! Panic handler that keeps the message for the next boot.
//!
//! Over WSL USB/IP the serial log is rarely caught (see docs/FLASHING.md), so the message
//! is written to RTC fast memory, which survives the reset and the deep sleep that follow
//! a panic. The next boot takes it with [`take`] and draws it on the panel.

use core::fmt::Write as _;
use core::panic::PanicInfo;

use esp_hal::time::Duration;
use zectrix_board::power;

const MAGIC: u32 = 0x504e_4943; // "PNIC"
const CAPACITY: usize = 120;

/// After a panic: device off on battery; on USB retry after this long.
const PANIC_RETRY: Duration = Duration::from_secs(2 * 60);

// Persistent memory starts with arbitrary contents after power-up; `MAGIC` and the
// length check tell a real message apart from noise.
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut LOG_MAGIC: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut LOG_LEN: u32 = 0;
#[esp_hal::ram(unstable(rtc_fast, persistent))]
static mut LOG_MSG: [u8; CAPACITY] = [0; CAPACITY];

/// The message of the panic that ended the previous run, if any. Clears it.
pub fn take() -> Option<heapless::String<CAPACITY>> {
    // SAFETY: single-threaded at boot; the panic handler is the only other writer.
    let (magic, len, msg) = unsafe {
        (
            &mut *core::ptr::addr_of_mut!(LOG_MAGIC),
            *core::ptr::addr_of!(LOG_LEN) as usize,
            &*core::ptr::addr_of!(LOG_MSG),
        )
    };
    let valid = *magic == MAGIC && len <= CAPACITY;
    *magic = 0;
    if !valid {
        return None;
    }
    let text = core::str::from_utf8(&msg[..len]).ok()?;
    heapless::String::try_from(text).ok()
}

/// Writes into a fixed buffer and silently drops what does not fit.
struct Truncating<'a> {
    buf: &'a mut [u8; CAPACITY],
    len: usize,
}

impl core::fmt::Write for Truncating<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for ch in s.chars() {
            let mut tmp = [0u8; 4];
            let bytes = ch.encode_utf8(&mut tmp).as_bytes();
            if self.len + bytes.len() > CAPACITY {
                break;
            }
            self.buf[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    esp_println::println!("PANIC: {info}");
    // SAFETY: nothing else runs once we are panicking.
    let mut w = Truncating {
        buf: unsafe { &mut *core::ptr::addr_of_mut!(LOG_MSG) },
        len: 0,
    };
    if let Some(loc) = info.location() {
        let file = loc.file().rsplit('/').next().unwrap_or(loc.file());
        let _ = write!(w, "{file}:{} ", loc.line());
    }
    let _ = write!(w, "{}", info.message());
    // SAFETY: as above.
    unsafe {
        *core::ptr::addr_of_mut!(LOG_LEN) = w.len as u32;
        *core::ptr::addr_of_mut!(LOG_MAGIC) = MAGIC;
    }
    power::halt_after_panic(PANIC_RETRY)
}
