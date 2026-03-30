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

    /// Write `[type_url] { fields }` for a registered `Any` type, or do
    /// nothing and return `false` if the URL isn't registered.
    ///
    /// `true` means the expanded form was written — the caller skips its
    /// vanilla `type_url: "..." value: "..."` fallback. `false` means
    /// nothing was written — the caller should fall through.
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
        let Some(entry) = crate::type_registry::global_text_any(type_url) else {
            return Ok(false);
        };
        self.write_extension_name(type_url)?;
        (entry.text_encode)(value, self)?;
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

    /// Write an `f32` value. NaN → `nan`, infinities → `inf`/`-inf`.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the underlying writer.
    pub fn write_f32(&mut self, v: f32) -> core::fmt::Result {
        self.prepare(Last::Value)?;
        self.w.write_str(": ")?;
        write_float(self.w, v as f64)
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

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
