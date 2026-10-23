//! Battery voltage maths. The percentage curve originates in xiaozhi-esp32's Zectrix board
//! file (`main/boards/zectrix/zectrix-s3-epaper-4.2/zectrix-s3-epaper-4.2.cc`, MIT,
//! Shenzhen Xinzhi Future Technology Co., Ltd. and Project Contributors), carried on by
//! LazyYoun's firmware and emini-home `home_battery.c` (see THIRD_PARTY_NOTICES.md). It is
//! a rough estimate, not a fuel gauge, and means nothing while charging.

/// Readings outside this window are treated as "no reading" (emini-home).
pub const VALID_MV: core::ops::RangeInclusive<u32> = 2_800..=4_350;

/// The ADC sees half the battery voltage.
pub const DIVIDER: u32 = 2;

/// Battery millivolts from the calibrated ADC pin millivolts.
pub const fn battery_mv(pin_mv: u32) -> u32 {
    pin_mv * DIVIDER
}

/// `Some(mv)` if the reading is plausible.
pub fn validate(mv: u32) -> Option<u32> {
    VALID_MV.contains(&mv).then_some(mv)
}

/// Estimated charge in percent: `(-V² + 9016·V − 19 189 000) / 10 000`, clamped to 0–100.
pub fn percent(mv: u32) -> u8 {
    let v = mv as i64;
    let p = (-v * v + 9016 * v - 19_189_000) / 10_000;
    p.clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_endpoints() {
        assert_eq!(percent(4_200), 100);
        assert_eq!(percent(4_350), 100);
        assert_eq!(percent(3_000), 0);
        let mid = percent(3_700);
        assert!((30..=70).contains(&mid), "3.7 V gave {mid} %");
    }

    #[test]
    fn curve_is_monotonic_in_useful_range() {
        let mut last = 0;
        for mv in (3_300..=4_200).step_by(10) {
            let p = percent(mv);
            assert!(p >= last, "{mv} mV: {p} < {last}");
            last = p;
        }
    }

    #[test]
    fn validation_window() {
        assert_eq!(validate(battery_mv(1_900)), Some(3_800));
        assert_eq!(validate(100), None);
        assert_eq!(validate(5_000), None);
    }
}
