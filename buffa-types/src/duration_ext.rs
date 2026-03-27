//! Ergonomic helpers for [`google::protobuf::Duration`](crate::google::protobuf::Duration).

use crate::google::protobuf::Duration;

/// Errors that can occur when converting a protobuf [`Duration`] to a Rust type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DurationError {
    /// The duration is negative and cannot be represented as [`std::time::Duration`].
    #[error("negative protobuf Duration cannot be converted to std::time::Duration")]
    NegativeDuration,
    /// The `nanos` field is outside its valid range, or its sign is inconsistent
    /// with the `seconds` field.
    ///
    /// Per the protobuf spec, `nanos` must be in `[-999_999_999, 999_999_999]`
    /// and must have the same sign (or be zero) as `seconds`.
    #[error("nanos field has invalid value or sign mismatch with seconds")]
    InvalidNanos,
}

#[cfg(feature = "std")]
impl TryFrom<Duration> for std::time::Duration {
    type Error = DurationError;

    /// Convert a protobuf [`Duration`] to a [`std::time::Duration`].
    ///
    /// # Errors
    ///
    /// Returns [`DurationError::InvalidNanos`] if `nanos` is outside
    /// `[-999_999_999, 999_999_999]` or if its sign is inconsistent with
    /// `seconds` (e.g. positive nanos with negative seconds).
    ///
    /// Returns [`DurationError::NegativeDuration`] if the duration is
    /// negative but otherwise well-formed (e.g. `seconds < 0`, `nanos ≤ 0`),
    /// since [`std::time::Duration`] cannot represent negative values.
    fn try_from(d: Duration) -> Result<Self, Self::Error> {
        // Protobuf spec: nanos ∈ [-999_999_999, 999_999_999].
        // Use a range check rather than .abs() to avoid overflow on i32::MIN.
        if !(-999_999_999..=999_999_999).contains(&d.nanos) {
            return Err(DurationError::InvalidNanos);
        }
        // Protobuf spec: nanos sign must match seconds sign (or nanos is zero).
        let sign_mismatch = (d.seconds > 0 && d.nanos < 0) || (d.seconds < 0 && d.nanos > 0);
        if sign_mismatch {
            return Err(DurationError::InvalidNanos);
        }
        // std::time::Duration is unsigned; reject well-formed negative durations.
        if d.seconds < 0 || d.nanos < 0 {
            return Err(DurationError::NegativeDuration);
        }
        Ok(Self::new(d.seconds as u64, d.nanos as u32))
    }
}

#[cfg(feature = "std")]
impl From<std::time::Duration> for Duration {
    /// Convert a [`std::time::Duration`] to a protobuf [`Duration`].
    ///
    /// # Saturation
    ///
    /// Durations whose `as_secs()` exceeds `i64::MAX` (~292 billion years) are
    /// saturated to `i64::MAX` seconds rather than wrapping, which would produce
    /// an incorrect negative value.
    fn from(d: std::time::Duration) -> Self {
        Self {
            // Saturate at i64::MAX rather than wrapping for extremely large durations.
            seconds: d.as_secs().min(i64::MAX as u64) as i64,
            nanos: d.subsec_nanos() as i32,
            ..Default::default()
        }
    }
}

// ── RFC 3339-style decimal-seconds formatting ─────────────────────────────────

/// Format a protobuf Duration as a decimal seconds string with an `s` suffix.
///
/// The nanos field is formatted with 0, 3, 6, or 9 fractional digits depending
/// on precision needed. Negative durations (where `seconds < 0` or
/// `seconds == 0 && nanos < 0`) are prefixed with `-`.
#[cfg(feature = "json")]
fn duration_to_string(secs: i64, nanos: i32) -> alloc::string::String {
    use alloc::format;
    use alloc::string::String;
    let negative = secs < 0 || (secs == 0 && nanos < 0);
    let abs_secs = secs.unsigned_abs();
    let abs_nanos = nanos.unsigned_abs();
    let sign = if negative { "-" } else { "" };
    let frac = if abs_nanos == 0 {
        String::new()
    } else if abs_nanos % 1_000_000 == 0 {
        format!(".{:03}", abs_nanos / 1_000_000)
    } else if abs_nanos % 1_000 == 0 {
        format!(".{:06}", abs_nanos / 1_000)
    } else {
        format!(".{:09}", abs_nanos)
    };
    format!("{sign}{abs_secs}{frac}s")
}

