//! `jiff` interop for [`google::protobuf::Duration`](crate::google::protobuf::Duration).
//!
//! Enabled with the `jiff` Cargo feature. The proto `Duration` maps to
//! [`jiff::SignedDuration`] — jiff's fixed (non-calendar) signed duration,
//! whose sign-consistent `seconds` + sub-second `nanos` representation matches
//! proto's exactly. `no_std`-compatible.
//!
//! [`jiff::Span`](jiff::Span) is deliberately *not* a conversion target: a
//! `Span` carries calendar units (years/months/days) whose length is only
//! defined relative to a reference date, whereas a proto `Duration` is an
//! absolute elapsed time. `SignedDuration` is the faithful analog.

use crate::google::protobuf::Duration;

/// Errors that can occur when converting a protobuf [`Duration`] to a
/// [`jiff::SignedDuration`].
///
/// Unlike the `chrono` conversion's
/// [`DurationChronoError`](crate::DurationChronoError), this has no `Overflow`
/// mode: a validated `nanos` (`|nanos| < 1_000_000_000`) never carries into the
/// seconds field, and both types store `seconds` as `i64`, so every well-formed
/// proto `Duration` maps into [`jiff::SignedDuration`] in range. The only
/// failure is a malformed `nanos` field.
///
/// This enum is `#[non_exhaustive]`: `match` arms over it must include a
/// wildcard arm.
#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DurationJiffError {
    /// The `nanos` field is outside `[-999_999_999, 999_999_999]` or its sign
    /// is inconsistent with `seconds`.
    #[error("nanos field has invalid value or sign mismatch with seconds")]
    InvalidNanos,
}

#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl From<jiff::SignedDuration> for Duration {
    /// Convert a [`jiff::SignedDuration`] to a protobuf [`Duration`].
    ///
    /// Infallible: both types represent a signed duration as a sign-consistent
    /// `seconds` + sub-second `nanos` pair, so this is a direct field copy.
    ///
    /// # Warning: proto JSON spec range
    ///
    /// `jiff::SignedDuration` ranges to ±`i64::MAX` seconds (~2.9e11 years),
    /// while the proto spec restricts `Duration` to ±315,576,000,000 seconds
    /// (~10,000 years). A `SignedDuration` beyond that converts without error
    /// here — binary encoding round-trips it — but the resulting `Duration`
    /// will fail JSON serialization (`json` feature), which enforces the spec
    /// range.
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Duration;
    ///
    /// let sd = jiff::SignedDuration::new(1, 500_000_000);
    /// let proto: Duration = sd.into();
    /// assert_eq!(proto.seconds, 1);
    /// assert_eq!(proto.nanos, 500_000_000);
    /// ```
    fn from(d: jiff::SignedDuration) -> Self {
        Self {
            seconds: d.as_secs(),
            nanos: d.subsec_nanos(),
            ..Default::default()
        }
    }
}

#[cfg_attr(docsrs, doc(cfg(feature = "jiff")))]
impl TryFrom<Duration> for jiff::SignedDuration {
    type Error = DurationJiffError;

