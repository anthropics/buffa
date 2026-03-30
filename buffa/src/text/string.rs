//! Textproto string-literal escaping and unescaping.
//!
//! The textproto string grammar is byte-oriented: `\xNN` and `\NNN` octal
//! escapes produce raw bytes, so a string literal may decode to arbitrary
//! non-UTF-8 data. This is intentional — proto `bytes` fields use the same
//! literal syntax. [`unescape`] therefore returns `Vec<u8>`, and the
//! UTF-8-validating wrapper [`unescape_str`] sits on top.
//!
//! Escape sequences (reference: protobuf-go `decode_string.go`):
//!
//! | escape      | result                                                  |
//! |-------------|---------------------------------------------------------|
//! | `\n \r \t`  | newline, carriage return, tab                           |
//! | `\" \' \\`  | literal quote, apostrophe, backslash                    |
//! | `\?`        | literal `?` (C legacy)                                  |
//! | `\a \b`     | BEL (0x07), BS (0x08)                                   |
//! | `\f \v`     | FF (0x0C), VT (0x0B)                                    |
//! | `\NNN`      | 1–3 octal digits → one byte (value must be ≤ 255)       |
//! | `\xNN`      | 1–2 hex digits → one byte                               |
//! | `\uNNNN`    | 4 hex digits → UTF-8 encoding of that code point        |
//! | `\UNNNNNNNN`| 8 hex digits → UTF-8 encoding of that code point        |
//!
//! `\u` is BMP-only — surrogates (U+D800..U+DFFF) are rejected, even as a
//! well-formed pair. For non-BMP code points use `\U` (e.g. `\U0001F600`).

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::fmt::Write;

/// Error returned by [`unescape`] and [`unescape_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnescapeError {
    /// The unescaped bytes are not valid UTF-8. Only produced by
    /// [`unescape_str`]; [`unescape`] is byte-level and never returns this.
    InvalidUtf8,
    /// A malformed escape sequence or structural problem in the literal.
    /// The string describes the specific failure.
    BadEscape(&'static str),
}

