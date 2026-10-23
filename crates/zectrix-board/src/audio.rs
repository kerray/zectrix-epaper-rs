//! Audio helpers that need no hardware: the I2S frame layout used with the ES8311, beep
//! synthesis and choosing the microphone slot. The rail and amplifier pins are in
//! `power::AudioPower` (feature `hal`). See `docs/AUDIO.md`.
//!
//! One full-duplex I2S bus carries both directions, so capture and playback share one
//! sample rate. Frames are 16-bit stereo (Philips): 4 bytes, slot 0 first, little endian.
//! The ES8311 is mono: it plays slot 0 (both slots get the same sample) and records the
//! mic into one slot; with the DAC reference on, the other slot carries the playback.

/// Sample rate of the audio bus (both directions).
pub const SAMPLE_RATE: u32 = 16_000;
/// Bytes per stereo 16-bit frame.
pub const FRAME_BYTES: usize = 4;

/// A stereo frame from its two 16-bit slots.
pub const fn frame(slot0: i16, slot1: i16) -> [u8; FRAME_BYTES] {
    let a = slot0.to_le_bytes();
    let b = slot1.to_le_bytes();
    [a[0], a[1], b[0], b[1]]
}

/// The two slots of a received frame.
pub const fn slots(bytes: [u8; FRAME_BYTES]) -> (i16, i16) {
    (
        i16::from_le_bytes([bytes[0], bytes[1]]),
        i16::from_le_bytes([bytes[2], bytes[3]]),
    )
}

/// Running peak level of both slots, to find which one carries the microphone.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SlotLevels {
    pub peak: [u16; 2],
}

impl SlotLevels {
    pub fn add(&mut self, slot0: i16, slot1: i16) {
        self.peak[0] = self.peak[0].max(slot0.unsigned_abs());
        self.peak[1] = self.peak[1].max(slot1.unsigned_abs());
    }

    /// The slot with the larger peak; slot 0 on a tie. With the codec's DAC reference on,
    /// the other slot holds the (silent) playback while recording, so the louder slot is
    /// the microphone whichever way round the codec and the ESP32-S3 order the slots.
    pub fn mic_slot(&self) -> usize {
        usize::from(self.peak[1] > self.peak[0])
    }
}

/// Sine approximation (Bhaskara I) for a phase in 0..65536 = one period. Integer only,
/// error below 0.2 % of full scale, which is plenty for beeps.
pub fn sine(phase: u16, amplitude: i16) -> i16 {
    let half = phase >= 0x8000;
    // x in 0..32768 represents 0..π.
    let x = i64::from(phase & 0x7FFF);
    let pi = 32_768i64;
    let p = x * (pi - x);
    let v = 16 * p * i64::from(amplitude) / (5 * pi * pi - 4 * p);
    let v = v as i16;
    if half { -v } else { v }
}

/// A beep: a sine tone with a short linear fade in and out against clicks.
/// `freq_hz == 0` is a pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tone {
    pub freq_hz: u16,
    pub ms: u16,
    pub amplitude: i16,
}

impl Tone {
    pub const fn new(freq_hz: u16, ms: u16, amplitude: i16) -> Self {
        Self {
            freq_hz,
            ms,
            amplitude,
        }
    }

    pub const fn pause(ms: u16) -> Self {
        Self::new(0, ms, 0)
    }

    /// Length in frames at [`SAMPLE_RATE`].
    pub const fn frames(&self) -> u32 {
        self.ms as u32 * SAMPLE_RATE / 1000
    }
}

/// Plays one [`Tone`] sample by sample.
#[derive(Debug, Clone)]
pub struct ToneGen {
    tone: Tone,
    n: u32,
    total: u32,
    phase: u32,
    step: u32,
}

/// Fade length in frames (4 ms).
const FADE: u32 = SAMPLE_RATE / 250;

impl ToneGen {
    pub fn new(tone: Tone) -> Self {
        Self {
            tone,
            n: 0,
            total: tone.frames(),
            phase: 0,
            // Phase step per sample in 1/65536 of a period, ×256 for precision.
            step: ((u64::from(tone.freq_hz) << 24) / u64::from(SAMPLE_RATE)) as u32,
        }
    }
}

