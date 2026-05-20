//! Shared formatting and parsing helpers for well-known type JSON forms.
//!
//! These are the proto3-canonical-JSON representations for `Timestamp`,
//! `Duration`, and `FieldMask`. They operate on raw scalars (`(i64, i32)`,
//! `&str`) rather than the WKT structs so they can be shared between
//! `buffa-types`'s typed serde impls and `buffa-descriptor`'s reflective
//! [`DynamicMessage`](https://docs.rs/buffa-descriptor) JSON codec without
//! a dependency edge between those crates. Both crates depend on `buffa`.
//!
//! Sharing the implementation is load-bearing: the conformance suite exercises
//! both the typed and reflective JSON paths, and a divergence between them
//! (e.g. one accepting a fractional-second precision the other rejects)
//! would be a user-visible inconsistency.

use crate::alloc::format;
use crate::alloc::string::String;
use crate::alloc::vec::Vec;

// ── Bounds ──────────────────────────────────────────────────────────────────

/// Smallest valid `Timestamp.seconds`: `0001-01-01T00:00:00Z`.
pub const MIN_TIMESTAMP_SECS: i64 = -62_135_596_800;
/// Largest valid `Timestamp.seconds`: `9999-12-31T23:59:59Z`.
pub const MAX_TIMESTAMP_SECS: i64 = 253_402_300_799;
/// Largest valid `Duration.seconds` magnitude (10 000 years).
pub const MAX_DURATION_SECS: i64 = 315_576_000_000;

// ── Timestamp ───────────────────────────────────────────────────────────────

/// Format unix `(seconds, nanos)` as an RFC 3339 timestamp with a `Z` suffix
/// and the minimal fractional-seconds precision (0, 3, 6, or 9 digits).
///
/// # Errors
///
/// Returns an error if the timestamp is outside the proto3 `Timestamp`
/// range (`0001-01-01T00:00:00Z` through `9999-12-31T23:59:59.999999999Z`)
/// or `nanos` is outside `[0, 999_999_999]`.
pub fn fmt_timestamp(secs: i64, nanos: i32) -> Result<String, &'static str> {
    if !(0..1_000_000_000).contains(&nanos) {
        return Err("Timestamp nanos out of range");
    }
    if !(MIN_TIMESTAMP_SECS..=MAX_TIMESTAMP_SECS).contains(&secs) {
        return Err("Timestamp seconds out of range");
    }
    let (tod, day) = {
        let r = secs % 86_400;
        if r >= 0 {
            (r, secs / 86_400)
        } else {
            (r + 86_400, secs / 86_400 - 1)
        }
    };
    let (y, mo, d) = days_to_date(day);
    if !(1..=9999).contains(&y) {
        return Err("Timestamp year out of range");
    }
    let h = tod / 3600;
    let mi = (tod % 3600) / 60;
    let s = tod % 60;
    let frac = fmt_nanos_min(nanos);
    Ok(format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}{frac}Z"
    ))
}

