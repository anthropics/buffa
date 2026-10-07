//! `jiff` interop for [`google::protobuf::Timestamp`](crate::google::protobuf::Timestamp).
//!
//! Enabled with the `jiff` Cargo feature. `no_std`-compatible — `jiff` is
//! pulled in with `default-features = false` and its `alloc` feature.

use crate::google::protobuf::Timestamp;
use crate::timestamp_ext::{TimestampError, NANOS_MAX};

#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl From<jiff::Timestamp> for Timestamp {
    /// Convert a [`jiff::Timestamp`] to a protobuf [`Timestamp`].
    ///
    /// Infallible: every [`jiff::Timestamp`] fits the `Timestamp` fields.
    /// The protobuf `Timestamp` type is valid only for years 0001–9999,
    /// though, and jiff reaches back to about year -9999.
    ///
    /// # Warning: Timestamp valid range
    ///
    /// The protobuf `Timestamp` spec restricts valid values to years
    /// 0001–9999. A pre-year-1 instant still converts and round-trips through
    /// binary encoding, but the result is outside the valid `Timestamp` range
    /// and JSON serialization (`json` feature) rejects it.
    ///
    /// # Sign normalization
    ///
    /// [`jiff::Timestamp`] reports its sub-second component with the *same
    /// sign* as the overall instant — a pre-epoch instant has a negative
    /// [`subsec_nanosecond`](jiff::Timestamp::subsec_nanosecond) — whereas
    /// proto `Timestamp.nanos` is always in `[0, 999_999_999]`. The conversion
    /// re-normalizes by borrowing a second for negative sub-second components,
    /// so `-1.5s` becomes `{ seconds: -2, nanos: 500_000_000 }`.
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Timestamp;
    ///
    /// let jt = jiff::Timestamp::new(1_700_000_000, 123_456_789).unwrap();
    /// let ts: Timestamp = jt.into();
    /// assert_eq!(ts.seconds, 1_700_000_000);
    /// assert_eq!(ts.nanos, 123_456_789);
    /// ```
    fn from(ts: jiff::Timestamp) -> Self {
        let seconds = ts.as_second();
        let nanos = ts.subsec_nanosecond();
        if nanos < 0 {
            // `seconds` is >= jiff's MIN second (-377_705_023_201), so the
            // borrow `seconds - 1` cannot underflow i64.
            Self {
                seconds: seconds - 1,
                nanos: nanos + 1_000_000_000,
                ..Default::default()
            }
        } else {
            Self {
                seconds,
                nanos,
                ..Default::default()
            }
        }
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl TryFrom<Timestamp> for jiff::Timestamp {
    type Error = TimestampError;

    /// Convert a protobuf [`Timestamp`] to a [`jiff::Timestamp`].
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Timestamp;
    ///
    /// let ts = Timestamp {
    ///     seconds: 1_700_000_000,
    ///     nanos: 0,
    ///     ..Default::default()
    /// };
    /// let jt: jiff::Timestamp = ts.try_into().unwrap();
    /// assert_eq!(jt.as_second(), 1_700_000_000);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError::InvalidNanos`] if `nanos` is outside
    /// `[0, 999_999_999]`, or [`TimestampError::Overflow`] if the instant is
    /// outside [`jiff::Timestamp`]'s representable range. This conversion
    /// does not check the protobuf `Timestamp` range of years 0001–9999.
    fn try_from(ts: Timestamp) -> Result<Self, Self::Error> {
        Self::try_from(&ts)
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl TryFrom<&Timestamp> for jiff::Timestamp {
    type Error = TimestampError;

    /// Convert a borrowed protobuf [`Timestamp`] to a [`jiff::Timestamp`],
    /// leaving the message in place.
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Timestamp;
    ///
    /// let ts = Timestamp::from_unix_secs(1_700_000_000);
    /// let jt = jiff::Timestamp::try_from(&ts).unwrap();
    /// assert_eq!(jt.as_second(), 1_700_000_000);
    /// assert_eq!(ts.seconds, 1_700_000_000);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError::InvalidNanos`] if `nanos` is outside
    /// `[0, 999_999_999]`, or [`TimestampError::Overflow`] if the instant is
    /// outside [`jiff::Timestamp`]'s representable range. This conversion
    /// does not check the protobuf `Timestamp` range of years 0001–9999.
    fn try_from(ts: &Timestamp) -> Result<Self, Self::Error> {
        if ts.nanos < 0 || ts.nanos > NANOS_MAX {
            return Err(TimestampError::InvalidNanos);
        }
        // Nanos validated above, so the only remaining failure is an
        // out-of-range second.
        jiff::Timestamp::new(ts.seconds, ts.nanos).map_err(|_| TimestampError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_epoch_roundtrip() {
        let jt = jiff::Timestamp::new(1_700_000_000, 123_456_789).unwrap();
        let ts: Timestamp = jt.into();
        assert_eq!(ts.seconds, 1_700_000_000);
        assert_eq!(ts.nanos, 123_456_789);
        let back: jiff::Timestamp = ts.try_into().unwrap();
        assert_eq!(back, jt);
    }

    #[test]
    fn epoch_roundtrip() {
        let jt = jiff::Timestamp::new(0, 0).unwrap();
        let ts: Timestamp = jt.into();
        assert_eq!(ts.seconds, 0);
        assert_eq!(ts.nanos, 0);
        let back: jiff::Timestamp = ts.try_into().unwrap();
        assert_eq!(back, jt);
    }

    #[test]
    fn pre_epoch_borrows_second() {
        // -1.5 seconds. jiff stores this sign-consistently as
        // (as_second = -1, subsec_nanosecond = -500_000_000); the proto form
        // must borrow a second to keep nanos non-negative.
        let jt = jiff::Timestamp::new(-2, 500_000_000).unwrap();
        assert_eq!(jt.as_second(), -1);
        assert_eq!(jt.subsec_nanosecond(), -500_000_000);
        let ts: Timestamp = jt.into();
        assert_eq!(ts.seconds, -2);
        assert_eq!(ts.nanos, 500_000_000);
        let back: jiff::Timestamp = ts.try_into().unwrap();
        assert_eq!(back, jt);
    }

    #[test]
    fn exact_pre_epoch_second_roundtrip() {
        // Whole second before the epoch: no borrow needed.
        let jt = jiff::Timestamp::new(-2, 0).unwrap();
        let ts: Timestamp = jt.into();
        assert_eq!(ts.seconds, -2);
        assert_eq!(ts.nanos, 0);
        let back: jiff::Timestamp = ts.try_into().unwrap();
        assert_eq!(back, jt);
    }

    #[test]
    fn nanos_upper_boundary_roundtrip() {
        let ts = Timestamp {
            seconds: 5,
            nanos: 999_999_999,
            ..Default::default()
        };
        let jt: jiff::Timestamp = ts.clone().try_into().expect("upper boundary converts");
        let back: Timestamp = jt.into();
        assert_eq!(back, ts);
    }

    #[test]
    fn invalid_nanos_rejected() {
        let neg = Timestamp {
            seconds: 0,
            nanos: -1,
            ..Default::default()
        };
        let r: Result<jiff::Timestamp, _> = neg.try_into();
        assert_eq!(r, Err(TimestampError::InvalidNanos));

        let too_big = Timestamp {
            seconds: 0,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let r2: Result<jiff::Timestamp, _> = too_big.try_into();
        assert_eq!(r2, Err(TimestampError::InvalidNanos));
    }

    #[test]
    fn out_of_range_seconds_is_overflow() {
        // These i64 values are outside jiff's representable range.
        let huge = Timestamp {
            seconds: i64::MAX,
            nanos: 0,
            ..Default::default()
        };
        let r: Result<jiff::Timestamp, _> = huge.try_into();
        assert_eq!(r, Err(TimestampError::Overflow));

        let tiny = Timestamp {
            seconds: i64::MIN,
            nanos: 0,
            ..Default::default()
        };
        let r2: Result<jiff::Timestamp, _> = tiny.try_into();
        assert_eq!(r2, Err(TimestampError::Overflow));
    }

    #[test]
    fn jiff_extremes_roundtrip() {
        // Both ends of jiff's range survive the binary Timestamp conversion.
        for jt in [jiff::Timestamp::MIN, jiff::Timestamp::MAX] {
            let ts: Timestamp = jt.into();
            assert!(
                (0..=NANOS_MAX).contains(&ts.nanos),
                "nanos must stay within proto invariant: got {}",
                ts.nanos
            );
            let back: jiff::Timestamp = ts.try_into().expect("jiff extreme must convert back");
            assert_eq!(back, jt);
        }
    }

    #[test]
    fn borrow_at_near_min_roundtrip() {
        // The subtlest borrow case: one nanosecond short of jiff's MIN second.
        // jiff reports it as (MIN + 1, -999_999_999); the borrow produces proto
        // { seconds: MIN, nanos: 1 }, and the conversion back must accept a
        // positive nanos at MIN (jiff permits it — only negative nanos at MIN
        // are out of range).
        let min_second = jiff::Timestamp::MIN.as_second();
        let jt = jiff::Timestamp::new(min_second + 1, -999_999_999).unwrap();
        let ts: Timestamp = jt.into();
        assert_eq!(ts.seconds, min_second);
        assert_eq!(ts.nanos, 1);
        let back: jiff::Timestamp = ts.try_into().expect("near-MIN borrow must convert back");
        assert_eq!(back, jt);
    }

    #[cfg(feature = "json")]
    #[test]
    fn pre_year_one_conversion_is_rejected_by_json() {
        let first_valid = jiff::Timestamp::new(-62_135_596_800, 0).unwrap();
        let before_first_valid = jiff::Timestamp::new(-62_135_596_801, 0).unwrap();

        let valid: Timestamp = first_valid.into();
        let invalid: Timestamp = before_first_valid.into();

        assert!(serde_json::to_string(&valid).is_ok());
        assert!(serde_json::to_string(&invalid).is_err());
    }
}
