#![cfg(any(feature = "std", feature = "chrono", feature = "jiff"))]

use buffa_types::{Timestamp, TimestampError};

/// Converts a range of timestamps by reference: each success converts back to
/// the same `Timestamp`, each failure is `Overflow`, and the by-value
/// conversion agrees. Out-of-range `nanos` is `InvalidNanos`.
fn assert_matches_owned<T>()
where
    T: TryFrom<Timestamp, Error = TimestampError>
        + for<'a> TryFrom<&'a Timestamp, Error = TimestampError>
        + core::fmt::Debug
        + PartialEq,
    Timestamp: From<T>,
{
    for seconds in [
        i64::MIN,
        -62_135_596_800,
        -2,
        -1,
        0,
        1,
        1_700_000_000,
        253_402_300_799,
        i64::MAX,
    ] {
        for nanos in [0, 1, 500_000_000, 999_999_999] {
            let timestamp = Timestamp::from_unix(seconds, nanos);
            assert_eq!(
                T::try_from(&timestamp),
                T::try_from(timestamp.clone()),
                "{timestamp:?}"
            );
            match T::try_from(&timestamp) {
                Ok(converted) => assert_eq!(Timestamp::from(converted), timestamp),
                Err(err) => assert_eq!(err, TimestampError::Overflow, "{timestamp:?}"),
            }
        }
    }
    for seconds in [i64::MIN, 0, i64::MAX] {
        for nanos in [i32::MIN, -1, 1_000_000_000, i32::MAX] {
            let timestamp = Timestamp {
                seconds,
                nanos,
                ..Default::default()
            };
            assert_eq!(
                T::try_from(&timestamp),
                Err(TimestampError::InvalidNanos),
                "{timestamp:?}",
            );
        }
    }
}

#[cfg(feature = "std")]
#[test]
fn borrowed_system_time_matches_owned_conversion_and_errors() {
    assert_matches_owned::<std::time::SystemTime>();
}

#[cfg(feature = "std")]
#[test]
fn borrowed_system_time_handles_fractional_pre_epoch_instants() {
    let timestamp = Timestamp::from_unix(-2, 500_000_000);
    let time: std::time::SystemTime = (&timestamp).try_into().unwrap();
    assert_eq!(
        time,
        std::time::UNIX_EPOCH - std::time::Duration::from_millis(1_500)
    );
}

#[cfg(feature = "chrono")]
#[test]
fn borrowed_chrono_matches_owned_conversion_and_errors() {
    assert_matches_owned::<chrono::DateTime<chrono::Utc>>();
}

#[cfg(feature = "chrono")]
#[test]
fn borrowed_chrono_handles_fractional_pre_epoch_instants() {
    let timestamp = Timestamp::from_unix(-2, 500_000_000);
    let time: chrono::DateTime<chrono::Utc> = (&timestamp).try_into().unwrap();
    assert_eq!(time.timestamp(), -2);
    assert_eq!(time.timestamp_subsec_nanos(), 500_000_000);
}

#[cfg(feature = "chrono")]
#[test]
fn borrowed_chrono_handles_target_boundaries() {
    for time in [
        chrono::DateTime::<chrono::Utc>::MIN_UTC,
        chrono::DateTime::<chrono::Utc>::MAX_UTC,
    ] {
        let timestamp = Timestamp::from(time);
        assert_eq!(
            chrono::DateTime::<chrono::Utc>::try_from(&timestamp),
            Ok(time)
        );
    }
    for seconds in [i64::MIN, i64::MAX] {
        let timestamp = Timestamp::from_unix_secs(seconds);
        assert_eq!(
            chrono::DateTime::<chrono::Utc>::try_from(&timestamp),
            Err(TimestampError::Overflow)
        );
    }
}

#[cfg(feature = "jiff")]
#[test]
fn borrowed_jiff_matches_owned_conversion_and_errors() {
    assert_matches_owned::<jiff::Timestamp>();
}

#[cfg(feature = "jiff")]
#[test]
fn borrowed_jiff_handles_fractional_pre_epoch_instants() {
    let timestamp = Timestamp::from_unix(-2, 500_000_000);
    let time: jiff::Timestamp = (&timestamp).try_into().unwrap();
    assert_eq!(time.as_second(), -1);
    assert_eq!(time.subsec_nanosecond(), -500_000_000);
}

#[cfg(feature = "jiff")]
#[test]
fn borrowed_jiff_handles_target_boundaries() {
    for time in [jiff::Timestamp::MIN, jiff::Timestamp::MAX] {
        let timestamp = Timestamp::from(time);
        assert_eq!(jiff::Timestamp::try_from(&timestamp), Ok(time));
    }
    for seconds in [i64::MIN, i64::MAX] {
        let timestamp = Timestamp::from_unix_secs(seconds);
        assert_eq!(
            jiff::Timestamp::try_from(&timestamp),
            Err(TimestampError::Overflow)
        );
    }
}