/// Parse an RFC 3339 timestamp into unix `(seconds, nanos)`.
///
/// Accepts an uppercase `Z` suffix and `+HH:MM` / `-HH:MM` UTC offsets.
/// Rejects lowercase `t`/`z` (the proto3 JSON spec requires uppercase) and
/// anything outside the proto3 `Timestamp` range.
///
/// # Errors
///
/// Returns an error describing the first malformed component.
pub fn parse_timestamp(s: &str) -> Result<(i64, i32), &'static str> {
    // RFC 3339 timestamps are pure ASCII. Reject non-ASCII early so byte
    // slices below stay on char boundaries.
    if !s.is_ascii() {
        return Err("non-ASCII in timestamp");
    }
    // Proto3 JSON spec requires uppercase 'Z' suffix (not lowercase).
    let (dt, tz_offset) = if let Some(rest) = s.strip_suffix('Z') {
        (rest, 0i64)
    } else {
        let len = s.len();
        if len < 6 {
            return Err("timestamp too short");
        }
        let sign: i64 = match s.as_bytes()[len - 6] {
            b'+' => -1,
            b'-' => 1,
            _ => return Err("missing or malformed timezone"),
        };
        if s.as_bytes()[len - 3] != b':' {
            return Err("malformed timezone offset");
        }
        let oh: i64 = s[len - 5..len - 3]
            .parse()
            .map_err(|_| "bad offset hours")?;
        let om: i64 = s[len - 2..].parse().map_err(|_| "bad offset minutes")?;
        if !(0..=23).contains(&oh) || !(0..=59).contains(&om) {
            return Err("offset out of range");
        }
        (&s[..len - 6], sign * (oh * 3600 + om * 60))
    };

    // Proto3 JSON spec requires uppercase 'T' separator (not lowercase).
    let t = dt.find('T').ok_or("missing 'T' separator")?;
    let (date, time) = (&dt[..t], &dt[t + 1..]);
    if date.len() != 10 || time.len() < 8 {
        return Err("malformed date or time");
    }

    let date_b = date.as_bytes();
    let time_b = time.as_bytes();
    if date_b[4] != b'-' || date_b[7] != b'-' || time_b[2] != b':' || time_b[5] != b':' {
        return Err("malformed separators");
    }

    let year: i64 = date[0..4].parse().map_err(|_| "bad year")?;
    let month: u8 = date[5..7].parse().map_err(|_| "bad month")?;
    let day: u8 = date[8..10].parse().map_err(|_| "bad day")?;
    let hour: i64 = time[0..2].parse().map_err(|_| "bad hour")?;
    let min: i64 = time[3..5].parse().map_err(|_| "bad minute")?;
    let sec: i64 = time[6..8].parse().map_err(|_| "bad second")?;
    // Proto3 Timestamp uses unix epoch seconds, which has no leap-second
    // representation, so reject second 60.
    if !(0..=23).contains(&hour) || !(0..=59).contains(&min) || !(0..=59).contains(&sec) {
        return Err("time component out of range");
    }

    let nanos = if time.len() > 8 {
        if time.as_bytes()[8] != b'.' {
            return Err("malformed fractional seconds");
        }
        let frac = &time[9..];
        // All chars must be digits — `i32::parse` accepts '-' and '+', which
        // would let "T23:59:59.-3Z" produce negative nanos.
        if frac.is_empty() || frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err("bad fractional seconds");
        }
        let n: i32 = frac.parse().map_err(|_| "bad fractional seconds")?;
        n * 10_i32.pow(9 - frac.len() as u32)
    } else {
        0
    };

    if !(1..=9999).contains(&year) {
        return Err("Timestamp year out of range");
    }
    let days = date_to_days(year, month, day).ok_or("invalid date")?;
    let unix = days * 86_400 + hour * 3600 + min * 60 + sec + tz_offset;
    // The offset can push a boundary timestamp past the valid range
    // (`"9999-12-31T23:59:59-23:59"` has year 9999 but the UTC equivalent
    // is year 10000).
    if !(MIN_TIMESTAMP_SECS..=MAX_TIMESTAMP_SECS).contains(&unix) {
        return Err("Timestamp out of range after applying offset");
    }
    Ok((unix, nanos))
}

// ── Duration ────────────────────────────────────────────────────────────────

/// Format `(seconds, nanos)` as a `"3.5s"`-style decimal seconds string with
/// the minimal fractional precision.
///
/// # Errors
///
/// Returns an error if `(seconds, nanos)` violate the `Duration` invariants:
/// magnitude bounds, or `seconds` and `nanos` having opposite signs.
pub fn fmt_duration(secs: i64, nanos: i32) -> Result<String, &'static str> {
    validate_duration(secs, nanos)?;
    let negative = secs < 0 || (secs == 0 && nanos < 0);
    let abs_secs = secs.unsigned_abs();
    let abs_nanos = nanos.unsigned_abs();
    let sign = if negative { "-" } else { "" };
    // `validate_duration` bounded `|nanos|` so the cast is provably safe.
    let frac = fmt_nanos_min(i32::try_from(abs_nanos).expect("validated nanos fit in i32"));
    Ok(format!("{sign}{abs_secs}{frac}s"))
}

