//! Textproto (text format) encoding and decoding.
//!
//! The protobuf text format is a human-readable debug representation:
//!
//! ```text
//! name: "Alice"
//! id: 42
//! address {
//!   street: "1 High St"
//!   city: "London"
//! }
//! tags: "a"
//! tags: "b"
//! ```
//!
//! It is **not** a stable wire format — the spec permits implementations to
//! vary whitespace, field ordering, and float formatting. Use binary or JSON
//! for interchange. Textproto is for config files, golden-file tests, and
//! logging.
//!
//! # Usage
//!
//! Generated message types implement [`TextFormat`] when the `text` feature
//! is enabled in codegen. The convenience functions cover the common cases:
//!
//! ```ignore
//! use buffa::text::{encode_to_string, decode_from_str};
//!
//! let s = encode_to_string(&my_msg);
//! let parsed: MyMsg = decode_from_str(&s)?;
//! ```
//!
//! For streaming or reuse, use [`TextEncoder`] / [`TextDecoder`] directly.
//!
//! # `no_std`
//!
//! Fully `no_std` + `alloc`. No external dependencies beyond the runtime
//! crate itself.

mod decoder;
mod encoder;
mod error;
mod string;
mod token;

pub use decoder::TextDecoder;
pub use encoder::TextEncoder;
pub use error::{ParseError, ParseErrorKind};
pub use string::{escape_bytes, escape_str, unescape, unescape_str, UnescapeError};
pub use token::{NameKind, ScalarKind, Token, TokenKind, Tokenizer};

use alloc::string::String;

/// Textproto serialization for a [`Message`](crate::Message) type.
///
/// Implemented by generated code when the `text` feature is enabled in
/// codegen. Hand-implementation is possible but — as with
/// [`Message`](crate::Message) itself — discouraged outside of tests.
///
/// The trait is deliberately minimal: one method per direction, taking a
/// mutable encoder/decoder. The encoder/decoder own all formatting and
/// parsing state; the impl just dispatches on field names.
pub trait TextFormat: crate::Message {
    /// Write this message's fields to `enc`.
    ///
    /// Implementations call [`TextEncoder::write_field_name`] followed by
    /// the appropriate `write_*` value method for each set field. Unset
    /// singular fields and empty repeated fields are skipped.
    ///
    /// # Errors
    ///
    /// Propagates [`core::fmt::Error`] from the encoder's sink. When writing
    /// to a `String`, this is always `Ok`.
    fn encode_text(&self, enc: &mut TextEncoder<'_>) -> core::fmt::Result;

    /// Merge fields from `dec` into this message.
    ///
    /// Implementations loop on [`TextDecoder::read_field_name`] until it
    /// returns `None`, dispatching each name to the matching `read_*` call.
    /// Unknown names either [`skip_value`](TextDecoder::skip_value) or fail
    /// with [`unknown_field`](TextDecoder::unknown_field), depending on
    /// codegen configuration.
    ///
    /// # Errors
    ///
    /// Any [`ParseError`] from the decoder.
    fn merge_text(&mut self, dec: &mut TextDecoder<'_>) -> Result<(), ParseError>;
}

/// Encode a message as a single-line textproto string.
///
/// Fields are separated by single spaces; no trailing newline.
#[must_use]
pub fn encode_to_string<M: TextFormat>(msg: &M) -> String {
    let mut out = String::new();
    let mut enc = TextEncoder::new(&mut out);
    // Writing to a String never fails.
    let _ = msg.encode_text(&mut enc);
    out
}

/// Encode a message as a multi-line textproto string with 2-space indent.
///
/// One field per line; nested messages are indented. A trailing newline is
/// appended when the output is non-empty, matching `txtpbfmt` and POSIX
/// text-file conventions.
#[must_use]
pub fn encode_to_string_pretty<M: TextFormat>(msg: &M) -> String {
    let mut out = String::new();
    let mut enc = TextEncoder::new_pretty(&mut out);
    let _ = msg.encode_text(&mut enc);
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Decode a message from a textproto string.
///
/// # Errors
///
/// Any [`ParseError`] — syntax errors, unknown fields (if the generated
/// `merge_text` is strict), type mismatches, or depth limit exceeded.
pub fn decode_from_str<M: TextFormat + Default>(s: &str) -> Result<M, ParseError> {
    let mut msg = M::default();
    merge_from_str(&mut msg, s)?;
    Ok(msg)
}

/// Merge a textproto string into an existing message.
///
/// Proto merge semantics: scalar fields are overwritten, repeated fields are
/// appended, message fields are recursively merged.
///
/// # Errors
///
/// As [`decode_from_str`].
pub fn merge_from_str<M: TextFormat>(msg: &mut M, s: &str) -> Result<(), ParseError> {
    let mut dec = TextDecoder::new(s);
    msg.merge_text(&mut dec)
}

#[cfg(test)]
mod map_entry_tests {
    use super::*;

    // Quick sanity check that the closure-taking map-entry methods on
    // TextEncoder / TextDecoder produce and consume the expected syntax.
    // The real exercise is in buffa-test via generated code on Inventory.

    #[test]
    fn encode_map_entry() {
        let mut s = String::new();
        let mut enc = TextEncoder::new(&mut s);
        enc.write_field_name("m").unwrap();
        let k = 42i32;
        let v = "hello";
        enc.write_map_entry(|enc| {
            enc.write_field_name("key")?;
            enc.write_i32(k)?;
            enc.write_field_name("value")?;
            enc.write_string(v)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(s, r#"m {key: 42 value: "hello"}"#);
    }

    #[test]
    fn decode_map_entry() {
        let mut dec = TextDecoder::new(r#"m { key: 42 value: "hello" }"#);
        assert_eq!(dec.read_field_name().unwrap(), Some("m"));
        let mut k: Option<i32> = None;
        let mut v: Option<String> = None;
        dec.merge_map_entry(|d| {
            while let Some(n) = d.read_field_name()? {
                match n {
                    "key" => k = Some(d.read_i32()?),
                    "value" => v = Some(d.read_string()?.into_owned()),
                    _ => d.skip_value()?,
                }
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(k, Some(42));
        assert_eq!(v.as_deref(), Some("hello"));
    }
}
