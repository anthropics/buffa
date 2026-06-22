//! `chrono` interop for [`google::protobuf::Timestamp`](crate::google::protobuf::Timestamp).
//!
//! Enabled with the `chrono` Cargo feature. `no_std`-compatible — `chrono` is
//! pulled in with `default-features = false`.

use crate::google::protobuf::Timestamp;
use crate::timestamp_ext::{TimestampError, NANOS_MAX};

#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
impl<Tz: chrono::TimeZone> From<chrono::DateTime<Tz>> for Timestamp {
    /// Convert a [`chrono::DateTime`] in any time zone to a protobuf
    /// [`Timestamp`].
    ///
    /// Infallible. The instant is preserved — proto `Timestamp` is always
    /// UTC, so the offset is folded in (a `DateTime<FixedOffset>` of
    /// `2023-11-15T03:13:20+05:00` and a `DateTime<Utc>` of
    /// `2023-11-14T22:13:20Z` produce the same `Timestamp`).
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Timestamp;
    /// use chrono::{DateTime, Utc};
    ///
    /// let dt = DateTime::<Utc>::from_timestamp(1_700_000_000, 123_456_789).unwrap();
    /// let ts: Timestamp = dt.into();
    /// assert_eq!(ts.seconds, 1_700_000_000);
    /// assert_eq!(ts.nanos, 123_456_789);
    /// ```
    ///
    /// # Warning: proto JSON spec range
    ///
    /// `chrono::DateTime` supports years up to ~262143, while the proto
    /// JSON spec restricts `Timestamp` to years 0001–9999. A `DateTime`
    /// outside that range converts without error here, but the resulting
    /// `Timestamp` will fail JSON serialization (`json` feature).
    ///
    /// # Leap seconds
    ///
    /// A leap-second `DateTime` (constructed via
    /// `NaiveTime::from_hms_nano_opt(_, _, 59, 1_000_000_000)`) reports
    /// `timestamp_subsec_nanos()` of `1_000_000_000`, which exceeds the proto
    /// `Timestamp.nanos` upper bound of `999_999_999`. The conversion clamps
    /// the nanos field to `999_999_999` — the leap second collapses to the
    /// final representable nanosecond of the same POSIX second
    /// (`23:59:59.999999999`).
    fn from(dt: chrono::DateTime<Tz>) -> Self {
        Self {
            seconds: dt.timestamp(),
            // `timestamp_subsec_nanos` returns `[0, 999_999_999]` outside a
            // leap second, and `1_000_000_000` inside one. Clamping keeps the
            // proto invariant `nanos ∈ [0, 999_999_999]` intact.
            nanos: dt.timestamp_subsec_nanos().min(NANOS_MAX as u32) as i32,
            ..Default::default()
        }
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "chrono")))]
impl TryFrom<Timestamp> for chrono::DateTime<chrono::Utc> {
    type Error = TimestampError;

