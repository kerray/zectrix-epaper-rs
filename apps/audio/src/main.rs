//! Audio loopback test (voice milestone V1) for the NOTE4 and NOTE4C. It never touches
//! the e-paper panel, so the same image runs on both devices.
//!
//! - Boot: power latch, audio rail, ES8311 init, startup beep.
//! - Hold OK: a high beep, then the microphone is recorded (16 kHz, up to 30 s) into PSRAM;
//!   the LED is on while recording.
//! - Release OK (or 30 s pass): two low beeps, then the recording plays back.
//! - Up / Down: louder / quieter in 10 % steps, with a tick at the new volume.
//! - After 60 s without a button press: a falling beep, codec, amplifier and audio rail off,
//!   deep sleep. OK wakes it.
//! - Errors: a long falling buzz; if the codec does not answer, the LED blinks fast and
//!   the device sleeps.
//!
//! Audio path: one I2S peripheral in full duplex. TX (ESP → codec) owns BCLK/WS and RX
//! shares them internally (signal loopback), so both directions always run at the same
//! 16 kHz. Both directions stream continuously through small DMA rings in internal RAM;
//! the main loop keeps TX fed (beeps, playback or silence) and drains RX. See docs/AUDIO.md.

#![no_std]
#![no_main]
// From the esp-generate 1.4.0 template (Copyright (c) 2024 esp-rs, MIT OR Apache-2.0).
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;

use alloc::vec::Vec;

use es8311::{Config as CodecConfig, Es8311};
use esp_backtrace as _;
use esp_hal::{
    Blocking,
    clock::CpuClock,
    delay::Delay,
    dma::{DmaRxStreamBuf, DmaTxStreamBuf},
    dma_rx_stream_buffer, dma_tx_stream_buffer,
    gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    i2s::master::{
        Channels, DataFormat, I2s, I2sRx, I2sRxDmaTransfer, I2sTx, I2sTxDmaTransfer, TdmConfig,
    },
    main,
    psram::{Psram, PsramConfig, PsramMode},
    time::{Duration, Instant, Rate},
};
use heapless::Deque;
use log::{error, info, warn};
use zectrix_board::audio::{
    FRAME_BYTES, SAMPLE_RATE, SlotLevels, Tone, ToneGen, beeps, frame, slots,
};
use zectrix_board::pins;
use zectrix_board::power::{self, AudioPower, PowerLatch};

esp_bootloader_esp_idf::esp_app_desc!();

/// After a panic: device off on battery; on USB retry after this long.
const PANIC_RETRY: Duration = Duration::from_secs(30 * 60);

#[unsafe(no_mangle)]
fn custom_halt() -> ! {
    power::audio_off_after_panic();
    power::halt_after_panic(PANIC_RETRY)
}

/// Longest recording: 30 s, 1.9 MB of stereo frames in PSRAM.
const MAX_RECORD_FRAMES: usize = 30 * SAMPLE_RATE as usize;
/// Deep sleep after this long without a button press.
const IDLE_SLEEP: Duration = Duration::from_secs(60);
/// Each DMA ring: 8 KiB = 2048 frames = 128 ms, in 1 KiB descriptors.
const DMA_BYTES: usize = 8 * 1024;
const DMA_CHUNK: usize = 1024;
/// Worst-case delay between pushing a frame to TX and hearing it.
const TX_LATENCY_FRAMES: u32 = (DMA_BYTES / FRAME_BYTES) as u32;
/// No RX data for this long means the I2S stream stalled; it is restarted.
const STALL: Duration = Duration::from_millis(500);
const BUTTON_POLL: Duration = Duration::from_millis(10);
const VOLUME_DEFAULT: u8 = 70;
const VOLUME_STEP: u8 = 10;
/// The codec rail needs a moment before the first I2C transfer.
const RAIL_SETTLE_MS: u32 = 20;

type RxTransfer = I2sRxDmaTransfer<'static, Blocking, DmaRxStreamBuf>;
type TxTransfer = I2sTxDmaTransfer<'static, Blocking, DmaTxStreamBuf>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Idle,
    /// Recording; the first `skip` frames are dropped (they hold the start beep).
    Recording {
        skip: u32,
    },
    /// Playing the recording from `pos`, slot `slot`, once the beeps have finished.
    Playing {
        pos: usize,
        slot: usize,
    },
}

/// What goes out on TX: queued beeps first, then playback, otherwise silence.
struct Playout {
    tones: Deque<Tone, 8>,
    current: Option<ToneGen>,
    /// Frames pushed to TX so far.
    pushed: u32,
    /// `pushed` at the last non-silent frame.
    last_sound: u32,
}

