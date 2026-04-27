//! Textproto decoder.
//!
//! Wraps a [`Tokenizer`] and adds type-directed value interpretation. A
//! generated `merge_text` implementation calls
//! [`read_field_name`](TextDecoder::read_field_name) in a loop, dispatches on
//! the returned name, and calls the appropriate `read_*` method for that
//! field's type.

use alloc::borrow::Cow;
use alloc::vec::Vec;

use super::error::{ParseError, ParseErrorKind};
use super::string::{unescape, unescape_str, UnescapeError};
use super::token::{
    lex_number, number_for_parse, NumKind, ScalarKind, Token, TokenKind, Tokenizer,
};

/// Stateful textproto reader.
///
/// Drives a [`Tokenizer`] and interprets scalar tokens as the requested Rust
/// type. Recursion depth is enforced by the tokenizer's fixed-size open-stack
/// (see [`RECURSION_LIMIT`](crate::RECURSION_LIMIT)).
pub struct TextDecoder<'a> {
    tok: Tokenizer<'a>,
    /// Byte position of the last name returned by `read_field_name`, for
    /// [`unknown_field`](Self::unknown_field) error reporting.
    last_name_pos: usize,
}

impl<'a> TextDecoder<'a> {
    /// Create a decoder over `input`.
    pub fn new(input: &'a str) -> Self {
        Self {
            tok: Tokenizer::new(input),
            last_name_pos: 0,
        }
    }

    /// Construct a parse error at a token's position.
    #[inline]
    fn err_at(&self, tok: &Token<'_>, kind: ParseErrorKind) -> ParseError {
        let (line, col) = self.tok.line_col(tok.pos);
        ParseError::new(line, col, kind)
    }

