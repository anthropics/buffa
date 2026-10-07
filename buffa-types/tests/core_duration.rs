#![no_std]

#[cfg(feature = "std")]
extern crate std;

use core::time::Duration as CoreDuration;

use buffa::Message;
use buffa_types::google::protobuf::Duration;
use buffa_types::DurationError;

#[test]
fn core_duration_roundtrip() {
    for seconds in [0, 1, 12, 315_576_000_000, i64::MAX as u64] {
        for nanos in [0, 1, 345_678_901, 999_999_999] {
            let duration = CoreDuration::new(seconds, nanos);
            let proto = Duration::from(duration);
            assert_eq!(proto.seconds, seconds as i64);
            assert_eq!(proto.nanos, nanos as i32);
            assert!(proto.__buffa_unknown_fields.is_empty());
            assert_eq!(CoreDuration::try_from(proto), Ok(duration));
        }
    }
}

#[test]
fn core_duration_saturates_seconds_and_preserves_nanos() {
    for seconds in [i64::MAX as u64 + 1, u64::MAX] {
        for nanos in [0, 1, 999_999_999] {
            let proto = Duration::from(CoreDuration::new(seconds, nanos));
            assert_eq!(proto.seconds, i64::MAX);
            assert_eq!(proto.nanos, nanos as i32);
            assert_eq!(
                CoreDuration::try_from(proto),
                Ok(CoreDuration::new(i64::MAX as u64, nanos))
            );
        }
    }
}

#[test]
fn negative_duration_is_rejected() {
    for seconds in [i64::MIN, -5, -1, 0] {
        for nanos in [-999_999_999, -1, 0] {
            if seconds == 0 && nanos == 0 {
                continue;
            }
            assert_eq!(
                CoreDuration::try_from(Duration::from_secs_nanos(seconds, nanos)),
                Err(DurationError::NegativeDuration)
            );
        }
    }
}

#[test]
fn invalid_nanos_are_rejected_before_negative_durations() {
    for seconds in [-1, 0, 1] {
        for nanos in [i32::MIN, -1_000_000_000, 1_000_000_000, i32::MAX] {
            let proto = Duration {
                seconds,
                nanos,
                ..Default::default()
            };
            assert_eq!(
                CoreDuration::try_from(proto),
                Err(DurationError::InvalidNanos)
            );
        }
    }
}

#[test]
fn inconsistent_signs_are_rejected() {
    for (seconds, nanos) in [
        (i64::MIN, 1),
        (-1, 999_999_999),
        (1, -1),
        (i64::MAX, -999_999_999),
    ] {
        let proto = Duration {
            seconds,
            nanos,
            ..Default::default()
        };
        assert_eq!(
            CoreDuration::try_from(proto),
            Err(DurationError::InvalidNanos)
        );
    }
}

#[test]
fn core_duration_wire_roundtrip() {
    let duration = CoreDuration::new(315_576_000_000, 999_999_999);
    let bytes = Duration::from(duration).encode_to_vec();
    let decoded = Duration::decode_from_slice(&bytes).unwrap();
    assert_eq!(CoreDuration::try_from(decoded), Ok(duration));
}

#[cfg(feature = "std")]
#[test]
fn std_duration_uses_the_same_conversions() {
    let duration = std::time::Duration::new(7, 123_456_789);
    let proto = Duration::from(duration);
    let converted: std::time::Duration = proto.try_into().unwrap();
    assert_eq!(converted, duration);
}
