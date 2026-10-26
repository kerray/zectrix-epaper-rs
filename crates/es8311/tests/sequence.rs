//! Host tests: a fake ES8311 register file on a fake I2C bus records every access, and the
//! tests compare the log with the sequence of Espressif `esp_codec_dev` `es8311.c`
//! (open → set_fs → enable → set_mic_gain → set_vol → unmute), as the Zectrix demo runs it.

use embedded_hal::i2c::{self, ErrorKind, ErrorType, I2c, Operation};
use es8311::{ADDRESS, Config, Error, Es8311, MicGain, reg};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    W(u8, u8),
    R(u8),
}
use Op::{R, W};

/// Register file with a few non-zero power-on values, so read-modify-write is visible.
struct FakeCodec {
    regs: [u8; 256],
    log: Vec<Op>,
    nack: bool,
}

impl FakeCodec {
    fn new() -> Self {
        let mut regs = [0u8; 256];
        regs[0xFD] = 0x83;
        regs[0xFE] = 0x11;
        regs[0x06] = 0x23; // BCLK inverted + some divider: both must be replaced
        regs[0x07] = 0xC5; // tri-state bits 7:6 must survive
        regs[0x09] = 0x5F; // muted, odd format: must end as 0x0C
        regs[0x0A] = 0x40;
        regs[0x0D] = 0xFC;
        regs[0x31] = 0x01;
        Self {
            regs,
            log: Vec::new(),
            nack: false,
        }
    }

    fn writes(&self) -> Vec<Op> {
        self.log
            .iter()
            .copied()
            .filter(|o| matches!(o, W(..)))
            .collect()
    }
}

#[derive(Debug, PartialEq)]
struct Nack;
impl i2c::Error for Nack {
    fn kind(&self) -> ErrorKind {
        ErrorKind::NoAcknowledge(i2c::NoAcknowledgeSource::Address)
    }
}

impl ErrorType for FakeCodec {
    type Error = Nack;
}

