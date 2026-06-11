//! Ergonomic helpers for [`google::protobuf::Timestamp`](crate::google::protobuf::Timestamp).

use crate::google::protobuf::Timestamp;

/// Maximum value of the `nanos` field per the protobuf `Timestamp` spec.
///
/// `Timestamp.nanos` is constrained to `[0, NANOS_MAX]`. Shared with
/// `timestamp_chrono` so the validation range cannot drift between files.
pub(crate) const NANOS_MAX: i32 = 999_999_999;

/// Errors that can occur when converting a [`Timestamp`] to a Rust time type.
///
/// Deliberately shared by the `std` conversion (`Timestamp` →
/// `std::time::SystemTime`) and the `chrono` conversion (`Timestamp` →
/// `chrono::DateTime<Utc>`): the failure modes map identically for both
/// targets, so a separate error enum per target would add API surface
/// without adding information.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimestampError {
    /// The nanoseconds field is outside the valid range `[0, 999_999_999]`.
    #[error("nanos field must be in [0, 999_999_999]")]
    InvalidNanos,
    /// The timestamp is too far in the past or future for the target type.
    #[error("timestamp is out of range for the target type")]
    Overflow,
}

impl Timestamp {
    /// Create a [`Timestamp`] from a Unix epoch offset.
    ///
    /// `seconds` is the number of seconds since (or before, if negative) the
    /// Unix epoch.  `nanos` must be in `[0, 999_999_999]`.
    ///
    /// # Panics
    ///
    /// Panics in debug mode if `nanos` is outside `[0, 999_999_999]`.
    /// In release mode the value is stored as-is, producing an invalid
    /// timestamp.  Use [`Timestamp::from_unix_checked`] for a checked
    /// variant that returns `None` on invalid input.
    pub fn from_unix(seconds: i64, nanos: i32) -> Self {
        debug_assert!(
            (0..=NANOS_MAX).contains(&nanos),
            "nanos ({nanos}) must be in [0, 999_999_999]"
        );
        Self {
            seconds,
            nanos,
            ..Default::default()
        }
    }

    /// Create a [`Timestamp`] from a whole number of Unix seconds (nanoseconds = 0).
    ///
    /// This is a convenience shorthand for `Timestamp::from_unix(seconds, 0)`.
    pub fn from_unix_secs(seconds: i64) -> Self {
        Self {
            seconds,
            nanos: 0,
            ..Default::default()
        }
    }

    /// Create a [`Timestamp`] from a Unix epoch offset, returning `None` if
    /// `nanos` is outside `[0, 999_999_999]`.
    pub fn from_unix_checked(seconds: i64, nanos: i32) -> Option<Self> {
        if (0..=NANOS_MAX).contains(&nanos) {
            Some(Self {
                seconds,
                nanos,
                ..Default::default()
            })
        } else {
            None
        }
    }

    /// Return the current wall-clock time as a [`Timestamp`].
    ///
    /// Requires the `std` feature.
    #[cfg(feature = "std")]
    pub fn now() -> Self {
        std::time::SystemTime::now().into()
    }
}

#[cfg(feature = "std")]
impl TryFrom<Timestamp> for std::time::SystemTime {
    type Error = TimestampError;

