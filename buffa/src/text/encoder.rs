//! Textproto encoder.
//!
//! A thin wrapper around a [`core::fmt::Write`] sink that emits fields in
//! textproto syntax. Generated `encode_text` implementations call
//! [`write_field_name`](TextEncoder::write_field_name) followed by exactly one
//! `write_*` value method per field; the encoder tracks whether to emit a `:`
//! (scalars) or ` ` (messages) between them, and handles indentation in
//! pretty mode.
//!
//! Output uses `{` `}` for message delimiters. Single-line mode separates
//! fields with a single space; pretty mode puts each field on its own line
//! indented by two spaces per nesting level.

use core::fmt::Write;

use super::string::{escape_bytes, escape_str};
use crate::type_registry::MAX_ANY_EXPANSION_DEPTH;
use crate::unknown_fields::{UnknownFieldData, UnknownFields};

/// Depth cap for heuristically parsing length-delimited unknown fields as
/// nested messages. Matches C++ `kUnknownFieldRecursionLimit`
/// (text_format.cc:2306). Independent of the parse-time [`RECURSION_LIMIT`]
/// — this bounds an encode-time *printing* heuristic, not wire decode.
///
/// [`RECURSION_LIMIT`]: crate::RECURSION_LIMIT
const UNKNOWN_LD_RECURSE_BUDGET: u32 = 10;

/// What the encoder last emitted — drives separator logic in
/// [`prepare`](TextEncoder::prepare).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    /// Nothing yet, or just after an open brace.
    Open,
    /// A field name. Next write is the value.
    Name,
    /// A scalar value or a close brace. Next write starts a new field.
    Value,
}

/// Stateful textproto writer.
///
/// Writes to any [`core::fmt::Write`] — a `String`, an adapter over an
/// `io::Write`, etc. Holds no buffer of its own.
///
/// All `write_*` methods return [`core::fmt::Result`]; failure is only
/// possible if the underlying writer fails. When writing to a `String`, the
/// result is always `Ok`.
pub struct TextEncoder<'a> {
    w: &'a mut dyn Write,
    depth: u32,
    /// Nesting depth of `Any` expansions on the current path, bounded by
    /// [`MAX_ANY_EXPANSION_DEPTH`]. Separate from `depth`, which counts
    /// textproto braces for indentation and is not a bound.
    any_depth: u32,
    pretty: bool,
    emit_unknown: bool,
    last: Last,
}

impl<'a> TextEncoder<'a> {
    /// Create a single-line encoder: fields separated by spaces, no newlines.
    pub fn new(w: &'a mut dyn Write) -> Self {
        Self {
            w,
            depth: 0,
            any_depth: 0,
            pretty: false,
            emit_unknown: false,
            last: Last::Open,
        }
    }

    /// Create a multi-line encoder: one field per line, 2-space indent per
    /// nesting level.
    pub fn new_pretty(w: &'a mut dyn Write) -> Self {
        Self {
            w,
            depth: 0,
            any_depth: 0,
            pretty: true,
            emit_unknown: false,
            last: Last::Open,
        }
    }

    /// Enable printing of unknown fields (by field number). Off by default:
    /// unknowns are debug-only — the output may not roundtrip because field
    /// number and wire type aren't enough to determine the proto type.
    ///
    /// When off, [`write_unknown_fields`](Self::write_unknown_fields) is a
    /// no-op; generated `encode_text` impls call it unconditionally.
    #[must_use]
    pub fn emit_unknown(mut self, yes: bool) -> Self {
        self.emit_unknown = yes;
        self
    }

    /// Emit inter-token separator and indentation appropriate for the last
    /// thing written and the next thing about to be written.
    ///
    /// Reference: protobuf-go `encode.go` `prepareNext`.
    fn prepare(&mut self, next: Last) -> core::fmt::Result {
        let prev = self.last;
        self.last = next;
        if !self.pretty {
            // Single line: space between end-of-field and start of next name.
            if prev == Last::Value && next == Last::Name {
                self.w.write_char(' ')?;
            }
            return Ok(());
        }
        // Multi-line.
        match (prev, next) {
            (Last::Name, _) => {
                // Nothing: each scalar write_* emits `": "` and write_message
                // emits a space itself. Avoid doubling up.
            }
            (Last::Open, Last::Name) => {
                // First field after an open brace. At top level (depth 0) the
                // "open" is virtual — no brace was written — so no newline.
                if self.depth > 0 {
                    self.w.write_char('\n')?;
                    self.write_indent()?;
                }
            }
            (Last::Value, Last::Name) | (Last::Value, Last::Value) => {
                // Next field, or close brace dedenting below its contents.
                self.w.write_char('\n')?;
                self.write_indent()?;
            }
            (Last::Open, Last::Value) | (Last::Open, Last::Open) | (Last::Value, Last::Open) => {
                // Empty message body (`{}`) or nothing to separate.
            }
        }
        Ok(())
    }

