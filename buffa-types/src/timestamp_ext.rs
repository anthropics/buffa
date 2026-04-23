//! Ergonomic helpers for [`google::protobuf::Timestamp`](crate::google::protobuf::Timestamp).

use crate::google::protobuf::Timestamp;

/// Errors that can occur when converting a [`Timestamp`] to a Rust time type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TimestampError {
    /// The nanoseconds field is outside the valid range `[0, 999_999_999]`.
    #[error("nanos field must be in [0, 999_999_999]")]
    InvalidNanos,
    /// The timestamp is too far in the past or future for the target type.
    #[error("timestamp is out of range for SystemTime")]
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
            (0..=999_999_999).contains(&nanos),
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
        if (0..=999_999_999).contains(&nanos) {
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
        if ts.nanos < 0 || ts.nanos > 999_999_999 {
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

/// Convert unix-epoch days to a proleptic Gregorian (year, month, day).
/// Uses Howard Hinnant's civil calendar algorithm.
#[cfg(feature = "json")]
fn days_to_date(days: i64) -> (i64, u8, u8) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        yoe + era * 400 + if m <= 2 { 1 } else { 0 },
        m as u8,
        d as u8,
    )
}

/// Convert a proleptic Gregorian date to unix-epoch days.
///
/// Returns `None` if the date components are out of range or do not form a
/// valid calendar date (e.g. February 30 or June 31).  Validity is checked by
/// a round-trip through [`days_to_date`]: if the computed day number maps back
/// to a different date the input was not a real calendar date.
#[cfg(feature = "json")]
fn date_to_days(y: i64, m: u8, d: u8) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let (ya, ma) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = (if ya >= 0 { ya } else { ya - 399 }) / 400;
    let yoe = ya - era * 400;
    let doy = (153 * ma as i64 + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    // Verify the computed day number round-trips back to the same date.
    // This rejects dates like "Feb 30" that the formula maps to a different day.
    if days_to_date(days) != (y, m, d) {
        return None;
    }
    Some(days)
}

/// Format a unix timestamp as an RFC 3339 string (UTC, Z suffix).
/// Nanosecond precision is auto-detected (0, 3, 6, or 9 fractional digits).
#[cfg(feature = "json")]
fn timestamp_to_rfc3339(secs: i64, nanos: i32) -> alloc::string::String {
    use alloc::format;
    use alloc::string::String;
    let (tod, day) = {
        let r = secs % 86400;
        if r >= 0 {
            (r, secs / 86400)
        } else {
            (r + 86400, secs / 86400 - 1)
        }
    };
    let (y, mo, d) = days_to_date(day);
    let h = tod / 3600;
    let mi = (tod % 3600) / 60;
    let s = tod % 60;
    let frac = if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{:09}", nanos)
    };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{frac}Z")
}

/// Parse an RFC 3339 string to (unix_seconds, nanos). Accepts uppercase `Z`
/// suffix and UTC offsets (`+HH:MM` / `-HH:MM`). Lowercase `t` and `z`
/// are rejected per the proto3 JSON spec.
#[cfg(feature = "json")]
fn parse_rfc3339(s: &str) -> Option<(i64, i32)> {
    // RFC 3339 timestamps are pure ASCII. Reject non-ASCII early to avoid
    // panics from byte-offset string slicing on multi-byte UTF-8 input.
    if !s.is_ascii() {
        return None;
    }
    // Proto3 JSON spec requires uppercase 'Z' suffix (not lowercase).
    let (dt, tz_offset) = if let Some(rest) = s.strip_suffix('Z') {
        (rest, 0i64)
    } else {
        let len = s.len();
        if len < 6 {
            return None;
        }
        let sign: i64 = match s.as_bytes()[len - 6] {
            b'+' => -1,
            b'-' => 1,
            _ => return None,
        };
        // Offset must be `(+|-)HH:MM` with colon separator and valid ranges.
        if s.as_bytes()[len - 3] != b':' {
            return None;
        }
        let oh: i64 = s[len - 5..len - 3].parse().ok()?;
        let om: i64 = s[len - 2..].parse().ok()?;
        if !(0..=23).contains(&oh) || !(0..=59).contains(&om) {
            return None;
        }
        (&s[..len - 6], sign * (oh * 3600 + om * 60))
    };

    // Proto3 JSON spec requires uppercase 'T' separator (not lowercase).
    let t = dt.find('T')?;
    let (date, time) = (&dt[..t], &dt[t + 1..]);
    if date.len() != 10 || time.len() < 8 {
        return None;
    }

    // Validate structural separators (hyphens in date, colons in time).
    let date_b = date.as_bytes();
    let time_b = time.as_bytes();
    if date_b[4] != b'-' || date_b[7] != b'-' || time_b[2] != b':' || time_b[5] != b':' {
        return None;
    }

    let year: i64 = date[0..4].parse().ok()?;
    let month: u8 = date[5..7].parse().ok()?;
    let day: u8 = date[8..10].parse().ok()?;
    let hour: i64 = time[0..2].parse().ok()?;
    let min: i64 = time[3..5].parse().ok()?;
    let sec: i64 = time[6..8].parse().ok()?;
    // RFC 3339: hour 0-23, minute 0-59, second 0-60 (leap seconds).
    // Proto3 JSON spec inherits RFC 3339 but Timestamp uses Unix epoch
    // seconds which has no leap-second representation, so reject 60.
    if !(0..=23).contains(&hour) || !(0..=59).contains(&min) || !(0..=59).contains(&sec) {
        return None;
    }

    let nanos = if time.len() > 8 {
        if time.as_bytes()[8] != b'.' {
            return None;
        }
        let frac = &time[9..];
        // All chars must be digits (i32::parse accepts '-' and '+', which
        // would let e.g. "T23:59:59.-3Z" produce negative nanos).
        if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: i32 = frac.parse().ok()?;
        n * 10_i32.pow(9 - frac.len() as u32)
    } else {
        0
    };

    // Proto spec: year must be in [1, 9999]. Check BEFORE applying the
    // offset (cheap reject for most bad input)...
    if !(1..=9999).contains(&year) {
        return None;
    }
    let days = date_to_days(year, month, day)?;
    let unix = days * 86400 + hour * 3600 + min * 60 + sec + tz_offset;
    // ...and AFTER, because the offset can push a boundary timestamp past the
    // valid range (e.g. "9999-12-31T23:59:59-23:59" has year 9999 but the
    // UTC-equivalent is year 10000).
    if !(MIN_TIMESTAMP_SECS..=MAX_TIMESTAMP_SECS).contains(&unix) {
        return None;
    }
    Some((unix, nanos))
}

// ── serde impls ──────────────────────────────────────────────────────────────

// Protobuf spec: Timestamp is restricted to years 0001–9999.
#[cfg(feature = "json")]
const MIN_TIMESTAMP_SECS: i64 = -62_135_596_800; // 0001-01-01T00:00:00Z
#[cfg(feature = "json")]
const MAX_TIMESTAMP_SECS: i64 = 253_402_300_799; // 9999-12-31T23:59:59Z

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
        if !(0..=999_999_999).contains(&self.nanos) {
            return Err(serde::ser::Error::custom(format!(
                "invalid Timestamp: nanos {} is outside [0, 999_999_999]",
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