/// Parse a decimal seconds string (e.g. `"1.5s"`, `"-0.001s"`) to (seconds, nanos).
/// Returns `None` if the string is malformed.
#[cfg(feature = "json")]
fn parse_duration_string(s: &str) -> Option<(i64, i32)> {
    let body = s.strip_suffix('s')?;
    let negative = body.starts_with('-');
    let body = if negative {
        body.strip_prefix('-')?
    } else {
        body
    };
    // Reject residual sign after stripping: "--5s" would otherwise parse as
    // -5 via i64::parse and the double negation would yield +5 silently.
    if body.starts_with(['-', '+']) {
        return None;
    }

    let (sec_str, nano_str) = match body.find('.') {
        Some(dot) => (&body[..dot], &body[dot + 1..]),
        None => (body, ""),
    };

    let abs_secs: i64 = sec_str.parse().ok()?;
    let abs_nanos: i32 = if nano_str.is_empty() {
        0
    } else {
        // All chars must be digits (i32::parse accepts '+'/'-', which would
        // let e.g. "5.-3s" produce negative nanos).
        if nano_str.len() > 9 || !nano_str.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n: i32 = nano_str.parse().ok()?;
        n * 10_i32.pow(9 - nano_str.len() as u32)
    };

    let (secs, nanos) = if negative {
        (-abs_secs, -abs_nanos)
    } else {
        (abs_secs, abs_nanos)
    };
    if !is_valid_duration(secs, nanos) {
        return None;
    }
    Some((secs, nanos))
}

// ── serde impls ──────────────────────────────────────────────────────────────

// Protobuf spec: Duration is restricted to ±10,000 years ≈ ±315,576,000,000s.
#[cfg(feature = "json")]
const MAX_DURATION_SECS: i64 = 315_576_000_000;

#[cfg(feature = "json")]
fn is_valid_duration(secs: i64, nanos: i32) -> bool {
    if !(-999_999_999..=999_999_999).contains(&nanos) {
        return false;
    }
    if !(-MAX_DURATION_SECS..=MAX_DURATION_SECS).contains(&secs) {
        return false;
    }
    // Sign consistency: nanos must match seconds sign (or be zero).
    if (secs > 0 && nanos < 0) || (secs < 0 && nanos > 0) {
        return false;
    }
    true
}

#[cfg(feature = "json")]
impl serde::Serialize for Duration {
    /// Serializes as a decimal seconds string (e.g. `"1.5s"`, `"-0.001s"`).
    ///
    /// # Errors
    ///
    /// Returns a serialization error if the duration is outside the proto
    /// spec range of ±315,576,000,000 seconds, or if nanos is invalid.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use alloc::format;
        if !is_valid_duration(self.seconds, self.nanos) {
            return Err(serde::ser::Error::custom(format!(
                "invalid Duration: seconds={}, nanos={} is out of range",
                self.seconds, self.nanos
            )));
        }
        s.serialize_str(&duration_to_string(self.seconds, self.nanos))
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Duration {
    /// Deserializes from a decimal seconds string (e.g. `"1.5s"`).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use alloc::{format, string::String};
        let s: String = serde::Deserialize::deserialize(d)?;
        let (seconds, nanos) = parse_duration_string(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid Duration string: {s}")))?;
        Ok(Self {
            seconds,
            nanos,
            ..Default::default()
        })
    }
}

impl Duration {
    /// Create a [`Duration`] from a whole number of seconds.
    pub fn from_secs(seconds: i64) -> Self {
        Self {
            seconds,
            nanos: 0,
            ..Default::default()
        }
    }