impl I2c for FakeCodec {
    fn transaction(&mut self, address: u8, ops: &mut [Operation<'_>]) -> Result<(), Nack> {
        assert_eq!(address, ADDRESS);
        if self.nack {
            return Err(Nack);
        }
        let mut pointer = None;
        for op in ops {
            match op {
                Operation::Write(bytes) => match bytes {
                    [r] => pointer = Some(*r),
                    [r, v] => {
                        self.regs[*r as usize] = *v;
                        self.log.push(W(*r, *v));
                    }
                    _ => panic!("unexpected write {bytes:?}"),
                },
                Operation::Read(buf) => {
                    let r = pointer.expect("read without register pointer");
                    assert_eq!(buf.len(), 1);
                    buf[0] = self.regs[r as usize];
                    self.log.push(R(r));
                }
            }
        }
        Ok(())
    }
}

#[test]
fn init_16khz_matches_reference_sequence() {
    let mut codec = Es8311::new(FakeCodec::new(), ADDRESS);
    codec.init(&Config::default()).unwrap();
    let fake = codec.release();

    let expected = vec![
        R(0xFD),
        R(0xFE),
        // es8311_open
        R(0x0D),
        W(0x0D, 0xFA),
        W(0x44, 0x08),
        W(0x44, 0x08),
        W(0x01, 0x30),
        W(0x02, 0x00),
        W(0x03, 0x10),
        W(0x16, 0x24),
        W(0x04, 0x10),
        W(0x05, 0x00),
        W(0x0B, 0x00),
        W(0x0C, 0x00),
        W(0x10, 0x1F),
        W(0x11, 0x7F),
        W(0x00, 0x80),
        W(0x01, 0x3F),
        R(0x06),
        W(0x06, 0x03),
        W(0x13, 0x10),
        W(0x1B, 0x0A),
        W(0x1C, 0x6A),
        W(0x44, 0x58),
        // set_fs: 16 bit, I2S
        R(0x09),
        W(0x09, 0x4C),
        R(0x0A),
        W(0x0A, 0x4C),
        // config_sample: MCLK 4.096 MHz, 16 kHz
        W(0x02, 0x00),
        W(0x05, 0x00),
        W(0x03, 0x10),
        W(0x04, 0x20),
        R(0x07),
        W(0x07, 0xC0),
        W(0x08, 0xFF),
        R(0x06),
        W(0x06, 0x03),
        // es8311_start
        W(0x00, 0x80),
        W(0x01, 0x3F),
        R(0x09),
        W(0x09, 0x0C),
        R(0x0A),
        W(0x0A, 0x0C),
        W(0x17, 0xBF),
        W(0x0E, 0x02),
        W(0x12, 0x00),
        W(0x14, 0x1A),
        W(0x0D, 0x01),
        W(0x15, 0x40),
        W(0x37, 0x08),
        W(0x45, 0x00),
        // mic gain 30 dB, volume 80 %, unmute
        W(0x16, 0x05),
        W(0x32, 0xB2),
        R(0x31),
        W(0x31, 0x01),
    ];
    assert_eq!(fake.log, expected);
}

#[test]
fn already_reset_codec_skips_first_power_write() {
    let mut fake = FakeCodec::new();
    fake.regs[0x0D] = 0xFA;
    let mut codec = Es8311::new(fake, ADDRESS);
    codec.init(&Config::default()).unwrap();
    let fake = codec.release();
    assert_eq!(fake.log[2..4], [R(0x0D), W(0x44, 0x08)]);
}

#[test]
fn init_24khz_uses_its_clock_row() {
    let mut codec = Es8311::new(FakeCodec::new(), ADDRESS);
    let config = Config {
        sample_rate: 24_000,
        dac_reference: false,
        ..Config::default()
    };
    codec.init(&config).unwrap();
    let fake = codec.release();
    let w = fake.writes();
    // 6.144 MHz / 24 kHz row: pre_div 1, ×1, osr 0x10 / 0x10; no DAC reference.
    assert!(w.contains(&W(0x44, 0x08)));
    assert!(!w.contains(&W(0x44, 0x58)));
    let i = w.iter().position(|o| *o == W(0x0A, 0x4C)).unwrap();
    assert_eq!(
        w[i + 1..i + 5],
        [W(0x02, 0x00), W(0x05, 0x00), W(0x03, 0x10), W(0x04, 0x10)]
    );
}

#[test]
fn unsupported_rate_is_rejected_before_any_bus_access() {
    let mut codec = Es8311::new(FakeCodec::new(), ADDRESS);
    let config = Config {
        sample_rate: 22_000,
        ..Config::default()
    };
    assert_eq!(
        codec.init(&config),
        Err(Error::UnsupportedRate {
            mclk: 22_000 * 256,
            rate: 22_000
        })
    );
    assert!(codec.release().log.is_empty());
}

#[test]
fn wrong_chip_id_stops_init() {
    let mut fake = FakeCodec::new();
    fake.regs[0xFD] = 0x00;
    let mut codec = Es8311::new(fake, ADDRESS);
    assert_eq!(
        codec.init(&Config::default()),
        Err(Error::WrongChip([0x00, 0x11]))
    );
    assert!(codec.release().writes().is_empty());
}

#[test]
fn missing_codec_reports_i2c_error() {
    let mut fake = FakeCodec::new();
    fake.nack = true;
    let mut codec = Es8311::new(fake, ADDRESS);
    assert!(matches!(
        codec.init(&Config::default()),
        Err(Error::I2c(Nack))
    ));
}

#[test]
fn mute_unmute_keeps_other_bits() {
    let mut fake = FakeCodec::new();
    fake.regs[0x31] = 0x9F;
    let mut codec = Es8311::new(fake, ADDRESS);
    codec.set_mute(true).unwrap();
    codec.set_mute(false).unwrap();
    let fake = codec.release();
    assert_eq!(fake.writes(), [W(0x31, 0xFF), W(0x31, 0x9F)]);
}

#[test]
fn volume_gain_and_raw_writes() {
    let mut codec = Es8311::new(FakeCodec::new(), ADDRESS);
    codec.set_volume(0).unwrap();
    codec.set_volume(50).unwrap();
    codec.set_dac_volume_raw(0xBF).unwrap();
    codec.set_mic_gain(MicGain::Db42).unwrap();
    let fake = codec.release();
    assert_eq!(
        fake.writes(),
        [W(0x32, 0x00), W(0x32, 148), W(0x32, 0xBF), W(0x16, 0x07)]
    );
}

#[test]
fn power_down_matches_reference_suspend() {
    let mut codec = Es8311::new(FakeCodec::new(), ADDRESS);
    codec.power_down().unwrap();
    let fake = codec.release();
    assert_eq!(
        fake.log,
        [
            R(0x31),
            W(0x31, 0x61),
            W(0x32, 0x00),
            W(0x17, 0x00),
            W(0x0E, 0xFF),
            W(0x12, 0x02),
            W(0x14, 0x00),
            W(0x0D, 0xFA),
            W(0x15, 0x00),
            W(0x02, 0x10),
            W(0x00, 0x00),
            W(0x00, 0x1F),
            W(0x01, 0x30),
            W(0x01, 0x00),
            W(0x45, 0x00),
            W(0x0D, 0xFC),
            W(0x02, 0x00),
        ]
    );
    // The register file ends powered down.
    assert_eq!(fake.regs[reg::SYSTEM_0D as usize], 0xFC);
}