    /// Convert a protobuf [`Duration`] to a [`jiff::SignedDuration`].
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::Duration;
    ///
    /// let proto = Duration {
    ///     seconds: 2,
    ///     nanos: 250_000_000,
    ///     ..Default::default()
    /// };
    /// let sd: jiff::SignedDuration = proto.try_into().unwrap();
    /// assert_eq!(sd, jiff::SignedDuration::new(2, 250_000_000));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`DurationJiffError::InvalidNanos`] if `nanos` is outside
    /// `[-999_999_999, 999_999_999]` or if its sign is inconsistent with
    /// `seconds`. Such values never come from the [`From<jiff::SignedDuration>`]
    /// impl, but `seconds` and `nanos` are independent wire fields, so a decoded
    /// `Duration` can carry any combination — the proto spec declares
    /// sign-mismatched ones invalid, and this conversion rejects them rather
    /// than letting `SignedDuration::new` silently re-normalize them.
    fn try_from(d: Duration) -> Result<Self, Self::Error> {
        if !(-999_999_999..=999_999_999).contains(&d.nanos) {
            return Err(DurationJiffError::InvalidNanos);
        }
        let sign_mismatch = (d.seconds > 0 && d.nanos < 0) || (d.seconds < 0 && d.nanos > 0);
        if sign_mismatch {
            return Err(DurationJiffError::InvalidNanos);
        }

        // `|nanos| < 1_000_000_000`, so `new` performs no second-carry and
        // cannot overflow i64.
        Ok(jiff::SignedDuration::new(d.seconds, d.nanos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_roundtrip() {
        let sd = jiff::SignedDuration::new(300, 500_000_000);
        let proto: Duration = sd.into();
        assert_eq!(proto.seconds, 300);
        assert_eq!(proto.nanos, 500_000_000);
        let back: jiff::SignedDuration = proto.try_into().unwrap();
        assert_eq!(back, sd);
    }

    #[test]
    fn zero_roundtrip() {
        let sd = jiff::SignedDuration::ZERO;
        let proto: Duration = sd.into();
        assert_eq!(proto.seconds, 0);
        assert_eq!(proto.nanos, 0);
        let back: jiff::SignedDuration = proto.try_into().unwrap();
        assert_eq!(back, sd);
    }

    #[test]
    fn negative_roundtrip() {
        // -1.5 seconds: jiff keeps both components negative, matching proto.
        let sd = jiff::SignedDuration::new(-1, -500_000_000);
        let proto: Duration = sd.into();
        assert_eq!(proto.seconds, -1);
        assert_eq!(proto.nanos, -500_000_000);
        let back: jiff::SignedDuration = proto.try_into().unwrap();
        assert_eq!(back, sd);
    }

    #[test]
    fn sub_second_negative_roundtrip() {
        let sd = jiff::SignedDuration::new(0, -500_000_000);
        let proto: Duration = sd.into();
        assert_eq!(proto.seconds, 0);
        assert_eq!(proto.nanos, -500_000_000);
        let back: jiff::SignedDuration = proto.try_into().unwrap();
        assert_eq!(back, sd);
    }

    #[test]
    fn invalid_nanos_rejected() {
        let bad = Duration {
            seconds: 1,
            nanos: 1_000_000_000,
            ..Default::default()
        };
        let r: Result<jiff::SignedDuration, _> = bad.try_into();
        assert_eq!(r, Err(DurationJiffError::InvalidNanos));
    }

    #[test]
    fn nanos_i32_min_is_invalid() {
        let bad = Duration {
            seconds: 0,
            nanos: i32::MIN,
            ..Default::default()
        };
        let r: Result<jiff::SignedDuration, _> = bad.try_into();
        assert_eq!(r, Err(DurationJiffError::InvalidNanos));
    }

    #[test]
    fn sign_mismatch_rejected() {
        let bad = Duration {
            seconds: 5,
            nanos: -1,
            ..Default::default()
        };
        let r: Result<jiff::SignedDuration, _> = bad.try_into();
        assert_eq!(r, Err(DurationJiffError::InvalidNanos));

        let bad2 = Duration {
            seconds: -5,
            nanos: 1,
            ..Default::default()
        };
        let r2: Result<jiff::SignedDuration, _> = bad2.try_into();
        assert_eq!(r2, Err(DurationJiffError::InvalidNanos));
    }

    #[test]
    fn signed_duration_extremes_roundtrip() {
        // `SignedDuration` spans ±i64::MAX seconds — wider than proto Duration's
        // spec range, but proto's `seconds` is also i64, so the binary form
        // round-trips both extremes exactly (no Overflow mode).
        for sd in [jiff::SignedDuration::MAX, jiff::SignedDuration::MIN] {
            let proto: Duration = sd.into();
            assert_eq!(proto.seconds, sd.as_secs());
            assert_eq!(proto.nanos, sd.subsec_nanos());
            let back: jiff::SignedDuration = proto.try_into().unwrap();
            assert_eq!(back, sd);
        }
    }
}