    /// Create a [`Duration`] from seconds and nanoseconds.
    ///
    /// # Panics
    ///
    /// Panics in debug mode if `nanos` is outside `[-999_999_999, 999_999_999]`
    /// or if its sign is inconsistent with `seconds`.  When `seconds` is zero,
    /// `nanos` may be positive, negative, or zero.  In release mode the value
    /// is stored as-is.  Use [`Duration::from_secs_nanos_checked`] for a variant
    /// that returns `None` on invalid input.
    pub fn from_secs_nanos(seconds: i64, nanos: i32) -> Self {
        // Use a range check rather than .abs() to avoid overflow on i32::MIN.
        debug_assert!(
            (-999_999_999..=999_999_999).contains(&nanos),
            "nanos ({nanos}) must be in [-999_999_999, 999_999_999]"
        );
        debug_assert!(
            !((seconds > 0 && nanos < 0) || (seconds < 0 && nanos > 0)),
            "nanos sign must be consistent with seconds sign"
        );
        Self {
            seconds,
            nanos,
            ..Default::default()
        }
    }

    /// Create a [`Duration`] from seconds and nanoseconds, returning `None`
    /// if `nanos` is out of range or has a sign inconsistent with `seconds`.
    pub fn from_secs_nanos_checked(seconds: i64, nanos: i32) -> Option<Self> {
        // Use a range check rather than .abs() to avoid overflow on i32::MIN.
        if !(-999_999_999..=999_999_999).contains(&nanos) {
            return None;
        }
        if (seconds > 0 && nanos < 0) || (seconds < 0 && nanos > 0) {
            return None;
        }
        Some(Self {
            seconds,
            nanos,
            ..Default::default()
        })
    }

    /// Create a [`Duration`] from a number of milliseconds.
    ///
    /// The sign of `millis` determines the sign of both `seconds` and the
    /// sub-second `nanos` field, per the protobuf sign-consistency rule.
    pub fn from_millis(millis: i64) -> Self {
        Self {
            seconds: millis / 1_000,
            // Remainder is in [-999, 999]; after ×1_000_000 → [-999_000_000, 999_000_000],
            // which fits in i32 (max ≈ ±2.1 billion). Cast is lossless.
            nanos: ((millis % 1_000) * 1_000_000) as i32,
            ..Default::default()
        }
    }

    /// Create a [`Duration`] from a number of microseconds.
    pub fn from_micros(micros: i64) -> Self {
        Self {
            seconds: micros / 1_000_000,
            // Remainder is in [-999_999, 999_999]; after ×1_000 → [-999_999_000, 999_999_000],
            // which fits in i32. Cast is lossless.
            nanos: ((micros % 1_000_000) * 1_000) as i32,
            ..Default::default()
        }
    }

    /// Create a [`Duration`] from a number of nanoseconds.
    pub fn from_nanos(nanos: i64) -> Self {
        Self {
            seconds: nanos / 1_000_000_000,
            // Remainder is in [-999_999_999, 999_999_999], which fits in i32. Cast is lossless.
            nanos: (nanos % 1_000_000_000) as i32,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "std")]
    #[test]
    fn std_duration_roundtrip() {
        let d = std::time::Duration::new(300, 500_000_000);
        let proto: Duration = d.into();
        assert_eq!(proto.seconds, 300);
        assert_eq!(proto.nanos, 500_000_000);
        let back: std::time::Duration = proto.try_into().unwrap();
        assert_eq!(back, d);
    }

    #[cfg(feature = "std")]
    #[test]
    fn zero_duration_roundtrip() {
        let d = std::time::Duration::ZERO;
        let proto: Duration = d.into();
        let back: std::time::Duration = proto.try_into().unwrap();
        assert_eq!(back, d);
    }

