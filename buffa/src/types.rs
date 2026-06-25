//! Protobuf scalar type encoding strategies.
//!
//! Each protobuf scalar type has a specific wire encoding. This module provides
//! encode/decode functions for every scalar type, grouped by wire type.
//!
//! | Proto type   | Rust type | Wire type        | Encode fn            |
//! |-------------|-----------|------------------|----------------------|
//! | double      | f64       | Fixed64          | `encode_double`      |
//! | float       | f32       | Fixed32          | `encode_float`       |
//! | int32       | i32       | Varint           | `encode_int32`       |
//! | int64       | i64       | Varint           | `encode_int64`       |
//! | uint32      | u32       | Varint           | `encode_uint32`      |
//! | uint64      | u64       | Varint           | `encode_uint64`      |
//! | sint32      | i32       | Varint (zigzag)  | `encode_sint32`      |
//! | sint64      | i64       | Varint (zigzag)  | `encode_sint64`      |
//! | fixed32     | u32       | Fixed32          | `encode_fixed32`     |
//! | fixed64     | u64       | Fixed64          | `encode_fixed64`     |
//! | sfixed32    | i32       | Fixed32          | `encode_sfixed32`    |
//! | sfixed64    | i64       | Fixed64          | `encode_sfixed64`    |
//! | bool        | bool      | Varint           | `encode_bool`        |
//! | string      | String    | LengthDelimited  | `encode_string`      |
//! | bytes       | `Vec<u8>` | LengthDelimited  | `encode_bytes`       |
//!
//! Each `encode_<type>` also has a fused `put_<type>_field` sibling that
//! writes the field tag and the payload in one call (plus
//! [`put_len_delimited_header`] / [`put_group_start`] / [`put_group_end`]
//! for message and group framing). Generated `write_to` bodies use the
//! fused forms; the `encode_*` primitives remain the building blocks for
//! packed payloads and hand-written codecs.
//!
//! # Wire format reference
//!
//! Fixed-width types are encoded as little-endian bytes on the wire, per the
//! protobuf encoding specification:
//! <https://protobuf.dev/programming-guides/encoding/#non-varint_numbers>
//!
//! - Wire type 5 (Fixed32): `float`, `fixed32`, `sfixed32` — 4 bytes LE
//! - Wire type 1 (Fixed64): `double`, `fixed64`, `sfixed64` — 8 bytes LE
//!
//! Varint types (wire type 0) are encoded using the variable-length base-128
//! encoding. Signed types without zigzag (`int32`, `int64`) sign-extend to 64
//! bits before encoding; negative `int32` values therefore always occupy 10
//! bytes. Use `sint32`/`sint64` (zigzag) when negative values are common.
//! <https://protobuf.dev/programming-guides/encoding/#signed-ints>

use alloc::string::String;
use alloc::vec::Vec;
use bytes::{Buf, BufMut, Bytes};

use crate::encoding::{decode_varint, encode_varint, varint_len, Tag, WireType};
use crate::error::DecodeError;

/// Validate UTF-8 and return the borrowed `&str`.
///
/// With the `fast-utf8` feature (on by default) this dispatches to
/// [`smoothutf8::to_str`], which is faster than [`core::str::from_utf8`] on
/// the short ASCII strings typical of protobuf field values and (when `std`
/// is also enabled) delegates inputs of 128 bytes or more to `simdutf8`.
/// Without the feature, it is exactly `core::str::from_utf8`. Either way the
/// check is the full Unicode §3.9 well-formedness rule, so the
/// `from_utf8_unchecked` calls that consume the result are sound.
// The explicit `return`s are required by the mutually-exclusive `#[cfg]` arms.
#[allow(clippy::needless_return)]
#[inline(always)]
pub(crate) fn validate_str(bytes: &[u8]) -> Result<&str, DecodeError> {
    #[cfg(feature = "fast-utf8")]
    return smoothutf8::to_str(bytes).ok_or(DecodeError::InvalidUtf8);
    #[cfg(not(feature = "fast-utf8"))]
    return core::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8);
}

/// Validate `buf[..len]` as UTF-8, where `buf` is a surrounding wire-buffer
/// slice that may extend past the field.
///
/// When at least [`smoothutf8::SLACK`] readable bytes follow `len`
/// (`buf.len() >= len + SLACK`), this takes [`smoothutf8::verify_with_slack`],
/// which skips the per-string tail copy that dominates on inputs shorter than
/// 16 bytes. The slack check is a runtime branch; in a protobuf decode it is
/// almost always taken — only a string ending within `SLACK` bytes of the
/// buffer's end falls back to the safe path.
///
/// This is the per-field-conditional integration of `verify_with_slack`. The
/// preferred shape — pad the wire buffer once at ingest with `SLACK` zero
/// bytes and call `verify_with_slack` unconditionally — requires the caller
/// (typically connectrpc-rs) to append the padding before `.freeze()`; that
/// is a follow-up. Until then the per-field branch costs one `cmp/jbe` that
/// is taken for every field except one ending in the last `SLACK` bytes of
/// the buffer, so a 1-bit predictor handles it.
///
/// # Safety
///
/// The caller must have established `len <= buf.len()` (the EOF check that
/// precedes every call site does).
// The explicit `return`s are required by the mutually-exclusive `#[cfg]` arms.
#[allow(clippy::needless_return)]
#[inline(always)]
pub(crate) unsafe fn validate_str_in(buf: &[u8], len: usize) -> Result<&str, DecodeError> {
    debug_assert!(len <= buf.len());
    // SAFETY: caller established `len <= buf.len()`; that bound also makes
    // `len + SLACK` non-overflowing (`buf.len() <= isize::MAX`).
    let field = unsafe { buf.get_unchecked(..len) };
    #[cfg(feature = "fast-utf8")]
    {
        // `len + SLACK` cannot overflow: `len <= buf.len() <= isize::MAX`.
        let ok = if buf.len() >= len + smoothutf8::SLACK {
            // SAFETY: `len + SLACK <= buf.len()` was just checked above,
            // satisfying `verify_with_slack`'s precondition that
            // `range.end + SLACK` bytes of `buf` are readable; `0 <= len`
            // satisfies `range.start <= range.end`.
            unsafe { smoothutf8::verify_with_slack(buf, 0..len) }
        } else {
            smoothutf8::verify(field)
        };
        return if ok {
            // SAFETY: `verify`/`verify_with_slack` confirmed `field` is
            // well-formed UTF-8; their correctness is mechanically verified
            // against the same Unicode §3.9 rule as `core::str::from_utf8`.
            Ok(unsafe { core::str::from_utf8_unchecked(field) })
        } else {
            Err(DecodeError::InvalidUtf8)
        };
    }
    #[cfg(not(feature = "fast-utf8"))]
    return core::str::from_utf8(field).map_err(|_| DecodeError::InvalidUtf8);
}

// ---------------------------------------------------------------------------
// ZigZag encoding (for sint32/sint64)
// ---------------------------------------------------------------------------

/// ZigZag encode a signed 32-bit integer.
///
/// Maps signed integers to unsigned integers so that values with small
/// absolute value have small varint encodings:
///   0 → 0, -1 → 1, 1 → 2, -2 → 3, 2 → 4, ...
///
/// Spec: <https://protobuf.dev/programming-guides/encoding/#signed-ints>
#[inline]
pub(crate) fn zigzag_encode_i32(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)) as u32
}

/// ZigZag decode an unsigned 32-bit integer to signed.
#[inline]
pub(crate) fn zigzag_decode_i32(value: u32) -> i32 {
    ((value >> 1) as i32) ^ (-((value & 1) as i32))
}

/// ZigZag encode a signed 64-bit integer.
#[inline]
pub(crate) fn zigzag_encode_i64(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

/// ZigZag decode an unsigned 64-bit integer to signed.
#[inline]
pub(crate) fn zigzag_decode_i64(value: u64) -> i64 {
    ((value >> 1) as i64) ^ (-((value & 1) as i64))
}

// ---------------------------------------------------------------------------
// Varint types (wire type 0)
// ---------------------------------------------------------------------------

/// Encode an `int32` value as a varint (wire type 0).
///
/// Negative values are sign-extended to 64 bits before encoding, always
/// producing a 10-byte varint. Use [`encode_sint32`] when negative values are
/// common.
#[inline]
pub fn encode_int32(value: i32, buf: &mut impl BufMut) {
    // Sign-extend to i64 then reinterpret as u64 per the protobuf spec.
    encode_varint(value as i64 as u64, buf);
}

/// Decode an `int32` value from a varint.
///
/// The low 32 bits of the decoded varint are returned; upper bits are
/// truncated (consistent with the protobuf spec for 32-bit integer types).
#[inline]
pub fn decode_int32(buf: &mut impl Buf) -> Result<i32, DecodeError> {
    let v = decode_varint(buf)?;
    Ok(v as i32)
}

/// Encode an `int64` value as a varint (wire type 0).
#[inline]
pub fn encode_int64(value: i64, buf: &mut impl BufMut) {
    encode_varint(value as u64, buf);
}

/// Decode an `int64` value from a varint.
#[inline]
pub fn decode_int64(buf: &mut impl Buf) -> Result<i64, DecodeError> {
    let v = decode_varint(buf)?;
    Ok(v as i64)
}

/// Encode a `uint32` value as a varint (wire type 0).
#[inline]
pub fn encode_uint32(value: u32, buf: &mut impl BufMut) {
    encode_varint(value as u64, buf);
}

/// Decode a `uint32` value from a varint.
///
/// The low 32 bits of the decoded varint are returned; upper bits are
/// truncated.
#[inline]
pub fn decode_uint32(buf: &mut impl Buf) -> Result<u32, DecodeError> {
    let v = decode_varint(buf)?;
    Ok(v as u32)
}

/// Encode a `uint64` value as a varint (wire type 0).
#[inline]
pub fn encode_uint64(value: u64, buf: &mut impl BufMut) {
    encode_varint(value, buf);
}

/// Decode a `uint64` value from a varint.
#[inline]
pub fn decode_uint64(buf: &mut impl Buf) -> Result<u64, DecodeError> {
    decode_varint(buf)
}

/// Encode a `sint32` value as a varint using ZigZag encoding (wire type 0).
///
/// ZigZag maps signed integers to unsigned so that values with small absolute
/// magnitude produce short varints. Prefer this over [`encode_int32`] when the
/// field regularly holds negative values.
#[inline]
pub fn encode_sint32(value: i32, buf: &mut impl BufMut) {
    encode_varint(zigzag_encode_i32(value) as u64, buf);
}

/// Decode a `sint32` value from a ZigZag-encoded varint.
#[inline]
pub fn decode_sint32(buf: &mut impl Buf) -> Result<i32, DecodeError> {
    let v = decode_varint(buf)?;
    // Truncate to u32 before ZigZag decode (spec: upper bits are discarded
    // for 32-bit types).
    Ok(zigzag_decode_i32(v as u32))
}

/// Encode a `sint64` value as a varint using ZigZag encoding (wire type 0).
#[inline]
pub fn encode_sint64(value: i64, buf: &mut impl BufMut) {
    encode_varint(zigzag_encode_i64(value), buf);
}

/// Decode a `sint64` value from a ZigZag-encoded varint.
#[inline]
pub fn decode_sint64(buf: &mut impl Buf) -> Result<i64, DecodeError> {
    let v = decode_varint(buf)?;
    Ok(zigzag_decode_i64(v))
}

/// Encode a `bool` value as a varint (wire type 0).
///
/// `true` encodes to `0x01`, `false` to `0x00`.
#[inline]
pub fn encode_bool(value: bool, buf: &mut impl BufMut) {
    // Both values are < 0x80, so a single byte is a valid varint.
    buf.put_u8(value as u8);
}

/// Decode a `bool` value from a varint.
///
/// Per the protobuf spec, zero decodes as `false`; any non-zero value decodes
/// as `true`.
#[inline]
pub fn decode_bool(buf: &mut impl Buf) -> Result<bool, DecodeError> {
    let v = decode_varint(buf)?;
    Ok(v != 0)
}

// ---------------------------------------------------------------------------
// Encoded size helpers — varint types
// ---------------------------------------------------------------------------

/// Encoded length of an `int32` value in bytes.
///
/// Negative values sign-extend to 64 bits and always produce a 10-byte varint.
#[inline]
pub fn int32_encoded_len(value: i32) -> usize {
    varint_len(value as i64 as u64)
}

/// Encoded length of an `int64` value in bytes.
#[inline]
pub fn int64_encoded_len(value: i64) -> usize {
    varint_len(value as u64)
}

/// Encoded length of a `uint32` value in bytes.
#[inline]
pub fn uint32_encoded_len(value: u32) -> usize {
    varint_len(value as u64)
}

/// Encoded length of a `uint64` value in bytes.
#[inline]
pub fn uint64_encoded_len(value: u64) -> usize {
    varint_len(value)
}

/// Encoded length of a `sint32` value in bytes.
#[inline]
pub fn sint32_encoded_len(value: i32) -> usize {
    varint_len(zigzag_encode_i32(value) as u64)
}

/// Encoded length of a `sint64` value in bytes.
#[inline]
pub fn sint64_encoded_len(value: i64) -> usize {
    varint_len(zigzag_encode_i64(value))
}

/// Encoded size of a `bool` value: always 1 byte.
pub const BOOL_ENCODED_LEN: usize = 1;

// ---------------------------------------------------------------------------
// Fixed-width 32-bit types (wire type 5)
// ---------------------------------------------------------------------------

/// Encode a `float` value as 4 bytes little-endian (wire type 5).
#[inline]
pub fn encode_float(value: f32, buf: &mut impl BufMut) {
    buf.put_f32_le(value);
}

/// Decode a `float` value from 4 bytes little-endian.
#[inline]
pub fn decode_float(buf: &mut impl Buf) -> Result<f32, DecodeError> {
    if buf.remaining() < 4 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_f32_le())
}