impl Iterator for ToneGen {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        if self.n >= self.total {
            return None;
        }
        let n = self.n;
        self.n += 1;
        if self.tone.freq_hz == 0 {
            return Some(0);
        }
        let s = sine((self.phase >> 8) as u16, self.tone.amplitude);
        self.phase = self.phase.wrapping_add(self.step);
        let left = self.total - n;
        let fade = n.min(left).min(FADE);
        Some((i32::from(s) * fade as i32 / FADE as i32) as i16)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = (self.total - self.n) as usize;
        (left, Some(left))
    }
}

/// The beeps the loopback app plays.
pub mod beeps {
    use super::Tone;

    const A: i16 = 8_000;
    /// Boot: two rising notes.
    pub const STARTUP: &[Tone] = &[
        Tone::new(660, 90, A),
        Tone::pause(30),
        Tone::new(990, 120, A),
    ];
    /// Recording starts: one high note.
    pub const RECORD: &[Tone] = &[Tone::new(1_320, 80, A)];
    /// Recording stopped: two short low notes.
    pub const STOP: &[Tone] = &[
        Tone::new(880, 50, A),
        Tone::pause(40),
        Tone::new(660, 70, A),
    ];
    /// Volume changed: a short tick (played at the new volume).
    pub const TICK: &[Tone] = &[Tone::new(1_760, 25, A)];
    /// Something failed: a long low buzz, falling.
    pub const ERROR: &[Tone] = &[
        Tone::new(330, 250, A),
        Tone::pause(60),
        Tone::new(220, 400, A),
    ];
    /// Going to sleep: two falling notes.
    pub const SLEEP: &[Tone] = &[
        Tone::new(990, 90, A),
        Tone::pause(30),
        Tone::new(660, 120, A),
    ];
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    #[test]
    fn frame_roundtrip() {
        let f = frame(-2, 0x1234);
        assert_eq!(f, [0xFE, 0xFF, 0x34, 0x12]);
        assert_eq!(slots(f), (-2, 0x1234));
    }

    #[test]
    fn sine_shape() {
        assert_eq!(sine(0, 10_000), 0);
        assert_eq!(sine(0x8000, 10_000), 0);
        assert!((sine(0x4000, 10_000) - 10_000).abs() <= 20);
        assert!((sine(0xC000, 10_000) + 10_000).abs() <= 20);
        // sin(π/6) = 0.5
        assert!((sine(5_461, 10_000) - 5_000).abs() <= 40);
    }

    #[test]
    fn tone_length_fades_and_frequency() {
        let tone = Tone::new(1_000, 100, 8_000);
        let samples: Vec<i16> = ToneGen::new(tone).collect();
        assert_eq!(samples.len(), 1_600);
        assert_eq!(samples[0], 0);
        assert!(samples[1_599].abs() < 8_000 / 50);
        let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap();
        assert!((7_900..=8_000).contains(&peak), "peak {peak}");
        // 1 kHz over 100 ms: 100 periods, so about 200 sign changes.
        let crossings = samples
            .windows(2)
            .filter(|w| (w[0] < 0) != (w[1] < 0))
            .count();
        assert!((195..=205).contains(&crossings), "{crossings}");
    }

    #[test]
    fn pause_is_silent() {
        let samples: Vec<i16> = ToneGen::new(Tone::pause(10)).collect();
        assert_eq!(samples.len(), 160);
        assert!(samples.iter().all(|&s| s == 0));
    }

    #[test]
    fn louder_slot_is_the_mic() {
        let mut levels = SlotLevels::default();
        assert_eq!(levels.mic_slot(), 0);
        levels.add(10, -3_000);
        levels.add(-20, 100);
        assert_eq!(levels.peak, [20, 3_000]);
        assert_eq!(levels.mic_slot(), 1);
        levels.add(i16::MIN, 0);
        assert_eq!(levels.mic_slot(), 0);
    }
}