    /// Convert a protobuf [`Timestamp`] to a [`chrono::DateTime<Utc>`](chrono::DateTime).
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Timestamp;
    /// use chrono::{DateTime, Utc};
    ///
    /// let ts = Timestamp {
    ///     seconds: 1_700_000_000,
    ///     nanos: 0,
    ///     ..Default::default()
    /// };
    /// let dt: DateTime<Utc> = ts.try_into().unwrap();
    /// assert_eq!(dt.timestamp(), 1_700_000_000);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError::InvalidNanos`] if `nanos` is outside
    /// `[0, 999_999_999]`, or [`TimestampError::Overflow`] if the value is
    /// outside the range `chrono::DateTime<Utc>` can represent.
    fn try_from(ts: Timestamp) -> Result<Self, Self::Error> {
        if ts.nanos < 0 || ts.nanos > NANOS_MAX {
            return Err(TimestampError::InvalidNanos);
        }
        // MSRV: `i32::cast_unsigned` requires 1.87. The range check above
        // guarantees `nanos` is non-negative, so the `as` cast is value-preserving.
        #[allow(clippy::cast_sign_loss)]
        Self::from_timestamp(ts.seconds, ts.nanos as u32).ok_or(TimestampError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    #[test]
    fn datetime_post_epoch_roundtrip() {
        let dt = Utc.with_ymd_and_hms(2023, 11, 14, 22, 13, 20).unwrap()
            + chrono::TimeDelta::nanoseconds(123_456_789);
        let ts: Timestamp = dt.into();
        assert_eq!(ts.seconds, 1_700_000_000);
        assert_eq!(ts.nanos, 123_456_789);
        let back: DateTime<Utc> = ts.try_into().unwrap();
        assert_eq!(back, dt);
    }

    #[test]
    fn datetime_epoch_roundtrip() {
        let dt = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        let ts: Timestamp = dt.into();
        assert_eq!(ts.seconds, 0);
        assert_eq!(ts.nanos, 0);
        let back: DateTime<Utc> = ts.try_into().unwrap();
        assert_eq!(back, dt);
    }

    #[test]
    fn datetime_pre_epoch_roundtrip() {
        // 1.5 seconds before epoch. chrono normalises to (secs=-2, nanos=500_000_000)
        // internally, which matches the proto `Timestamp` convention exactly
        // (nanos always non-negative).
        let dt = DateTime::<Utc>::from_timestamp(-2, 500_000_000).unwrap();
        let ts: Timestamp = dt.into();
        assert_eq!(ts.seconds, -2);
        assert_eq!(ts.nanos, 500_000_000);
        let back: DateTime<Utc> = ts.try_into().unwrap();
        assert_eq!(back, dt);
    }

    #[test]
    fn datetime_fixed_offset_preserves_instant() {
        use chrono::FixedOffset;
        // 2023-11-15T03:13:20+05:00 is the same instant as 2023-11-14T22:13:20Z.
        let tz = FixedOffset::east_opt(5 * 3600).unwrap();
        let dt = tz.with_ymd_and_hms(2023, 11, 15, 3, 13, 20).unwrap();
        let ts: Timestamp = dt.into();
        assert_eq!(ts.seconds, 1_700_000_000);
        assert_eq!(ts.nanos, 0);

        let utc_equivalent = Utc.with_ymd_and_hms(2023, 11, 14, 22, 13, 20).unwrap();
        assert_eq!(ts, Timestamp::from(utc_equivalent));
    }

    #[test]
    fn nanos_upper_boundary_accepted() {
        let ts = Timestamp {
            seconds: 0,
            nanos: 999_999_999,
            ..Default::default()
        };
        let dt: DateTime<Utc> = ts.try_into().expect("upper boundary must convert");
        assert_eq!(dt.timestamp_subsec_nanos(), 999_999_999);
    }

    #[test]
    fn invalid_nanos_rejected() {
        let ts = Timestamp {
            seconds: 0,
            nanos: -1,
            ..Default::default()
        };
        let result: Result<DateTime<Utc>, _> = ts.try_into();
        assert_eq!(result, Err(TimestampError::InvalidNanos));

        let ts2 = Timestamp {
            seconds: 0,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let result2: Result<DateTime<Utc>, _> = ts2.try_into();
        assert_eq!(result2, Err(TimestampError::InvalidNanos));
    }

    #[test]
    fn out_of_range_seconds_is_overflow() {
        // `chrono::DateTime<Utc>` tops out around year 262143; i64::MAX seconds
        // is far beyond that.
        let ts = Timestamp {
            seconds: i64::MAX,
            nanos: 0,
            ..Default::default()
        };
        let result: Result<DateTime<Utc>, _> = ts.try_into();
        assert_eq!(result, Err(TimestampError::Overflow));
    }

    #[test]
    fn i64_min_seconds_is_overflow_not_panic() {
        let ts = Timestamp {
            seconds: i64::MIN,
            nanos: 0,
            ..Default::default()
        };
        let result: Result<DateTime<Utc>, _> = ts.try_into();
        assert_eq!(result, Err(TimestampError::Overflow));
    }

    #[test]
    fn leap_second_datetime_clamps_nanos() {
        // A leap-second NaiveDateTime returns `timestamp_subsec_nanos() == 1_000_000_000`,
        // which is outside the proto `nanos ∈ [0, 999_999_999]` invariant. The
        // `From` impl clamps to `999_999_999` so the resulting `Timestamp` stays
        // valid (and round-trips via `TryFrom`).
        use chrono::NaiveDate;
        let leap = NaiveDate::from_ymd_opt(2016, 12, 31)
            .unwrap()
            .and_hms_nano_opt(23, 59, 59, 1_000_000_000)
            .expect("leap-second construction");
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(leap, Utc);
        assert_eq!(dt.timestamp_subsec_nanos(), 1_000_000_000);

        let ts: Timestamp = dt.into();
        assert!(
            (0..=999_999_999).contains(&ts.nanos),
            "nanos must stay within proto invariant: got {}",
            ts.nanos
        );
        assert_eq!(ts.nanos, 999_999_999);
        // Reverse direction must succeed.
        let _: DateTime<Utc> = ts.try_into().expect("clamped Timestamp must convert");
    }
}