/// Encode a `fixed32` value as 4 bytes little-endian (wire type 5).
#[inline]
pub fn encode_fixed32(value: u32, buf: &mut impl BufMut) {
    buf.put_u32_le(value);
}

/// Decode a `fixed32` value from 4 bytes little-endian.
#[inline]
pub fn decode_fixed32(buf: &mut impl Buf) -> Result<u32, DecodeError> {
    if buf.remaining() < 4 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_u32_le())
}

/// Encode an `sfixed32` value as 4 bytes little-endian (wire type 5).
///
/// `sfixed32` is a signed 32-bit integer stored as its two's complement
/// representation in little-endian order — identical bit pattern to `fixed32`.
#[inline]
pub fn encode_sfixed32(value: i32, buf: &mut impl BufMut) {
    buf.put_i32_le(value);
}

/// Decode an `sfixed32` value from 4 bytes little-endian.
#[inline]
pub fn decode_sfixed32(buf: &mut impl Buf) -> Result<i32, DecodeError> {
    if buf.remaining() < 4 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_i32_le())
}

// ---------------------------------------------------------------------------
// Fixed-width 64-bit types (wire type 1)
// ---------------------------------------------------------------------------

/// Encode a `double` value as 8 bytes little-endian (wire type 1).
#[inline]
pub fn encode_double(value: f64, buf: &mut impl BufMut) {
    buf.put_f64_le(value);
}

/// Decode a `double` value from 8 bytes little-endian.
#[inline]
pub fn decode_double(buf: &mut impl Buf) -> Result<f64, DecodeError> {
    if buf.remaining() < 8 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_f64_le())
}

/// Encode a `fixed64` value as 8 bytes little-endian (wire type 1).
#[inline]
pub fn encode_fixed64(value: u64, buf: &mut impl BufMut) {
    buf.put_u64_le(value);
}

/// Decode a `fixed64` value from 8 bytes little-endian.
#[inline]
pub fn decode_fixed64(buf: &mut impl Buf) -> Result<u64, DecodeError> {
    if buf.remaining() < 8 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_u64_le())
}

/// Encode an `sfixed64` value as 8 bytes little-endian (wire type 1).
///
/// `sfixed64` is a signed 64-bit integer stored as its two's complement
/// representation in little-endian order — identical bit pattern to `fixed64`.
#[inline]
pub fn encode_sfixed64(value: i64, buf: &mut impl BufMut) {
    buf.put_i64_le(value);
}

/// Decode an `sfixed64` value from 8 bytes little-endian.
#[inline]
pub fn decode_sfixed64(buf: &mut impl Buf) -> Result<i64, DecodeError> {
    if buf.remaining() < 8 {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.get_i64_le())
}

// ---------------------------------------------------------------------------
// Encoded size helpers (fixed-width types are constant-size)
// ---------------------------------------------------------------------------

/// Encoded size of a `float`, `fixed32`, or `sfixed32` value: always 4 bytes.
pub const FIXED32_ENCODED_LEN: usize = 4;

/// Encoded size of a `double`, `fixed64`, or `sfixed64` value: always 8 bytes.
pub const FIXED64_ENCODED_LEN: usize = 8;

// ---------------------------------------------------------------------------
// Fused tag + payload field writers
// ---------------------------------------------------------------------------
//
// Generated `write_to` bodies pair every payload write with a tag write.
// These helpers fuse the two so each field arm is one call; the presence
// check (`if !x.is_empty()`, `if let Some(v)`, …) stays in generated code
// where the per-field semantics are visible. They are `#[inline(always)]`
// shims over the `Tag::encode` + `encode_*` primitives: with the field number a
// literal at every call site, forced inlining lets the optimizer const-fold the
// tag to constant byte store(s) — recovering the codegen the pre-fusion
// two-statement expansion produced. The fold relies on `Tag::encode` /
// `encode_varint` continuing to inline at constant input. Plain `#[inline]` was
// not sufficient: the inliner declined it for the larger string/bytes variants,
// silently reintroducing a per-call runtime tag varint (~6-9% on encode-heavy
// paths). They are shared by owned and view `write_to` impls (duck-typed:
// `&String` / `&str` and `&Vec<u8>` / `&[u8]` both coerce to the borrowed
// parameter).

/// Stamp a fused `put_<type>_field` writer over an existing `encode_<type>`.
macro_rules! put_field_fn {
    ($(#[$doc:meta])* $name:ident, $value:ty, $wire:expr, $encode:ident) => {
        $(#[$doc])*
        ///
        #[doc = concat!(
            "Fused tag+payload sibling of [`", stringify!($encode), "`]; ",
            "exists so generated `write_to` bodies are one call per field."
        )]
        // `#[inline(always)]`, applied uniformly by the macro. It is load-bearing
        // for the larger string/bytes variants — the inliner declined the plain
        // `#[inline]` hint there, leaving `field_number` a runtime arg that
        // re-encodes the tag varint per call — and harmless for the small scalar
        // variants (already inlined). With the field number a literal at the call
        // site, inlining const-folds the tag to constant byte store(s): one byte
        // for field numbers 1-15, a few for larger numbers.
        #[inline(always)]
        pub fn $name(field_number: u32, value: $value, buf: &mut impl BufMut) {
            Tag::new(field_number, $wire).encode(buf);
            $encode(value, buf);
        }
    };
}

put_field_fn!(
    /// Write a tagged `int32` field (tag + varint payload).
    put_int32_field, i32, WireType::Varint, encode_int32
);
put_field_fn!(
    /// Write a tagged `int64` field (tag + varint payload).
    put_int64_field, i64, WireType::Varint, encode_int64
);
put_field_fn!(
    /// Write a tagged `uint32` field (tag + varint payload).
    put_uint32_field, u32, WireType::Varint, encode_uint32
);
put_field_fn!(
    /// Write a tagged `uint64` field (tag + varint payload).
    put_uint64_field, u64, WireType::Varint, encode_uint64
);
put_field_fn!(
    /// Write a tagged `sint32` field (tag + zigzag varint payload).
    put_sint32_field, i32, WireType::Varint, encode_sint32
);
put_field_fn!(
    /// Write a tagged `sint64` field (tag + zigzag varint payload).
    put_sint64_field, i64, WireType::Varint, encode_sint64
);
put_field_fn!(
    /// Write a tagged `bool` field (tag + one-byte payload).
    put_bool_field, bool, WireType::Varint, encode_bool
);
put_field_fn!(
    /// Write a tagged `fixed32` field (tag + 4-byte payload).
    put_fixed32_field, u32, WireType::Fixed32, encode_fixed32
);
put_field_fn!(
    /// Write a tagged `fixed64` field (tag + 8-byte payload).
    put_fixed64_field, u64, WireType::Fixed64, encode_fixed64
);
put_field_fn!(
    /// Write a tagged `sfixed32` field (tag + 4-byte payload).
    put_sfixed32_field, i32, WireType::Fixed32, encode_sfixed32
);
put_field_fn!(
    /// Write a tagged `sfixed64` field (tag + 8-byte payload).
    put_sfixed64_field, i64, WireType::Fixed64, encode_sfixed64
);
put_field_fn!(
    /// Write a tagged `float` field (tag + 4-byte payload).
    put_float_field, f32, WireType::Fixed32, encode_float
);
put_field_fn!(
    /// Write a tagged `double` field (tag + 8-byte payload).
    put_double_field, f64, WireType::Fixed64, encode_double
);
put_field_fn!(
    /// Write a tagged `string` field (tag + length-prefixed UTF-8 payload).
    put_string_field, &str, WireType::LengthDelimited, encode_string
);
put_field_fn!(
    /// Write a tagged `bytes` field (tag + length-prefixed payload).
    put_bytes_field, &[u8], WireType::LengthDelimited, encode_bytes
);

/// Write a length-delimited field header: tag + payload-length varint.
///
/// Used for sub-message fields (the payload follows via `write_to`, its
/// length coming from the [`SizeCache`](crate::SizeCache)) and for packed
/// repeated fields (the payload loop follows).
///
/// The arguments are `(field_number, len)` — both `u32`, so transposing them
/// compiles but emits a structurally-valid-but-wrong header.
#[inline(always)]
pub fn put_len_delimited_header(field_number: u32, len: u32, buf: &mut impl BufMut) {
    Tag::new(field_number, WireType::LengthDelimited).encode(buf);
    encode_varint(len as u64, buf);
}

/// Write a group field's `StartGroup` tag.
#[inline(always)]
pub fn put_group_start(field_number: u32, buf: &mut impl BufMut) {
    Tag::new(field_number, WireType::StartGroup).encode(buf);
}

/// Write a group field's `EndGroup` tag.
#[inline(always)]
pub fn put_group_end(field_number: u32, buf: &mut impl BufMut) {
    Tag::new(field_number, WireType::EndGroup).encode(buf);
}

// ---------------------------------------------------------------------------
// Length-delimited types (wire type 2): string and bytes
// ---------------------------------------------------------------------------

/// Encode a `string` value as a varint length prefix followed by UTF-8 bytes
/// (wire type 2).
#[inline]
pub fn encode_string(value: &str, buf: &mut impl BufMut) {
    encode_varint(value.len() as u64, buf);
    buf.put_slice(value.as_bytes());
}

/// Decode a `string` value: read a varint length prefix, then that many bytes,
/// and validate that the bytes are valid UTF-8.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - [`DecodeError::InvalidUtf8`] if the bytes are not valid UTF-8.
#[inline]
pub fn decode_string(buf: &mut impl Buf) -> Result<String, DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let chunk = buf.chunk();
    if chunk.len() >= len {
        // Field is contiguous in the source: validate against the source
        // chunk, then own. `validate_str_in` itself checks
        // `chunk.len() >= len + SLACK` and takes the slack-buffer fast path
        // when satisfied (the common case — the chunk continues past this
        // field whenever more wire data follows), else the safe path. Advance
        // the cursor regardless of validity, matching the
        // consume-before-validate behaviour below.
        //
        // SAFETY: `chunk.len() >= len` was just checked.
        let r = unsafe { validate_str_in(chunk, len) }.map(alloc::borrow::ToOwned::to_owned);
        buf.advance(len);
        return r;
    }
    // Non-contiguous: own first (the source has no single slice to validate
    // against), then validate the owned bytes via the safe path.
    //
    // SAFETY: `copy_to_slice` writes exactly `len` bytes into the buffer,
    // and we verified `buf.remaining() >= len` above. The Vec has capacity
    // for `len` bytes. This avoids a redundant zero-initialization that
    // `vec![0u8; len]` would perform.
    #[allow(clippy::uninit_vec)]
    let mut bytes = {
        let mut v = alloc::vec::Vec::with_capacity(len);
        unsafe { v.set_len(len) };
        v
    };
    buf.copy_to_slice(&mut bytes);
    validate_str(&bytes)?;
    // SAFETY: `validate_str` just confirmed `bytes` is well-formed UTF-8.
    Ok(unsafe { String::from_utf8_unchecked(bytes) })
}

