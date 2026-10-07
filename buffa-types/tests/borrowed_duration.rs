#![no_std]

use core::time::Duration as CoreDuration;

use buffa_types::{Duration, DurationError};

fn duration(seconds: i64, nanos: i32) -> Duration {
    Duration {
        seconds,
        nanos,
        ..Default::default()
    }
}

#[test]
fn core_conversion_by_reference_matches_by_value() {
    for seconds in [0, 1, 315_576_000_000, i64::MAX] {
        for nanos in [0, 1, 345_678_901, 999_999_999] {
            let proto = duration(seconds, nanos);
            let expected = CoreDuration::new(seconds as u64, nanos as u32);
            let converted: CoreDuration = (&proto).try_into().unwrap();
            assert_eq!(converted, expected);
            assert_eq!(CoreDuration::try_from(&proto), Ok(expected));
            assert_eq!(CoreDuration::try_from(proto), Ok(expected));
        }
    }
}

#[test]
fn core_conversion_rejects_well_formed_negative_durations() {
    for seconds in [i64::MIN, -1, 0] {
        for nanos in [-999_999_999, -1, 0] {
            if seconds == 0 && nanos == 0 {
                continue;
            }
            let proto = duration(seconds, nanos);
            assert_eq!(
                CoreDuration::try_from(&proto),
                Err(DurationError::NegativeDuration)
            );
            assert_eq!(
                CoreDuration::try_from(proto),
                Err(DurationError::NegativeDuration)
            );
        }
    }
}

fn assert_invalid_nanos(proto: &Duration) {
    assert_eq!(
        CoreDuration::try_from(proto),
        Err(DurationError::InvalidNanos)
    );
    assert_eq!(
        CoreDuration::try_from(proto.clone()),
        Err(DurationError::InvalidNanos)
    );
    #[cfg(feature = "chrono")]
    {
        assert_eq!(
            chrono::TimeDelta::try_from(proto),
            Err(buffa_types::DurationChronoError::InvalidNanos)
        );
        assert_eq!(
            chrono::TimeDelta::try_from(proto.clone()),
            Err(buffa_types::DurationChronoError::InvalidNanos)
        );
    }
    #[cfg(feature = "jiff")]
    {
        assert_eq!(
            jiff::SignedDuration::try_from(proto),
            Err(buffa_types::DurationJiffError::InvalidNanos)
        );
        assert_eq!(
            jiff::SignedDuration::try_from(proto.clone()),
            Err(buffa_types::DurationJiffError::InvalidNanos)
        );
    }
}

#[test]
fn borrowed_conversions_reject_out_of_range_nanos() {
    for seconds in [i64::MIN, -1, 0, 1, i64::MAX] {
        for nanos in [i32::MIN, -1_000_000_000, 1_000_000_000, i32::MAX] {
            assert_invalid_nanos(&duration(seconds, nanos));
        }
    }
}

#[test]
fn borrowed_conversions_reject_inconsistent_signs() {
    for (seconds, nanos) in [
        (i64::MIN, 1),
        (-1, 999_999_999),
        (1, -1),
        (i64::MAX, -999_999_999),
    ] {
        assert_invalid_nanos(&duration(seconds, nanos));
    }
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_conversion_by_reference_matches_by_value() {
    for expected in [
        chrono::TimeDelta::zero(),
        chrono::TimeDelta::nanoseconds(1),
        chrono::TimeDelta::nanoseconds(-1),
        chrono::TimeDelta::milliseconds(1_500),
        chrono::TimeDelta::milliseconds(-1_500),
        chrono::TimeDelta::milliseconds(i64::MAX),
        chrono::TimeDelta::milliseconds(-i64::MAX),
    ] {
        let proto = duration(expected.num_seconds(), expected.subsec_nanos());
        let converted: chrono::TimeDelta = (&proto).try_into().unwrap();
        assert_eq!(converted, expected);
        assert_eq!(chrono::TimeDelta::try_from(&proto), Ok(expected));
        assert_eq!(chrono::TimeDelta::try_from(proto), Ok(expected));
    }
}

#[cfg(feature = "chrono")]
#[test]
fn chrono_conversion_rejects_overflow() {
    for (seconds, nanos) in [
        (i64::MIN, 0),
        (i64::MAX, 0),
        (i64::MAX / 1_000, 999_999_999),
        (-(i64::MAX / 1_000), -999_999_999),
    ] {
        let proto = duration(seconds, nanos);
        assert_eq!(
            chrono::TimeDelta::try_from(&proto),
            Err(buffa_types::DurationChronoError::Overflow)
        );
        assert_eq!(
            chrono::TimeDelta::try_from(proto),
            Err(buffa_types::DurationChronoError::Overflow)
        );
    }
}

#[cfg(feature = "jiff")]
#[test]
fn jiff_conversion_by_reference_matches_by_value() {
    for expected in [
        jiff::SignedDuration::ZERO,
        jiff::SignedDuration::new(0, 1),
        jiff::SignedDuration::new(0, -1),
        jiff::SignedDuration::new(1, 500_000_000),
        jiff::SignedDuration::new(-1, -500_000_000),
        jiff::SignedDuration::MIN,
        jiff::SignedDuration::MAX,
    ] {
        let proto = duration(expected.as_secs(), expected.subsec_nanos());
        let converted: jiff::SignedDuration = (&proto).try_into().unwrap();
        assert_eq!(converted, expected);
        assert_eq!(jiff::SignedDuration::try_from(&proto), Ok(expected));
        assert_eq!(jiff::SignedDuration::try_from(proto), Ok(expected));
    }
}