    /// Convert a protobuf [`Timestamp`] to a [`std::time::SystemTime`].
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError::InvalidNanos`] if `nanos` is outside
    /// `[0, 999_999_999]`, or [`TimestampError::Overflow`] if the result
    /// does not fit in a [`std::time::SystemTime`].
    fn try_from(ts: Timestamp) -> Result<Self, Self::Error> {
        if ts.nanos < 0 || ts.nanos > NANOS_MAX {
            return Err(TimestampError::InvalidNanos);
        }

        if ts.seconds >= 0 {
            let offset = std::time::Duration::new(ts.seconds as u64, ts.nanos as u32);
            std::time::UNIX_EPOCH
                .checked_add(offset)
                .ok_or(TimestampError::Overflow)
        } else {
            // ts.seconds is negative: move backward from epoch, then forward by nanos.
            //
            // For example, ts.seconds = -2, ts.nanos = 500_000_000 represents
            // -1.5 seconds from epoch (i.e. 1.5 s before epoch):
            //   result = UNIX_EPOCH - 2s + 0.5s = UNIX_EPOCH - 1.5s
            //
            // unsigned_abs() avoids the overflow that `(-ts.seconds) as u64` would
            // cause when ts.seconds == i64::MIN (which cannot be negated in i64).
            let neg_secs = ts.seconds.unsigned_abs();
            let base = std::time::UNIX_EPOCH
                .checked_sub(std::time::Duration::from_secs(neg_secs))
                .ok_or(TimestampError::Overflow)?;
            if ts.nanos == 0 {
                Ok(base)
            } else {
                base.checked_add(std::time::Duration::from_nanos(ts.nanos as u64))
                    .ok_or(TimestampError::Overflow)
            }
        }
    }
}

#[cfg(feature = "std")]
impl From<std::time::SystemTime> for Timestamp {
    /// Convert a [`std::time::SystemTime`] to a protobuf [`Timestamp`].
    ///
    /// Pre-epoch times (where `t < UNIX_EPOCH`) are represented with a
    /// negative `seconds` field and a non-negative `nanos` field, following
    /// the protobuf convention that `nanos` is always in `[0, 999_999_999]`.
    ///
    /// # Saturation
    ///
    /// Times more than ~292 billion years from the epoch (beyond `i64::MAX`
    /// seconds) are saturated to `i64::MAX` seconds rather than wrapping,
    /// which would produce a semantically incorrect negative timestamp.
    fn from(t: std::time::SystemTime) -> Self {
        match t.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => Self {
                // Saturate at i64::MAX to avoid wrapping for times far in the future.
                seconds: d.as_secs().min(i64::MAX as u64) as i64,
                nanos: d.subsec_nanos() as i32,
                ..Default::default()
            },
            Err(e) => {
                // `e.duration()` is how far `t` is *before* the epoch.
                // We need: seconds = floor(t - epoch), nanos = (t - epoch) - seconds.
                //
                // Example: t is 1.5s before epoch → duration = 1.5s
                //   floor = -2 (the largest integer ≤ -1.5)
                //   nanos = -1.5 - (-2) = 0.5s = 500_000_000 ns
                //
                // In terms of the subtraction duration `dur = e.duration()`:
                //   If dur.subsec_nanos() == 0:
                //     seconds = -(dur.as_secs() as i64), nanos = 0
                //   Else:
                //     seconds = -(dur.as_secs() as i64 + 1)
                //     nanos = 1_000_000_000 - dur.subsec_nanos()
                //
                // Saturate at i64::MAX to avoid wrapping for extreme pre-epoch times.
                let dur = e.duration();
                if dur.subsec_nanos() == 0 {
                    let secs = dur.as_secs().min(i64::MAX as u64) as i64;
                    Self {
                        seconds: -secs,
                        nanos: 0,
                        ..Default::default()
                    }
                } else {
                    // saturating_add avoids overflow when dur.as_secs() == u64::MAX,
                    // then clamp to i64::MAX before converting.
                    let neg_secs = dur.as_secs().saturating_add(1).min(i64::MAX as u64) as i64;
                    Self {
                        seconds: -neg_secs,
                        nanos: (1_000_000_000u32 - dur.subsec_nanos()) as i32,
                        ..Default::default()
                    }
                }
            }
        }
    }
}

// ── RFC 3339 formatting ──────────────────────────────────────────────────────
//
// The shared formatting and parsing primitives live in
// `buffa::json_helpers::wkt`. Both this typed serde impl and `buffa-descriptor`'s
// reflective JSON codec call into the same code, so the two paths can't drift
// on edge cases the conformance suite exercises. The functions below are thin
// adapters that preserve the `Option`-returning private API the test suite
// targets.

#[cfg(feature = "json")]
use buffa::json_helpers::wkt::{MAX_TIMESTAMP_SECS, MIN_TIMESTAMP_SECS};
// The civil-calendar helpers are exercised directly by the test module.
#[cfg(all(test, feature = "json"))]
use buffa::json_helpers::wkt::{date_to_days, days_to_date};

#[cfg(feature = "json")]
fn timestamp_to_rfc3339(secs: i64, nanos: i32) -> alloc::string::String {
    // The serde `Serialize` impl validates `seconds` and `nanos` bounds
    // before calling this; `expect` documents the invariant.
    buffa::json_helpers::wkt::fmt_timestamp(secs, nanos)
        .expect("Timestamp validated before formatting")
}

#[cfg(feature = "json")]
fn parse_rfc3339(s: &str) -> Option<(i64, i32)> {
    buffa::json_helpers::wkt::parse_timestamp(s).ok()
}

// ── serde impls ──────────────────────────────────────────────────────────────

#[cfg(feature = "json")]
impl serde::Serialize for Timestamp {
    /// Serializes as an RFC 3339 string (e.g. `"2021-01-01T00:00:00Z"`).
    ///
    /// # Errors
    ///
    /// Returns a serialization error if `nanos` is outside `[0, 999_999_999]`
    /// or if `seconds` is outside the proto spec range (years 0001–9999).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use alloc::format;
        if !(0..=NANOS_MAX).contains(&self.nanos) {
            return Err(serde::ser::Error::custom(format!(
                "invalid Timestamp: nanos {} is outside [0, {NANOS_MAX}]",
                self.nanos
            )));
        }
        if !(MIN_TIMESTAMP_SECS..=MAX_TIMESTAMP_SECS).contains(&self.seconds) {
            return Err(serde::ser::Error::custom(format!(
                "invalid Timestamp: seconds {} is outside [{}, {}]",
                self.seconds, MIN_TIMESTAMP_SECS, MAX_TIMESTAMP_SECS
            )));
        }
        s.serialize_str(&timestamp_to_rfc3339(self.seconds, self.nanos))
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Timestamp {
    /// Deserializes from an RFC 3339 string.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use alloc::{format, string::String};
        let s: String = serde::Deserialize::deserialize(d)?;
        let (secs, nanos) = parse_rfc3339(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid RFC 3339 timestamp: {s}")))?;
        Ok(Self {
            seconds: secs,
            nanos,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_unix_secs_sets_nanos_to_zero() {
        let ts = Timestamp::from_unix_secs(1_700_000_000);
        assert_eq!(ts.seconds, 1_700_000_000);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_secs_zero() {
        let ts = Timestamp::from_unix_secs(0);
        assert_eq!(ts.seconds, 0);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_secs_negative() {
        let ts = Timestamp::from_unix_secs(-1);
        assert_eq!(ts.seconds, -1);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_secs_i64_min() {
        let ts = Timestamp::from_unix_secs(i64::MIN);
        assert_eq!(ts.seconds, i64::MIN);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_secs_i64_max() {
        let ts = Timestamp::from_unix_secs(i64::MAX);
        assert_eq!(ts.seconds, i64::MAX);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_basic() {
        let ts = Timestamp::from_unix(1_000_000_000, 500_000_000);
        assert_eq!(ts.seconds, 1_000_000_000);
        assert_eq!(ts.nanos, 500_000_000);
    }

    #[test]
    fn from_unix_zero() {
        let ts = Timestamp::from_unix(0, 0);
        assert_eq!(ts.seconds, 0);
        assert_eq!(ts.nanos, 0);
    }

    #[test]
    fn from_unix_checked_valid() {
        assert!(Timestamp::from_unix_checked(0, 0).is_some());
        assert!(Timestamp::from_unix_checked(-100, 999_999_999).is_some());
    }

    #[test]
    fn from_unix_checked_invalid_nanos() {
        assert!(Timestamp::from_unix_checked(0, -1).is_none());
        assert!(Timestamp::from_unix_checked(0, 1_000_000_000).is_none());
    }

    #[cfg(feature = "std")]
    #[test]
    fn systemtime_roundtrip_post_epoch() {
        let ts = Timestamp::from_unix(1_700_000_000, 123_456_789);
        let st: std::time::SystemTime = ts.clone().try_into().unwrap();
        let ts2: Timestamp = st.into();
        assert_eq!(ts, ts2);
    }

    #[cfg(feature = "std")]
    #[test]
    fn systemtime_roundtrip_pre_epoch() {
        // -1.5 seconds before epoch: seconds = -2, nanos = 500_000_000
        let ts = Timestamp::from_unix(-2, 500_000_000);
        let st: std::time::SystemTime = ts.clone().try_into().unwrap();
        let ts2: Timestamp = st.into();
        assert_eq!(ts, ts2);
    }

    #[cfg(feature = "std")]
    #[test]
    fn systemtime_roundtrip_exact_pre_epoch() {
        // Exactly 2 seconds before epoch.
        let ts = Timestamp::from_unix(-2, 0);
        let st: std::time::SystemTime = ts.clone().try_into().unwrap();
        let ts2: Timestamp = st.into();
        assert_eq!(ts, ts2);
    }

    #[cfg(feature = "std")]
    #[test]
    fn systemtime_roundtrip_epoch() {
        let ts = Timestamp::from_unix(0, 0);
        let st: std::time::SystemTime = ts.clone().try_into().unwrap();
        let ts2: Timestamp = st.into();
        assert_eq!(ts, ts2);
    }

    #[cfg(feature = "std")]
    #[test]
    fn invalid_nanos_rejected() {
        let ts = Timestamp {
            seconds: 0,
            nanos: -1,
            ..Default::default()
        };
        let result: Result<std::time::SystemTime, _> = ts.try_into();
        assert_eq!(result, Err(TimestampError::InvalidNanos));

        let ts2 = Timestamp {
            seconds: 0,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let result2: Result<std::time::SystemTime, _> = ts2.try_into();
        assert_eq!(result2, Err(TimestampError::InvalidNanos));
    }

    #[cfg(feature = "std")]
    #[test]
    fn i64_min_seconds_does_not_panic() {
        // i64::MIN cannot be negated in i64; unsigned_abs() must be used.
        let ts = Timestamp {
            seconds: i64::MIN,
            nanos: 0,
            ..Default::default()
        };
        // The conversion should either succeed or return Overflow, never panic.
        let _: Result<std::time::SystemTime, _> = ts.try_into();
    }

    #[cfg(feature = "std")]
    #[test]
    fn now_is_positive() {
        let ts = Timestamp::now();
        assert!(ts.seconds > 0, "current time should be after Unix epoch");
    }

    #[test]
    fn timestamp_view_round_trip() {
        use crate::google::protobuf::__buffa::view::TimestampView;
        use crate::google::protobuf::Timestamp;
        use buffa::{Message, MessageView};

        let ts = Timestamp {
            seconds: 1_700_000_000,
            nanos: 123_456_789,
            ..Default::default()
        };
        let bytes = ts.encode_to_vec();
        let view = TimestampView::decode_view(&bytes).expect("decode_view");
        assert_eq!(view.seconds, ts.seconds);
        assert_eq!(view.nanos, ts.nanos);

        let owned = view.to_owned_message();
        assert_eq!(owned, ts);
    }

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        // ---- RFC 3339 helper unit tests -----------------------------------

        #[test]
        fn days_to_date_epoch() {
            assert_eq!(days_to_date(0), (1970, 1, 1));
        }

        #[test]
        fn days_to_date_known_date() {
            // 2021-01-01: days since epoch = 18628
            assert_eq!(days_to_date(18628), (2021, 1, 1));
        }

        #[test]
        fn date_to_days_roundtrip() {
            let (y, m, d) = days_to_date(18628);
            assert_eq!(date_to_days(y, m, d), Some(18628));
        }

        #[test]
        fn date_to_days_invalid_month() {
            assert_eq!(date_to_days(2021, 13, 1), None);
            assert_eq!(date_to_days(2021, 0, 1), None);
        }

        #[test]
        fn rfc3339_epoch() {
            assert_eq!(timestamp_to_rfc3339(0, 0), "1970-01-01T00:00:00Z");
        }

        #[test]
        fn rfc3339_half_second() {
            assert_eq!(
                timestamp_to_rfc3339(0, 500_000_000),
                "1970-01-01T00:00:00.500Z"
            );
        }

        #[test]
        fn rfc3339_one_nanosecond() {
            assert_eq!(timestamp_to_rfc3339(0, 1), "1970-01-01T00:00:00.000000001Z");
        }

        #[test]
        fn parse_epoch() {
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some((0, 0)));
        }

        #[test]
        fn parse_with_fractional_seconds() {
            assert_eq!(
                parse_rfc3339("1970-01-01T00:00:00.5Z"),
                Some((0, 500_000_000))
            );
        }

        #[test]
        fn parse_with_positive_offset() {
            // +05:00 means local is 5h ahead, so UTC = local - 5h
            assert_eq!(parse_rfc3339("1970-01-01T05:00:00+05:00"), Some((0, 0)));
        }

        #[test]
        fn parse_invalid() {
            assert_eq!(parse_rfc3339("not-a-date"), None);
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00"), None); // missing tz
        }

        // ---- serde roundtrips ---------------------------------------------

        #[test]
        fn timestamp_epoch_roundtrip() {
            let ts = Timestamp::from_unix(0, 0);
            let json = serde_json::to_string(&ts).unwrap();
            assert_eq!(json, r#""1970-01-01T00:00:00Z""#);
            let back: Timestamp = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, 0);
            assert_eq!(back.nanos, 0);
        }

        #[test]
        fn timestamp_with_nanos_roundtrip() {
            let ts = Timestamp::from_unix(1_000_000_000, 500_000_000);
            let json = serde_json::to_string(&ts).unwrap();
            let back: Timestamp = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, ts.seconds);
            assert_eq!(back.nanos, ts.nanos);
        }

        #[test]
        fn timestamp_pre_epoch_roundtrip() {
            // -1.5 seconds before epoch: seconds = -2, nanos = 500_000_000
            let ts = Timestamp::from_unix(-2, 500_000_000);
            let json = serde_json::to_string(&ts).unwrap();
            let back: Timestamp = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, ts.seconds);
            assert_eq!(back.nanos, ts.nanos);
        }

        #[test]
        fn timestamp_invalid_string_is_error() {
            let result: Result<Timestamp, _> = serde_json::from_str(r#""not-a-date""#);
            assert!(result.is_err());
        }

        #[test]
        fn timestamp_invalid_nanos_is_serialize_error() {
            let ts = Timestamp {
                seconds: 0,
                nanos: -1,
                ..Default::default()
            };
            let result = serde_json::to_string(&ts);
            assert!(result.is_err(), "negative nanos must fail serialization");
        }

        #[test]
        fn parse_lowercase_separators_rejected() {
            // Proto3 JSON spec requires uppercase 'T' and 'Z'.
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00z"), None);
            assert_eq!(parse_rfc3339("1970-01-01t00:00:00Z"), None);
            assert_eq!(parse_rfc3339("1970-01-01t00:00:00z"), None);
        }

        #[test]
        fn parse_date_to_days_rejects_feb_30() {
            // "Feb 30" is not a real date; parse_rfc3339 must return None.
            assert_eq!(parse_rfc3339("2021-02-30T00:00:00Z"), None);
        }

        #[test]
        fn parse_time_component_range_rejected() {
            // Hour, minute, second must be in valid ranges.
            assert_eq!(parse_rfc3339("2021-01-01T24:00:00Z"), None, "hour 24");
            assert_eq!(parse_rfc3339("2021-01-01T25:00:00Z"), None, "hour 25");
            assert_eq!(parse_rfc3339("2021-01-01T00:60:00Z"), None, "min 60");
            assert_eq!(parse_rfc3339("2021-01-01T00:99:00Z"), None, "min 99");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:60Z"), None, "sec 60 (leap)");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:99Z"), None, "sec 99");
            // Valid boundaries.
            assert!(parse_rfc3339("2021-01-01T23:59:59Z").is_some());
            assert!(parse_rfc3339("2021-01-01T00:00:00Z").is_some());
        }

        #[test]
        fn parse_offset_range_rejected() {
            assert_eq!(parse_rfc3339("2021-01-01T00:00:00+24:00"), None, "oh 24");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:00+99:00"), None, "oh 99");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:00+00:60"), None, "om 60");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:00+99:99"), None, "both");
            // Valid boundaries.
            assert!(parse_rfc3339("2021-01-01T00:00:00+23:59").is_some());
            assert!(parse_rfc3339("2021-01-01T00:00:00-23:59").is_some());
        }

        #[test]
        fn parse_separator_chars_rejected() {
            // Hyphens in date, colons in time, colon in offset are required.
            assert_eq!(parse_rfc3339("2021X01-01T00:00:00Z"), None, "date[4]");
            assert_eq!(parse_rfc3339("2021-01X01T00:00:00Z"), None, "date[7]");
            assert_eq!(parse_rfc3339("2021-01-01T00X00:00Z"), None, "time[2]");
            assert_eq!(parse_rfc3339("2021-01-01T00:00X00Z"), None, "time[5]");
            assert_eq!(parse_rfc3339("2021-01-01T00:00:00+05X30"), None, "off");
            // All separators wrong at once.
            assert_eq!(parse_rfc3339("2021X01X01T00X00X00Z"), None);
        }

        #[test]
        fn parse_fractional_seconds_rejects_non_digits() {
            // Regression (fuzzer-found): i32::parse accepts '-' and '+',
            // which previously allowed "T23:59:59.-3Z" → nanos = -30_000_000.
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00.-3Z"), None, "minus");
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00.+3Z"), None, "plus");
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00.3aZ"), None, "alpha");
            assert_eq!(parse_rfc3339("1970-01-01T00:00:00. Z"), None, "space");
            // Edge: 9999-12-31T23:59:59.-3Z — the fuzzer's original crash input.
            assert_eq!(parse_rfc3339("9999-12-31T23:59:59.-3Z"), None);
            // Valid digits still work.
            assert_eq!(
                parse_rfc3339("1970-01-01T00:00:00.5Z"),
                Some((0, 500_000_000))
            );
            assert_eq!(
                parse_rfc3339("1970-01-01T00:00:00.000000001Z"),
                Some((0, 1))
            );
        }

        #[test]
        fn parse_offset_pushes_past_boundary_rejected() {
            // Year is 9999 (passes pre-offset check), but -23:59 offset means
            // UTC is in year 10000 — must be rejected per proto Timestamp range.
            assert_eq!(parse_rfc3339("9999-12-31T23:59:59-23:59"), None);
            // Year is 0001 (passes), but +23:59 offset means UTC is in year 0.
            assert_eq!(parse_rfc3339("0001-01-01T00:00:00+23:59"), None);
            // Boundary values that just fit are OK.
            assert_eq!(
                parse_rfc3339("9999-12-31T23:59:59Z"),
                Some((MAX_TIMESTAMP_SECS, 0))
            );
            assert_eq!(
                parse_rfc3339("0001-01-01T00:00:00Z"),
                Some((MIN_TIMESTAMP_SECS, 0))
            );
        }
    }
}
