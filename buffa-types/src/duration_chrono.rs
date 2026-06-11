//! `chrono` interop for [`google::protobuf::Duration`](crate::google::protobuf::Duration).
//!
//! Enabled with the `chrono` Cargo feature. `no_std`-compatible — `chrono` is
//! pulled in with `default-features = false`.

use crate::google::protobuf::Duration;

/// Errors that can occur when converting a protobuf [`Duration`] to a
/// [`chrono::TimeDelta`].
///
/// Distinct from [`crate::duration_ext::DurationError`] because `TimeDelta`'s
/// representable range (`±i64::MAX` milliseconds) is narrower than proto
/// `Duration`'s, so this conversion has an `Overflow` failure mode that
/// `std::time::Duration` does not.
///
/// This enum is `#[non_exhaustive]` (unlike the older `DurationError` /
/// `TimestampError`, which predate that convention): `match` arms over it
/// must include a wildcard arm.
#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DurationChronoError {
    /// The `nanos` field is outside `[-999_999_999, 999_999_999]` or its sign
    /// is inconsistent with `seconds`.
    #[error("nanos field has invalid value or sign mismatch with seconds")]
    InvalidNanos,
    /// The duration exceeds `chrono::TimeDelta`'s representable range
    /// (`±i64::MAX` milliseconds).
    #[error("duration is out of range for chrono::TimeDelta")]
    Overflow,
}