/// Merge a length-delimited string into an existing `String`, reusing its
/// heap allocation when possible.
///
/// Reads a varint length prefix, then that many bytes from `buf`, validates
/// UTF-8, and replaces the contents of `value` without deallocating its
/// backing buffer (provided the existing capacity is sufficient).
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - [`DecodeError::InvalidUtf8`] if the bytes are not valid UTF-8.
#[inline]
pub fn merge_string(value: &mut String, buf: &mut impl Buf) -> Result<(), DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let chunk = buf.chunk();
    if chunk.len() >= len {
        // Field is contiguous in the source: validate against the source
        // chunk, then copy into `value` reusing its allocation.
        // `validate_str_in` itself checks `chunk.len() >= len + SLACK` and
        // takes the slack-buffer fast path when satisfied, else the safe
        // path. Advance the cursor regardless of validity.
        //
        // SAFETY: `chunk.len() >= len` was just checked.
        let r: Result<(), DecodeError> = match unsafe { validate_str_in(chunk, len) } {
            Ok(s) => {
                value.clear();
                value.push_str(s);
                Ok(())
            }
            Err(e) => {
                value.clear(); // leave in a valid (empty) state on error
                Err(e)
            }
        };
        buf.advance(len);
        return r;
    }
    // Non-contiguous: own first into `value`'s buffer, then validate via the
    // safe path.
    //
    // SAFETY: `as_mut_vec` requires that the vec contains valid UTF-8 when
    // the String is next used as a string. We validate UTF-8 below before
    // returning Ok, and clear the vec on validation failure.
    let vec = unsafe { value.as_mut_vec() };
    vec.clear();
    vec.reserve(len);
    // SAFETY: `copy_to_slice` writes exactly `len` bytes into the vec, and we
    // verified `buf.remaining() >= len` above. The vec was just cleared (len 0)
    // and reserved to hold at least `len` bytes, so `set_len(len)` is within
    // the allocated capacity. The bytes will be fully initialized by
    // `copy_to_slice` before any read occurs.
    #[allow(clippy::uninit_vec)]
    unsafe {
        vec.set_len(len);
    }
    buf.copy_to_slice(vec);
    // Validate UTF-8 on the new content.
    if validate_str(vec).is_err() {
        vec.clear(); // leave in a valid state on error
        return Err(DecodeError::InvalidUtf8);
    }
    Ok(())
}

/// The raw, length-delimited payload of a `string` or `bytes` field, handed to
/// [`ProtoString::from_wire`] / [`ProtoBytes::from_wire`] so a representation can
/// construct itself directly from the wire — validating (or skipping validation)
/// and choosing borrow-vs-own on its own terms.
///
/// The decoder hands over `Borrowed` when the field's bytes are contiguous in
/// the current input chunk (the common case for slice- and `Bytes`-backed
/// sources) and `Owned` only otherwise (e.g. a field straddling a `Chain`
/// boundary). A representation validates-and-borrows with
/// [`to_str`](Self::to_str), reads the raw bytes with
/// [`as_slice`](Self::as_slice) (always zero-copy), or takes ownership with
/// [`into_bytes`](Self::into_bytes) (zero-copy only for an `Owned` payload —
/// see that method).
#[derive(Debug, Clone)]
pub enum WirePayload<'a> {
    /// The field's bytes borrowed directly from the input buffer.
    Borrowed(&'a [u8]),
    /// The field's bytes owned as `Bytes` (reference-counted). Produced today
    /// only for multi-chunk sources; a single-chunk source — including a single
    /// `Bytes` buffer — currently arrives as `Borrowed`.
    Owned(Bytes),
}

impl WirePayload<'_> {
    /// Borrow the field's bytes (zero-copy in both variants).
    #[inline]
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        match self {
            WirePayload::Borrowed(s) => s,
            WirePayload::Owned(b) => b,
        }
    }

    /// Borrow the payload as a `&str` if it is valid UTF-8.
    ///
    /// Convenience for [`ProtoString::from_wire`] implementations; uses
    /// buffa's UTF-8 validator (so it picks up the `fast-utf8` feature when
    /// enabled) and returns the same [`DecodeError::InvalidUtf8`] the
    /// built-in `String` representation would.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::InvalidUtf8`] if the bytes are not valid UTF-8.
    #[inline]
    #[must_use = "the validated `&str` is the only way to use the payload as a string"]
    pub fn to_str(&self) -> Result<&str, DecodeError> {
        validate_str(self.as_slice())
    }

    /// Take ownership of the field's bytes as [`Bytes`].
    ///
    /// Zero-copy only for an `Owned` payload, which today is produced only for
    /// multi-chunk sources — a single-chunk source (including a single `Bytes`
    /// buffer) arrives as `Borrowed` and is copied here. For a guaranteed
    /// zero-copy `bytes` field path use the built-in `bytes::Bytes` representation
    /// ([`decode_bytes_to_bytes`]); single-chunk-`Bytes` zero-copy for custom
    /// types is a planned additive enhancement.
    #[inline]
    #[must_use]
    pub fn into_bytes(self) -> Bytes {
        match self {
            WirePayload::Borrowed(s) => Bytes::copy_from_slice(s),
            WirePayload::Owned(b) => b,
        }
    }
}

/// Read one length-delimited field payload from `buf` and pass it to `f` as a
/// [`WirePayload`], advancing `buf` past the field.
///
/// The payload is `Borrowed` when the whole field is contiguous in the current
/// chunk (zero-copy), and `Owned` (via [`Buf::copy_to_bytes`], itself zero-copy
/// for a `Bytes`-backed `buf`) when it is not. `f` produces an owned value, so
/// the borrow never escapes and `buf` can be advanced afterwards.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer is shorter than the declared
///   length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - Any error returned by `f` (e.g. [`DecodeError::InvalidUtf8`]).
///
/// On any error the `buf` cursor position is unspecified: a decode error aborts
/// the whole decode, so the buffer is not left in a recoverable state.
#[inline]
pub(crate) fn read_field_payload<R>(
    buf: &mut impl Buf,
    f: impl FnOnce(WirePayload<'_>) -> Result<R, DecodeError>,
) -> Result<R, DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let chunk = buf.chunk();
    if chunk.len() >= len {
        // Whole field is contiguous: hand over a borrowed slice (zero-copy).
        let r = f(WirePayload::Borrowed(&chunk[..len]))?;
        buf.advance(len);
        Ok(r)
    } else {
        // Field straddles chunk boundaries: take an owned `Bytes` (zero-copy
        // when `buf` is `Bytes`-backed, a copy otherwise).
        f(WirePayload::Owned(buf.copy_to_bytes(len)))
    }
}

/// Compute the encoded byte count of a `string` value (varint length prefix +
/// UTF-8 byte count), excluding the field tag.
#[inline]
pub fn string_encoded_len(value: &str) -> usize {
    let len = value.len();
    varint_len(len as u64) + len
}

/// The bound generated code places on the Rust type used for a proto `string`
/// field.
///
/// buffa implements it for the default [`String`]. Select another representation
/// with `buffa_build`'s `string_type` / `string_type_custom`. There is
/// intentionally **no blanket impl**, and a foreign type cannot implement this
/// trait (orphan rule) — wrap it in a local newtype that implements the trait;
/// see the `buffa-smolstr` crate for the canonical template.
///
/// The bounds are exactly what generated code requires of a string field:
///
/// - `from_wire` (the required method, below) — the binary decode constructor.
/// - `Clone + PartialEq + Default + Debug` — for the `#[derive(...)]` and the
///   hand-written `Debug` impl on message structs, and for `clear()` (which
///   resets the field to [`Default`] rather than relying on a `String`-specific
///   `clear`, since a substituted type may be immutable).
/// - `Send + Sync` — so a message owning such a field stays `Send + Sync`;
///   without this bound an exotic string type could silently make every
///   containing message thread-unsafe.
/// - `Deref<Target = str>` and [`AsRef<str>`] — generated code borrows the field
///   as `&str` by plain reference coercion (`&self.field` where
///   [`encode_string`] / [`string_encoded_len`] expect `&str`), so the
///   representation must `Deref` to `str`; `AsRef<str>` is also required for the
///   call sites that ask for it explicitly.
/// - `From<String>` and `From<&str>` — used by the JSON, text-format, and
///   view→owned paths to construct the field from freshly decoded text (binary
///   decode uses [`from_wire`](ProtoString::from_wire) instead).
///
/// For the default `String` representation every conversion is the identity, so
/// the generic path costs nothing relative to the specialized one.
///
/// # Contract
///
/// The bounds are structural and cannot capture these invariants; an
/// implementation must uphold them:
///
/// - `Default` is the empty string — generated `clear()` resets to
///   `Default::default()` and implicit-presence encoding skips empty values, so
///   a non-empty `Default` silently drops or corrupts cleared fields.
/// - `Deref`, `AsRef`, and the constructors observe the same content — encoding
///   borrows via `Deref` / `AsRef` and the view / reflect paths read the same
///   way; if they disagree, a value encodes differently than it reads back.
/// - `from_wire` is value-equivalent to `From<String>` / `From<&str>` — binary
///   decode uses `from_wire` while JSON / text / view→owned use `From`, so a
///   representation must not transform the text (e.g. case-fold) in one path but
///   not the other.
///
/// # Limitations
///
/// These apply to a custom type used as a `repeated` element or in a `map` slot
/// (`map<string, V>` key, `map<K, string>` value). Singular / optional / oneof
/// uses have none of them and work with a foreign type directly. See the user
/// guide's "String and bytes field representations" section for the full table.
///
/// - **Must be crate-local.** A custom type in a `repeated` element or `map`
///   slot needs codegen-emitted `ReflectElement` / `ReflectMapKey` impls (for
///   vtable reflection), which the orphan rule permits only when the type is
///   local to the generating crate. A *foreign* custom type in those positions
///   fails to compile — wrap it in a crate-local newtype.
/// - **JSON needs native `serde`.** A custom string used as a `repeated` element
///   or in a `map` serializes through its own `serde`, so it must derive
///   `Serialize` / `Deserialize` (and, for an external type, enable its `serde`
///   feature). Singular / optional / oneof custom strings use the `proto_string`
///   with-module and need no `serde` impl.
/// - **A `map` key needs `Hash + Eq`** (default / `HashMap` container) or `Ord`
///   (`map_type(BTreeMap)`); the bound is enforced at the generated field type.
/// - **No `Arbitrary` impl required, except in a `map`.** Under the `arbitrary`
///   feature, singular / optional / `repeated` fields get a generic builder, so
///   a custom type needs no native `arbitrary::Arbitrary` impl. The `map`
///   arbitrary path currently has no per-key shim, so a custom string used as a
///   `map` key or value must derive `Arbitrary` itself.
#[rustversion::attr(
    since(1.78),
    diagnostic::on_unimplemented(
        message = "`{Self}` cannot be used as a buffa custom string type",
        note = "buffa owns `ProtoString`, so a foreign type can't implement it directly (orphan rule). \
                Wrap it in a crate-local newtype and implement `ProtoString` on the newtype. \
                See the `buffa-smolstr` crate for a template."
    )
)]
pub trait ProtoString:
    Clone
    + PartialEq
    + Default
    + core::fmt::Debug
    + Send
    + Sync
    + core::ops::Deref<Target = str>
    + AsRef<str>
    + From<String>
    + for<'a> From<&'a str>
{
    /// Construct the representation from a decoded `string` field's wire payload.
    ///
    /// This is the decode constructor: it owns the validation/ownership choice,
    /// so a representation can borrow-and-inline a short string (no transient
    /// heap allocation) or validate UTF-8 only when it must. Validate-and-borrow
    /// with [`WirePayload::to_str`] (which uses buffa's UTF-8 validator and so
    /// picks up the `fast-utf8` feature), or read raw bytes with
    /// [`WirePayload::as_slice`]. There is
    /// intentionally no blanket impl — every representation provides its own
    /// optimal `from_wire`; the `From<String>`/`From<&str>` supertraits remain
    /// for the JSON, text, and view→owned paths.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::InvalidUtf8`] if the payload is not valid UTF-8. A
    /// representation that enforces additional invariants can reject the value
    /// with [`DecodeError::Custom`] (carrying a static reason), or return any
    /// other [`DecodeError`] variant.
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError>;
}

impl ProtoString for String {
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(alloc::borrow::ToOwned::to_owned)
    }
}

// The default representation must always satisfy the bound; freeze that
// invariant against future changes to the trait's supertraits.
const _: fn() = || {
    fn assert_proto_string<S: ProtoString>() {}
    assert_proto_string::<String>();
};

/// Decode a length-delimited `string` into a configurable [`ProtoString`] type
/// by handing its wire payload to [`ProtoString::from_wire`].
///
/// The representation's `from_wire` decides validation and borrow-vs-own, so an
/// inline-capable type avoids the transient `String` allocation that a
/// `From<String>` path would force. Generated code uses the in-place
/// [`merge_string`] for default `String` fields (allocation reuse) and this
/// helper for every other [`ProtoString`] type.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer is shorter than the declared
///   length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - [`DecodeError::InvalidUtf8`] (or another error) as returned by the
///   representation's [`from_wire`](ProtoString::from_wire).
#[inline]
pub fn decode_string_to<S: ProtoString>(buf: &mut impl Buf) -> Result<S, DecodeError> {
    read_field_payload(buf, S::from_wire)
}

/// Encode a `bytes` value as a varint length prefix followed by raw bytes
/// (wire type 2).
#[inline]
pub fn encode_bytes(value: &[u8], buf: &mut impl BufMut) {
    encode_varint(value.len() as u64, buf);
    buf.put_slice(value);
}