    /// Read the next field name, or return `None` at end-of-message / EOF.
    ///
    /// The returned slice is borrowed directly from the input. For bracketed
    /// type names (`[pkg.ext]`) it includes the brackets — generated code
    /// matches against `"[pkg.ext]"` literally.
    ///
    /// Does **not** validate that a `:` separator was present; the colon is
    /// optional before message values, so the check is deferred to after the
    /// caller dispatches on the name.
    ///
    /// # Errors
    ///
    /// Any tokenizer error — malformed name, delimiter mismatch, etc.
    pub fn read_field_name(&mut self) -> Result<Option<&'a str>, ParseError> {
        let tok = self.tok.peek()?;
        match tok.kind {
            TokenKind::Eof | TokenKind::MessageClose => Ok(None),
            TokenKind::Name => {
                self.tok.read()?;
                self.last_name_pos = tok.pos;
                Ok(Some(tok.raw))
            }
            _ => Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: "field name",
                },
            )),
        }
    }

    /// Construct an unknown-field error pointing at the last name returned
    /// by [`read_field_name`](Self::read_field_name).
    ///
    /// Generated `merge_text` wildcard arms use this when they want to fail
    /// fast on unknown fields rather than [`skip_value`](Self::skip_value).
    pub fn unknown_field(&self) -> ParseError {
        let (line, col) = self.tok.line_col(self.last_name_pos);
        ParseError::new(line, col, ParseErrorKind::UnknownField)
    }

    /// Read a scalar token, checking it has the right kind.
    fn read_scalar(&mut self, want: ScalarKind) -> Result<Token<'a>, ParseError> {
        let tok = self.tok.read()?;
        if tok.kind != TokenKind::Scalar || tok.scalar_kind != want {
            return Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: match want {
                        ScalarKind::Number => "number",
                        ScalarKind::String => "string literal",
                        ScalarKind::Literal => "identifier",
                    },
                },
            ));
        }
        Ok(tok)
    }

    /// Parse a number token's text as an integer. Shared by all integer readers.
    ///
    /// Strips hex/oct prefixes and dispatches to the appropriate radix parser.
    /// Rejects floats and (for unsigned) negatives.
    fn parse_int<T>(
        &self,
        tok: &Token<'_>,
        signed: bool,
        from_dec: fn(&str) -> Option<T>,
        from_radix: fn(&str, u32) -> Option<T>,
    ) -> Result<T, ParseError> {
        let num = lex_number(tok.raw.as_bytes())
            .ok_or_else(|| self.err_at(tok, ParseErrorKind::InvalidNumber))?;
        if num.kind == NumKind::Float {
            return Err(self.err_at(tok, ParseErrorKind::InvalidNumber));
        }
        if !signed && num.neg {
            return Err(self.err_at(tok, ParseErrorKind::InvalidNumber));
        }
        let cow = number_for_parse(tok.raw, &num);
        let s: &str = &cow;
        let parsed = match num.kind {
            NumKind::Dec => from_dec(s),
            NumKind::Hex | NumKind::Oct => {
                // Strip `-` (if any), strip the base prefix, then parse with
                // the radix. For signed negative hex/oct, re-prepend `-` —
                // `i32::from_str_radix("-1F", 16)` works.
                let (neg, rest) = match s.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, s),
                };
                let (radix, digits) = if num.kind == NumKind::Hex {
                    let d = rest
                        .strip_prefix("0x")
                        .or_else(|| rest.strip_prefix("0X"))
                        .ok_or_else(|| self.err_at(tok, ParseErrorKind::InvalidNumber))?;
                    (16, d)
                } else {
                    // Oct: lex only flags Oct when there IS a leading 0 and
                    // at least one more digit.
                    (8, rest.strip_prefix('0').unwrap_or(rest))
                };
                if !neg {
                    from_radix(digits, radix)
                } else {
                    // Stitch `-` + digits for signed radix parse.
                    let mut tmp = alloc::string::String::with_capacity(1 + digits.len());
                    tmp.push('-');
                    tmp.push_str(digits);
                    from_radix(&tmp, radix)
                }
            }
            NumKind::Float => unreachable!("rejected above"),
        };
        parsed.ok_or_else(|| self.err_at(tok, ParseErrorKind::InvalidNumber))
    }

    /// Read an `i32` value.
    ///
    /// Accepts decimal, `0x` hex, `0` octal. Rejects floats and out-of-range.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::InvalidNumber`] if not a valid in-range integer, or
    /// [`ParseErrorKind::UnexpectedToken`] if the value is not a number at all.
    pub fn read_i32(&mut self) -> Result<i32, ParseError> {
        let tok = self.read_scalar(ScalarKind::Number)?;
        self.parse_int(
            &tok,
            true,
            |s| s.parse().ok(),
            |s, r| i32::from_str_radix(s, r).ok(),
        )
    }

    /// Read an `i64` value.
    ///
    /// # Errors
    ///
    /// As [`read_i32`](Self::read_i32).
    pub fn read_i64(&mut self) -> Result<i64, ParseError> {
        let tok = self.read_scalar(ScalarKind::Number)?;
        self.parse_int(
            &tok,
            true,
            |s| s.parse().ok(),
            |s, r| i64::from_str_radix(s, r).ok(),
        )
    }

    /// Read a `u32` value. Rejects negatives.
    ///
    /// # Errors
    ///
    /// As [`read_i32`](Self::read_i32).
    pub fn read_u32(&mut self) -> Result<u32, ParseError> {
        let tok = self.read_scalar(ScalarKind::Number)?;
        self.parse_int(
            &tok,
            false,
            |s| s.parse().ok(),
            |s, r| u32::from_str_radix(s, r).ok(),
        )
    }

    /// Read a `u64` value. Rejects negatives.
    ///
    /// # Errors
    ///
    /// As [`read_i32`](Self::read_i32).
    pub fn read_u64(&mut self) -> Result<u64, ParseError> {
        let tok = self.read_scalar(ScalarKind::Number)?;
        self.parse_int(
            &tok,
            false,
            |s| s.parse().ok(),
            |s, r| u64::from_str_radix(s, r).ok(),
        )
    }

    /// Read an `f32` value.
    ///
    /// Accepts any numeric form plus the case-insensitive literals `nan`,
    /// `inf`, `infinity`, each optionally with a leading `-`. Overflow
    /// saturates to ±∞ (matching C++ text-format behaviour).
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::InvalidNumber`] if the token is neither a number nor
    /// a recognised float literal.
    pub fn read_f32(&mut self) -> Result<f32, ParseError> {
        self.read_f64().map(|v| v as f32)
    }

    /// Read an `f64` value. See [`read_f32`](Self::read_f32).
    ///
    /// # Errors
    ///
    /// As [`read_f32`](Self::read_f32).
    pub fn read_f64(&mut self) -> Result<f64, ParseError> {
        let tok = self.tok.read()?;
        if tok.kind != TokenKind::Scalar {
            return Err(self.err_at(&tok, ParseErrorKind::UnexpectedToken { expected: "number" }));
        }
        match tok.scalar_kind {
            ScalarKind::Literal => {
                // nan, inf, infinity, -inf, -infinity (case-insensitive).
                // trim_start: the tokenizer accepts `- inf` with whitespace
                // between the sign and the literal, so the raw span may
                // contain it.
                let (neg, lit) = match tok.raw.strip_prefix('-') {
                    Some(r) => (true, r.trim_start()),
                    None => (false, tok.raw),
                };
                let v = if lit.eq_ignore_ascii_case("nan") {
                    f64::NAN
                } else if lit.eq_ignore_ascii_case("inf") || lit.eq_ignore_ascii_case("infinity") {
                    f64::INFINITY
                } else {
                    return Err(self.err_at(&tok, ParseErrorKind::InvalidNumber));
                };
                Ok(if neg { -v } else { v })
            }
            ScalarKind::Number => {
                let num = lex_number(tok.raw.as_bytes())
                    .ok_or_else(|| self.err_at(&tok, ParseErrorKind::InvalidNumber))?;
                match num.kind {
                    NumKind::Dec | NumKind::Float => {
                        // Rust's f64 parse saturates to ±∞ on overflow and to
                        // ±0.0 on underflow (since 1.55), both of which are
                        // the behaviours the textproto spec requires. The sign
                        // is preserved: `"-0"` and `"-1e-400"` parse to -0.0.
                        number_for_parse(tok.raw, &num)
                            .parse::<f64>()
                            .map_err(|_| self.err_at(&tok, ParseErrorKind::InvalidNumber))
                    }
                    NumKind::Hex | NumKind::Oct => {
                        // The textproto spec's FLOAT production is base-10
                        // only; `0x1` and `01` are not float literals. The
                        // conformance suite explicitly tests rejection.
                        Err(self.err_at(&tok, ParseErrorKind::InvalidNumber))
                    }
                }
            }
            ScalarKind::String => {
                Err(self.err_at(&tok, ParseErrorKind::UnexpectedToken { expected: "number" }))
            }
        }
    }

    /// Read a `bool` value.
    ///
    /// Accepts `true`, `True`, `t`, `false`, `False`, `f`, and `0`/`1`
    /// (in any integer base). These are the exact literals the C++ text
    /// parser accepts.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::UnexpectedToken`] if the token is not a recognised
    /// boolean form.
    pub fn read_bool(&mut self) -> Result<bool, ParseError> {
        let tok = self.tok.read()?;
        if tok.kind != TokenKind::Scalar {
            return Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: "boolean",
                },
            ));
        }
        match tok.scalar_kind {
            ScalarKind::Literal => match tok.raw {
                "true" | "True" | "t" => Ok(true),
                "false" | "False" | "f" => Ok(false),
                _ => Err(self.err_at(
                    &tok,
                    ParseErrorKind::UnexpectedToken {
                        expected: "boolean",
                    },
                )),
            },
            ScalarKind::Number => {
                // 0/1 in any base (00, 0x1, 01 all accepted by C++).
                let n: u64 = self.parse_int(
                    &tok,
                    false,
                    |s| s.parse().ok(),
                    |s, r| u64::from_str_radix(s, r).ok(),
                )?;
                match n {
                    0 => Ok(false),
                    1 => Ok(true),
                    _ => Err(self.err_at(
                        &tok,
                        ParseErrorKind::UnexpectedToken {
                            expected: "boolean",
                        },
                    )),
                }
            }
            ScalarKind::String => Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: "boolean",
                },
            )),
        }
    }

    /// Read a `string` value. Unescapes and UTF-8-validates.
    ///
    /// Borrows the input when the token is a single literal with no escapes.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::InvalidString`] for malformed escapes,
    /// [`ParseErrorKind::InvalidUtf8`] if the unescaped bytes are not valid
    /// UTF-8.
    pub fn read_string(&mut self) -> Result<Cow<'a, str>, ParseError> {
        let tok = self.read_scalar(ScalarKind::String)?;
        unescape_str(tok.raw).map_err(|e| {
            self.err_at(
                &tok,
                match e {
                    UnescapeError::InvalidUtf8 => ParseErrorKind::InvalidUtf8,
                    UnescapeError::BadEscape(why) => ParseErrorKind::InvalidString(why),
                },
            )
        })
    }

    /// Read a `bytes` value. Unescapes but does not UTF-8-validate.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::InvalidString`] for malformed escapes.
    pub fn read_bytes(&mut self) -> Result<Vec<u8>, ParseError> {
        let tok = self.read_scalar(ScalarKind::String)?;
        unescape(tok.raw).map_err(|e| {
            // unescape (byte-level) never returns InvalidUtf8.
            let UnescapeError::BadEscape(why) = e else {
                unreachable!("unescape is byte-level")
            };
            self.err_at(&tok, ParseErrorKind::InvalidString(why))
        })
    }

    /// Read an enum token into `(i32_value, had_known_name, token)`.
    ///
    /// Shared by the open and closed enum read methods. For a named variant,
    /// looks up via [`Enumeration::from_proto_name`](crate::Enumeration::from_proto_name)
    /// and `had_known_name` is true. For a numeric form, parses the i32 and
    /// `had_known_name` is false. The caller decides whether unknown numbers
    /// are acceptable.
    fn read_enum_inner<E: crate::Enumeration>(
        &mut self,
    ) -> Result<(i32, bool, Token<'a>), ParseError> {
        let tok = self.tok.read()?;
        if tok.kind != TokenKind::Scalar {
            return Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: "enum value",
                },
            ));
        }
        match tok.scalar_kind {
            ScalarKind::Literal if !tok.raw.starts_with('-') => E::from_proto_name(tok.raw)
                .map(|e| (e.to_i32(), true, tok))
                .ok_or_else(|| self.err_at(&tok, ParseErrorKind::UnknownEnumValue)),
            ScalarKind::Number => {
                let n = self.parse_int(
                    &tok,
                    true,
                    |s| s.parse().ok(),
                    |s, r| i32::from_str_radix(s, r).ok(),
                )?;
                Ok((n, false, tok))
            }
            _ => Err(self.err_at(
                &tok,
                ParseErrorKind::UnexpectedToken {
                    expected: "enum value",
                },
            )),
        }
    }

    /// Read an enum value by variant name or by number (open-enum semantics).
    ///
    /// Returns the `i32` wire value. Any in-range integer is accepted — the
    /// proto3 open-enum model preserves unknown numeric values.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::UnknownEnumValue`] if the name is not a known
    /// variant; [`ParseErrorKind::InvalidNumber`] if a numeric form is out
    /// of i32 range.
    pub fn read_enum_by_name<E: crate::Enumeration>(&mut self) -> Result<i32, ParseError> {
        self.read_enum_inner::<E>().map(|(n, _, _)| n)
    }

    /// Read a closed-enum value by variant name or by number.
    ///
    /// Returns the enum variant directly. Unknown numeric values are
    /// **rejected** — the proto2 closed-enum model does not accept values
    /// outside the defined set in text format. (Binary decode routes them to
    /// unknown fields; text format has no analogous mechanism, so it errors.)
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::UnknownEnumValue`] if the name is not a known
    /// variant, **or** if a numeric form does not map to a defined variant.
    pub fn read_closed_enum_by_name<E: crate::Enumeration>(&mut self) -> Result<E, ParseError> {
        let (n, by_name, tok) = self.read_enum_inner::<E>()?;
        if by_name {
            // from_proto_name already succeeded; from_i32 on its to_i32()
            // is infallible. Avoid a second name lookup.
            return Ok(E::from_i32(n).expect("from_proto_name returned a valid variant"));
        }
        E::from_i32(n).ok_or_else(|| self.err_at(&tok, ParseErrorKind::UnknownEnumValue))
    }

    /// Enter a `{` or `<`, merge into `msg`, then consume the matching close.
    ///
    /// # Errors
    ///
    /// Any tokenizer or `merge_text` error, including
    /// [`ParseErrorKind::RecursionLimitExceeded`] if nesting exceeds
    /// [`RECURSION_LIMIT`](crate::RECURSION_LIMIT).
    pub fn merge_message<M: super::TextFormat>(&mut self, msg: &mut M) -> Result<(), ParseError> {
        self.merge_map_entry(|dec| msg.merge_text(dec))
    }

    /// Consume `{` or `<`, run `f` over the body, then consume the close.
    ///
    /// Closure-taking counterpart to [`merge_message`](Self::merge_message) —
    /// same rationale as [`TextEncoder::write_map_entry`]. Generated
    /// `map<K, V>` decode captures `&mut Option<K>` / `&mut Option<V>` and
    /// dispatches on `"key"` / `"value"` inside the closure without naming a
    /// concrete entry type.
    ///
    /// `#[doc(hidden)]` — codegen support, not public API.
    ///
    /// # Errors
    ///
    /// Any tokenizer or `f` error.
    #[doc(hidden)]
    pub fn merge_map_entry(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<(), ParseError>,
    ) -> Result<(), ParseError> {
        let open = self.tok.read()?;
        if open.kind != TokenKind::MessageOpen {
            return Err(self.err_at(
                &open,
                ParseErrorKind::UnexpectedToken {
                    expected: "'{' or '<'",
                },
            ));
        }
        f(self)?;
        let close = self.tok.read()?;
        if close.kind != TokenKind::MessageClose {
            return Err(self.err_at(
                &close,
                ParseErrorKind::UnexpectedToken {
                    expected: "'}' or '>'",
                },
            ));
        }
        Ok(())
    }

    /// Read one-or-more values into `out`.
    ///
    /// Handles both repeated-scalar forms: `f: [1, 2, 3]` (consumes `[` and
    /// `]`) and `f: 1` (reads exactly one element). Generated code calls this
    /// once per `f` occurrence; the `f: 1 f: 2` form is handled by the outer
    /// `read_field_name` loop seeing `f` twice.
    ///
    /// # Errors
    ///
    /// Any error from `read_one` or the tokenizer.
    pub fn read_repeated_into<T>(
        &mut self,
        out: &mut Vec<T>,
        mut read_one: impl FnMut(&mut Self) -> Result<T, ParseError>,
    ) -> Result<(), ParseError> {
        if self.tok.peek()?.kind == TokenKind::ListOpen {
            self.tok.read()?; // consume `[`
            if self.tok.peek()?.kind == TokenKind::ListClose {
                self.tok.read()?; // empty: `[]`
                return Ok(());
            }
            loop {
                out.push(read_one(self)?);
                // The tokenizer consumes the comma between elements. When
                // there are no more elements, `ListClose` is next.
                if self.tok.peek()?.kind == TokenKind::ListClose {
                    self.tok.read()?;
                    return Ok(());
                }
            }
        }
        // Single-value form.
        out.push(read_one(self)?);
        Ok(())
    }

    /// Parse an `Any`-expansion body: the `[type_url] { fields }` form.
    ///
    /// `name` is the bracketed name as returned by
    /// [`read_field_name`](Self::read_field_name), e.g.
    /// `"[type.googleapis.com/pkg.Foo]"`. The brackets are stripped here and
    /// the result is looked up in the global text-format `Any` map (installed
    /// via [`set_type_registry`]); the registered `text_merge` then consumes
    /// the `{ ... }` body and re-encodes to wire bytes suitable for `Any.value`.
    ///
    /// Returns `(stripped_url, value_bytes)`.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::UnknownField`] if the URL is not registered — this
    /// matches `AnyFieldWithInvalidType` in the conformance suite, which
    /// expects parse failure on an unknown URL.
    ///
    /// [`set_type_registry`]: crate::type_registry::set_type_registry
    pub fn read_any_expansion(&mut self, name: &'a str) -> Result<(&'a str, Vec<u8>), ParseError> {
        // read_field_name only returns bracketed names for NameKind::TypeName;
        // a missing bracket here means the caller dispatched wrong.
        // trim: the grammar permits whitespace inside brackets, and
        // `Token.raw` is a slice of the input so it preserves that.
        let url = name
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .map(str::trim)
            .ok_or_else(|| self.unknown_field())?;
        let entry =
            crate::type_registry::global_text_any(url).ok_or_else(|| self.unknown_field())?;
        let bytes = (entry.text_merge)(self)?;
        Ok((url, bytes))
    }

    /// Parse an extension bracket body: the `[pkg.ext] { ... }` form.
    ///
    /// `name` is the bracketed name as returned by
    /// [`read_field_name`](Self::read_field_name). The brackets are stripped
    /// and the result is looked up by `full_name` in the global text-format
    /// extension map (installed via [`set_type_registry`]); the registered
    /// `text_merge` consumes the value and produces unknown-field records at
    /// the extension's field number.
    ///
    /// # Errors
    ///
    /// [`ParseErrorKind::UnknownField`] if the name is not registered or the
    /// registered entry extends a different message. Strict by default —
    /// protobuf-go's `prototext` behaviour, and what the
    /// `GroupFieldExtensionGroupName` conformance test expects.
    ///
    /// [`set_type_registry`]: crate::type_registry::set_type_registry
    pub fn read_extension(
        &mut self,
        name: &str,
        extendee: &str,
    ) -> Result<Vec<crate::UnknownField>, ParseError> {
        let full = name
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .map(str::trim)
            .ok_or_else(|| self.unknown_field())?;
        let Some(entry) = crate::type_registry::global_text_ext_by_name(full) else {
            return Err(self.unknown_field());
        };
        if entry.extendee != extendee {
            return Err(self.unknown_field());
        }
        (entry.text_merge)(self, entry.number)
    }

    /// Consume a field's value without interpreting it.
    ///
    /// Used for unknown fields when the caller wants to skip rather than
    /// fail. Handles scalars, messages, and lists (recursively).
    ///
    /// # Errors
    ///
    /// Any tokenizer error in the skipped span.
    pub fn skip_value(&mut self) -> Result<(), ParseError> {
        let tok = self.tok.peek()?;
        match tok.kind {
            TokenKind::Scalar => {
                self.tok.read()?;
                Ok(())
            }
            TokenKind::MessageOpen => {
                self.tok.read()?;
                // read_field_name returns None at MessageClose — loop until then.
                while self.read_field_name()?.is_some() {
                    self.skip_value()?;
                }
                // Consume the MessageClose.
                let close = self.tok.read()?;
                debug_assert_eq!(close.kind, TokenKind::MessageClose);
                Ok(())
            }
            TokenKind::ListOpen => {
                self.tok.read()?;
                loop {
                    if self.tok.peek()?.kind == TokenKind::ListClose {
                        self.tok.read()?;
                        return Ok(());
                    }
                    self.skip_value()?;
                }
            }
            _ => Err(self.err_at(&tok, ParseErrorKind::UnexpectedToken { expected: "value" })),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::encoder::TextEncoder;
    use super::super::{decode_from_str, encode_to_string, encode_to_string_pretty, TextFormat};
    use super::*;
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec;

    // ── test enum ───────────────────────────────────────────────────────────

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Color {
        Red,
        Green,
        Blue,
    }

    impl crate::Enumeration for Color {
        fn from_i32(v: i32) -> Option<Self> {
            match v {
                0 => Some(Color::Red),
                1 => Some(Color::Green),
                2 => Some(Color::Blue),
                _ => None,
            }
        }
        fn to_i32(&self) -> i32 {
            match self {
                Color::Red => 0,
                Color::Green => 1,
                Color::Blue => 2,
            }
        }
        fn proto_name(&self) -> &'static str {
            match self {
                Color::Red => "RED",
                Color::Green => "GREEN",
                Color::Blue => "BLUE",
            }
        }
        fn from_proto_name(name: &str) -> Option<Self> {
            match name {
                "RED" => Some(Color::Red),
                "GREEN" => Some(Color::Green),
                "BLUE" => Some(Color::Blue),
                _ => None,
            }
        }
    }

    // ── test message ────────────────────────────────────────────────────────
    //
    // Hand-implemented to avoid depending on codegen in the runtime crate's
    // own tests. Mirrors the pattern in `extension.rs` tests.

    #[derive(Default, Clone, PartialEq, Debug)]
    struct TestMsg {
        i: i32,
        s: String,
        items: Vec<i64>,
        child: Option<Box<TestMsg>>,
    }

    impl crate::DefaultInstance for TestMsg {
        fn default_instance() -> &'static Self {
            static INST: crate::__private::OnceBox<TestMsg> = crate::__private::OnceBox::new();
            INST.get_or_init(|| Box::new(TestMsg::default()))
        }
    }

    impl crate::Message for TestMsg {
        fn compute_size(&self) -> u32 {
            0
        }
        fn write_to(&self, _buf: &mut impl bytes::BufMut) {}
        fn merge_field(
            &mut self,
            tag: crate::encoding::Tag,
            buf: &mut impl bytes::Buf,
            _depth: u32,
        ) -> Result<(), crate::DecodeError> {
            crate::encoding::skip_field(tag, buf)
        }
        fn cached_size(&self) -> u32 {
            0
        }
        fn clear(&mut self) {
            *self = Self::default();
        }
    }

    impl TextFormat for TestMsg {
        fn encode_text(&self, enc: &mut TextEncoder<'_>) -> core::fmt::Result {
            if self.i != 0 {
                enc.write_field_name("i")?;
                enc.write_i32(self.i)?;
            }
            if !self.s.is_empty() {
                enc.write_field_name("s")?;
                enc.write_string(&self.s)?;
            }
            for &item in &self.items {
                enc.write_field_name("items")?;
                enc.write_i64(item)?;
            }
            if let Some(child) = &self.child {
                enc.write_field_name("child")?;
                enc.write_message(child.as_ref())?;
            }
            Ok(())
        }

        fn merge_text(&mut self, dec: &mut TextDecoder<'_>) -> Result<(), ParseError> {
            while let Some(name) = dec.read_field_name()? {
                match name {
                    "i" => self.i = dec.read_i32()?,
                    "s" => self.s = dec.read_string()?.into_owned(),
                    "items" => dec.read_repeated_into(&mut self.items, |d| d.read_i64())?,
                    "child" => {
                        let child = self.child.get_or_insert_with(Default::default);
                        dec.merge_message(child.as_mut())?;
                    }
                    _ => dec.skip_value()?,
                }
            }
            Ok(())
        }
    }

    // ── scalar read tables ──────────────────────────────────────────────────

    #[test]
    fn read_i32_table() {
        #[rustfmt::skip]
        let cases: &[(&str, Option<i32>)] = &[
            ("42",          Some(42)),
            ("-7",          Some(-7)),
            ("0",           Some(0)),
            ("0x1F",        Some(31)),
            ("0X1f",        Some(31)),
            ("-0x10",       Some(-16)),
            ("0777",        Some(511)),
            ("-010",        Some(-8)),
            ("2147483647",  Some(i32::MAX)),
            ("-2147483648", Some(i32::MIN)),
            // errors:
            ("2147483648",  None),  // overflow
            ("1.5",         None),  // float
            ("1f",          None),  // float suffix
        ];
        for &(input, want) in cases {
            // Wrap in a field so the tokenizer is in value-expecting state.
            let full = alloc::format!("f: {input}");
            let mut d = TextDecoder::new(&full);
            d.read_field_name().unwrap();
            assert_eq!(d.read_i32().ok(), want, "input: {input}");
        }
    }

    #[test]
    fn read_u32_rejects_negative() {
        let mut d = TextDecoder::new("f: -1");
        d.read_field_name().unwrap();
        assert_eq!(d.read_u32().ok(), None);
    }

    #[test]
    fn read_u64_table() {
        #[rustfmt::skip]
        let cases: &[(&str, Option<u64>)] = &[
            ("f: 0",                       Some(0)),
            ("f: 18446744073709551615",    Some(u64::MAX)),
            ("f: 0xFFFFFFFFFFFFFFFF",      Some(u64::MAX)),
            ("f: 18446744073709551616",    None),  // overflow
            ("f: -1",                      None),  // negative
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            assert_eq!(d.read_u64().ok(), want, "input: {input}");
        }
    }

    #[test]
    fn read_f64_table() {
        // Floats can't use Option<f64> + == (NaN), so check classes.
        #[rustfmt::skip]
        let cases: &[(&str, &str)] = &[
            ("f: 1.5",        "1.5"),
            ("f: -2.5",       "-2.5"),
            ("f: .5",         "0.5"),
            ("f: 1e3",        "1000"),
            ("f: 1.5f",       "1.5"),     // f suffix stripped
            ("f: 42",         "42"),      // int → float
            ("f: 0",          "0"),       // plain zero (Dec, not Oct)
            ("f: 0.0",        "0"),
            ("f: 0e0",        "0"),
            ("f: inf",        "inf"),
            ("f: -inf",       "-inf"),
            ("f: - inf",      "-inf"),    // whitespace after sign
            ("f: -\tinf",     "-inf"),
            ("f: infinity",   "inf"),
            ("f: Infinity",   "inf"),
            ("f: -INFINITY",  "-inf"),
            ("f: nan",        "nan"),
            ("f: NaN",        "nan"),
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            let v = d
                .read_f64()
                .unwrap_or_else(|e| panic!("input {input}: {e}"));
            let got = if v.is_nan() {
                String::from("nan")
            } else if v.is_infinite() {
                String::from(if v > 0.0 { "inf" } else { "-inf" })
            } else {
                alloc::format!("{v}")
            };
            assert_eq!(got, want, "input: {input}");
        }
    }

    #[test]
    fn read_f64_negative_zero() {
        // -0.0 must be preserved through the parse path. `f64::from_str`
        // handles this correctly for `"-0"`, `"-0.0"`, and underflow like
        // `"-1e-400"` — the sign bit survives. This test guards against
        // any future integer-parse-then-cast shortcut that would lose it.
        //
        // `-0.0 == 0.0` is true in IEEE float comparison, so assertions
        // must check the bit pattern (or sign) explicitly.
        #[rustfmt::skip]
        let cases: &[&str] = &[
            "-0",        // NumKind::Dec
            "-0.0",      // NumKind::Float
            "-0f",       // NumKind::Float, f suffix
            "-0F",       // NumKind::Float, F suffix
            "-0.0f",
            "-0e0",      // NumKind::Float, exponent form
            "-1e-400",   // underflow → -0.0
            "- 0",       // whitespace between sign and digits
        ];
        for &input in cases {
            let full = alloc::format!("f: {input}");
            let mut d = TextDecoder::new(&full);
            d.read_field_name().unwrap();
            let v = d.read_f64().unwrap_or_else(|e| panic!("{input}: {e}"));
            assert!(
                v == 0.0 && v.is_sign_negative(),
                "{input}: want -0.0, got {v:?} (bits {:#018x})",
                v.to_bits()
            );
        }
    }

    #[test]
    fn read_f32_negative_zero() {
        // f32 goes via read_f64 then `as f32`; the cast preserves the sign.
        let mut d = TextDecoder::new("f: -0");
        d.read_field_name().unwrap();
        let v = d.read_f32().unwrap();
        assert!(v == 0.0 && v.is_sign_negative());
        assert_eq!(v.to_bits(), (-0.0f32).to_bits());
    }

    #[test]
    fn read_f64_rejects_hex_and_octal() {
        // The textproto FLOAT grammar is base-10 only. Conformance tests
        // `FloatFieldNoHex`, `FloatFieldNoOctal` (and their negative forms)
        // require rejection.
        #[rustfmt::skip]
        let cases: &[&str] = &[
            "0x1",   "-0x1",   "0xFF",
            "01",    "-01",    "0777",
        ];
        for &input in cases {
            let full = alloc::format!("f: {input}");
            let mut d = TextDecoder::new(&full);
            d.read_field_name().unwrap();
            let err = d
                .read_f64()
                .expect_err(&alloc::format!("{input}: should reject hex/oct for float"));
            assert_eq!(err.kind, ParseErrorKind::InvalidNumber, "input: {input}");
        }
        // Sanity: these zero forms start with `0` but are valid floats
        // (Dec or Float, never Oct — lex_number only flags Oct when a
        // second octal digit follows).
        for &input in &["0", "0.0", "0e0", "0.", "0f"] {
            let full = alloc::format!("f: {input}");
            let mut d = TextDecoder::new(&full);
            d.read_field_name().unwrap();
            assert!(d.read_f64().is_ok(), "{input}: should be accepted");
        }
    }

    #[test]
    fn read_f64_rejects_non_float_literal() {
        let mut d = TextDecoder::new("f: hello");
        d.read_field_name().unwrap();
        assert!(d.read_f64().is_err());
    }

    #[test]
    fn read_bool_table() {
        #[rustfmt::skip]
        let cases: &[(&str, Option<bool>)] = &[
            ("f: true",   Some(true)),
            ("f: True",   Some(true)),
            ("f: t",      Some(true)),
            ("f: false",  Some(false)),
            ("f: False",  Some(false)),
            ("f: f",      Some(false)),
            ("f: 1",      Some(true)),
            ("f: 0",      Some(false)),
            ("f: 0x1",    Some(true)),
            ("f: 01",     Some(true)),   // octal 1
            // errors:
            ("f: 2",      None),
            ("f: yes",    None),
            ("f: TRUE",   None),  // not in C++'s accepted set
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            assert_eq!(d.read_bool().ok(), want, "input: {input}");
        }
    }

    #[test]
    fn read_string_table() {
        #[rustfmt::skip]
        let cases: &[(&str, Option<&str>)] = &[
            (r#"f: "hello""#,         Some("hello")),
            (r#"f: 'world'"#,         Some("world")),
            (r#"f: "say \"hi\"""#,    Some("say \"hi\"")),
            (r#"f: "foo" "bar""#,     Some("foobar")),
            (r#"f: """#,              Some("")),
            (r#"f: 42"#,              None),  // not a string
            (r#"f: "\xFF""#,          None),  // invalid UTF-8
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            let got = d.read_string().ok();
            assert_eq!(got.as_deref(), want, "input: {input}");
        }
    }

    #[test]
    fn read_bytes_accepts_non_utf8() {
        let mut d = TextDecoder::new(r#"f: "\xFF\x00\x01""#);
        d.read_field_name().unwrap();
        assert_eq!(d.read_bytes().unwrap(), vec![0xFF, 0x00, 0x01]);
    }

    #[test]
    fn read_enum_table() {
        #[rustfmt::skip]
        let cases: &[(&str, Option<i32>)] = &[
            ("f: RED",    Some(0)),
            ("f: GREEN",  Some(1)),
            ("f: BLUE",   Some(2)),
            ("f: 0",      Some(0)),     // numeric form
            ("f: 99",     Some(99)),    // unknown number still OK (open enum)
            ("f: PURPLE", None),        // unknown name → error
            ("f: -RED",   None),        // negative-prefixed name not an enum
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            assert_eq!(d.read_enum_by_name::<Color>().ok(), want, "input: {input}");
        }
    }

    #[test]
    fn read_enum_unknown_name_error_kind() {
        let mut d = TextDecoder::new("f: PURPLE");
        d.read_field_name().unwrap();
        let err = d.read_enum_by_name::<Color>().unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnknownEnumValue);
    }

    #[test]
    fn read_closed_enum_table() {
        // Closed enums (proto2 semantics) reject unknown NUMERIC values in
        // text format. Open enums accept them; see `read_enum_table` above
        // where `99` → `Some(99)`.
        #[rustfmt::skip]
        let cases: &[(&str, Option<Color>)] = &[
            ("f: RED",    Some(Color::Red)),
            ("f: GREEN",  Some(Color::Green)),
            ("f: 0",      Some(Color::Red)),     // numeric → known variant
            ("f: 2",      Some(Color::Blue)),
            ("f: 99",     None),                 // unknown number → error
            ("f: -1",     None),                 // negative → error
            ("f: PURPLE", None),                 // unknown name → error (same as open)
        ];
        for &(input, want) in cases {
            let mut d = TextDecoder::new(input);
            d.read_field_name().unwrap();
            let got = d.read_closed_enum_by_name::<Color>().ok();
            assert_eq!(got, want, "input: {input}");
        }
    }

    #[test]
    fn read_closed_enum_unknown_number_error_kind() {
        let mut d = TextDecoder::new("f: 99");
        d.read_field_name().unwrap();
        let err = d.read_closed_enum_by_name::<Color>().unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnknownEnumValue);
    }

    // ── repeated ────────────────────────────────────────────────────────────

    #[test]
    fn repeated_both_forms() {
        // `items: 1 items: 2 items: [3, 4] items: 5` → [1,2,3,4,5]
        let mut m = TestMsg::default();
        let mut d = TextDecoder::new("items: 1 items: 2 items: [3, 4] items: 5");
        m.merge_text(&mut d).unwrap();
        assert_eq!(m.items, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn repeated_empty_list() {
        let mut m = TestMsg::default();
        let mut d = TextDecoder::new("items: []");
        m.merge_text(&mut d).unwrap();
        assert_eq!(m.items, Vec::<i64>::new());
    }

    #[test]
    fn repeated_message_list() {
        // `[{...}, {...}]` — exercises ListOpen → MessageOpen in the tokenizer.
        // TestMsg uses `child: Option<_>` not `Vec<_>`, so use a Vec<TestMsg>
        // directly through read_repeated_into.
        let mut d = TextDecoder::new("f: [{i: 1}, {i: 2}, <i: 3>]");
        d.read_field_name().unwrap();
        let mut out: Vec<TestMsg> = Vec::new();
        d.read_repeated_into(&mut out, |d| {
            let mut m = TestMsg::default();
            d.merge_message(&mut m)?;
            Ok(m)
        })
        .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].i, 1);
        assert_eq!(out[1].i, 2);
        assert_eq!(out[2].i, 3);
    }

    #[test]
    fn merge_from_str_appends() {
        use super::super::merge_from_str;
        let mut m = TestMsg {
            i: 1,
            items: vec![10],
            ..Default::default()
        };
        merge_from_str(&mut m, "i: 2 items: 20").unwrap();
        assert_eq!(m.i, 2); // scalar overwritten
        assert_eq!(m.items, vec![10, 20]); // repeated appended
    }

    // ── roundtrip ───────────────────────────────────────────────────────────

    #[test]
    fn roundtrip_simple() {
        let orig = TestMsg {
            i: 42,
            s: String::from("hello"),
            items: vec![1, 2, 3],
            child: None,
        };
        let text = encode_to_string(&orig);
        assert_eq!(text, r#"i: 42 s: "hello" items: 1 items: 2 items: 3"#);
        let back: TestMsg = decode_from_str(&text).unwrap();
        assert_eq!(back, orig);
    }

    #[test]
    fn roundtrip_nested() {
        let orig = TestMsg {
            i: 1,
            s: String::new(),
            items: vec![],
            child: Some(Box::new(TestMsg {
                i: 2,
                s: String::from("inner"),
                items: vec![10, 20],
                child: None,
            })),
        };
        let text = encode_to_string(&orig);
        let back: TestMsg = decode_from_str(&text).unwrap();
        assert_eq!(back, orig);
    }

    #[test]
    fn roundtrip_pretty() {
        let orig = TestMsg {
            i: 1,
            s: String::new(),
            items: vec![],
            child: Some(Box::new(TestMsg {
                i: 2,
                s: String::new(),
                items: vec![],
                child: None,
            })),
        };
        let text = encode_to_string_pretty(&orig);
        assert_eq!(text, "i: 1\nchild {\n  i: 2\n}\n");
        let back: TestMsg = decode_from_str(&text).unwrap();
        assert_eq!(back, orig);
    }

    #[test]
    fn parse_angle_delimiters() {
        // `<` `>` in place of `{` `}`.
        let m: TestMsg = decode_from_str("i: 1 child < i: 2 >").unwrap();
        assert_eq!(m.i, 1);
        assert_eq!(m.child.unwrap().i, 2);
    }

    #[test]
    fn parse_canonical_cpp_output() {
        // What protoc --encode would produce for this message shape.
        let input = "i: 42\ns: \"hello\"\nitems: 1\nitems: 2\nchild {\n  i: 7\n}\n";
        let m: TestMsg = decode_from_str(input).unwrap();
        assert_eq!(m.i, 42);
        assert_eq!(m.s, "hello");
        assert_eq!(m.items, vec![1, 2]);
        assert_eq!(m.child.unwrap().i, 7);
    }

    #[test]
    fn parse_with_comments_and_separators() {
        let input = "# header\ni: 1, # inline\ns: \"x\"; # trailing\n";
        let m: TestMsg = decode_from_str(input).unwrap();
        assert_eq!(m.i, 1);
        assert_eq!(m.s, "x");
    }

    // ── skip_value ──────────────────────────────────────────────────────────

    #[test]
    fn skip_unknown_scalar() {
        let m: TestMsg = decode_from_str("unknown: 999 i: 42").unwrap();
        assert_eq!(m.i, 42);
    }

    #[test]
    fn skip_unknown_message() {
        let m: TestMsg = decode_from_str("unknown { x: 1 y: 2 } i: 42").unwrap();
        assert_eq!(m.i, 42);
    }

    #[test]
    fn skip_unknown_nested_message() {
        let m: TestMsg = decode_from_str("unknown { inner { deep: 1 } } i: 42").unwrap();
        assert_eq!(m.i, 42);
    }

    #[test]
    fn skip_unknown_list() {
        let m: TestMsg = decode_from_str("unknown: [1, 2, 3] i: 42").unwrap();
        assert_eq!(m.i, 42);
    }

    #[test]
    fn skip_unknown_message_list() {
        let m: TestMsg = decode_from_str("unknown: [{a: 1}, {a: 2}] i: 42").unwrap();
        assert_eq!(m.i, 42);
    }

    // ── errors ──────────────────────────────────────────────────────────────

    #[test]
    fn unknown_field_error() {
        // A merge_text impl that doesn't skip unknowns.
        #[derive(Default, Clone, PartialEq, Debug)]
        struct Strict {
            i: i32,
        }
        impl crate::DefaultInstance for Strict {
            fn default_instance() -> &'static Self {
                static I: crate::__private::OnceBox<Strict> = crate::__private::OnceBox::new();
                I.get_or_init(|| Box::new(Strict::default()))
            }
        }
        impl crate::Message for Strict {
            fn compute_size(&self) -> u32 {
                0
            }
            fn write_to(&self, _: &mut impl bytes::BufMut) {}
            fn merge_field(
                &mut self,
                t: crate::encoding::Tag,
                b: &mut impl bytes::Buf,
                _: u32,
            ) -> Result<(), crate::DecodeError> {
                crate::encoding::skip_field(t, b)
            }
            fn cached_size(&self) -> u32 {
                0
            }
            fn clear(&mut self) {}
        }
        impl TextFormat for Strict {
            fn encode_text(&self, _: &mut TextEncoder<'_>) -> core::fmt::Result {
                Ok(())
            }
            fn merge_text(&mut self, d: &mut TextDecoder<'_>) -> Result<(), ParseError> {
                while let Some(name) = d.read_field_name()? {
                    match name {
                        "i" => self.i = d.read_i32()?,
                        _ => return Err(d.unknown_field()),
                    }
                }
                Ok(())
            }
        }

        let err = decode_from_str::<Strict>("i: 1\nbad: 2").unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnknownField);
        assert_eq!(err.line, 2);
        assert_eq!(err.col, 1);
    }

    #[test]
    fn depth_limit_exceeded() {
        // Build a chain of `child { child { ... } }` past the limit.
        let depth = crate::message::RECURSION_LIMIT as usize + 1;
        let mut s = String::new();
        for _ in 0..depth {
            s.push_str("child { ");
        }
        for _ in 0..depth {
            s.push('}');
        }
        let err = decode_from_str::<TestMsg>(&s).unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::RecursionLimitExceeded);
    }

    #[test]
    fn error_position_in_value() {
        // Error on line 2, in the value.
        let err = decode_from_str::<TestMsg>("i: 1\ni: notanumber").unwrap_err();
        assert_eq!(err.line, 2);
        assert!(matches!(err.kind, ParseErrorKind::UnexpectedToken { .. }));
    }
}