/// Parse a `"3.5s"`-style decimal seconds string into `(seconds, nanos)`.
///
/// # Errors
///
/// Returns an error on a malformed string or a value outside the `Duration`
/// range.
pub fn parse_duration(s: &str) -> Result<(i64, i32), &'static str> {
    let body = s.strip_suffix('s').ok_or("missing 's' suffix")?;
    let negative = body.starts_with('-');
    let body = if negative {
        body.strip_prefix('-').ok_or("malformed sign")?
    } else {
        body
    };
    // Reject residual sign after stripping: "--5s" would otherwise parse as
    // -5 via i64::parse and the double negation would yield +5 silently.
    if body.starts_with(['-', '+']) {
        return Err("malformed sign");
    }
    let (sec_str, nano_str) = match body.find('.') {
        Some(dot) => (&body[..dot], &body[dot + 1..]),
        None => (body, ""),
    };
    let abs_secs: i64 = sec_str.parse().map_err(|_| "bad seconds")?;
    let abs_nanos: i32 = if nano_str.is_empty() {
        0
    } else {
        if nano_str.len() > 9 || !nano_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err("bad fractional seconds");
        }
        let n: i32 = nano_str.parse().map_err(|_| "bad fractional seconds")?;
        n * 10_i32.pow(9 - nano_str.len() as u32)
    };
    let (secs, nanos) = if negative {
        (-abs_secs, -abs_nanos)
    } else {
        (abs_secs, abs_nanos)
    };
    validate_duration(secs, nanos)?;
    Ok((secs, nanos))
}

/// Validate the proto3 `Duration` invariants: `|seconds| ≤ 315 576 000 000`,
/// `|nanos| ≤ 999 999 999`, and `seconds` and `nanos` have the same sign or
/// one of them is zero.
///
/// # Errors
///
/// Returns an error naming the violated invariant.
pub fn validate_duration(secs: i64, nanos: i32) -> Result<(), &'static str> {
    if !(-999_999_999..=999_999_999).contains(&nanos) {
        return Err("Duration nanos out of range");
    }
    if !(-MAX_DURATION_SECS..=MAX_DURATION_SECS).contains(&secs) {
        return Err("Duration seconds out of range");
    }
    if (secs > 0 && nanos < 0) || (secs < 0 && nanos > 0) {
        return Err("Duration seconds and nanos have opposite signs");
    }
    Ok(())
}