/// Decode a `bytes` value: read a varint length prefix, then that many bytes.
///
/// See also [`decode_bytes_to_bytes`] for a [`Bytes`]-returning variant that
/// is zero-copy when `buf` is itself `Bytes`-backed.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
#[inline]
pub fn decode_bytes(buf: &mut impl Buf) -> Result<Vec<u8>, DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    // SAFETY: `copy_to_slice` writes exactly `len` bytes into the buffer,
    // and we verified `buf.remaining() >= len` above. The Vec has capacity
    // for `len` bytes. This avoids a redundant zero-initialization.
    #[allow(clippy::uninit_vec)]
    let mut bytes = {
        let mut v = alloc::vec::Vec::with_capacity(len);
        unsafe { v.set_len(len) };
        v
    };
    buf.copy_to_slice(&mut bytes);
    Ok(bytes)
}

/// Decode a `bytes` value into a [`Bytes`]: read a varint length prefix,
/// then take that many bytes via [`Buf::copy_to_bytes`].
///
/// When `buf` is itself backed by [`Bytes`], `copy_to_bytes` is a zero-copy
/// refcount bump (via `split_to`); for `&[u8]` and other inputs it falls
/// back to an allocation + copy, equivalent to wrapping [`decode_bytes`] in
/// `Bytes::from`.
///
/// # Memory retention
///
/// In the zero-copy case the returned `Bytes` aliases the source allocation,
/// so the entire source buffer is retained until every aliased `Bytes`
/// drops. To detach a field from a large source buffer, deep-copy it with
/// `Bytes::copy_from_slice(&field)`.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
#[inline]
pub fn decode_bytes_to_bytes(buf: &mut impl Buf) -> Result<Bytes, DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    Ok(buf.copy_to_bytes(len))
}

/// The bound generated code places on the Rust type used for a proto `bytes`
/// field.
///
/// buffa implements it for the default [`Vec<u8>`] and for
/// [`bytes::Bytes`](crate::bytes::Bytes) (which decodes zero-copy from a
/// `Bytes`-backed buffer). Select another representation with `buffa_build`'s
/// `bytes_type` / `bytes_type_custom`. There is intentionally **no blanket
/// impl**, and a foreign type cannot implement this trait (orphan rule) — wrap
/// it in a local newtype that implements the trait; see the `buffa-smolstr`
/// crate for the canonical template (the `bytes` side mirrors it).
///
/// This is the `bytes`-side twin of [`ProtoString`]; the bounds are exactly what
/// generated code requires of a `bytes` field:
///
/// - `from_wire` (the required method, below) — the binary decode constructor.
/// - `Clone + PartialEq + Default + Debug` — for the `#[derive(...)]` and the
///   hand-written `Debug` impl on message structs, and for `clear()` (which
///   resets the field to [`Default`] rather than relying on a `Vec`-specific
///   `clear`, since a substituted type may be immutable).
/// - `Send + Sync` — so a message owning such a field stays `Send + Sync`.
/// - `Deref<Target = [u8]>` and [`AsRef<[u8]>`](AsRef) — generated code borrows
///   the field as `&[u8]` by plain reference coercion (`&self.field` where
///   [`encode_bytes`] / [`bytes_encoded_len`] expect `&[u8]`), so the
///   representation must `Deref` to `[u8]`; `AsRef<[u8]>` is also required for
///   the call sites that ask for it explicitly.
/// - `From<Vec<u8>>` — used by the JSON and view→owned paths to construct the
///   field from freshly decoded bytes (binary decode uses
///   [`from_wire`](ProtoBytes::from_wire) instead). Note that `From<&[u8]>` is
///   deliberately *not* required: `bytes::Bytes` implements it only for
///   `&'static [u8]`, so requiring it would exclude `Bytes` itself.
///
/// For the default `Vec<u8>` representation every conversion is the identity, so
/// the generic path costs nothing relative to the specialized one.
///
/// # Contract
///
/// The bounds are structural and cannot capture these invariants; an
/// implementation must uphold them:
///
/// - `Default` is the empty value — generated `clear()` resets to
///   `Default::default()` and implicit-presence encoding skips empty values, so
///   a non-empty `Default` silently drops or corrupts cleared fields.
/// - `Deref`, `AsRef`, and the constructors observe the same content — encoding
///   borrows via `Deref` / `AsRef` and the view / reflect paths read the same
///   way; if they disagree, a value encodes differently than it reads back.
/// - `from_wire` is value-equivalent to `From<Vec<u8>>` — binary decode uses
///   `from_wire` while JSON / view→owned use `From`, so a representation must not
///   transform the bytes in one path but not the other.
///
/// # Limitations
///
/// - **`repeated` elements and `map<K, bytes>` values must be crate-local.** A
///   custom type used as the element of a `repeated` field — or as a
///   `map<K, bytes>` value — needs codegen-emitted `ReflectElement` (vtable) and
///   base64 `ProtoElemJson` (JSON) impls, which the orphan rule permits only when
///   the type is local to the generating crate. A *foreign* custom type in that
///   position fails to compile — wrap it in a crate-local newtype. Singular,
///   optional, and oneof uses work with a foreign type directly. (A custom
///   `bytes` map value is honored just like the built-in `bytes::Bytes`; only the
///   `map<bytes, bytes>` carve-out keeps `Vec<u8>` values.)
/// - **No `Arbitrary` impl required.** Under the `arbitrary` feature codegen
///   attaches a generic builder, so a custom type needs no native
///   `arbitrary::Arbitrary` impl.
#[rustversion::attr(
    since(1.78),
    diagnostic::on_unimplemented(
        message = "`{Self}` cannot be used as a buffa custom bytes type",
        note = "buffa owns `ProtoBytes`, so a foreign type can't implement it directly (orphan rule). \
                Wrap it in a crate-local newtype and implement `ProtoBytes` on the newtype. \
                See the `custom-types` example in the buffa repository for a template."
    )
)]
pub trait ProtoBytes:
    Clone
    + PartialEq
    + Default
    + core::fmt::Debug
    + Send
    + Sync
    + core::ops::Deref<Target = [u8]>
    + AsRef<[u8]>
    + From<Vec<u8>>
{
    /// Construct the representation from a decoded `bytes` field's wire payload.
    ///
    /// This is the decode constructor: it owns the borrow-vs-own choice. A
    /// `Bytes`-backed representation takes ownership via
    /// [`WirePayload::into_bytes`], which is zero-copy only for an `Owned`
    /// payload — today produced only for multi-chunk sources, so a single-chunk
    /// source (including a single `Bytes` buffer) currently yields `Borrowed`
    /// and copies. For a guaranteed zero-copy `bytes` field use the built-in
    /// `bytes::Bytes` representation; single-chunk-`Bytes` zero-copy share for
    /// custom types is a planned additive enhancement. There is intentionally no
    /// blanket impl; the `From<Vec<u8>>` supertrait remains for the JSON and
    /// view→owned paths.
    ///
    /// # Errors
    ///
    /// The built-in representations are infallible. A representation that
    /// enforces additional invariants (e.g. a fixed length) can reject the
    /// value with [`DecodeError::Custom`] (carrying a static reason), or return
    /// any other [`DecodeError`] variant.
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError>;
}

impl ProtoBytes for Vec<u8> {
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(payload.as_slice().to_vec())
    }
}

impl ProtoBytes for Bytes {
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        // Zero-copy for an `Owned` payload (multi-chunk sources today); a
        // single-chunk source arrives `Borrowed` and is copied. The default
        // `bytes::Bytes` field path uses `decode_bytes_to_bytes` for guaranteed
        // zero-copy from a `Bytes`-backed buffer.
        Ok(payload.into_bytes())
    }
}

// The two built-in representations must always satisfy the bound; freeze that
// invariant against future changes to the trait's supertraits.
const _: fn() = || {
    fn assert_proto_bytes<B: ProtoBytes>() {}
    assert_proto_bytes::<Vec<u8>>();
    assert_proto_bytes::<Bytes>();
};

/// Decode a length-delimited `bytes` value into a configurable [`ProtoBytes`]
/// type.
///
/// This is the generic counterpart to [`decode_bytes`]: it hands the field's
/// wire payload to [`ProtoBytes::from_wire`]. A `Bytes`-backed representation can
/// take ownership zero-copy only for an `Owned` payload (multi-chunk sources
/// today); for guaranteed zero-copy from a single `Bytes` buffer, the default
/// `bytes::Bytes` field path uses [`decode_bytes_to_bytes`] instead. Generated
/// code uses the in-place [`merge_bytes`] for default `Vec<u8>` fields
/// (allocation reuse) and this helper for every other [`ProtoBytes`] type
/// (including `bytes::Bytes`).
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer is shorter than the declared
///   length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - Any error returned by the representation's
///   [`from_wire`](ProtoBytes::from_wire).
#[inline]
pub fn decode_bytes_to<B: ProtoBytes>(buf: &mut impl Buf) -> Result<B, DecodeError> {
    read_field_payload(buf, B::from_wire)
}

// ---------------------------------------------------------------------------
// Pluggable repeated-field collections (ProtoList)
// ---------------------------------------------------------------------------

/// The owned collection type backing a proto `repeated` field.
///
/// The default is [`Vec<T>`]; `buffa_build`'s `repeated_type_custom` knob
/// substitutes any type that implements `ProtoList<T>` — for example a
/// `SmallVec`-backed inline collection that avoids a heap allocation for short
/// repeated fields. The wire format is identical regardless of the collection;
/// only the in-memory owned type changes, and view types keep borrowing
/// `&[T]`.
///
/// There is intentionally no blanket impl — the built-in `Vec<T>` impl below is
/// the only one buffa provides. Because `ProtoList` is a buffa-owned trait, a
/// *foreign* collection (e.g. `smallvec::SmallVec`) cannot implement it
/// directly (orphan rule). Always wrap a foreign collection in a **crate-local
/// newtype** and implement `ProtoList` on the newtype, exactly like the
/// `ProtoString` newtype pattern (see the `buffa-smolstr` crate). This holds
/// even for binary-only builds — the generated decode/clear paths require
/// `Field: ProtoList`.
///
/// # Contract
///
/// - The collection must be **growable**: [`push`](Self::push) is infallible
///   (no `try_push`), so the decoder appends one element per wire element with
///   no way to reject an oversized field. A truly capacity-bounded collection
///   (e.g. a fixed-capacity `ArrayVec`) will **panic** on input larger than its
///   capacity rather than return a decode error — do not use one as a
///   `ProtoList`. `SmallVec` is fine because it spills to the heap.
/// - The supertraits and methods must agree on contents: `push` appends one
///   element in order, [`Deref<Target = [T]>`](core::ops::Deref) exposes the
///   elements as a slice in that same order (the encode and `compute_size`
///   paths iterate it), [`FromIterator`] rebuilds the collection for the
///   view→owned conversion, [`From<Vec<T>>`](From) is the ergonomic constructor
///   for building a field by hand (`vec![..].into()`, mirroring `ProtoBytes`'s
///   `From<Vec<u8>>`), and [`Default`] produces the empty collection (the
///   field's cleared state).
/// - A **generic** newtype's [`Default`] must be hand-written (not
///   `#[derive]`d), or the derive forces a spurious `T: Default` bound; expect a
///   `clippy::derivable_impls` lint there and `#[allow]` it (see the example).
///
/// # Limitations (for a *custom* collection)
///
/// - **Reflection / vtable:** a collection used under the reflection or vtable
///   path must implement `buffa_descriptor`'s `ReflectList`. It is not
///   derivable, but a `Vec`-backed newtype can delegate all three methods to
///   the inner `Vec<T>: ReflectList` impl (which requires `T: ReflectElement`).
/// - **JSON / connectrpc:** a collection used in a JSON-enabled build must
///   implement `serde::Serialize` / `Deserialize` (its own impls, or the
///   newtype's). Note that frameworks built on buffa may require the message's
///   serde derive unconditionally — for example `connectrpc` bounds its handler
///   and client message types on `Serialize` / `DeserializeOwned` even when JSON
///   is never used at runtime — so a custom collection used through such a
///   framework must be serde-capable regardless.
/// - **`arbitrary`:** under the `arbitrary` feature a collection must implement
///   `arbitrary::Arbitrary` (trivially derivable on a newtype).
///
/// # Examples
///
/// A minimal crate-local newtype wrapping `smallvec::SmallVec` for binary use:
///
/// ```rust,ignore
/// #[derive(Clone, PartialEq, Debug)]
/// pub struct SmallList<T>(pub smallvec::SmallVec<[T; 4]>);
///
/// // Hand-written so it does not require `T: Default`.
/// #[allow(clippy::derivable_impls)]
/// impl<T> Default for SmallList<T> {
///     fn default() -> Self { SmallList(smallvec::SmallVec::new()) }
/// }
/// impl<T> core::ops::Deref for SmallList<T> {
///     type Target = [T];
///     fn deref(&self) -> &[T] { &self.0 }
/// }
/// impl<T> FromIterator<T> for SmallList<T> {
///     fn from_iter<I: IntoIterator<Item = T>>(it: I) -> Self {
///         SmallList(smallvec::SmallVec::from_iter(it))
///     }
/// }
/// impl<T> From<Vec<T>> for SmallList<T> {        // enables `vec![..].into()`
///     fn from(v: Vec<T>) -> Self { SmallList(smallvec::SmallVec::from_vec(v)) }
/// }
/// impl<T: Clone + PartialEq + core::fmt::Debug + Send + Sync> buffa::ProtoList<T>
///     for SmallList<T>
/// {
///     fn push(&mut self, v: T) { self.0.push(v); }
///     fn clear(&mut self) { self.0.clear(); }
///     // reserve left as the advisory no-op default, so a byte-count hint
///     // never spills the inline storage.
/// }
/// ```
///
/// Then point a field at it: `repeated_type_custom("::my_crate::SmallList<*>")`.
#[rustversion::attr(
    since(1.78),
    diagnostic::on_unimplemented(
        message = "`{Self}` cannot be used as a buffa custom list type",
        note = "buffa owns `ProtoList`, so a foreign type can't implement it directly (orphan rule). \
                Wrap it in a crate-local newtype and implement `ProtoList` on the newtype. \
                See the `custom-types` example in the buffa repository for a template."
    )
)]
pub trait ProtoList<T>:
    Default
    + Clone
    + PartialEq
    + core::fmt::Debug
    + Send
    + Sync
    + FromIterator<T>
    + From<Vec<T>>
    + core::ops::Deref<Target = [T]>
{
    /// Append one decoded element to the end of the collection. Infallible —
    /// the collection must be growable (see the trait's `# Contract`).
    fn push(&mut self, value: T);

    /// Remove all elements (the field's cleared / default state), retaining
    /// capacity where the underlying type allows.
    fn clear(&mut self);

    /// Best-effort capacity hint from the packed-scalar decoder, sized in
    /// elements.
    ///
    /// This is **advisory**: the default implementation does nothing, so a
    /// bounded inline collection is never forced to pre-allocate (or spill its
    /// inline storage) from an attacker-influenced length prefix. The built-in
    /// `Vec<T>` impl overrides it to call [`Vec::reserve`]. The decoder invokes
    /// it through the trait (so an inherent `reserve` on the collection does not
    /// shadow this advisory contract); decoders may pass a loose upper bound, so
    /// never rely on the requested capacity being honored.
    #[inline]
    fn reserve(&mut self, additional: usize) {
        let _ = additional;
    }
}