impl Playout {
    fn new() -> Self {
        Self {
            tones: Deque::new(),
            current: None,
            pushed: 0,
            last_sound: 0,
        }
    }

    fn queue(&mut self, tones: &[Tone]) {
        for t in tones {
            if self.tones.push_back(*t).is_err() {
                warn!("tone queue full");
                break;
            }
        }
    }

    fn tones_pending(&self) -> bool {
        self.current.is_some() || !self.tones.is_empty()
    }

    /// The next beep sample, if a beep is playing.
    fn next_tone(&mut self) -> Option<i16> {
        loop {
            if let Some(s) = self.current.as_mut().and_then(Iterator::next) {
                return Some(s);
            }
            self.current = Some(ToneGen::new(self.tones.pop_front()?));
        }
    }

    /// True once everything pushed so far has had time to play.
    fn drained(&self) -> bool {
        !self.tones_pending() && self.pushed.wrapping_sub(self.last_sound) > TX_LATENCY_FRAMES
    }
}

/// The two streaming DMA transfers. Kept together so a stall can restart both.
struct Streams {
    rx: Option<RxTransfer>,
    tx: Option<TxTransfer>,
}

impl Streams {
    fn start(
        rx: I2sRx<'static, Blocking>,
        tx: I2sTx<'static, Blocking>,
        rx_buf: DmaRxStreamBuf,
        tx_buf: DmaTxStreamBuf,
    ) -> Self {
        // RX first: it is a slave of TX's clocks (signal loopback) and waits for them.
        let rx = rx.read(rx_buf).map_err(|(e, ..)| e).expect("I2S RX start");
        // The TX ring starts full of zeros (silence).
        let tx = tx.write(tx_buf).map_err(|(e, ..)| e).expect("I2S TX start");
        Self {
            rx: Some(rx),
            tx: Some(tx),
        }
    }

    fn rx(&mut self) -> &mut RxTransfer {
        self.rx.as_mut().expect("rx running")
    }

    fn tx(&mut self) -> &mut TxTransfer {
        self.tx.as_mut().expect("tx running")
    }

    fn stop(
        &mut self,
    ) -> (
        I2sRx<'static, Blocking>,
        I2sTx<'static, Blocking>,
        DmaRxStreamBuf,
        DmaTxStreamBuf,
    ) {
        let (tx, tx_buf) = self.tx.take().expect("tx running").stop();
        let (rx, rx_buf) = self.rx.take().expect("rx running").stop();
        (rx, tx, rx_buf, tx_buf)
    }

    fn restart(&mut self) {
        let (rx, tx, rx_buf, tx_buf) = self.stop();
        *self = Self::start(rx, tx, rx_buf, tx_buf);
    }
}

/// Edge detector for an active-low button.
struct Button {
    pin: Input<'static>,
    down: bool,
}

impl Button {
    fn new(pin: Input<'static>) -> Self {
        let down = pin.is_low();
        Self { pin, down }
    }

    /// `Some(true)` on press, `Some(false)` on release.
    fn poll(&mut self) -> Option<bool> {
        let down = self.pin.is_low();
        if down == self.down {
            return None;
        }
        self.down = down;
        Some(down)
    }
}