/// Format `nanos` (0–999 999 999) as `"."`-prefixed fractional seconds
/// with the minimal precision (3, 6, or 9 digits) that loses no information,
/// or the empty string for zero.
fn fmt_nanos_min(nanos: i32) -> String {
    if nanos == 0 {
        String::new()
    } else if nanos % 1_000_000 == 0 {
        format!(".{:03}", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!(".{:06}", nanos / 1_000)
    } else {
        format!(".{nanos:09}")
    }
}

// ── FieldMask ───────────────────────────────────────────────────────────────

/// Convert a snake_case field-mask path to lowerCamelCase, handling dotted
/// sub-paths.
///
/// Does **not** validate round-trip safety — pair with a
/// `field_mask_path_round_trips` check before serializing per the spec.
#[must_use]
pub fn snake_to_camel(path: &str) -> String {
    path.split('.')
        .map(|component| {
            let mut out = String::with_capacity(component.len());
            let mut capitalize_next = false;
            for ch in component.chars() {
                if ch == '_' {
                    capitalize_next = true;
                } else if capitalize_next {
                    out.extend(ch.to_uppercase());
                    capitalize_next = false;
                } else {
                    out.push(ch);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Convert a lowerCamelCase field-mask path to snake_case, handling dotted
/// sub-paths.
#[must_use]
pub fn camel_to_snake(path: &str) -> String {
    path.split('.')
        .map(|component| {
            let mut out = String::with_capacity(component.len() + 4);
            for ch in component.chars() {
                if ch.is_uppercase() {
                    // No underscore before the first char of a component,
                    // even if it's uppercase (PascalCase → snake, not _snake).
                    if !out.is_empty() {
                        out.push('_');
                    }
                    out.extend(ch.to_lowercase());
                } else {
                    out.push(ch);
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Whether a snake_case `FieldMask` path round-trips through camelCase
/// without information loss.
///
/// The proto3 JSON spec requires rejecting paths that can't round-trip:
/// double underscores (`foo__bar`), digits after underscores (`foo_3_bar`),
/// and uppercase in the snake form (`fooBar`) all violate the invariant
/// `camel_to_snake(snake_to_camel(p)) == p`.
#[must_use]
pub fn field_mask_path_round_trips(path: &str) -> bool {
    camel_to_snake(&snake_to_camel(path)) == path
}

// ── Civil calendar ──────────────────────────────────────────────────────────
//
// Howard Hinnant's algorithms — see <http://howardhinnant.github.io/date_algorithms.html>.
// These convert between days-since-epoch (1970-01-01) and (year, month, day)
// civil dates without a calendar table or branching on month lengths.

/// Convert days-since-unix-epoch to a proleptic Gregorian `(year, month, day)`.
#[must_use]
pub fn days_to_date(z: i64) -> (i64, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Convert a proleptic Gregorian `(year, month, day)` to days-since-unix-epoch.
///
/// Returns `None` if the date is invalid (out-of-range month, or day exceeding
/// the Gregorian month length including the leap-year rule for February).
#[must_use]
pub fn date_to_days(y: i64, m: u8, d: u8) -> Option<i64> {
    if !(1..=12).contains(&m) || d == 0 || u32::from(d) > days_in_month(y, u32::from(m)) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// Days in `month` of `year` (1-indexed month). Validates the Gregorian
/// leap-year rule.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_round_trip() {
        let cases = [
            (0, 0, "1970-01-01T00:00:00Z"),
            (1, 0, "1970-01-01T00:00:01Z"),
            (-1, 0, "1969-12-31T23:59:59Z"),
            (0, 500_000_000, "1970-01-01T00:00:00.500Z"),
            (0, 1, "1970-01-01T00:00:00.000000001Z"),
            (1_609_459_200, 0, "2021-01-01T00:00:00Z"),
            (
                MAX_TIMESTAMP_SECS,
                999_999_999,
                "9999-12-31T23:59:59.999999999Z",
            ),
            (MIN_TIMESTAMP_SECS, 0, "0001-01-01T00:00:00Z"),
        ];
        for (secs, nanos, expected) in cases {
            assert_eq!(fmt_timestamp(secs, nanos).unwrap(), expected);
            assert_eq!(parse_timestamp(expected).unwrap(), (secs, nanos));
        }
    }

    #[test]
    fn timestamp_rejects_invalid() {
        assert!(parse_timestamp("0000-01-01T00:00:00Z").is_err()); // year 0
        assert!(parse_timestamp("2021-02-30T00:00:00Z").is_err()); // Feb 30
        assert!(parse_timestamp("2021-01-01t00:00:00Z").is_err()); // lowercase t
        assert!(parse_timestamp("2021-01-01T00:00:00z").is_err()); // lowercase z
        assert!(parse_timestamp("2021-01-01T00:00:60Z").is_err()); // leap second
        assert!(parse_timestamp("2021-13-01T00:00:00Z").is_err()); // month 13
        assert!(parse_timestamp("ñotrfc3339").is_err()); // non-ASCII
        assert!(fmt_timestamp(MAX_TIMESTAMP_SECS + 1, 0).is_err());
        assert!(fmt_timestamp(0, -1).is_err());
    }

    #[test]
    fn duration_round_trip() {
        let cases = [
            (0, 0, "0s"),
            (1, 500_000_000, "1.500s"),
            (-1, -500_000_000, "-1.500s"),
            (0, -1, "-0.000000001s"),
            (MAX_DURATION_SECS, 999_999_999, "315576000000.999999999s"),
        ];
        for (secs, nanos, expected) in cases {
            assert_eq!(fmt_duration(secs, nanos).unwrap(), expected);
            assert_eq!(parse_duration(expected).unwrap(), (secs, nanos));
        }
    }

    #[test]
    fn duration_rejects_invalid() {
        assert!(fmt_duration(MAX_DURATION_SECS + 1, 0).is_err());
        assert!(fmt_duration(1, -1).is_err()); // opposite signs
        assert!(fmt_duration(0, 1_000_000_000).is_err()); // nanos overflow
        assert!(parse_duration("--5s").is_err()); // double sign
        assert!(parse_duration("1.5").is_err()); // no suffix
        assert!(parse_duration("1.5e9s").is_err()); // exponent
    }

    #[test]
    fn field_mask_round_trip() {
        assert_eq!(snake_to_camel("foo_bar"), "fooBar");
        assert_eq!(camel_to_snake("fooBar"), "foo_bar");
        assert_eq!(snake_to_camel("user.first_name"), "user.firstName");
        assert!(field_mask_path_round_trips("foo_bar"));
        assert!(field_mask_path_round_trips("user.first_name"));
        assert!(!field_mask_path_round_trips("foo__bar"));
        assert!(!field_mask_path_round_trips("foo_3_bar"));
        assert!(!field_mask_path_round_trips("fooBar"));
    }
}
