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
use bytes::{Buf, BufMut};

use crate::encoding::{decode_varint, encode_varint, varint_len};
use crate::error::DecodeError;

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
    String::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
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
    if core::str::from_utf8(vec).is_err() {
        vec.clear(); // leave in a valid state on error
        return Err(DecodeError::InvalidUtf8);
    }
    Ok(())
}

/// Compute the encoded byte count of a `string` value (varint length prefix +
/// UTF-8 byte count), excluding the field tag.
#[inline]
pub fn string_encoded_len(value: &str) -> usize {
    let len = value.len();
    varint_len(len as u64) + len
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
    let bytes = &buf[..len];
    // Advance the cursor unconditionally before UTF-8 validation.  This
    // matches the behaviour of `decode_string` (where `copy_to_slice`
    // consumes the bytes before `from_utf8`) and is correct for protobuf
    // error recovery: the field payload has been consumed regardless of
    // whether its contents were valid.
    *buf = &buf[len..];
    core::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8)
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