    fn write_indent(&mut self) -> core::fmt::Result {
        for _ in 0..self.depth {
            self.w.write_str("  ")?;
        }
        Ok(())
    }

    /// Write a field name. The next `write_*` call supplies the value.
    ///
    /// Does **not** write a `:` — each `write_*` scalar method writes its own
    /// `": "`, and `write_message` writes `" "` before the `{`. This is the
    /// simplest way to implement the "colon required for scalars, optional
    /// for messages" rule.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_field_name(&mut self, name: &str) -> core::fmt::Result {
        self.prepare(Last::Name)?;
        self.w.write_str(name)
    }

    /// Write an extension field name wrapped in brackets: `[pkg.ext]`.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_extension_name(&mut self, name: &str) -> core::fmt::Result {
        self.prepare(Last::Name)?;
        self.w.write_char('[')?;
        self.w.write_str(name)?;
        self.w.write_char(']')
    }

    /// Write a nested message value: `{ ... }` (or `{}` if empty).
    ///
    /// Calls `msg.encode_text(self)` with `depth` incremented and `last`
    /// reset so the inner encoder starts fresh.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer, or from
    /// `msg.encode_text`.
    pub fn write_message<M: super::TextFormat>(&mut self, msg: &M) -> core::fmt::Result {
        self.write_map_entry(|enc| msg.encode_text(enc))
    }

    /// Write a `{ ... }` block via a closure instead of a [`TextFormat`] impl.
    ///
    /// Exists so generated map-entry encoding doesn't need a full
    /// [`Message`](crate::Message) type per `map<K, V>` field. [`TextFormat`]
    /// has a `Message` supertrait bound, and `Message` requires `Default +
    /// 'static + Clone + PartialEq + Send + Sync` — bounds that a
    /// closure-over-references adapter can't satisfy. Taking a closure
    /// directly here sidesteps the bound entirely.
    ///
    /// `#[doc(hidden)]` — codegen support, not public API.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer or from `f`.
    #[doc(hidden)]
    pub fn write_map_entry(
        &mut self,
        f: impl FnOnce(&mut Self) -> core::fmt::Result,
    ) -> core::fmt::Result {
        // No `:` before a message value — just a space before `{`.
        self.prepare(Last::Value)?;
        self.w.write_str(" {")?;
        self.depth += 1;
        let outer_last = self.last;
        self.last = Last::Open;
        f(self)?;
        // Close brace: dedent first.
        self.depth -= 1;
        if self.pretty && self.last != Last::Open {
            self.w.write_char('\n')?;
            self.write_indent()?;
        }
        self.last = outer_last;
        self.w.write_char('}')
    }

    /// Write `[type_url] { fields }` when the registry has an entry for
    /// `type_url` or for its message full name; otherwise write nothing and
    /// return `false`.
    ///
    /// `true` means the expanded form was written — the caller skips its
    /// vanilla `type_url: "..." value: "..."` fallback. `false` means
    /// nothing was written — the caller should fall through.
    ///
    /// The URL is written between the brackets as given, with no escaping,
    /// so it is expanded only when it is made of ASCII letters, digits and
    /// `-._~/:%`. Any other URL returns `false`, and the caller's quoted
    /// `type_url` field carries it exactly.
    ///
    /// Consults the text-format `Any` map installed via
    /// [`set_type_registry`](crate::type_registry::set_type_registry).
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn try_write_any_expanded(
        &mut self,
        type_url: &str,
        value: &[u8],
    ) -> Result<bool, core::fmt::Error> {
        // `type_url` is input data: a URL with any prefix resolves by its
        // message name. A `]` would end the bracketed name early and let the
        // rest of the URL be read as further fields, and whitespace or `#`
        // would not survive the trip back through the decoder.
        if !type_url.bytes().all(is_bracket_safe_url_byte) {
            return Ok(false);
        }
        let Some(entry) = crate::type_registry::global_text_any(type_url) else {
            return Ok(false);
        };
        // An `Any` whose payload is another `Any` re-enters here through the
        // registered encoder. Past the cap, fall back to the vanilla
        // `type_url`/`value` form: still valid textproto, and finite.
        if self.any_depth >= MAX_ANY_EXPANSION_DEPTH {
            return Ok(false);
        }
        self.write_extension_name(type_url)?;
        self.any_depth += 1;
        let r = (entry.text_encode)(value, self);
        self.any_depth -= 1;
        r?;
        Ok(true)
    }

    /// Write registered extensions from `fields` as `[full_name] { ... }`
    /// entries. Unregistered field numbers are left for the caller's
    /// [`write_unknown_fields`](Self::write_unknown_fields) (debug-only,
    /// default off).
    ///
    /// Called by generated `encode_text` on messages with extension ranges.
    /// Never a no-op in the way `write_unknown_fields` is — extensions in
    /// text format are part of the canonical output, not debug-only.
    ///
    /// Consults the text-format extension map installed via
    /// [`set_type_registry`](crate::type_registry::set_type_registry).
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_extension_fields(
        &mut self,
        extendee: &str,
        fields: &UnknownFields,
    ) -> core::fmt::Result {
        if fields.is_empty() {
            return Ok(());
        }
        // One emit per field number — the entry's text_encode reads all
        // records at that number (merge semantics). Mirrors JSON's
        // serialize_extensions dedup loop.
        let mut seen = alloc::collections::BTreeSet::new();
        for uf in fields.iter() {
            if !seen.insert(uf.number) {
                continue;
            }
            let Some(entry) = crate::type_registry::global_text_ext_by_number(extendee, uf.number)
            else {
                continue;
            };
            self.write_extension_name(entry.full_name)?;
            (entry.text_encode)(uf.number, fields, self)?;
        }
        Ok(())
    }

    /// Write a message's preserved unknown fields by field number.
    ///
    /// No-op unless [`emit_unknown`](Self::emit_unknown) was set. Generated
    /// `encode_text` impls call this unconditionally at the end of each
    /// message, after the known fields.
    ///
    /// Format per wire type (matches protobuf C++ `TextFormat::Printer::PrintUnknownFields`):
    ///
    /// | Wire type | Output | Example |
    /// |---|---|---|
    /// | varint | decimal | `1001: 42` |
    /// | fixed32 / fixed64 | hex | `1002: 0x3f800000` |
    /// | length-delimited | nested `{ }` if parseable, else bytes | `1003 { 1: 111 }` or `1003: "hello"` |
    /// | group | nested `{ ... }` (recursive) | `1004 { 1: 0 }` |
    ///
    /// Length-delimited bytes are speculatively parsed as wire-format records
    /// (same heuristic as C++ text_format.cc:2926 / Java TextFormat.java:87):
    /// if the parse succeeds, print nested; otherwise print as escaped bytes.
    /// Capped at 10 levels deep. False positives are possible — a string that
    /// happens to look like valid wire format will be printed as `{ }`.
    ///
    /// This is debug output: the parser can't round-trip it because wire type
    /// doesn't determine proto type (a varint could be int32, sint64, bool, an
    /// enum, …).
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_unknown_fields(&mut self, fields: &UnknownFields) -> core::fmt::Result {
        if !self.emit_unknown {
            return Ok(());
        }
        self.write_unknown_inner(fields, UNKNOWN_LD_RECURSE_BUDGET)
    }

    fn write_unknown_inner(&mut self, fields: &UnknownFields, budget: u32) -> core::fmt::Result {
        for f in fields.iter() {
            self.prepare(Last::Name)?;
            write!(self.w, "{}", f.number)?;
            match &f.data {
                UnknownFieldData::Varint(v) => {
                    self.prepare(Last::Value)?;
                    write!(self.w, ": {v}")?;
                }
                UnknownFieldData::Fixed32(v) => {
                    self.prepare(Last::Value)?;
                    write!(self.w, ": 0x{v:x}")?;
                }
                UnknownFieldData::Fixed64(v) => {
                    self.prepare(Last::Value)?;
                    write!(self.w, ": 0x{v:x}")?;
                }
                UnknownFieldData::LengthDelimited(bytes) => {
                    // Heuristic: try to parse the bytes as wire-format records.
                    // If it parses, it's probably a sub-message — print nested.
                    // If not (or we're out of budget), print as escaped bytes.
                    // Matches C++ text_format.cc:2926-2966 and Java
                    // TextFormat.java:87-102.
                    if budget > 0 && !bytes.is_empty() {
                        if let Ok(inner) = UnknownFields::decode_from_slice(bytes) {
                            self.write_map_entry(|enc| {
                                enc.write_unknown_inner(&inner, budget - 1)
                            })?;
                            continue;
                        }
                    }
                    self.prepare(Last::Value)?;
                    self.w.write_str(": ")?;
                    escape_bytes(bytes, self.w)?;
                }
                UnknownFieldData::Group(inner) => {
                    // Groups don't consume budget — they were already
                    // validated at decode time (C++ text_format.cc:3009).
                    self.write_map_entry(|enc| enc.write_unknown_inner(inner, budget))?;
                }
            }
        }
        Ok(())
    }

    // ── scalar writers ──────────────────────────────────────────────────────

    /// Write an `i32` value with a `": "` prefix.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_i32(&mut self, v: i32) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        write!(self.w, ": {v}")
    }

    /// Write an `i64` value with a `": "` prefix.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_i64(&mut self, v: i64) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        write!(self.w, ": {v}")
    }

    /// Write a `u32` value with a `": "` prefix.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_u32(&mut self, v: u32) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        write!(self.w, ": {v}")
    }

    /// Write a `u64` value with a `": "` prefix.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_u64(&mut self, v: u64) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        write!(self.w, ": {v}")
    }

    /// Write an `f32` value as the shortest decimal that reads back as the
    /// same `f32`, in positional notation (`0.1`, not `0.10000000149011612`).
    /// NaN is `nan` and the infinities are `inf` and `-inf`.
    ///
    /// Where a reader that parses an `f64` and narrows it would take those
    /// digits to a neighbouring `f32`, the value is written with its `f64`
    /// digits instead.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_f32(&mut self, v: f32) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        if v.is_finite() {
            let mut digits = F32Digits::new();
            if write!(digits, "{v}").is_ok() {
                // A reader that parses a `float` as an `f64` and narrows it,
                // as protobuf C++ and protobuf-go do, rounds twice. For two
                // values, ±7.038531e-26, that takes the shortest `f32` digits
                // one ulp away from `v`. They get the widened digits, which
                // every reader takes back to `v`.
                if let Some(short) = digits.as_str().filter(|s| narrows_to(s, v)) {
                    return self.w.write_str(short);
                }
            }
        }
        write_float(self.w, f64::from(v))
    }

    /// Write an `f64` value. NaN → `nan`, infinities → `inf`/`-inf`.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_f64(&mut self, v: f64) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        write_float(self.w, v)
    }

    /// Write a `bool` value: `true` or `false`.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_bool(&mut self, v: bool) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(if v { ": true" } else { ": false" })
    }

    /// Write a `string` value as a quoted literal. UTF-8 codepoints pass
    /// through as-is; control characters are escaped.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_string(&mut self, v: &str) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        escape_str(v, self.w)
    }

    /// Write a `bytes` value as a quoted literal with non-printable bytes
    /// escaped as octal `\NNN`.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_bytes(&mut self, v: &[u8]) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        escape_bytes(v, self.w)
    }

    /// Write an enum variant name as a bare identifier (no quotes).
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_enum_name(&mut self, name: &str) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        self.w.write_str(name)
    }

    /// Write an enum value as its numeric `i32`. Fallback for unknown
    /// variants.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_enum_number(&mut self, v: i32) -> core::fmt::Result {
        self.write_i32(v)
    }
}