    #[cfg(feature = "std")]
    #[test]
    fn negative_duration_rejected() {
        let neg = Duration {
            seconds: -5,
            nanos: 0,
            ..Default::default()
        };
        let result: Result<std::time::Duration, _> = neg.try_into();
        assert_eq!(result, Err(DurationError::NegativeDuration));
    }

    #[cfg(feature = "std")]
    #[test]
    fn invalid_nanos_rejected() {
        let bad = Duration {
            seconds: 1,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let result: Result<std::time::Duration, _> = bad.try_into();
        assert_eq!(result, Err(DurationError::InvalidNanos));
    }

    // ---- from_millis / from_micros / from_nanos ---------------------------

    #[test]
    fn from_millis_positive() {
        let d = Duration::from_millis(1_500);
        assert_eq!(d.seconds, 1);
        assert_eq!(d.nanos, 500_000_000);
    }

    #[test]
    fn from_millis_negative() {
        let d = Duration::from_millis(-1_500);
        assert_eq!(d.seconds, -1);
        assert_eq!(d.nanos, -500_000_000);
    }

    #[test]
    fn from_millis_exact_seconds() {
        let d = Duration::from_millis(2_000);
        assert_eq!(d.seconds, 2);
        assert_eq!(d.nanos, 0);
    }

    #[test]
    fn from_micros_positive() {
        let d = Duration::from_micros(1_500_000);
        assert_eq!(d.seconds, 1);
        assert_eq!(d.nanos, 500_000_000);
    }

    #[test]
    fn from_micros_negative() {
        let d = Duration::from_micros(-750);
        assert_eq!(d.seconds, 0);
        assert_eq!(d.nanos, -750_000);
    }

    #[test]
    fn from_nanos_positive() {
        let d = Duration::from_nanos(1_500_000_000);
        assert_eq!(d.seconds, 1);
        assert_eq!(d.nanos, 500_000_000);
    }

    #[test]
    fn from_nanos_negative() {
        let d = Duration::from_nanos(-2_000_000_000);
        assert_eq!(d.seconds, -2);
        assert_eq!(d.nanos, 0);
    }

    #[test]
    fn from_nanos_sub_second() {
        let d = Duration::from_nanos(999_999_999);
        assert_eq!(d.seconds, 0);
        assert_eq!(d.nanos, 999_999_999);
    }

    #[test]
    fn from_millis_i64_min() {
        // i64::MIN = -9_223_372_036_854_775_808
        // remainder = i64::MIN % 1_000 = -808 (Rust truncation remainder)
        // nanos cast: -808 * 1_000_000 = -808_000_000, fits in i32
        let d = Duration::from_millis(i64::MIN);
        assert_eq!(d.nanos, -808_000_000_i32);
    }

    #[test]
    fn from_millis_i64_max() {
        // i64::MAX = 9_223_372_036_854_775_807; remainder = 807
        let d = Duration::from_millis(i64::MAX);
        assert_eq!(d.nanos, 807_000_000_i32);
    }

    #[test]
    fn from_micros_i64_min() {
        // remainder = i64::MIN % 1_000_000 = -775_808
        // nanos cast: -775_808 * 1_000 = -775_808_000, fits in i32
        let d = Duration::from_micros(i64::MIN);
        assert_eq!(d.nanos, -775_808_000_i32);
    }

    #[test]
    fn from_nanos_i64_min() {
        // remainder = i64::MIN % 1_000_000_000 = -854_775_808, fits in i32
        let d = Duration::from_nanos(i64::MIN);
        assert_eq!(d.nanos, -854_775_808_i32);
    }

    #[test]
    fn from_nanos_i64_max() {
        // remainder = i64::MAX % 1_000_000_000 = 854_775_807, fits in i32
        let d = Duration::from_nanos(i64::MAX);
        assert_eq!(d.nanos, 854_775_807_i32);
    }

    // ---- TryFrom edge cases -----------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn nanos_i32_min_is_invalid() {
        // i32::MIN cannot be represented as a valid protobuf nanos value
        // (valid range is [-999_999_999, 999_999_999]).  Using .abs() on
        // i32::MIN overflows; we use a range check to avoid that.
        let bad = Duration {
            seconds: 0,
            nanos: i32::MIN,
            ..Default::default()
        };
        let result: Result<std::time::Duration, _> = bad.try_into();
        assert_eq!(result, Err(DurationError::InvalidNanos));
    }