#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
impl From<chrono::TimeDelta> for Duration {
    /// Convert a [`chrono::TimeDelta`] to a protobuf [`Duration`].
    ///
    /// Both sides represent a signed duration as `seconds` + `subsec_nanos`
    /// with sign-consistent components, so this is a direct field copy.
    ///
    /// # Warning: proto JSON spec range
    ///
    /// `chrono::TimeDelta` ranges to ±`i64::MAX` milliseconds (~9.2e15
    /// seconds), while the proto spec restricts `Duration` to
    /// ±315,576,000,000 seconds (~10,000 years). A `TimeDelta` beyond that
    /// converts without error here — binary encoding round-trips it — but
    /// the resulting `Duration` will fail JSON serialization (`json`
    /// feature), which enforces the spec range.
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Duration;
    /// use chrono::TimeDelta;
    ///
    /// let proto: Duration = TimeDelta::milliseconds(1_500).into();
    /// assert_eq!(proto.seconds, 1);
    /// assert_eq!(proto.nanos, 500_000_000);
    /// ```
    fn from(d: chrono::TimeDelta) -> Self {
        Self {
            seconds: d.num_seconds(),
            // `TimeDelta::subsec_nanos` is signed and shares the duration's
            // overall sign, matching the proto Duration convention.
            nanos: d.subsec_nanos(),
            ..Default::default()
        }
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
impl TryFrom<Duration> for chrono::TimeDelta {
    type Error = DurationChronoError;

    /// Convert a protobuf [`Duration`] to a [`chrono::TimeDelta`].
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::{Duration, DurationChronoError};
    /// use chrono::TimeDelta;
    ///
    /// let proto = Duration {
    ///     seconds: 2,
    ///     nanos: 250_000_000,
    ///     ..Default::default()
    /// };
    /// let td: TimeDelta = proto.try_into().unwrap();
    /// assert_eq!(td, TimeDelta::milliseconds(2_250));
    ///
    /// let too_big = Duration {
    ///     seconds: i64::MAX,
    ///     nanos: 0,
    ///     ..Default::default()
    /// };
    /// assert_eq!(
    ///     TimeDelta::try_from(too_big),
    ///     Err(DurationChronoError::Overflow)
    /// );
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`DurationChronoError::InvalidNanos`] if `nanos` is outside
    /// `[-999_999_999, 999_999_999]` or if its sign is inconsistent with
    /// `seconds`. Such values never come from the [`From<chrono::TimeDelta>`]
    /// impl, but `seconds` and `nanos` are independent wire fields, so a
    /// decoded `Duration` can carry any combination — the proto spec declares
    /// sign-mismatched ones invalid, and this conversion rejects them rather
    /// than silently reinterpreting them arithmetically.
    ///
    /// Returns [`DurationChronoError::Overflow`] if the total duration
    /// exceeds `chrono::TimeDelta`'s representable range (`±i64::MAX`
    /// milliseconds).
    fn try_from(d: Duration) -> Result<Self, Self::Error> {
        if d.nanos < -999_999_999 || d.nanos > 999_999_999 {
            return Err(DurationChronoError::InvalidNanos);
        }
        // Decoding doesn't validate the spec's sign-consistency rule, so a
        // malformed message can carry e.g. {seconds: 5, nanos: -1}. Reject
        // per spec instead of guessing at an arithmetic interpretation.
        let sign_mismatch = (d.seconds > 0 && d.nanos < 0) || (d.seconds < 0 && d.nanos > 0);
        if sign_mismatch {
            return Err(DurationChronoError::InvalidNanos);
        }

        // `chrono::TimeDelta` is internally `i64` milliseconds; large second
        // values that fit in proto Duration can overflow it. Build from the
        // two components with checked arithmetic.
        //
        // `try_seconds` rejects |seconds| > i64::MAX / 1000 ≈ 9.22e15. After
        // that, `checked_add` is still needed because a `secs_part` close to
        // the i64-millisecond boundary plus a `nanos_part` of up to ±999 ms
        // can still push the sum over i64::MAX. `nanoseconds(_)` itself can
        // never overflow because |nanos| < 1e9 always fits in `TimeDelta`.
        let secs_part = Self::try_seconds(d.seconds).ok_or(DurationChronoError::Overflow)?;
        let nanos_part = Self::nanoseconds(i64::from(d.nanos));
        secs_part
            .checked_add(&nanos_part)
            .ok_or(DurationChronoError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_roundtrip() {
        let td = chrono::TimeDelta::new(300, 500_000_000).unwrap();
        let proto: Duration = td.into();
        assert_eq!(proto.seconds, 300);
        assert_eq!(proto.nanos, 500_000_000);
        let back: chrono::TimeDelta = proto.try_into().unwrap();
        assert_eq!(back, td);
    }

    #[test]
    fn zero_roundtrip() {
        let td = chrono::TimeDelta::zero();
        let proto: Duration = td.into();
        assert_eq!(proto.seconds, 0);
        assert_eq!(proto.nanos, 0);
        let back: chrono::TimeDelta = proto.try_into().unwrap();
        assert_eq!(back, td);
    }

    #[test]
    fn negative_roundtrip() {
        // -1.5 seconds. chrono returns num_seconds = -1, subsec_nanos = -500_000_000,
        // matching the proto convention.
        let td = chrono::TimeDelta::milliseconds(-1_500);
        let proto: Duration = td.into();
        assert_eq!(proto.seconds, -1);
        assert_eq!(proto.nanos, -500_000_000);
        let back: chrono::TimeDelta = proto.try_into().unwrap();
        assert_eq!(back, td);
    }

    #[test]
    fn sub_second_negative_roundtrip() {
        let td = chrono::TimeDelta::nanoseconds(-500_000_000);
        let proto: Duration = td.into();
        assert_eq!(proto.seconds, 0);
        assert_eq!(proto.nanos, -500_000_000);
        let back: chrono::TimeDelta = proto.try_into().unwrap();
        assert_eq!(back, td);
    }

    #[test]
    fn invalid_nanos_rejected() {
        let bad = Duration {
            seconds: 1,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let result: Result<chrono::TimeDelta, _> = bad.try_into();
        assert_eq!(result, Err(DurationChronoError::InvalidNanos));
    }

    #[test]
    fn nanos_i32_min_is_invalid() {
        let bad = Duration {
            seconds: 0,
            nanos: i32::MIN,
            ..Default::default()
        };
        let result: Result<chrono::TimeDelta, _> = bad.try_into();
        assert_eq!(result, Err(DurationChronoError::InvalidNanos));
    }

    #[test]
    fn sign_mismatch_rejected() {
        let bad = Duration {
            seconds: 5,
            nanos: -1,
            ..Default::default()
        };
        let result: Result<chrono::TimeDelta, _> = bad.try_into();
        assert_eq!(result, Err(DurationChronoError::InvalidNanos));

        let bad2 = Duration {
            seconds: -5,
            nanos: 1,
            ..Default::default()
        };
        let result2: Result<chrono::TimeDelta, _> = bad2.try_into();
        assert_eq!(result2, Err(DurationChronoError::InvalidNanos));
    }

    #[test]
    fn timedelta_extremes_roundtrip() {
        // `TimeDelta` spans ±i64::MAX milliseconds. Pin that both extremes
        // survive the proto roundtrip exactly (constructed via `milliseconds`,
        // which is total over i64, rather than the MIN/MAX consts that only
        // exist in newer chrono versions).
        let max = chrono::TimeDelta::milliseconds(i64::MAX);
        let proto: Duration = max.into();
        assert_eq!(proto.seconds, max.num_seconds());
        assert_eq!(proto.nanos, max.subsec_nanos());
        let back: chrono::TimeDelta = proto.try_into().unwrap();
        assert_eq!(back, max);

        let min = chrono::TimeDelta::milliseconds(-i64::MAX);
        let proto_min: Duration = min.into();
        let back_min: chrono::TimeDelta = proto_min.try_into().unwrap();
        assert_eq!(back_min, min);
    }

    #[test]
    fn nanos_addition_overflow_is_overflow() {
        // try_seconds accepts |seconds| up to i64::MAX / 1000. At that boundary
        // the resulting TimeDelta is within ~999 ms of i64::MAX milliseconds;
        // a positive nanos value tips checked_add over the edge.
        let boundary_secs = i64::MAX / 1_000;
        let near_max = Duration {
            seconds: boundary_secs,
            nanos: 999_999_999,
            ..Default::default()
        };
        let result: Result<chrono::TimeDelta, _> = near_max.try_into();
        assert_eq!(result, Err(DurationChronoError::Overflow));

        // Mirror for the negative boundary.
        let boundary_neg = -(i64::MAX / 1_000);
        let near_min = Duration {
            seconds: boundary_neg,
            nanos: -999_999_999,
            ..Default::default()
        };
        let result_neg: Result<chrono::TimeDelta, _> = near_min.try_into();
        assert_eq!(result_neg, Err(DurationChronoError::Overflow));
    }

    #[test]
    fn out_of_range_seconds_is_overflow() {
        // `chrono::TimeDelta` caps at `i64::MAX` milliseconds, so `i64::MAX`
        // seconds overflows by ~1000×.
        let huge = Duration {
            seconds: i64::MAX,
            nanos: 0,
            ..Default::default()
        };
        let result: Result<chrono::TimeDelta, _> = huge.try_into();
        assert_eq!(result, Err(DurationChronoError::Overflow));

        let tiny = Duration {
            seconds: i64::MIN,
            nanos: 0,
            ..Default::default()
        };
        let result2: Result<chrono::TimeDelta, _> = tiny.try_into();
        assert_eq!(result2, Err(DurationChronoError::Overflow));
    }
}