impl<T> ProtoList<T> for Vec<T>
where
    T: Clone + PartialEq + core::fmt::Debug + Send + Sync,
{
    #[inline]
    fn push(&mut self, value: T) {
        Vec::push(self, value);
    }

    #[inline]
    fn clear(&mut self) {
        Vec::clear(self);
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        Vec::reserve(self, additional);
    }
}

// The default representation must always satisfy the bound; freeze that
// invariant against future changes to the trait's supertraits.
const _: fn() = || {
    fn assert_proto_list<C: ProtoList<T>, T>() {}
    assert_proto_list::<Vec<u32>, u32>();
};

/// Merge length-delimited bytes into an existing `Vec<u8>`, reusing its
/// heap allocation when possible.
///
/// Reads a varint length prefix, then that many bytes from `buf`, and
/// replaces the contents of `value` without deallocating its backing buffer
/// (provided the existing capacity is sufficient).
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
#[inline]
pub fn merge_bytes(value: &mut Vec<u8>, buf: &mut impl Buf) -> Result<(), DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    value.clear();
    value.reserve(len);
    // SAFETY: `copy_to_slice` writes exactly `len` bytes into the vec, and we
    // verified `buf.remaining() >= len` above. The vec was just cleared (len 0)
    // and reserved to hold at least `len` bytes, so `set_len(len)` is within
    // the allocated capacity. The bytes will be fully initialized by
    // `copy_to_slice` before any read occurs.
    #[allow(clippy::uninit_vec)]
    unsafe {
        value.set_len(len);
    }
    buf.copy_to_slice(value);
    Ok(())
}

/// Compute the encoded byte count of a `bytes` value (varint length prefix +
/// byte count), excluding the field tag.
#[inline]
pub fn bytes_encoded_len(value: &[u8]) -> usize {
    let len = value.len();
    varint_len(len as u64) + len
}

/// Borrow a `string` value directly from the input buffer (zero-copy).
///
/// Reads a varint length prefix, then borrows that many bytes from `buf` as a
/// `&'a str`. The returned slice shares the lifetime `'a` of the input buffer,
/// so no allocation is required.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
/// - [`DecodeError::InvalidUtf8`] if the bytes are not valid UTF-8.
#[inline]
pub fn borrow_str<'a>(buf: &mut &'a [u8]) -> Result<&'a str, DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.len() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    // Validate against the surrounding slice (so the slack-buffer fast path
    // can read past `len` when the wire buffer continues), then advance the
    // cursor unconditionally — matching `decode_string` (where `copy_to_slice`
    // consumes the bytes before validation) and protobuf error-recovery
    // semantics: the field payload has been consumed regardless of whether
    // its contents were valid.
    //
    // SAFETY: `buf.len() >= len` was established by the EOF check above.
    let s = unsafe { validate_str_in(buf, len) };
    *buf = &buf[len..];
    s
}

/// Borrow a `bytes` value directly from the input buffer (zero-copy).
///
/// Reads a varint length prefix, then borrows that many bytes from `buf` as a
/// `&'a [u8]`. The returned slice shares the lifetime `'a` of the input
/// buffer, so no allocation is required.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer has fewer bytes than the
///   declared length.
/// - [`DecodeError::MessageTooLarge`] if the declared length overflows `usize`.
#[inline]
pub fn borrow_bytes<'a>(buf: &mut &'a [u8]) -> Result<&'a [u8], DecodeError> {
    let len = decode_varint(buf)?;
    let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.len() < len {
        return Err(DecodeError::UnexpectedEof);
    }
    let bytes = &buf[..len];
    *buf = &buf[len..];
    Ok(bytes)
}