#[main]
fn main() -> ! {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    // First: keep ourselves powered on battery.
    let _latch = PowerLatch::engage(p.GPIO17);

    esp_println::logger::init_logger_from_env();
    info!(
        "audio loopback {} (reset {:?})",
        env!("CARGO_PKG_VERSION"),
        esp_hal::system::reset_reason()
    );

    // The recording lives in the 8 MB octal PSRAM; it is the only heap.
    let psram = Psram::new(
        p.PSRAM,
        PsramConfig {
            mode: PsramMode::OctalSpi,
            ..PsramConfig::default()
        },
    );
    let (_, psram_size) = psram.raw_parts();
    if psram_size > 0 {
        esp_alloc::psram_allocator!(&psram);
    }
    info!("PSRAM {} KiB", psram_size / 1024);

    let delay = Delay::new();
    let mut led = Output::new(p.GPIO3, Level::High, OutputConfig::default()); // off
    let pull_up = InputConfig::default().with_pull(Pull::Up);
    let mut ok = Button::new(Input::new(p.GPIO0, pull_up));
    let mut up = Button::new(Input::new(p.GPIO39, pull_up));
    let mut down = Button::new(Input::new(p.GPIO18, pull_up));

    // GPIO18 is also the power key: if it switched us on, wait for its release so it is
    // not read as "quieter".
    let mut waited = 0;
    while down.pin.is_low() && waited < 5_000 {
        delay.delay_millis(10);
        waited += 10;
    }
    down.down = down.pin.is_low();

    // Audio rail on (also powers the I2C pull-ups), amplifier still off.
    let mut audio = AudioPower::on(p.GPIO42, p.GPIO46);
    delay.delay_millis(RAIL_SETTLE_MS);

    // I2S first, as in the reference driver: MCLK (256 × fs = 4.096 MHz), BCLK and WS run
    // before the codec is configured. The TX ring starts as 128 ms of silence, which
    // covers the codec setup below (about 60 short I2C transfers).
    let rx_buf = dma_rx_stream_buffer!(DMA_BYTES, DMA_CHUNK);
    let tx_buf = dma_tx_stream_buffer!(DMA_BYTES, DMA_CHUNK);
    let i2s = I2s::new(
        p.I2S0,
        p.DMA_CH0,
        TdmConfig::new_tdm_philips()
            .with_signal_loopback(true)
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("I2S config")
    .with_mclk(p.GPIO14);
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(p.GPIO15)
        .with_ws(p.GPIO38)
        .with_dout(p.GPIO45)
        .build();
    let i2s_rx = i2s.i2s_rx.with_din(p.GPIO16).build();
    let mut streams = Streams::start(i2s_rx, i2s_tx, rx_buf, tx_buf);

    let i2c = I2c::new(
        p.I2C0,
        I2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .expect("I2C config")
    .with_sda(p.GPIO47)
    .with_scl(p.GPIO48);
    let mut codec = Es8311::new(i2c, pins::I2C_ADDR_CODEC);
    let mut volume = VOLUME_DEFAULT;
    let codec_config = CodecConfig {
        volume,
        ..CodecConfig::default()
    };
    if let Err(e) = codec.init(&codec_config) {
        error!("ES8311 init failed: {e:?} (audio rail on? I2C pull-ups need it)");
        for _ in 0..10 {
            led.set_low();
            delay.delay_millis(100);
            led.set_high();
            delay.delay_millis(100);
        }
        let _ = streams.stop();
        audio.off_for_sleep();
        power::deep_sleep(p.LPWR, &mut ok.pin, None);
    }
    info!(
        "ES8311 up: {} Hz, MCLK {} Hz, mic +{} dB, volume {volume} %",
        codec_config.sample_rate,
        codec_config.mclk_hz(),
        codec_config.mic_gain.db()
    );

    let mut out = Playout::new();
    // Recording buffer: raw stereo frames, slot 0 in the low half.
    let mut rec: Vec<u32> = Vec::new();
    let have_buffer = rec.try_reserve_exact(MAX_RECORD_FRAMES).is_ok();
    if !have_buffer {
        error!("cannot allocate {MAX_RECORD_FRAMES} frames for recording (PSRAM?)");
        out.queue(beeps::ERROR);
    }

    // The rings ran unattended during setup; start from a fresh, aligned state.
    streams.restart();
    audio.set_speaker(true);
    out.queue(beeps::STARTUP);

    let mut mode = Mode::Idle;
    let mut levels = SlotLevels::default();
    // After a 30 s auto-stop, OK must be released before it starts a new recording.
    let mut ok_needs_release = ok.down;
    let mut last_activity = Instant::now();
    let mut last_poll = Instant::now();
    let mut last_rx = Instant::now();
    let mut going_to_sleep = false;
    let mut rx_scratch = [0u8; DMA_CHUNK];

    loop {
        // RX: drain the ring; keep frames while recording.
        let mut got = 0;
        loop {
            let n = streams.rx().pop(&mut rx_scratch);
            if n == 0 {
                break;
            }
            got += n;
            if let Mode::Recording { skip } = &mut mode {
                for f in rx_scratch[..n - n % FRAME_BYTES].chunks_exact(FRAME_BYTES) {
                    if *skip > 0 {
                        *skip -= 1;
                        continue;
                    }
                    if rec.len() < MAX_RECORD_FRAMES {
                        let bytes = [f[0], f[1], f[2], f[3]];
                        let (a, b) = slots(bytes);
                        levels.add(a, b);
                        rec.push(u32::from_le_bytes(bytes));
                    }
                }
            }
        }
        let now = Instant::now();
        if got > 0 {
            last_rx = now;
        } else if now - last_rx > STALL {
            error!("I2S stalled, restarting the streams");
            streams.restart();
            last_rx = Instant::now();
        }

        // TX: keep the ring full.
        loop {
            let tx = streams.tx();
            let n = tx.push_with(|buf| {
                let usable = buf.len() - buf.len() % FRAME_BYTES;
                for f in buf[..usable].chunks_exact_mut(FRAME_BYTES) {
                    let s = if let Some(s) = out.next_tone() {
                        s
                    } else if let Mode::Playing { pos, slot } = &mut mode {
                        match rec.get(*pos) {
                            Some(raw) => {
                                *pos += 1;
                                let (a, b) = slots(raw.to_le_bytes());
                                if *slot == 0 { a } else { b }
                            }
                            None => 0,
                        }
                    } else {
                        0
                    };
                    out.pushed = out.pushed.wrapping_add(1);
                    if s != 0 {
                        out.last_sound = out.pushed;
                    }
                    f.copy_from_slice(&frame(s, s));
                }
                usable
            });
            if n == 0 {
                break;
            }
        }

        // Playback finished?
        if let Mode::Playing { pos, .. } = mode
            && pos >= rec.len()
        {
            info!("playback done");
            mode = Mode::Idle;
        }
        // Recording reached the cap?
        if matches!(mode, Mode::Recording { .. }) && rec.len() >= MAX_RECORD_FRAMES {
            warn!("30 s limit reached");
            ok_needs_release = true;
            mode = stop_recording(&mut out, &mut audio, &mut led, &rec, &levels);
        }

        // Speaker only while something is audible (the amplifier hisses into the mic).
        let busy = out.tones_pending() || matches!(mode, Mode::Playing { .. });
        if busy || !out.drained() {
            last_activity = now;
        }
        if audio.speaker_is_on() && !busy && out.drained() {
            audio.set_speaker(false);
        }

        if going_to_sleep {
            if out.drained() {
                break;
            }
            continue;
        }

        if now - last_poll >= BUTTON_POLL {
            last_poll = now;
            match ok.poll() {
                Some(true) if !ok_needs_release => {
                    last_activity = now;
                    if !have_buffer {
                        error!("no recording buffer");
                        audio.set_speaker(true);
                        out.queue(beeps::ERROR);
                    } else {
                        info!("recording");
                        rec.clear();
                        levels = SlotLevels::default();
                        audio.set_speaker(true);
                        out.queue(beeps::RECORD);
                        let beep: u32 = beeps::RECORD.iter().map(Tone::frames).sum();
                        // Drop what the mic hears until the beep has left the speaker.
                        mode = Mode::Recording {
                            skip: beep + TX_LATENCY_FRAMES + SAMPLE_RATE / 50,
                        };
                        led.set_low();
                    }
                }
                Some(false) => {
                    last_activity = now;
                    ok_needs_release = false;
                    if matches!(mode, Mode::Recording { .. }) {
                        mode = stop_recording(&mut out, &mut audio, &mut led, &rec, &levels);
                    }
                }
                _ => {}
            }
            let step = match (up.poll(), down.poll()) {
                (Some(true), _) => Some(true),
                (_, Some(true)) => Some(false),
                _ => None,
            };
            if let Some(louder) = step
                && !matches!(mode, Mode::Recording { .. })
            {
                last_activity = now;
                volume = if louder {
                    volume.saturating_add(VOLUME_STEP).min(100)
                } else {
                    volume.saturating_sub(VOLUME_STEP)
                };
                audio.set_speaker(true);
                match codec.set_volume(volume) {
                    Ok(()) => {
                        info!("volume {volume} %");
                        out.queue(beeps::TICK);
                    }
                    Err(e) => {
                        error!("volume: {e:?}");
                        out.queue(beeps::ERROR);
                    }
                }
            }

            if mode == Mode::Idle && now - last_activity > IDLE_SLEEP && !ok.down {
                info!("idle for {} s, going to sleep", IDLE_SLEEP.as_secs());
                audio.set_speaker(true);
                out.queue(beeps::SLEEP);
                going_to_sleep = true;
            }
        }
    }

    // Shut down: streams, codec, amplifier, rail; then deep sleep until OK.
    let _ = streams.stop();
    audio.set_speaker(false);
    if let Err(e) = codec.power_down() {
        warn!("codec power down: {e:?}");
    }
    audio.off_for_sleep();
    led.set_high();
    power::hold_through_sleep(&mut led);
    info!("sleeping; OK wakes");
    power::deep_sleep(p.LPWR, &mut ok.pin, None)
}

/// End a recording: stop beep, then play it back from the slot that carries the mic.
fn stop_recording(
    out: &mut Playout,
    audio: &mut AudioPower<'_>,
    led: &mut Output<'_>,
    rec: &[u32],
    levels: &SlotLevels,
) -> Mode {
    led.set_high();
    let slot = levels.mic_slot();
    info!(
        "recorded {} ms; peaks slot0 {} slot1 {}; playing slot {slot}",
        rec.len() as u64 * 1000 / u64::from(SAMPLE_RATE),
        levels.peak[0],
        levels.peak[1],
    );
    audio.set_speaker(true);
    out.queue(beeps::STOP);
    if rec.is_empty() {
        Mode::Idle
    } else {
        Mode::Playing { pos: 0, slot }
    }
}