/// The `Display` text of one finite `f32`, held on the stack.
struct F32Digits {
    buf: [u8; Self::CAPACITY],
    len: usize,
}

impl F32Digits {
    /// Buffer length in bytes. A finite `f32` prints as at most 48 bytes (a
    /// sign and `0.` before 45 decimal places, for example the negative
    /// subnormal nearest zero), so 64 leaves a margin. A longer text fails
    /// the write, and `write_f32` then prints the `f64` digits.
    const CAPACITY: usize = 64;

    const fn new() -> Self {
        Self {
            buf: [0; Self::CAPACITY],
            len: 0,
        }
    }

    fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(self.buf.get(..self.len)?).ok()
    }
}

impl Write for F32Digits {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        let dst = self.buf.get_mut(self.len..end).ok_or(core::fmt::Error)?;
        dst.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Whether `digits`, parsed as an `f64` and narrowed, is exactly `v`.
fn narrows_to(digits: &str, v: f32) -> bool {
    digits
        .parse::<f64>()
        .is_ok_and(|wide| (wide as f32).to_bits() == v.to_bits())
}

/// Write a float with textproto conventions for non-finites.
fn write_float(w: &mut dyn Write, v: f64) -> core::fmt::Result {
    if v.is_nan() {
        w.write_str("nan")
    } else if v.is_infinite() {
        if v > 0.0 {
            w.write_str("inf")
        } else {
            w.write_str("-inf")
        }
    } else {
        // Rust's default float Display uses the shortest round-trip
        // representation, which is what we want here.
        write!(w, "{v}")
    }
}

/// Whether `b` can stand in a type URL written between brackets and be read
/// back unchanged: an ASCII letter or digit, or one of `-._~/:%`.
fn is_bracket_safe_url_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':' | b'%')
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    #[test]
    fn bracket_safe_url_bytes() {
        let safe = "type.googleapis.com/pkg.Msg_1~-:%";
        assert!(safe.bytes().all(is_bracket_safe_url_byte));
        for unsafe_byte in *b"[]{}#\"' \t\n\\<>," {
            assert!(!is_bracket_safe_url_byte(unsafe_byte), "{unsafe_byte:#04x}");
        }
        assert!(!"é".bytes().any(is_bracket_safe_url_byte));
    }

    #[test]
    fn single_line_scalars() {
        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s);
        enc.write_field_name("a").unwrap();
        enc.write_i32(42).unwrap();
        enc.write_field_name("b").unwrap();
        enc.write_string("hello").unwrap();
        assert_eq!(s, r#"a: 42 b: "hello""#);
    }

    #[test]
    fn pretty_scalars() {
        let mut s = String::new();
        let mut enc = TextEncoder::new_pretty(&mut s);
        enc.write_field_name("a").unwrap();
        enc.write_i32(42).unwrap();
        enc.write_field_name("b").unwrap();
        enc.write_i32(7).unwrap();
        assert_eq!(s, "a: 42\nb: 7");
    }

    #[test]
    fn all_scalar_types() {
        #[rustfmt::skip]
        let cases: &[(&str, &str)] = &[
            ("i32",   "f: -7"),
            ("i64",   "f: 9000000000"),
            ("u32",   "f: 42"),
            ("u64",   "f: 18000000000000000000"),
            ("bool",  "f: true"),
            ("str",   r#"f: "hi""#),
            ("bytes", r#"f: "\377""#),
            ("enum",  "f: FOO_BAR"),
        ];
        for &(which, want) in cases {
            let mut s = String::new();
            let mut enc = TextEncoder::new(&mut s);
            enc.write_field_name("f").unwrap();
            match which {
                "i32" => enc.write_i32(-7).unwrap(),
                "i64" => enc.write_i64(9_000_000_000).unwrap(),
                "u32" => enc.write_u32(42).unwrap(),
                "u64" => enc.write_u64(18_000_000_000_000_000_000).unwrap(),
                "bool" => enc.write_bool(true).unwrap(),
                "str" => enc.write_string("hi").unwrap(),
                "bytes" => enc.write_bytes(&[0xFF]).unwrap(),
                "enum" => enc.write_enum_name("FOO_BAR").unwrap(),
                _ => unreachable!(),
            }
            assert_eq!(s, want, "type: {which}");
        }
    }

    #[test]
    fn float_specials() {
        #[rustfmt::skip]
        let cases: &[(f64, &str)] = &[
            (1.5,                "f: 1.5"),
            (0.0,                "f: 0"),
            (f64::NAN,           "f: nan"),
            (f64::INFINITY,      "f: inf"),
            (f64::NEG_INFINITY,  "f: -inf"),
        ];
        for &(v, want) in cases {
            let mut s = String::new();
            let mut enc = TextEncoder::new(&mut s);
            enc.write_field_name("f").unwrap();
            enc.write_f64(v).unwrap();
            assert_eq!(s, want, "value: {v}");
        }
    }

    fn f32_text(v: f32) -> String {
        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s);
        enc.write_field_name("f").unwrap();
        enc.write_f32(v).unwrap();
        s
    }

    #[test]
    fn f32_uses_its_own_shortest_digits() {
        #[rustfmt::skip]
        let cases: &[(f32, &str)] = &[
            (0.1,               "f: 0.1"),
            (-2.2,              "f: -2.2"),
            (1.5,               "f: 1.5"),
            (-0.0,              "f: -0"),
            // Nine significant digits, the most an `f32` needs.
            (10.000_010_5,      "f: 10.0000105"),
            (f32::MAX,          "f: 340282350000000000000000000000000000000"),
            (f32::NAN,          "f: nan"),
            (f32::NEG_INFINITY, "f: -inf"),
        ];
        for &(v, want) in cases {
            assert_eq!(f32_text(v), want, "value: {v}");
        }
    }

    #[test]
    fn f32_digits_hold_the_longest_values() {
        // No `f32` is longer than the negative subnormal nearest zero, 48
        // bytes. A value that outgrew the buffer would print its widened
        // digits.
        for v in [
            -f32::from_bits(1),
            -f32::MIN_POSITIVE,
            f32::MIN,
            -f32::from_bits(0x007f_ffff),
        ] {
            let short = alloc::format!("{v}");
            assert!(short.len() <= F32Digits::CAPACITY, "value: {v}");
            assert_eq!(f32_text(v), alloc::format!("f: {short}"), "value: {v}");
        }
        assert_eq!(alloc::format!("{}", -f32::from_bits(1)).len(), 48);
    }

    #[test]
    fn f32_digits_reject_text_past_the_capacity() {
        let mut digits = F32Digits::new();
        for _ in 0..F32Digits::CAPACITY {
            digits.write_str("1").unwrap();
        }
        assert!(digits.write_str("1").is_err());
        assert_eq!(digits.as_str().map(str::len), Some(F32Digits::CAPACITY));
    }

    #[test]
    fn f32_keeps_widened_digits_where_a_narrowing_reader_would_miss() {
        // The shortest digits of this value are `7.038531e-26`. Read as an
        // `f64` and narrowed, they give its neighbour 0x15ae_43fe.
        // `f32_fallback_values_exhaustive` shows that this value and its
        // negation are the only ones.
        for v in [f32::from_bits(0x15ae_43fd), -f32::from_bits(0x15ae_43fd)] {
            let short = alloc::format!("{v}");
            assert!(!narrows_to(&short, v));

            let text = f32_text(v);
            let digits = text.strip_prefix("f: ").unwrap();
            assert_ne!(digits, short);
            assert_eq!(digits, alloc::format!("{}", f64::from(v)));
            // Both kinds of reader take the widened digits back to `v`.
            assert!(narrows_to(digits, v));
            assert_eq!(digits.parse::<f32>().unwrap().to_bits(), v.to_bits());
        }
    }

    #[test]
    #[ignore = "visits every f32; run with --release"]
    fn f32_fallback_values_exhaustive() {
        let mut fallbacks = alloc::vec::Vec::new();
        for bits in 0..=u32::MAX {
            let v = f32::from_bits(bits);
            if !v.is_finite() {
                continue;
            }
            let mut digits = F32Digits::new();
            write!(digits, "{v}").unwrap();
            let short = digits.as_str().unwrap();
            assert_eq!(short.parse::<f32>().unwrap().to_bits(), bits);
            if !narrows_to(short, v) {
                fallbacks.push(bits);
            }
        }
        assert_eq!(fallbacks, [0x15ae_43fd, 0x95ae_43fd]);
    }

    #[test]
    fn extension_name() {
        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s);
        enc.write_extension_name("pkg.ext").unwrap();
        enc.write_i32(1).unwrap();
        assert_eq!(s, "[pkg.ext]: 1");
    }

    // write_message is exercised by the integration tests in decoder.rs,
    // which need a TextFormat-implementing struct.

    #[test]
    fn unknown_fields_default_noop() {
        use crate::unknown_fields::UnknownField;
        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 1001,
            data: UnknownFieldData::Varint(42),
        });

        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s);
        enc.write_unknown_fields(&fields).unwrap();
        assert_eq!(s, ""); // emit_unknown off → nothing
    }

    #[test]
    fn unknown_fields_all_wire_types() {
        use crate::unknown_fields::UnknownField;
        use alloc::vec;
        let mut group_inner = UnknownFields::new();
        group_inner.push(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(7),
        });

        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 1001,
            data: UnknownFieldData::Varint(42),
        });
        fields.push(UnknownField {
            number: 1002,
            data: UnknownFieldData::Fixed32(0x3F80_0000),
        });
        fields.push(UnknownField {
            number: 1003,
            data: UnknownFieldData::Fixed64(0xDEAD_BEEF),
        });
        fields.push(UnknownField {
            number: 1004,
            data: UnknownFieldData::LengthDelimited(vec![0x01, 0xFF]),
        });
        fields.push(UnknownField {
            number: 1005,
            data: UnknownFieldData::Group(group_inner),
        });

        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s).emit_unknown(true);
        enc.write_unknown_fields(&fields).unwrap();
        assert_eq!(
            s,
            r#"1001: 42 1002: 0x3f800000 1003: 0xdeadbeef 1004: "\001\377" 1005 {1: 7}"#
        );
    }

    #[test]
    fn unknown_fields_after_known_fields() {
        // The common case: generated encode_text writes known fields first,
        // then calls write_unknown_fields. Separator must be correct.
        use crate::unknown_fields::UnknownField;
        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 99,
            data: UnknownFieldData::Varint(1),
        });

        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s).emit_unknown(true);
        enc.write_field_name("known").unwrap();
        enc.write_i32(5).unwrap();
        enc.write_unknown_fields(&fields).unwrap();
        assert_eq!(s, "known: 5 99: 1");
    }

    #[test]
    fn unknown_ld_heuristic_prints_nested() {
        // [0x08, 0x6F] is field-1-varint-111 — the conformance test case.
        // Heuristic parses it and emits nested `{ 1: 111 }` instead of bytes.
        use crate::unknown_fields::UnknownField;
        use alloc::vec;
        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 1003,
            data: UnknownFieldData::LengthDelimited(vec![0x08, 0x6F]),
        });

        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s).emit_unknown(true);
        enc.write_unknown_fields(&fields).unwrap();
        assert_eq!(s, "1003 {1: 111}");
    }

    #[test]
    fn unknown_ld_heuristic_falls_back_to_bytes() {
        // Empty bytes and unparseable bytes both fall back to the escaped
        // string form — the heuristic only fires when decode_from_slice
        // succeeds on non-empty input.
        use crate::unknown_fields::UnknownField;
        #[rustfmt::skip]
        let cases: &[(&[u8], &str)] = &[
            (&[],               r#"1: """#),           // empty → never nested
            (&[0x01, 0xFF],     r#"1: "\001\377""#),   // field 0 / bad wire → bytes
            (&[0x08],           r#"1: "\010""#),       // truncated varint → bytes
            (b"hello",          r#"1: "hello""#),      // 0x6C = wire type 4 (end-group)
                                                       // without a start → parse error,
                                                       // so readable ASCII stays readable
        ];
        for &(bytes, want) in cases {
            let mut fields = UnknownFields::new();
            fields.push(UnknownField {
                number: 1,
                data: UnknownFieldData::LengthDelimited(bytes.to_vec()),
            });
            let mut s = String::new();
            let mut enc = TextEncoder::new(&mut s).emit_unknown(true);
            enc.write_unknown_fields(&fields).unwrap();
            assert_eq!(s, want, "bytes: {bytes:02X?}");
        }
    }

    #[test]
    fn unknown_ld_heuristic_budget_caps_depth() {
        // Nest a valid sub-message 12 levels deep (budget is 10). The 11th
        // level should print as bytes, not nested.
        use crate::unknown_fields::UnknownField;
        use alloc::vec;
        // innermost: field 1, varint 7 → [0x08, 0x07]
        let mut bytes = vec![0x08, 0x07];
        for _ in 0..12 {
            let len = bytes.len() as u8;
            let mut wrapper = vec![0x0A, len]; // field 1, LD, length
            wrapper.extend_from_slice(&bytes);
            bytes = wrapper;
        }
        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 99,
            data: UnknownFieldData::LengthDelimited(bytes),
        });

        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s).emit_unknown(true);
        enc.write_unknown_fields(&fields).unwrap();
        // 10 levels of `{` then a bytes fallback — count open braces.
        assert_eq!(s.matches('{').count(), 10, "output: {s}");
        assert!(
            s.contains(r#": ""#),
            "expected bytes fallback at floor: {s}"
        );
    }
}