    #[cfg(feature = "std")]
    #[test]
    fn negative_seconds_and_negative_nanos_is_negative_duration() {
        // A well-formed negative duration (sign-consistent) must return
        // NegativeDuration, not InvalidNanos.
        let neg = Duration {
            seconds: -5,
            nanos: -500_000_000,
            ..Default::default()
        };
        let result: Result<std::time::Duration, _> = neg.try_into();
        assert_eq!(result, Err(DurationError::NegativeDuration));
    }

    // ---- from_secs --------------------------------------------------------

    #[test]
    fn from_secs_zero() {
        let d = Duration::from_secs(0);
        assert_eq!(d.seconds, 0);
        assert_eq!(d.nanos, 0);
    }

    #[test]
    fn from_secs_positive() {
        let d = Duration::from_secs(300);
        assert_eq!(d.seconds, 300);
        assert_eq!(d.nanos, 0);
    }

    #[test]
    fn from_secs_negative() {
        let d = Duration::from_secs(-7);
        assert_eq!(d.seconds, -7);
        assert_eq!(d.nanos, 0);
    }

    // ---- from_secs_nanos_checked ------------------------------------------

    #[test]
    fn from_secs_nanos_checked_valid_positive() {
        let d = Duration::from_secs_nanos_checked(1, 999_999_999).unwrap();
        assert_eq!(d.seconds, 1);
        assert_eq!(d.nanos, 999_999_999);
    }

    #[test]
    fn from_secs_nanos_checked_valid_negative() {
        let d = Duration::from_secs_nanos_checked(-1, -999_999_999).unwrap();
        assert_eq!(d.seconds, -1);
        assert_eq!(d.nanos, -999_999_999);
    }

    #[test]
    fn from_secs_nanos_checked_nanos_out_of_range() {
        assert!(Duration::from_secs_nanos_checked(1, 1_000_000_000).is_none());
    }

    #[test]
    fn from_secs_nanos_checked_i32_min_nanos_is_none() {
        // i32::MIN would overflow .abs(); the range check must handle it.
        assert!(Duration::from_secs_nanos_checked(0, i32::MIN).is_none());
    }

    #[test]
    fn from_secs_nanos_checked_sign_mismatch_is_none() {
        assert!(Duration::from_secs_nanos_checked(-1, 1).is_none());
        assert!(Duration::from_secs_nanos_checked(1, -1).is_none());
    }

    #[test]
    fn from_secs_nanos_checked_zero_seconds_allows_negative_nanos() {
        // When seconds == 0, the sign rule does not apply; nanos may be negative.
        let d = Duration::from_secs_nanos_checked(0, -500_000_000).unwrap();
        assert_eq!(d.seconds, 0);
        assert_eq!(d.nanos, -500_000_000);
    }

    // ---- from_secs_nanos (panic path tested via checked variant above) ----

    #[test]
    fn from_secs_nanos_valid() {
        let d = Duration::from_secs_nanos(2, 500_000_000);
        assert_eq!(d.seconds, 2);
        assert_eq!(d.nanos, 500_000_000);
    }

    // ---- saturation -------------------------------------------------------

    #[cfg(feature = "std")]
    #[test]
    fn large_std_duration_saturates_to_i64_max_seconds() {
        // std::time::Duration can represent values far beyond i64::MAX seconds
        // (its seconds are stored as u64).  The From impl must saturate rather
        // than wrap, which would produce a negative seconds value.
        let huge = std::time::Duration::from_secs(u64::MAX);
        let proto: Duration = huge.into();
        assert_eq!(proto.seconds, i64::MAX);
        // Subsecond nanos are zero because u64::MAX is a whole number of seconds.
        assert_eq!(proto.nanos, 0);
    }