/// Borrows the raw bytes of a group body from `buf`.
///
/// The opening StartGroup tag has already been consumed by the caller.
/// This function scans forward, skipping nested fields, until the matching
/// EndGroup tag with `field_number` is found. Returns the sub-slice
/// containing the group body (excluding the EndGroup tag) and advances
/// `buf` past the EndGroup tag.
///
/// `depth` is the remaining nesting budget for fields *inside* the group
/// body (i.e. the same value passed to the inner view decoder). Nested
/// `StartGroup` fields encountered while scanning for the EndGroup tag
/// consume from this budget.
///
/// # Errors
///
/// - [`DecodeError::UnexpectedEof`] if the buffer ends before the EndGroup tag.
/// - [`DecodeError::InvalidEndGroup`] if an EndGroup with a mismatched field
///   number is encountered.
/// - [`DecodeError::RecursionLimitExceeded`] if a nested `StartGroup` inside
///   the body exceeds the `depth` budget.
pub fn borrow_group<'a>(
    buf: &mut &'a [u8],
    field_number: u32,
    depth: u32,
) -> Result<&'a [u8], DecodeError> {
    let start = *buf;
    // Scan forward to find the EndGroup tag, tracking how many bytes we consume.
    let mut scan: &[u8] = start;
    loop {
        if scan.is_empty() {
            return Err(DecodeError::UnexpectedEof);
        }
        // Remember position before decoding the tag so we can compute the
        // body length without recomputing the tag's encoded size.
        let before_tag = scan.len();
        let tag = crate::encoding::Tag::decode(&mut scan)?;
        if tag.wire_type() == crate::encoding::WireType::EndGroup {
            if tag.field_number() != field_number {
                return Err(DecodeError::InvalidEndGroup(tag.field_number()));
            }
            // The group body is everything from start up to (but not
            // including) the EndGroup tag we just decoded.
            let body_len = start.len() - before_tag;
            let body = &start[..body_len];
            *buf = scan;
            return Ok(body);
        }
        crate::encoding::skip_field_depth(tag, &mut scan, depth)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── UTF-8 validation shims ────────────────────────────────────────────

    #[test]
    fn validate_str_accepts_valid_rejects_invalid() {
        assert_eq!(validate_str(b"hello").unwrap(), "hello");
        assert_eq!(validate_str("héllo 🌍".as_bytes()).unwrap(), "héllo 🌍");
        assert_eq!(validate_str(b""), Ok(""));
        assert!(matches!(
            validate_str(&[0xC0, 0x80]), // overlong NUL
            Err(DecodeError::InvalidUtf8)
        ));
    }

    #[test]
    fn validate_str_in_slack_and_tail_paths_agree() {
        // SAFETY: every call below has `len <= buf.len()`.
        unsafe {
            // 8-byte slack after the field → takes the slack-buffer fast path.
            let with_slack = b"hello\x00\x00\x00\x00\x00\x00\x00\x00";
            assert_eq!(validate_str_in(with_slack, 5).unwrap(), "hello");
            // No slack (field fills the buffer) → safe fallback.
            assert_eq!(validate_str_in(b"hello", 5).unwrap(), "hello");
            // Invalid input rejected on both paths.
            let bad_with_slack = b"\xC0\x80padpadpa";
            assert!(matches!(
                validate_str_in(bad_with_slack, 2),
                Err(DecodeError::InvalidUtf8)
            ));
            assert!(matches!(
                validate_str_in(&[0xC0, 0x80], 2),
                Err(DecodeError::InvalidUtf8)
            ));
            // Slack region is *not* validated: trailing junk past `len` is ignored.
            let junk_slack = b"ok\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF";
            assert_eq!(validate_str_in(junk_slack, 2).unwrap(), "ok");
        }
    }

    #[test]
    fn borrow_str_uses_surrounding_buffer_for_slack() {
        // Two length-delimited string fields back-to-back: the first has the
        // second's bytes as slack, the second falls back to the safe path.
        let mut wire = alloc::vec::Vec::new();
        for s in ["hi", "tail-no-slack"] {
            wire.push(s.len() as u8);
            wire.extend_from_slice(s.as_bytes());
        }
        let mut cur: &[u8] = &wire;
        assert_eq!(borrow_str(&mut cur).unwrap(), "hi");
        assert_eq!(borrow_str(&mut cur).unwrap(), "tail-no-slack");
        assert!(cur.is_empty());
    }

    #[test]
    fn wire_payload_to_str() {
        assert_eq!(WirePayload::Borrowed(b"abc").to_str().unwrap(), "abc");
        assert!(matches!(
            WirePayload::Borrowed(&[0xFF]).to_str(),
            Err(DecodeError::InvalidUtf8)
        ));
    }

    /// A custom string representation that enforces an extra invariant in
    /// `from_wire`, to prove a representation can surface `DecodeError::Custom`
    /// through `decode_string_to`.
    #[derive(Clone, PartialEq, Default, Debug)]
    struct Tiny(alloc::string::String);
    impl core::ops::Deref for Tiny {
        type Target = str;
        fn deref(&self) -> &str {
            &self.0
        }
    }
    impl AsRef<str> for Tiny {
        fn as_ref(&self) -> &str {
            &self.0
        }
    }
    impl From<alloc::string::String> for Tiny {
        fn from(s: alloc::string::String) -> Self {
            Tiny(s)
        }
    }
    impl From<&str> for Tiny {
        fn from(s: &str) -> Self {
            Tiny(s.into())
        }
    }
    impl ProtoString for Tiny {
        fn from_wire(p: WirePayload<'_>) -> Result<Self, DecodeError> {
            let s = core::str::from_utf8(p.as_slice()).map_err(|_| DecodeError::InvalidUtf8)?;
            if s.len() > 3 {
                return Err(DecodeError::Custom("string too long"));
            }
            Ok(Tiny(s.into()))
        }
    }

    #[test]
    fn from_wire_can_surface_custom_decode_error() {
        // Length-delimited "hello" (len 5) — rejected by Tiny's from_wire.
        let mut buf: &[u8] = b"\x05hello";
        assert_eq!(
            decode_string_to::<Tiny>(&mut buf).unwrap_err(),
            DecodeError::Custom("string too long"),
        );
        // A value within the limit decodes normally.
        let mut ok: &[u8] = b"\x02hi";
        assert_eq!(
            decode_string_to::<Tiny>(&mut ok).unwrap(),
            Tiny("hi".into())
        );
    }

    /// Each fused writer must emit exactly tag-then-payload.
    #[test]
    fn put_field_fns_match_tag_plus_encode() {
        macro_rules! check {
            ($put:ident, $encode:ident, $wire:expr, $value:expr) => {{
                let mut fused = Vec::new();
                $put(7, $value, &mut fused);
                let mut split = Vec::new();
                Tag::new(7, $wire).encode(&mut split);
                $encode($value, &mut split);
                assert_eq!(fused, split, stringify!($put));
            }};
        }
        check!(put_int32_field, encode_int32, WireType::Varint, -5i32);
        check!(put_int64_field, encode_int64, WireType::Varint, -5i64);
        check!(put_uint32_field, encode_uint32, WireType::Varint, 300u32);
        check!(put_uint64_field, encode_uint64, WireType::Varint, 300u64);
        check!(put_sint32_field, encode_sint32, WireType::Varint, -5i32);
        check!(put_sint64_field, encode_sint64, WireType::Varint, -5i64);
        check!(put_bool_field, encode_bool, WireType::Varint, true);
        check!(put_fixed32_field, encode_fixed32, WireType::Fixed32, 9u32);
        check!(put_fixed64_field, encode_fixed64, WireType::Fixed64, 9u64);
        check!(
            put_sfixed32_field,
            encode_sfixed32,
            WireType::Fixed32,
            -9i32
        );
        check!(
            put_sfixed64_field,
            encode_sfixed64,
            WireType::Fixed64,
            -9i64
        );
        check!(put_float_field, encode_float, WireType::Fixed32, 1.5f32);
        check!(put_double_field, encode_double, WireType::Fixed64, 1.5f64);
        check!(
            put_string_field,
            encode_string,
            WireType::LengthDelimited,
            "hi"
        );
        check!(
            put_bytes_field,
            encode_bytes,
            WireType::LengthDelimited,
            &[1u8, 2][..]
        );
    }

    #[test]
    fn put_len_delimited_header_and_group_tags() {
        let mut fused = Vec::new();
        put_len_delimited_header(3, 5, &mut fused);
        let mut split = Vec::new();
        Tag::new(3, WireType::LengthDelimited).encode(&mut split);
        encode_varint(5, &mut split);
        assert_eq!(fused, split);

        let mut fused = Vec::new();
        put_group_start(4, &mut fused);
        put_group_end(4, &mut fused);
        let mut split = Vec::new();
        Tag::new(4, WireType::StartGroup).encode(&mut split);
        Tag::new(4, WireType::EndGroup).encode(&mut split);
        assert_eq!(fused, split);
    }

    // -----------------------------------------------------------------------
    // ZigZag tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_zigzag_i32() {
        // Table from the spec:
        // https://protobuf.dev/programming-guides/encoding/#signed-ints
        assert_eq!(zigzag_encode_i32(0), 0);
        assert_eq!(zigzag_encode_i32(-1), 1);
        assert_eq!(zigzag_encode_i32(1), 2);
        assert_eq!(zigzag_encode_i32(-2), 3);
        assert_eq!(zigzag_encode_i32(2147483647), 4294967294);
        assert_eq!(zigzag_encode_i32(-2147483648), 4294967295);

        for v in [0, -1, 1, -2, 2, i32::MAX, i32::MIN] {
            assert_eq!(zigzag_decode_i32(zigzag_encode_i32(v)), v);
        }
    }

    #[test]
    fn test_zigzag_i64() {
        assert_eq!(zigzag_encode_i64(0), 0);
        assert_eq!(zigzag_encode_i64(-1), 1);
        assert_eq!(zigzag_encode_i64(1), 2);
        assert_eq!(zigzag_encode_i64(-2), 3);

        for v in [0, -1, 1, -2, 2, i64::MAX, i64::MIN] {
            assert_eq!(zigzag_decode_i64(zigzag_encode_i64(v)), v);
        }
    }

    // -----------------------------------------------------------------------
    // Float (32-bit IEEE 754) tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_float_roundtrip() {
        let values: &[f32] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MIN,
            f32::MAX,
            f32::MIN_POSITIVE,
            f32::EPSILON,
            f32::INFINITY,
            f32::NEG_INFINITY,
            core::f32::consts::PI,
            1.0e-38, // near subnormal boundary
            1.0e38,  // large value
        ];
        for &v in values {
            let mut buf = Vec::new();
            encode_float(v, &mut buf);
            assert_eq!(buf.len(), 4, "float should encode to exactly 4 bytes");
            let decoded = decode_float(&mut buf.as_slice()).unwrap();
            assert_eq!(
                v.to_bits(),
                decoded.to_bits(),
                "bit-exact roundtrip for {v}"
            );
        }
    }

    #[test]
    fn test_float_nan_roundtrip() {
        // NaN must round-trip with the same bit pattern.
        let nan = f32::NAN;
        let mut buf = Vec::new();
        encode_float(nan, &mut buf);
        let decoded = decode_float(&mut buf.as_slice()).unwrap();
        assert!(decoded.is_nan());
        assert_eq!(nan.to_bits(), decoded.to_bits());
    }

    #[test]
    fn test_float_little_endian() {
        // 1.0f32 = 0x3F800000 in IEEE 754.
        // Little-endian: [0x00, 0x00, 0x80, 0x3F]
        let mut buf = Vec::new();
        encode_float(1.0, &mut buf);
        assert_eq!(buf, &[0x00, 0x00, 0x80, 0x3F]);
    }

    #[test]
    fn test_float_decode_truncated() {
        let buf: &[u8] = &[0x00, 0x00, 0x80]; // only 3 bytes
        let result = decode_float(&mut &buf[..]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Double (64-bit IEEE 754) tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_double_roundtrip() {
        let values: &[f64] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            f64::MIN,
            f64::MAX,
            f64::MIN_POSITIVE,
            f64::EPSILON,
            f64::INFINITY,
            f64::NEG_INFINITY,
            core::f64::consts::PI,
            1.0e-307, // near subnormal boundary
            1.0e308,  // large value
        ];
        for &v in values {
            let mut buf = Vec::new();
            encode_double(v, &mut buf);
            assert_eq!(buf.len(), 8, "double should encode to exactly 8 bytes");
            let decoded = decode_double(&mut buf.as_slice()).unwrap();
            assert_eq!(
                v.to_bits(),
                decoded.to_bits(),
                "bit-exact roundtrip for {v}"
            );
        }
    }

    #[test]
    fn test_double_nan_roundtrip() {
        let nan = f64::NAN;
        let mut buf = Vec::new();
        encode_double(nan, &mut buf);
        let decoded = decode_double(&mut buf.as_slice()).unwrap();
        assert!(decoded.is_nan());
        assert_eq!(nan.to_bits(), decoded.to_bits());
    }

    #[test]
    fn test_double_little_endian() {
        // 1.0f64 = 0x3FF0000000000000 in IEEE 754.
        // Little-endian: [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0, 0x3F]
        let mut buf = Vec::new();
        encode_double(1.0, &mut buf);
        assert_eq!(buf, &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF0, 0x3F]);
    }

    #[test]
    fn test_double_decode_truncated() {
        let buf: &[u8] = &[0x00; 7]; // only 7 bytes
        let result = decode_double(&mut &buf[..]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // fixed32 / sfixed32 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_fixed32_roundtrip() {
        let values: &[u32] = &[0, 1, 255, 256, 65535, u32::MAX, 0xDEAD_BEEF];
        for &v in values {
            let mut buf = Vec::new();
            encode_fixed32(v, &mut buf);
            assert_eq!(buf.len(), 4);
            let decoded = decode_fixed32(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn test_fixed32_little_endian() {
        let mut buf = Vec::new();
        encode_fixed32(0x01020304, &mut buf);
        assert_eq!(buf, &[0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_sfixed32_roundtrip() {
        let values: &[i32] = &[0, 1, -1, 127, -128, i32::MAX, i32::MIN, 0x7FFF_FFFF];
        for &v in values {
            let mut buf = Vec::new();
            encode_sfixed32(v, &mut buf);
            assert_eq!(buf.len(), 4);
            let decoded = decode_sfixed32(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn test_sfixed32_negative_encoding() {
        // -1i32 is 0xFFFFFFFF in two's complement.
        // Little-endian: [0xFF, 0xFF, 0xFF, 0xFF]
        let mut buf = Vec::new();
        encode_sfixed32(-1, &mut buf);
        assert_eq!(buf, &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_fixed32_decode_truncated() {
        let result = decode_fixed32(&mut &[0x01, 0x02, 0x03][..]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // fixed64 / sfixed64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_fixed64_roundtrip() {
        let values: &[u64] = &[
            0,
            1,
            255,
            65535,
            u32::MAX as u64,
            u64::MAX,
            0xDEAD_BEEF_CAFE_BABE,
        ];
        for &v in values {
            let mut buf = Vec::new();
            encode_fixed64(v, &mut buf);
            assert_eq!(buf.len(), 8);
            let decoded = decode_fixed64(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn test_fixed64_little_endian() {
        let mut buf = Vec::new();
        encode_fixed64(0x0102030405060708, &mut buf);
        assert_eq!(buf, &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    }

    #[test]
    fn test_sfixed64_roundtrip() {
        let values: &[i64] = &[0, 1, -1, 127, -128, i64::MAX, i64::MIN];
        for &v in values {
            let mut buf = Vec::new();
            encode_sfixed64(v, &mut buf);
            assert_eq!(buf.len(), 8);
            let decoded = decode_sfixed64(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn test_sfixed64_negative_encoding() {
        let mut buf = Vec::new();
        encode_sfixed64(-1, &mut buf);
        assert_eq!(buf, &[0xFF; 8]);
    }

    #[test]
    fn test_fixed64_decode_truncated() {
        let result = decode_fixed64(&mut &[0x01; 7][..]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Decode from empty buffer
    // -----------------------------------------------------------------------

    #[test]
    fn test_decode_empty_buffer() {
        let empty: &[u8] = &[];
        // Fixed-width types
        assert!(decode_float(&mut &empty[..]).is_err());
        assert!(decode_double(&mut &empty[..]).is_err());
        assert!(decode_fixed32(&mut &empty[..]).is_err());
        assert!(decode_fixed64(&mut &empty[..]).is_err());
        assert!(decode_sfixed32(&mut &empty[..]).is_err());
        assert!(decode_sfixed64(&mut &empty[..]).is_err());
        // Varint types
        assert!(decode_int32(&mut &empty[..]).is_err());
        assert!(decode_int64(&mut &empty[..]).is_err());
        assert!(decode_uint32(&mut &empty[..]).is_err());
        assert!(decode_uint64(&mut &empty[..]).is_err());
        assert!(decode_sint32(&mut &empty[..]).is_err());
        assert!(decode_sint64(&mut &empty[..]).is_err());
        assert!(decode_bool(&mut &empty[..]).is_err());
    }

    // -----------------------------------------------------------------------
    // Encoded size constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_encoded_size_constants() {
        assert_eq!(FIXED32_ENCODED_LEN, 4);
        assert_eq!(FIXED64_ENCODED_LEN, 8);
        assert_eq!(BOOL_ENCODED_LEN, 1);
    }

    // -----------------------------------------------------------------------
    // int32 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_int32_roundtrip() {
        let values: &[i32] = &[0, 1, -1, 127, -128, i32::MAX, i32::MIN, 300, -300];
        for &v in values {
            let mut buf = Vec::new();
            encode_int32(v, &mut buf);
            let decoded = decode_int32(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "int32 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_int32_negative_is_ten_bytes() {
        // Negative int32 values sign-extend to 64 bits → 10-byte varint.
        let mut buf = Vec::new();
        encode_int32(-1, &mut buf);
        assert_eq!(buf.len(), 10, "negative int32 should encode to 10 bytes");
        assert_eq!(int32_encoded_len(-1), 10);
    }

    #[test]
    fn test_int32_positive_size() {
        assert_eq!(int32_encoded_len(0), 1);
        assert_eq!(int32_encoded_len(127), 1);
        assert_eq!(int32_encoded_len(128), 2);
        assert_eq!(int32_encoded_len(i32::MAX), 5);
    }

    // -----------------------------------------------------------------------
    // int64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_int64_roundtrip() {
        let values: &[i64] = &[0, 1, -1, 127, -128, i64::MAX, i64::MIN, 300, -300];
        for &v in values {
            let mut buf = Vec::new();
            encode_int64(v, &mut buf);
            let decoded = decode_int64(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "int64 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_int64_negative_is_ten_bytes() {
        let mut buf = Vec::new();
        encode_int64(-1, &mut buf);
        assert_eq!(buf.len(), 10);
        assert_eq!(int64_encoded_len(-1), 10);
    }

    // -----------------------------------------------------------------------
    // uint32 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_uint32_roundtrip() {
        let values: &[u32] = &[0, 1, 127, 128, 255, 300, u32::MAX];
        for &v in values {
            let mut buf = Vec::new();
            encode_uint32(v, &mut buf);
            let decoded = decode_uint32(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "uint32 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_uint32_encoded_len() {
        assert_eq!(uint32_encoded_len(0), 1);
        assert_eq!(uint32_encoded_len(127), 1);
        assert_eq!(uint32_encoded_len(128), 2);
        assert_eq!(uint32_encoded_len(u32::MAX), 5);
    }

    // -----------------------------------------------------------------------
    // uint64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_uint64_roundtrip() {
        let values: &[u64] = &[0, 1, 127, 128, u32::MAX as u64, u64::MAX];
        for &v in values {
            let mut buf = Vec::new();
            encode_uint64(v, &mut buf);
            let decoded = decode_uint64(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "uint64 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_uint64_encoded_len() {
        assert_eq!(uint64_encoded_len(0), 1);
        assert_eq!(uint64_encoded_len(u64::MAX), 10);
    }

    // -----------------------------------------------------------------------
    // sint32 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sint32_roundtrip() {
        let values: &[i32] = &[0, -1, 1, -2, 2, i32::MAX, i32::MIN, -300, 300];
        for &v in values {
            let mut buf = Vec::new();
            encode_sint32(v, &mut buf);
            let decoded = decode_sint32(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "sint32 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_sint32_negative_is_compact() {
        // -1 zigzag-encodes to 1: should fit in 1 byte, unlike int32.
        let mut buf = Vec::new();
        encode_sint32(-1, &mut buf);
        assert_eq!(buf.len(), 1);
        assert_eq!(sint32_encoded_len(-1), 1);

        // i32::MIN zigzag-encodes to u32::MAX (5 bytes).
        assert_eq!(sint32_encoded_len(i32::MIN), 5);
    }

    // -----------------------------------------------------------------------
    // sint64 tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_sint64_roundtrip() {
        let values: &[i64] = &[0, -1, 1, -2, 2, i64::MAX, i64::MIN, -300, 300];
        for &v in values {
            let mut buf = Vec::new();
            encode_sint64(v, &mut buf);
            let decoded = decode_sint64(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "sint64 roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_sint64_negative_is_compact() {
        let mut buf = Vec::new();
        encode_sint64(-1, &mut buf);
        assert_eq!(buf.len(), 1);
        assert_eq!(sint64_encoded_len(-1), 1);
    }

    // -----------------------------------------------------------------------
    // bool tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bool_roundtrip() {
        for v in [true, false] {
            let mut buf = Vec::new();
            encode_bool(v, &mut buf);
            assert_eq!(buf.len(), 1, "bool should encode to exactly 1 byte");
            let decoded = decode_bool(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "bool roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_bool_wire_values() {
        let mut buf = Vec::new();
        encode_bool(true, &mut buf);
        assert_eq!(buf, &[0x01]);

        buf.clear();
        encode_bool(false, &mut buf);
        assert_eq!(buf, &[0x00]);
    }

    #[test]
    fn test_bool_nonzero_decodes_as_true() {
        // Spec: any non-zero varint decodes as true.
        for byte in [0x02u8, 0x7F, 0xFF] {
            // Encode a raw varint whose low byte is `byte` (with continuation
            // bit clear so it's a 1-byte varint).
            let raw = byte & 0x7F;
            let decoded = decode_bool(&mut &[raw][..]).unwrap();
            assert!(decoded, "0x{raw:02X} should decode as true");
        }
    }

    #[test]
    fn test_bool_multibyte_varint_decodes_as_true() {
        // [0x80, 0x01] is a two-byte varint encoding of 128, which is non-zero
        // and must decode as true. Conforming encoders always write 0x00/0x01,
        // but the decoder must accept any non-zero varint per the spec.
        let decoded = decode_bool(&mut &[0x80u8, 0x01][..]).unwrap();
        assert!(decoded);
    }

    // -----------------------------------------------------------------------
    // Varint-type encoded length helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_varint_encoded_len_consistency() {
        // For each encode function, the encoded byte count must match the
        // corresponding *_encoded_len helper.
        let i32_cases: &[i32] = &[0, 1, -1, 127, 128, i32::MAX, i32::MIN];
        for &v in i32_cases {
            let mut buf = Vec::new();
            encode_int32(v, &mut buf);
            assert_eq!(
                buf.len(),
                int32_encoded_len(v),
                "int32_encoded_len mismatch for {v}"
            );
        }

        let i64_cases: &[i64] = &[0, 1, -1, i64::MAX, i64::MIN];
        for &v in i64_cases {
            let mut buf = Vec::new();
            encode_int64(v, &mut buf);
            assert_eq!(
                buf.len(),
                int64_encoded_len(v),
                "int64_encoded_len mismatch for {v}"
            );
        }

        let u32_cases: &[u32] = &[0, 1, 127, 128, u32::MAX];
        for &v in u32_cases {
            let mut buf = Vec::new();
            encode_uint32(v, &mut buf);
            assert_eq!(
                buf.len(),
                uint32_encoded_len(v),
                "uint32_encoded_len mismatch for {v}"
            );
        }

        let u64_cases: &[u64] = &[0, 1, 127, 128, u64::MAX];
        for &v in u64_cases {
            let mut buf = Vec::new();
            encode_uint64(v, &mut buf);
            assert_eq!(
                buf.len(),
                uint64_encoded_len(v),
                "uint64_encoded_len mismatch for {v}"
            );
        }

        let sint32_cases: &[i32] = &[0, -1, 1, i32::MAX, i32::MIN];
        for &v in sint32_cases {
            let mut buf = Vec::new();
            encode_sint32(v, &mut buf);
            assert_eq!(
                buf.len(),
                sint32_encoded_len(v),
                "sint32_encoded_len mismatch for {v}"
            );
        }

        let sint64_cases: &[i64] = &[0, -1, 1, i64::MAX, i64::MIN];
        for &v in sint64_cases {
            let mut buf = Vec::new();
            encode_sint64(v, &mut buf);
            assert_eq!(
                buf.len(),
                sint64_encoded_len(v),
                "sint64_encoded_len mismatch for {v}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // string tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_string_roundtrip() {
        for s in ["", "hello", "héllo", "世界", "a".repeat(128).as_str()] {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);
            assert_eq!(
                buf.len(),
                string_encoded_len(s),
                "string_encoded_len mismatch"
            );
            let decoded = decode_string(&mut buf.as_slice()).unwrap();
            assert_eq!(s, decoded);
        }
    }

    #[test]
    fn test_string_empty() {
        let mut buf = Vec::new();
        encode_string("", &mut buf);
        // Empty string: varint 0 (1 byte), no payload.
        assert_eq!(buf, &[0x00]);
        assert_eq!(string_encoded_len(""), 1);
        let decoded = decode_string(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded, "");
    }

    #[test]
    fn test_decode_string_to_roundtrip() {
        // `decode_string_to::<S>` must decode identically to `decode_string`
        // for any `ProtoString` type; `String` exercises the identity path.
        for s in ["", "hello", "héllo", "世界", "a".repeat(128).as_str()] {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);
            let decoded: String = decode_string_to(&mut buf.as_slice()).unwrap();
            assert_eq!(s, decoded);
        }
    }

    #[test]
    fn test_decode_string_to_propagates_errors() {
        // Invalid UTF-8 surfaces before the `From<String>` conversion.
        let bad: &[u8] = &[0x02, 0xFF, 0xFE];
        assert_eq!(
            decode_string_to::<String>(&mut &bad[..]),
            Err(DecodeError::InvalidUtf8)
        );
        // Truncated payload is reported as EOF.
        let short: &[u8] = &[0x04, 0x61, 0x62];
        assert_eq!(
            decode_string_to::<String>(&mut &short[..]),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn test_string_invalid_utf8() {
        // Length prefix = 2, payload = two invalid UTF-8 bytes.
        let buf: &[u8] = &[0x02, 0xFF, 0xFE];
        assert_eq!(decode_string(&mut &buf[..]), Err(DecodeError::InvalidUtf8));
    }

    #[test]
    fn test_string_truncated() {
        // Length prefix says 4 bytes but buffer only has 2.
        let buf: &[u8] = &[0x04, 0x61, 0x62];
        assert_eq!(
            decode_string(&mut &buf[..]),
            Err(DecodeError::UnexpectedEof)
        );
    }

    // -----------------------------------------------------------------------
    // bytes tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_bytes_roundtrip() {
        for b in [&[][..], &[0x00, 0xFF, 0x80], &[0u8; 128]] {
            let mut buf = Vec::new();
            encode_bytes(b, &mut buf);
            assert_eq!(
                buf.len(),
                bytes_encoded_len(b),
                "bytes_encoded_len mismatch"
            );
            let decoded = decode_bytes(&mut buf.as_slice()).unwrap();
            assert_eq!(b, decoded.as_slice());
        }
    }

    #[test]
    fn test_bytes_empty() {
        let mut buf = Vec::new();
        encode_bytes(&[], &mut buf);
        assert_eq!(buf, &[0x00]);
        assert_eq!(bytes_encoded_len(&[]), 1);
        let decoded = decode_bytes(&mut buf.as_slice()).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_bytes_truncated() {
        // Length prefix says 5 bytes but buffer only has 2.
        let buf: &[u8] = &[0x05, 0xAA, 0xBB];
        assert_eq!(decode_bytes(&mut &buf[..]), Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn test_decode_bytes_to_bytes_zero_copy_from_bytes() {
        let mut enc = Vec::new();
        encode_bytes(b"\xDE\xAD\xBE\xEF", &mut enc);
        let mut buf = Bytes::from(enc);
        let payload_ptr = buf.as_ptr() as usize + 1;
        let out = decode_bytes_to_bytes(&mut buf).unwrap();
        assert_eq!(&out[..], b"\xDE\xAD\xBE\xEF");
        // copy_to_bytes on a Bytes input is split_to: the result aliases the
        // source buffer rather than allocating a copy.
        assert_eq!(out.as_ptr() as usize, payload_ptr);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_decode_bytes_to_bytes_from_slice() {
        let mut enc = Vec::new();
        encode_bytes(b"hello", &mut enc);
        let out = decode_bytes_to_bytes(&mut enc.as_slice()).unwrap();
        assert_eq!(&out[..], b"hello");
    }

    #[test]
    fn test_decode_bytes_to_bytes_empty() {
        let mut buf = Bytes::from_static(&[0x00]);
        let out = decode_bytes_to_bytes(&mut buf).unwrap();
        assert!(out.is_empty());
        assert!(buf.is_empty());
    }

    #[test]
    fn test_decode_bytes_to_bytes_truncated() {
        let buf: &[u8] = &[0x05, 0xAA, 0xBB];
        assert_eq!(
            decode_bytes_to_bytes(&mut &buf[..]),
            Err(DecodeError::UnexpectedEof)
        );
    }

    // -----------------------------------------------------------------------
    // merge_string tests
    // -----------------------------------------------------------------------

    #[test]
    fn merge_string_reuses_allocation() {
        // Pre-allocate a string with known capacity.
        let mut value = String::with_capacity(64);
        value.push_str("old content");
        let ptr_before = value.as_ptr();

        let mut buf = Vec::new();
        encode_string("hello", &mut buf);
        merge_string(&mut value, &mut buf.as_slice()).unwrap();

        assert_eq!(value, "hello");
        // The allocation should be reused (same base pointer) since
        // "hello" (5 bytes) fits within the 64-byte capacity.
        assert_eq!(value.as_ptr(), ptr_before);
    }

    #[test]
    fn merge_string_empty() {
        let mut value = String::from("existing");
        let mut buf = Vec::new();
        encode_string("", &mut buf);
        merge_string(&mut value, &mut buf.as_slice()).unwrap();
        assert_eq!(value, "");
    }

    #[test]
    fn merge_string_invalid_utf8() {
        let mut value = String::from("valid");
        let buf: &[u8] = &[0x02, 0xFF, 0xFE];
        let result = merge_string(&mut value, &mut &buf[..]);
        assert_eq!(result, Err(DecodeError::InvalidUtf8));
        // Value should be left in a valid (empty) state.
        assert!(value.is_empty());
    }

    #[test]
    fn merge_string_truncated() {
        let mut value = String::from("existing");
        let buf: &[u8] = &[0x04, 0x61, 0x62];
        let result = merge_string(&mut value, &mut &buf[..]);
        assert_eq!(result, Err(DecodeError::UnexpectedEof));
        // Value should be preserved on UnexpectedEof (error returned before
        // any mutation).
        assert_eq!(value, "existing");
    }

    #[test]
    fn merge_string_roundtrip() {
        for s in ["", "hello", "héllo", "世界", &"a".repeat(128)] {
            let mut buf = Vec::new();
            encode_string(s, &mut buf);
            let mut value = String::new();
            merge_string(&mut value, &mut buf.as_slice()).unwrap();
            assert_eq!(s, value);
        }
    }

    // -----------------------------------------------------------------------
    // merge_bytes tests
    // -----------------------------------------------------------------------

    #[test]
    fn merge_bytes_reuses_allocation() {
        let mut value = Vec::with_capacity(64);
        value.extend_from_slice(b"old content");
        let ptr_before = value.as_ptr();

        let mut buf = Vec::new();
        encode_bytes(&[0xDE, 0xAD], &mut buf);
        merge_bytes(&mut value, &mut buf.as_slice()).unwrap();

        assert_eq!(value, &[0xDE, 0xAD]);
        assert_eq!(value.as_ptr(), ptr_before);
    }

    #[test]
    fn merge_bytes_empty() {
        let mut value = vec![1, 2, 3];
        let mut buf = Vec::new();
        encode_bytes(&[], &mut buf);
        merge_bytes(&mut value, &mut buf.as_slice()).unwrap();
        assert!(value.is_empty());
    }

    #[test]
    fn merge_bytes_truncated() {
        let mut value = vec![0xAA];
        let buf: &[u8] = &[0x05, 0xAA, 0xBB];
        let result = merge_bytes(&mut value, &mut &buf[..]);
        assert_eq!(result, Err(DecodeError::UnexpectedEof));
        // Value should be preserved on UnexpectedEof.
        assert_eq!(value, &[0xAA]);
    }

    #[test]
    fn merge_bytes_roundtrip() {
        for b in [&[][..], &[0x00, 0xFF, 0x80], &[0u8; 128]] {
            let mut buf = Vec::new();
            encode_bytes(b, &mut buf);
            let mut value = Vec::new();
            merge_bytes(&mut value, &mut buf.as_slice()).unwrap();
            assert_eq!(b, value.as_slice());
        }
    }

    // -----------------------------------------------------------------------
    // borrow_str / borrow_bytes tests
    // -----------------------------------------------------------------------

    #[test]
    fn borrow_str_valid_borrows_from_source() {
        // Encode "hello" and verify the borrowed slice points into the same bytes.
        let mut encoded = Vec::new();
        encode_string("hello", &mut encoded);
        let source: &[u8] = &encoded;
        let mut cur: &[u8] = source;
        let s = borrow_str(&mut cur).unwrap();
        assert_eq!(s, "hello");
        // Verify zero-copy: the returned &str must be a sub-slice of the input buffer.
        assert!(core::ptr::eq(s.as_bytes().as_ptr(), source[1..].as_ptr()));
        // Cursor should be advanced past the string payload.
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_str_empty_string() {
        let mut encoded = Vec::new();
        encode_string("", &mut encoded);
        let mut cur: &[u8] = &encoded;
        let s = borrow_str(&mut cur).unwrap();
        assert_eq!(s, "");
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_str_advances_cursor() {
        // Two strings back-to-back; each borrow_str call should advance past its own payload.
        let mut encoded = Vec::new();
        encode_string("ab", &mut encoded);
        encode_string("cd", &mut encoded);
        let mut cur: &[u8] = &encoded;
        assert_eq!(borrow_str(&mut cur).unwrap(), "ab");
        assert_eq!(borrow_str(&mut cur).unwrap(), "cd");
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_str_invalid_utf8_returns_error() {
        // Length prefix 2, then two invalid UTF-8 bytes.
        let buf: &[u8] = &[0x02, 0xFF, 0xFE];
        let mut cur: &[u8] = buf;
        assert_eq!(borrow_str(&mut cur), Err(DecodeError::InvalidUtf8));
        // Cursor is advanced past the bytes even on error.
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_str_truncated_returns_eof() {
        let buf: &[u8] = &[0x04, 0x61, 0x62]; // length 4, only 2 bytes
        let mut cur: &[u8] = buf;
        assert_eq!(borrow_str(&mut cur), Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn borrow_bytes_valid_borrows_from_source() {
        let mut encoded = Vec::new();
        encode_bytes(&[0xDE, 0xAD, 0xBE, 0xEF], &mut encoded);
        let source: &[u8] = &encoded;
        let mut cur: &[u8] = source;
        let b = borrow_bytes(&mut cur).unwrap();
        assert_eq!(b, &[0xDE, 0xAD, 0xBE, 0xEF]);
        // Verify zero-copy: the returned &[u8] must be a sub-slice of the input buffer.
        assert!(core::ptr::eq(b.as_ptr(), source[1..].as_ptr()));
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_bytes_empty() {
        let mut encoded = Vec::new();
        encode_bytes(&[], &mut encoded);
        let mut cur: &[u8] = &encoded;
        let b = borrow_bytes(&mut cur).unwrap();
        assert!(b.is_empty());
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_bytes_advances_cursor() {
        let mut encoded = Vec::new();
        encode_bytes(&[1, 2], &mut encoded);
        encode_bytes(&[3, 4, 5], &mut encoded);
        let mut cur: &[u8] = &encoded;
        assert_eq!(borrow_bytes(&mut cur).unwrap(), &[1, 2]);
        assert_eq!(borrow_bytes(&mut cur).unwrap(), &[3, 4, 5]);
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_bytes_truncated_returns_eof() {
        let buf: &[u8] = &[0x05, 0xAA, 0xBB]; // length 5, only 2 bytes
        let mut cur: &[u8] = buf;
        assert_eq!(borrow_bytes(&mut cur), Err(DecodeError::UnexpectedEof));
    }

    // -----------------------------------------------------------------------
    // borrow_group tests
    // -----------------------------------------------------------------------

    /// Helper: build a byte buffer with the given tag+value pairs followed
    /// by an EndGroup tag for `group_field_number`.
    fn build_group_bytes(
        fields: &[(u32, crate::encoding::WireType, &[u8])],
        group_field_number: u32,
    ) -> Vec<u8> {
        use crate::encoding::Tag;
        let mut buf = Vec::new();
        for &(fnum, wt, data) in fields {
            Tag::new(fnum, wt).encode(&mut buf);
            buf.extend_from_slice(data);
        }
        Tag::new(group_field_number, crate::encoding::WireType::EndGroup).encode(&mut buf);
        buf
    }

    #[test]
    fn borrow_group_empty() {
        // Group with no fields: just EndGroup(1).
        let data = build_group_bytes(&[], 1);
        let mut cur: &[u8] = &data;
        let body = borrow_group(&mut cur, 1, crate::RECURSION_LIMIT).unwrap();
        assert!(body.is_empty());
        assert!(cur.is_empty());
    }

    #[test]
    fn borrow_group_one_varint_field() {
        // Group with one varint field (field 2 = 150).
        let mut varint_buf = Vec::new();
        crate::encoding::encode_varint(150, &mut varint_buf);
        let data = build_group_bytes(&[(2, crate::encoding::WireType::Varint, &varint_buf)], 1);
        let mut cur: &[u8] = &data;
        let body = borrow_group(&mut cur, 1, crate::RECURSION_LIMIT).unwrap();

        // Body should contain the tag for field 2 + the varint 150.
        let expected_body_len = data.len() - crate::encoding::varint_len(((1u64) << 3) | 4);
        assert_eq!(body.len(), expected_body_len);
        assert!(cur.is_empty());

        // Verify the body can be decoded as field 2 = 150.
        let mut body_cur = body;
        let tag = crate::encoding::Tag::decode(&mut body_cur).unwrap();
        assert_eq!(tag.field_number(), 2);
        assert_eq!(crate::encoding::decode_varint(&mut body_cur).unwrap(), 150);
        assert!(body_cur.is_empty());
    }

    #[test]
    fn borrow_group_nested_group() {
        // Outer group (field 1) containing an inner group (field 3) with a
        // varint field (field 4 = 42), then EndGroup(3), then EndGroup(1).
        use crate::encoding::{Tag, WireType};
        let mut data = Vec::new();
        // Inner group: StartGroup(3), varint field 4 = 42, EndGroup(3)
        Tag::new(3, WireType::StartGroup).encode(&mut data);
        Tag::new(4, WireType::Varint).encode(&mut data);
        crate::encoding::encode_varint(42, &mut data);
        Tag::new(3, WireType::EndGroup).encode(&mut data);
        // Outer EndGroup(1)
        Tag::new(1, WireType::EndGroup).encode(&mut data);

        let mut cur: &[u8] = &data;
        let body = borrow_group(&mut cur, 1, crate::RECURSION_LIMIT).unwrap();
        assert!(cur.is_empty());

        // Body should contain the entire inner group (StartGroup through
        // EndGroup for field 3).
        assert!(!body.is_empty());

        // Verify inner group can be parsed from the body.
        let mut body_cur = body;
        let inner_tag = crate::encoding::Tag::decode(&mut body_cur).unwrap();
        assert_eq!(inner_tag.field_number(), 3);
        assert_eq!(inner_tag.wire_type(), WireType::StartGroup);
        let inner_body = borrow_group(&mut body_cur, 3, crate::RECURSION_LIMIT).unwrap();
        assert!(body_cur.is_empty());

        // Inner body: field 4 = 42
        let mut inner_cur = inner_body;
        let f4_tag = crate::encoding::Tag::decode(&mut inner_cur).unwrap();
        assert_eq!(f4_tag.field_number(), 4);
        assert_eq!(crate::encoding::decode_varint(&mut inner_cur).unwrap(), 42);
        assert!(inner_cur.is_empty());
    }

    #[test]
    fn borrow_group_mismatched_end() {
        // EndGroup with wrong field number.
        use crate::encoding::{Tag, WireType};
        let mut data = Vec::new();
        Tag::new(99, WireType::EndGroup).encode(&mut data);

        let mut cur: &[u8] = &data;
        assert_eq!(
            borrow_group(&mut cur, 1, crate::RECURSION_LIMIT),
            Err(DecodeError::InvalidEndGroup(99))
        );
    }

    #[test]
    fn borrow_group_truncated() {
        // Buffer ends without EndGroup.
        use crate::encoding::{Tag, WireType};
        let mut data = Vec::new();
        Tag::new(2, WireType::Varint).encode(&mut data);
        crate::encoding::encode_varint(42, &mut data);
        // No EndGroup tag.

        let mut cur: &[u8] = &data;
        assert_eq!(
            borrow_group(&mut cur, 1, crate::RECURSION_LIMIT),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn borrow_group_empty_buffer() {
        let mut cur: &[u8] = &[];
        assert_eq!(
            borrow_group(&mut cur, 1, crate::RECURSION_LIMIT),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn borrow_group_trailing_data_preserved() {
        // Group followed by extra data that should remain in the buffer.
        use crate::encoding::{Tag, WireType};
        let mut data = Vec::new();
        Tag::new(1, WireType::EndGroup).encode(&mut data);
        data.extend_from_slice(&[0xDE, 0xAD]); // trailing bytes

        let mut cur: &[u8] = &data;
        let body = borrow_group(&mut cur, 1, crate::RECURSION_LIMIT).unwrap();
        assert!(body.is_empty());
        assert_eq!(cur, &[0xDE, 0xAD]);
    }

    #[test]
    fn borrow_group_nested_start_group_respects_depth_limit() {
        // Group body contains a nested StartGroup(2)..EndGroup(2). With
        // depth=0, scanning past the nested group must fail with
        // RecursionLimitExceeded (it requires one level of depth budget).
        use crate::encoding::{Tag, WireType};
        let mut data = Vec::new();
        Tag::new(2, WireType::StartGroup).encode(&mut data);
        Tag::new(2, WireType::EndGroup).encode(&mut data);
        Tag::new(1, WireType::EndGroup).encode(&mut data);

        let mut cur: &[u8] = &data;
        assert_eq!(
            borrow_group(&mut cur, 1, 0),
            Err(DecodeError::RecursionLimitExceeded)
        );

        // With depth=1, the nested group (one level deep) is skippable.
        let mut cur: &[u8] = &data;
        let body = borrow_group(&mut cur, 1, 1).unwrap();
        assert!(!body.is_empty());
        assert!(cur.is_empty());
    }

    // --- 32-bit specific tests ---
    // These exercise the `MessageTooLarge` error path in decode functions where
    // a varint length prefix exceeds `usize::MAX`. On 64-bit targets, `usize`
    // is as wide as `u64` so `usize::try_from(u64)` never fails; the error is
    // only reachable on 32-bit (or narrower) targets.

    /// Varint encoding of 0x1_0000_0000 (u32::MAX + 1) — exceeds 32-bit usize.
    #[cfg(target_pointer_width = "32")]
    const OVERSIZED_VARINT: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x10];

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn decode_string_rejects_oversized_length_on_32bit() {
        let mut buf = OVERSIZED_VARINT;
        assert_eq!(decode_string(&mut buf), Err(DecodeError::MessageTooLarge));
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn decode_bytes_rejects_oversized_length_on_32bit() {
        let mut buf = OVERSIZED_VARINT;
        assert_eq!(decode_bytes(&mut buf), Err(DecodeError::MessageTooLarge));
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn merge_string_rejects_oversized_length_on_32bit() {
        let mut value = String::new();
        let mut buf = OVERSIZED_VARINT;
        assert_eq!(
            merge_string(&mut value, &mut buf),
            Err(DecodeError::MessageTooLarge)
        );
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn merge_bytes_rejects_oversized_length_on_32bit() {
        let mut value = Vec::new();
        let mut buf = OVERSIZED_VARINT;
        assert_eq!(
            merge_bytes(&mut value, &mut buf),
            Err(DecodeError::MessageTooLarge)
        );
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn borrow_str_rejects_oversized_length_on_32bit() {
        let mut buf: &[u8] = OVERSIZED_VARINT;
        assert_eq!(borrow_str(&mut buf), Err(DecodeError::MessageTooLarge));
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn borrow_bytes_rejects_oversized_length_on_32bit() {
        let mut buf: &[u8] = OVERSIZED_VARINT;
        assert_eq!(borrow_bytes(&mut buf), Err(DecodeError::MessageTooLarge));
    }
}