/// Unescape a textproto string token — one or more adjacent quoted literals —
/// into a byte vector.
///
/// `raw` must begin with `"` or `'`. Adjacent literals (`"foo" 'bar'`) are
/// concatenated: whitespace between them is consumed, the enclosing quotes are
/// stripped, and escapes are resolved. This matches the textproto grammar's
/// treatment of `"a" "b"` as a single scalar value `"ab"`.
///
/// # Errors
///
/// Returns [`UnescapeError::BadEscape`] on the first malformed escape or
/// structural problem encountered.
pub fn unescape(raw: &str) -> Result<Vec<u8>, UnescapeError> {
    debug_assert!(
        matches!(raw.as_bytes().first(), Some(b'"' | b'\'')),
        "unescape input must start with a quote; got {raw:?}"
    );
    let mut out = Vec::new();
    let mut s = raw.as_bytes();
    loop {
        // Each iteration consumes one quoted literal.
        let Some(&quote) = s.first() else {
            return Err(UnescapeError::BadEscape("unterminated string"));
        };
        if quote != b'"' && quote != b'\'' {
            // Not a string-literal opener — we're done with adjacent concatenation.
            break;
        }
        s = &s[1..];
        loop {
            match s.first() {
                None => return Err(UnescapeError::BadEscape("unterminated string")),
                Some(&c) if c == quote => {
                    s = &s[1..];
                    break;
                }
                Some(b'\n') | Some(0) => {
                    return Err(UnescapeError::BadEscape(
                        "raw newline/NUL in string literal",
                    ));
                }
                Some(b'\\') => {
                    s = &s[1..];
                    let Some(&esc) = s.first() else {
                        return Err(UnescapeError::BadEscape("unterminated escape"));
                    };
                    s = &s[1..];
                    match esc {
                        b'"' | b'\'' | b'\\' | b'?' => out.push(esc),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'a' => out.push(0x07),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'v' => out.push(0x0B),
                        b'0'..=b'7' => {
                            // 1–3 octal digits, first already consumed.
                            let mut v = (esc - b'0') as u32;
                            let mut n = 0;
                            while n < 2 {
                                match s.first() {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        s = &s[1..];
                                        n += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if v > 0xFF {
                                return Err(UnescapeError::BadEscape("octal escape out of range"));
                            }
                            out.push(v as u8);
                        }
                        b'x' => {
                            // 1–2 hex digits.
                            let (v, consumed) = take_hex(s, 2);
                            if consumed == 0 {
                                return Err(UnescapeError::BadEscape("invalid \\x escape"));
                            }
                            s = &s[consumed..];
                            out.push(v as u8);
                        }
                        b'u' => {
                            let (cp, n) = take_hex(s, 4);
                            if n != 4 {
                                return Err(UnescapeError::BadEscape("invalid \\u escape"));
                            }
                            s = &s[4..];
                            // Textproto's \u is BMP-only — surrogate codepoints are
                            // rejected outright (no UTF-16 pair recombination). For
                            // non-BMP, use \U00010437 etc.
                            if (0xD800..0xE000).contains(&cp) {
                                return Err(UnescapeError::BadEscape(
                                    "\\u escape is surrogate; use \\U for non-BMP",
                                ));
                            }
                            push_utf8(&mut out, cp)?;
                        }
                        b'U' => {
                            let (cp, n) = take_hex(s, 8);
                            if n != 8 {
                                return Err(UnescapeError::BadEscape("invalid \\U escape"));
                            }
                            s = &s[8..];
                            push_utf8(&mut out, cp)?;
                        }
                        _ => return Err(UnescapeError::BadEscape("unrecognised escape sequence")),
                    }
                }
                Some(&c) => {
                    out.push(c);
                    s = &s[1..];
                }
            }
        }
        // After a closing quote: skip whitespace, then check for another quote.
        while let Some(&c) = s.first() {
            if super::token::is_textproto_ws(c) {
                s = &s[1..];
            } else {
                break;
            }
        }
        if !matches!(s.first(), Some(b'"') | Some(b'\'')) {
            break;
        }
    }
    Ok(out)
}

/// Unescape a textproto string token and validate as UTF-8.
///
/// Borrows the input when the token is a single literal with no escapes and
/// no adjacent concatenation — the common case for identifiers and short
/// strings. Otherwise allocates.
///
/// # Errors
///
/// As [`unescape`], plus [`UnescapeError::InvalidUtf8`] if the unescaped
/// bytes are not a valid UTF-8 sequence.
pub fn unescape_str(raw: &str) -> Result<Cow<'_, str>, UnescapeError> {
    // Fast path: single literal, ASCII quote, no escapes, no adjacent concat.
    // Scan once for anything that would force owned output.
    let bytes = raw.as_bytes();
    if let Some(&quote) = bytes.first() {
        if (quote == b'"' || quote == b'\'') && bytes.len() >= 2 {
            let inner = &bytes[1..];
            let mut i = 0;
            while i < inner.len() {
                let b = inner[i];
                if b == quote {
                    // Found closing quote. If nothing follows, we can borrow.
                    // Trailing whitespace is fine (tokenizer may include it).
                    let tail = &inner[i + 1..];
                    if tail.iter().all(|&c| super::token::is_textproto_ws(c)) {
                        // The original `raw` is &str, so this slice is valid
                        // UTF-8 by construction (no escapes present means no
                        // byte-level rewriting happened).
                        return Ok(Cow::Borrowed(&raw[1..1 + i]));
                    }
                    break;
                }
                if b == b'\\' || b == b'\n' || b == 0 {
                    break;
                }
                i += 1;
            }
        }
    }
    // Slow path.
    let owned = unescape(raw)?;
    alloc::string::String::from_utf8(owned)
        .map(Cow::Owned)
        .map_err(|_| UnescapeError::InvalidUtf8)
}

/// Consume up to `max` hex digits from the front of `s`, returning the
/// accumulated value and the count of digits consumed.
#[inline]
fn take_hex(s: &[u8], max: usize) -> (u32, usize) {
    let mut v: u32 = 0;
    let mut n = 0;
    while n < max && n < s.len() {
        let d = match s[n] {
            c @ b'0'..=b'9' => c - b'0',
            c @ b'a'..=b'f' => c - b'a' + 10,
            c @ b'A'..=b'F' => c - b'A' + 10,
            _ => break,
        };
        v = (v << 4) | d as u32;
        n += 1;
    }
    (v, n)
}

/// Push the UTF-8 encoding of `cp` onto `out`.
#[inline]
fn push_utf8(out: &mut Vec<u8>, cp: u32) -> Result<(), UnescapeError> {
    let c = char::from_u32(cp).ok_or(UnescapeError::BadEscape("invalid unicode code point"))?;
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    Ok(())
}

/// Write `bytes` as a double-quoted textproto string literal to `w`.
///
/// Printable ASCII passes through unchanged except for `"` and `\`, which are
/// escaped. All other bytes (control characters, 0x7F, high-bit bytes) are
/// emitted as `\NNN` three-digit octal escapes. This produces pure-ASCII
/// output regardless of the input bytes.
///
/// # Errors
///
/// Propagates [`core::fmt::Error`] from the underlying writer.
pub fn escape_bytes<W: Write + ?Sized>(bytes: &[u8], w: &mut W) -> core::fmt::Result {
    w.write_char('"')?;
    for &b in bytes {
        match b {
            b'"' => w.write_str("\\\"")?,
            b'\\' => w.write_str("\\\\")?,
            b'\n' => w.write_str("\\n")?,
            b'\r' => w.write_str("\\r")?,
            b'\t' => w.write_str("\\t")?,
            0x20..=0x7E => w.write_char(b as char)?,
            _ => {
                // Three-digit octal. Always three digits so the next char
                // can't be misread as continuing the escape.
                w.write_char('\\')?;
                w.write_char((b'0' + (b >> 6)) as char)?;
                w.write_char((b'0' + ((b >> 3) & 7)) as char)?;
                w.write_char((b'0' + (b & 7)) as char)?;
            }
        }
    }
    w.write_char('"')
}

/// Write `s` as a double-quoted textproto string literal to `w`, preserving
/// multi-byte UTF-8 codepoints as-is.
///
/// Control characters (`< 0x20`), `"`, `\`, and DEL (0x7F) are escaped.
/// Everything else — including non-ASCII codepoints — passes through
/// unmodified. Output is therefore UTF-8, not pure ASCII.
///
/// # Errors
///
/// Propagates [`core::fmt::Error`] from the underlying writer.
pub fn escape_str<W: Write + ?Sized>(s: &str, w: &mut W) -> core::fmt::Result {
    w.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => w.write_str("\\\"")?,
            '\\' => w.write_str("\\\\")?,
            '\n' => w.write_str("\\n")?,
            '\r' => w.write_str("\\r")?,
            '\t' => w.write_str("\\t")?,
            '\x00'..='\x1F' | '\x7F' => {
                // Octal escape for control chars.
                let b = c as u8;
                w.write_char('\\')?;
                w.write_char((b'0' + (b >> 6)) as char)?;
                w.write_char((b'0' + ((b >> 3) & 7)) as char)?;
                w.write_char((b'0' + (b & 7)) as char)?;
            }
            _ => w.write_char(c)?,
        }
    }
    w.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    // ── unescape ────────────────────────────────────────────────────────────

    #[test]
    fn unescape_table() {
        // Some(bytes) = success, None = error.
        #[rustfmt::skip]
        let cases: &[(&str, Option<&[u8]>)] = &[
            (r#""hello""#,             Some(b"hello")),
            (r#"'hello'"#,             Some(b"hello")),
            (r#""""#,                  Some(b"")),
            (r#""\n""#,                Some(b"\n")),
            (r#""\r\t""#,              Some(b"\r\t")),
            (r#""\"\\\'""#,            Some(b"\"\\'")),
            (r#""\?""#,                Some(b"?")),
            (r#""\a\b\f\v""#,          Some(&[0x07, 0x08, 0x0C, 0x0B])),
            (r#""\0""#,                Some(&[0x00])),
            (r#""\7""#,                Some(&[0x07])),
            (r#""\77""#,               Some(&[0o77])),       // 63
            (r#""\377""#,              Some(&[0xFF])),       // 255
            (r#""\1234""#,             Some(&[0o123, b'4'])),// 3 digits max, then literal
            (r#""\x41""#,              Some(b"A")),
            (r#""\xa""#,               Some(&[0x0A])),       // 1 hex digit ok
            (r#""\xFF""#,              Some(&[0xFF])),
            (r#""\u0041""#,            Some(b"A")),
            (r#""\u00e9""#,            Some("é".as_bytes())),
            (r#""\U0001F600""#,        Some("😀".as_bytes())),
            (r#""foo" "bar""#,         Some(b"foobar")),        // adjacent concat
            (r#""foo"'bar'"baz""#,     Some(b"foobarbaz")),     // no ws required
            (r#""a"  "b""#,            Some(b"ab")),            // ws between is ok
            // errors:
            (r#""unterminated"#,       None),
            (r#""\"#,                  None),  // lone backslash
            (r#""\400""#,              None),  // octal > 255
            (r#""\x""#,                None),  // \x with no digits
            (r#""\u00""#,              None),  // \u needs 4
            (r#""\U0000""#,            None),  // \U needs 8
            (r#""\uD800""#,            None),  // surrogate (BMP-only: use \U)
            (r#""\uDC00""#,            None),  // surrogate
            (r#""\uD83D\uDE00""#,      None),  // even a well-formed pair — no JSON-style recombination
            (r#""\z""#,                None),  // unknown escape
            ("\"line\nbreak\"",        None),  // raw newline
        ];
        for &(input, expected) in cases {
            let got = unescape(input).ok();
            assert_eq!(got.as_deref(), expected, "input: {input:?}");
        }
    }

    #[test]
    fn unescape_str_borrows_when_trivial() {
        let got = unescape_str(r#""hello""#).unwrap();
        assert!(matches!(got, Cow::Borrowed("hello")));
    }

    #[test]
    fn unescape_str_owns_when_escaped() {
        let got = unescape_str(r#""hel\nlo""#).unwrap();
        assert!(matches!(got, Cow::Owned(_)));
        assert_eq!(got, "hel\nlo");
    }

    #[test]
    fn unescape_str_owns_when_concatenated() {
        let got = unescape_str(r#""foo" "bar""#).unwrap();
        assert!(matches!(got, Cow::Owned(_)));
        assert_eq!(got, "foobar");
    }

    #[test]
    fn unescape_str_borrows_with_trailing_ws() {
        // Tokenizer may hand over a raw span with trailing whitespace.
        let got = unescape_str("\"hello\"  ").unwrap();
        assert!(matches!(got, Cow::Borrowed("hello")));
    }

    #[test]
    fn unescape_str_rejects_bad_utf8() {
        // \xFF is a valid textproto byte escape but not a valid UTF-8 byte.
        assert!(unescape(r#""\xFF""#).is_ok());
        assert!(unescape_str(r#""\xFF""#).is_err());
    }

    // ── escape ──────────────────────────────────────────────────────────────

    #[test]
    fn escape_bytes_table() {
        #[rustfmt::skip]
        let cases: &[(&[u8], &str)] = &[
            (b"hello",        r#""hello""#),
            (b"",             r#""""#),
            (b"\n\r\t",       r#""\n\r\t""#),
            (b"\"\\",         r#""\"\\""#),
            (&[0x00],         r#""\000""#),
            (&[0x01],         r#""\001""#),
            (&[0x7F],         r#""\177""#),
            (&[0xFF],         r#""\377""#),
            (&[0x80, 0x81],   r#""\200\201""#),   // high bytes → octal
            (b"a'b",          r#""a'b""#),        // ' not escaped under "
        ];
        for &(input, want) in cases {
            let mut out = String::new();
            escape_bytes(input, &mut out).unwrap();
            assert_eq!(out, want, "input: {input:?}");
        }
    }

    #[test]
    fn escape_str_preserves_unicode() {
        let mut out = String::new();
        escape_str("café 😀", &mut out).unwrap();
        assert_eq!(out, r#""café 😀""#);
    }

    #[test]
    fn escape_str_escapes_controls() {
        let mut out = String::new();
        escape_str("\x01\x7F", &mut out).unwrap();
        assert_eq!(out, r#""\001\177""#);
    }

    #[test]
    fn escape_bytes_vs_str_non_ascii() {
        // escape_bytes treats each byte of "é" (0xC3 0xA9) as a raw byte → octal.
        // escape_str treats it as one codepoint → passthrough.
        let mut a = String::new();
        let mut b = String::new();
        escape_bytes("é".as_bytes(), &mut a).unwrap();
        escape_str("é", &mut b).unwrap();
        assert_eq!(a, r#""\303\251""#);
        assert_eq!(b, r#""é""#);
    }

    #[test]
    fn escape_unescape_roundtrip() {
        // escape_bytes output is always valid unescape input.
        #[rustfmt::skip]
        let cases: &[&[u8]] = &[
            b"",
            b"plain",
            b"\x00\x01\x02\xFD\xFE\xFF",
            b"mix\nall\"the\\things\x7F",
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
        ];
        for &input in cases {
            let mut escaped = String::new();
            escape_bytes(input, &mut escaped).unwrap();
            let back = unescape(&escaped).unwrap();
            assert_eq!(back, input, "escaped form: {escaped}");
        }
    }
}