    // ---- serde ----------------------------------------------------------------

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;

        #[test]
        fn duration_zero_roundtrip() {
            let d = Duration::from_secs(0);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""0s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, 0);
            assert_eq!(back.nanos, 0);
        }

        #[test]
        fn duration_positive_whole_seconds_roundtrip() {
            let d = Duration::from_secs(300);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""300s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, 300);
            assert_eq!(back.nanos, 0);
        }

        #[test]
        fn duration_millis_precision_roundtrip() {
            let d = Duration::from_secs_nanos(1, 500_000_000);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""1.500s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, 1);
            assert_eq!(back.nanos, 500_000_000);
        }

        #[test]
        fn duration_micros_precision_roundtrip() {
            let d = Duration::from_secs_nanos(0, 1_000);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""0.000001s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.nanos, 1_000);
        }

        #[test]
        fn duration_nanos_precision_roundtrip() {
            let d = Duration::from_secs_nanos(0, 1);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""0.000000001s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.nanos, 1);
        }

        #[test]
        fn duration_negative_roundtrip() {
            let d = Duration::from_secs_nanos(-1, -500_000_000);
            let json = serde_json::to_string(&d).unwrap();
            assert_eq!(json, r#""-1.500s""#);
            let back: Duration = serde_json::from_str(&json).unwrap();
            assert_eq!(back.seconds, -1);
            assert_eq!(back.nanos, -500_000_000);
        }

        #[test]
        fn duration_invalid_string_is_error() {
            let result: Result<Duration, _> = serde_json::from_str(r#""1.5""#); // missing 's'
            assert!(result.is_err());
        }

        #[test]
        fn parse_duration_rejects_double_sign() {
            // Regression: "--5s" used to strip one '-' then parse "-5"
            // via i64::parse, yielding +5 via double negation. Now rejected.
            assert_eq!(parse_duration_string("--5s"), None);
            assert_eq!(parse_duration_string("-+5s"), None);
            assert_eq!(parse_duration_string("+5s"), None); // '+' never valid
                                                            // The fractional variant was already caught by sign mismatch,
                                                            // but verify it still is.
            assert_eq!(parse_duration_string("--5.5s"), None);
            // Sanity: valid negative still works.
            assert_eq!(parse_duration_string("-5s"), Some((-5, 0)));
        }

        #[test]
        fn parse_duration_rejects_non_digit_fractional() {
            // Regression (fuzzer-found): "5.-3s" previously parsed with
            // nano_str="-3" → i32::parse accepts it → nanos=-300000000.
            // Same class as the double-sign bug but in the fractional part.
            assert_eq!(parse_duration_string("5.-3s"), None, "minus in frac");
            assert_eq!(parse_duration_string("5.+3s"), None, "plus in frac");
            assert_eq!(parse_duration_string("-5.-3s"), None, "double neg frac");
            assert_eq!(parse_duration_string("5.3as"), None, "alpha in frac");
            assert_eq!(parse_duration_string("5. s"), None, "space in frac");
            // Valid fractional still works.
            assert_eq!(parse_duration_string("5.3s"), Some((5, 300_000_000)));
            assert_eq!(parse_duration_string("-5.3s"), Some((-5, -300_000_000)));
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn negative_nanos_on_positive_seconds_is_invalid_nanos() {
        // Duration { seconds: 5, nanos: -1 } has a sign mismatch — nanos is negative
        // while seconds is positive.  This should be InvalidNanos, not NegativeDuration.
        let bad = Duration {
            seconds: 5,
            nanos: -1,
            ..Default::default()
        };
        let result: Result<std::time::Duration, _> = bad.try_into();
        assert_eq!(result, Err(DurationError::InvalidNanos));
    }
}
