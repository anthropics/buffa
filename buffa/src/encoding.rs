//! Wire format encoding and decoding primitives.
//!
//! Implements the protobuf binary wire format: varints, fixed-width integers,
//! length-delimited fields, and tag parsing.

use bytes::{Buf, BufMut};

use crate::error::DecodeError;

/// The maximum valid protobuf field number, 2^29 − 1.
///
/// The wire-format tag packs `(field_number << 3) | wire_type` into a
/// u32-decodable varint; the low 3 bits carry the wire type, leaving 29
/// bits for the field number. See the [protobuf encoding spec][spec].
///
/// [spec]: https://protobuf.dev/programming-guides/encoding/#structure
pub const MAX_FIELD_NUMBER: u32 = (1 << 29) - 1;

/// Protobuf wire types.
///
/// Only wire types 0–5 are currently defined by the protobuf specification;
/// values 6 and 7 are reserved for future use.  This enum is
/// `#[non_exhaustive]` so that adding new wire types in a future crate
/// version is not a breaking change for downstream match arms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[non_exhaustive]
pub enum WireType {
    Varint = 0,
    Fixed64 = 1,
    LengthDelimited = 2,
    StartGroup = 3,
    EndGroup = 4,
    Fixed32 = 5,
}

impl WireType {
    /// Converts a raw `u32` wire type value to a [`WireType`] variant.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::InvalidWireType`] if `value` is not a
    /// recognised wire type (i.e. not in 0–5).
    pub fn from_u32(value: u32) -> Result<Self, DecodeError> {
        match value {
            0 => Ok(WireType::Varint),
            1 => Ok(WireType::Fixed64),
            2 => Ok(WireType::LengthDelimited),
            3 => Ok(WireType::StartGroup),
            4 => Ok(WireType::EndGroup),
            5 => Ok(WireType::Fixed32),
            _ => Err(DecodeError::InvalidWireType(value)),
        }
    }
}

/// A parsed field tag (field number + wire type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tag {
    field_number: u32,
    wire_type: WireType,
}

impl Tag {
    /// Create a new tag.
    ///
    /// # Panics
    ///
    /// Panics if `field_number` is not in the valid range
    /// `[1, MAX_FIELD_NUMBER]`. This is a programming error (generated
    /// code always uses valid field numbers); the panic fires in all
    /// build profiles.
    pub fn new(field_number: u32, wire_type: WireType) -> Self {
        assert!(
            (1..=MAX_FIELD_NUMBER).contains(&field_number),
            "field_number must be in [1, {MAX_FIELD_NUMBER}], got {field_number}"
        );
        Self {
            field_number,
            wire_type,
        }
    }

    /// Returns the field number carried by this tag.
    #[inline]
    pub fn field_number(&self) -> u32 {
        self.field_number
    }

    /// Returns the wire type carried by this tag.
    #[inline]
    pub fn wire_type(&self) -> WireType {
        self.wire_type
    }

    /// Encode a tag to a buffer.
    #[inline]
    pub fn encode(&self, buf: &mut impl BufMut) {
        // Cast to u64 before shifting to avoid overflow for large (invalid)
        // field numbers; valid field numbers fit in 29 bits so the result
        // always fits in 32 bits when the field number is in range.
        let value = ((self.field_number as u64) << 3) | (self.wire_type as u64);
        encode_varint(value, buf);
    }

    /// Decode a tag from a buffer.
    ///
    /// Single-byte tags (field numbers 1–15, any wire type) are handled
    /// inline without a call into [`decode_varint`]. With plain `#[inline]`,
    /// LLVM often declines to inline `decode_varint` (three code paths:
    /// single-byte, unrolled-slice, slow-fallback) into per-field decode
    /// loops, so handling the one-byte case here avoids the out-of-line
    /// call for the overwhelmingly common case.
    #[inline]
    pub fn decode(buf: &mut impl Buf) -> Result<Self, DecodeError> {
        // Fast path: one-byte tag. Covers field numbers 1–15 with any
        // wire type (bits 0-2 = wire type, bits 3-6 = field number, bit 7
        // clear). For protos that keep frequently-used fields in the 1–15
        // range — which the style guides recommend — this is the only
        // branch the decode loop ever takes.
        let chunk = buf.chunk();
        if !chunk.is_empty() && chunk[0] < 0x80 {
            let b = chunk[0];
            buf.advance(1);
            return Self::from_raw_u32(b as u32);
        }

        // Multi-byte tag (field number ≥ 16).
        let value = decode_varint(buf)?;
        // A tag value above u32::MAX implies a field number above 2^29 – 1
        // (the protobuf maximum), since the lower three bits carry the wire
        // type and the remaining bits carry the field number.
        if value > u32::MAX as u64 {
            return Err(DecodeError::InvalidFieldNumber);
        }
        Self::from_raw_u32(value as u32)
    }

    /// Construct a tag from its raw u32 wire representation
    /// (`field_number << 3 | wire_type`).
    ///
    /// Validates the wire type and rejects field number 0. `decode` guards
    /// against values above `u32::MAX` before casting down and calling this.
    #[inline]
    fn from_raw_u32(value: u32) -> Result<Self, DecodeError> {
        let wire_type = WireType::from_u32(value & 0x07)?;
        let field_number = value >> 3;
        if field_number == 0 {
            return Err(DecodeError::InvalidFieldNumber);
        }
        // `field_number` is a u32 right-shifted by 3, so it is bounded by
        // u32::MAX >> 3 = MAX_FIELD_NUMBER. No upper range check required.
        Ok(Tag {
            field_number,
            wire_type,
        })
    }
}

/// Encode a varint to a buffer.
///
/// Terminates in at most 10 iterations (⌈ 64/7 ⌉) for any `u64` input
/// because `value >>= 7` monotonically decreases and eventually satisfies
/// `value < 0x80`. An unbounded `loop` is used intentionally: a bounded
/// `for _ in 0..10` adds loop-counter overhead that LLVM cannot eliminate
/// (it cannot prove the inner `return` always fires), and this function is
/// called for every tag and varint field on the encode hot path.
#[inline]
pub fn encode_varint(mut value: u64, buf: &mut impl BufMut) {
    loop {
        if value < 0x80 {
            buf.put_u8(value as u8);
            return;
        }
        buf.put_u8(((value & 0x7F) | 0x80) as u8);
        value >>= 7;
    }
}

/// Decode a varint from a buffer.
///
/// Uses a chunk-based strategy for performance:
/// 1. Single-byte fast path for values < 128 (common for tags, small lengths).
/// 2. Unrolled slice decode when the contiguous chunk is large enough.
/// 3. Byte-at-a-time fallback for non-contiguous or fragmented buffers.
#[inline]
pub fn decode_varint(buf: &mut impl Buf) -> Result<u64, DecodeError> {
    let chunk = buf.chunk();
    let len = chunk.len();
    if len == 0 {
        return Err(DecodeError::UnexpectedEof);
    }

    // Fast path: single-byte varint (values 0–127). This covers field tags
    // for field numbers 1–15 and many small integer values.
    let first = chunk[0];
    if first < 0x80 {
        buf.advance(1);
        return Ok(first as u64);
    }

    // The chunk either contains the full varint (len > 10, or the last byte
    // in the chunk has its continuation bit clear) or it may be split across
    // chunks. In the first case we can decode directly from the slice.
    if len > 10 || chunk[len - 1] < 0x80 {
        let (value, advance) = decode_varint_slice(chunk)?;
        buf.advance(advance);
        Ok(value)
    } else {
        decode_varint_slow(buf)
    }
}

/// Decode a varint from a contiguous byte slice, returning the value and the
/// number of bytes consumed.
///
/// The caller must ensure that `bytes` is non-empty and that either
/// `bytes.len() > 10` or the last byte in `bytes` has its continuation bit
/// clear (< 0x80). Under these conditions every index up to the terminating
/// byte is guaranteed to be in bounds, so no per-byte bounds check is needed
/// beyond the initial assertions.
///
/// # Panics
///
/// Panics if `bytes` is empty or if the last byte has its continuation bit
/// set while `bytes.len() <= 10`. These conditions are guaranteed by the
/// caller (`decode_varint`), so the assertions serve as optimizer hints.
#[inline]
fn decode_varint_slice(bytes: &[u8]) -> Result<(u64, usize), DecodeError> {
    // These assertions are always satisfied by `decode_varint`'s dispatch
    // logic and exist so the optimizer can prove all subsequent indexing is
    // in-bounds, eliminating per-byte bounds checks after inlining.
    assert!(!bytes.is_empty());
    assert!(bytes.len() > 10 || bytes[bytes.len() - 1] < 0x80);

    // Unrolled varint decoding split into three 32-bit accumulators to reduce
    // 64-bit arithmetic on 32-bit targets and improve pipelining everywhere.

    let mut b: u8 = bytes[0];
    let mut part0: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((u64::from(part0), 1));
    }
    part0 -= 0x80;

    b = bytes[1];
    part0 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((u64::from(part0), 2));
    }
    part0 -= 0x80 << 7;

    b = bytes[2];
    part0 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((u64::from(part0), 3));
    }
    part0 -= 0x80 << 14;

    b = bytes[3];
    part0 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((u64::from(part0), 4));
    }
    part0 -= 0x80 << 21;

    let value = u64::from(part0);

    b = bytes[4];
    let mut part1: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 5));
    }
    part1 -= 0x80;

    b = bytes[5];
    part1 += u32::from(b) << 7;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 6));
    }
    part1 -= 0x80 << 7;

    b = bytes[6];
    part1 += u32::from(b) << 14;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 7));
    }
    part1 -= 0x80 << 14;

    b = bytes[7];
    part1 += u32::from(b) << 21;
    if b < 0x80 {
        return Ok((value + (u64::from(part1) << 28), 8));
    }
    part1 -= 0x80 << 21;

    let value = value + (u64::from(part1) << 28);

    b = bytes[8];
    let mut part2: u32 = u32::from(b);
    if b < 0x80 {
        return Ok((value + (u64::from(part2) << 56), 9));
    }
    part2 -= 0x80;

    b = bytes[9];
    part2 += u32::from(b) << 7;

    // 10th byte: only bit 0 maps to bit 63 of the result. A byte >= 0x02
    // means either overflow bits are set or the continuation bit implies an
    // 11th byte — both are malformed.
    if b >= 0x02 {
        return Err(DecodeError::VarintTooLong);
    }

    Ok((value + (u64::from(part2) << 56), 10))
}

/// Byte-at-a-time varint decode for non-contiguous or fragmented buffers.
///
/// This is the slow path used when the contiguous chunk from `buf.chunk()`
/// does not contain the complete varint. Marked `#[cold]` because this path
/// is rarely taken with typical `Bytes` or `&[u8]` inputs.
#[inline(never)]
#[cold]
fn decode_varint_slow(buf: &mut impl Buf) -> Result<u64, DecodeError> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    let limit = core::cmp::min(10, buf.remaining());
    for _ in 0..limit {
        let byte = buf.get_u8();
        if shift < 63 {
            value |= ((byte & 0x7F) as u64) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
            shift += 7;
        } else {
            // 10th byte: only bit 0 maps to bit 63 of the result. A byte
            // > 0x01 means either data overflow (bits 1-6 set) or an 11th
            // byte (continuation bit 0x80 set). This is equivalent to the
            // `b >= 0x02` check in `decode_varint_slice`.
            if byte > 0x01 {
                return Err(DecodeError::VarintTooLong);
            }
            value |= (byte as u64) << 63;
            return Ok(value);
        }
    }
    Err(DecodeError::UnexpectedEof)
}

/// Compute the encoded length of a varint.
#[inline]
pub const fn varint_len(value: u64) -> usize {
    if value == 0 {
        return 1;
    }
    let bits = 64 - value.leading_zeros() as usize;
    bits.div_ceil(7)
}

/// Skip one field value from `buf` according to the wire type in `tag`.
///
/// Used by generated [`Message::merge`](crate::message::Message::merge)
/// implementations to advance past unknown or unrecognised fields during
/// decoding.  After this call `buf` is positioned immediately after the
/// skipped field, ready for the next tag.
///
///
/// # Errors
///
/// Returns an error if the buffer is too short, if a length-delimited payload
/// length overflows `usize`, or if the wire type is a group.
#[inline]
pub fn skip_field(tag: Tag, buf: &mut impl Buf) -> Result<(), DecodeError> {
    skip_field_depth(tag, buf, crate::RECURSION_LIMIT)
}

/// Skip a field's payload, with an explicit recursion depth budget for groups.
///
/// Generated code must call this (not [`skip_field`]) when a `depth` parameter
/// is in scope, to prevent unknown group fields from resetting the recursion
/// budget and allowing depth-doubling attacks.
///
/// `depth` is the remaining nesting budget. For group fields this function
/// calls itself recursively, decrementing `depth` by one each level.
///
/// # Errors
///
/// Returns an error if the buffer is too short, if a length-delimited payload
/// length overflows `usize`, if the wire type is `EndGroup` (malformed stream),
/// or if group nesting exceeds `depth`.
pub fn skip_field_depth(tag: Tag, buf: &mut impl Buf, depth: u32) -> Result<(), DecodeError> {
    match tag.wire_type() {
        WireType::Varint => {
            decode_varint(buf)?;
        }
        WireType::Fixed64 => {
            if buf.remaining() < 8 {
                return Err(DecodeError::UnexpectedEof);
            }
            buf.advance(8);
        }
        WireType::LengthDelimited => {
            let len = decode_varint(buf)?;
            let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
            if buf.remaining() < len {
                return Err(DecodeError::UnexpectedEof);
            }
            buf.advance(len);
        }
        WireType::Fixed32 => {
            if buf.remaining() < 4 {
                return Err(DecodeError::UnexpectedEof);
            }
            buf.advance(4);
        }
        WireType::StartGroup => {
            let depth = depth
                .checked_sub(1)
                .ok_or(DecodeError::RecursionLimitExceeded)?;
            // Skip nested fields until the matching EndGroup tag.
            loop {
                let nested_tag = Tag::decode(buf)?;
                if nested_tag.wire_type() == WireType::EndGroup {
                    if nested_tag.field_number() != tag.field_number() {
                        return Err(DecodeError::InvalidEndGroup(nested_tag.field_number()));
                    }
                    break;
                }
                skip_field_depth(nested_tag, buf, depth)?;
            }
        }
        // EndGroup is consumed by the StartGroup handler above; seeing one
        // here means the stream is malformed.
        wt => {
            return Err(DecodeError::InvalidWireType(wt as u8 as u32));
        }
    }
    Ok(())
}

/// Decode one unknown field's value from `buf` and return it as an
/// [`UnknownField`](crate::unknown_fields::UnknownField).
///
/// The `tag` must already have been decoded; this function reads only the
/// payload that follows it on the wire. Groups are decoded recursively until
/// their matching `EndGroup` tag.
///
/// `depth` is the remaining nesting budget.  For group fields this function
/// calls itself recursively, decrementing `depth` by one each time.  When it
/// reaches zero [`DecodeError::RecursionLimitExceeded`] is returned.  Pass
/// [`crate::message::RECURSION_LIMIT`] at the outermost call site; generated
/// code passes the `depth` value received by the enclosing `merge`.
///
/// # Errors
///
/// Returns an error if the buffer is truncated, the wire type is
/// `EndGroup` (which indicates a structural mismatch in the wire data),
/// or the recursion limit is exceeded.
///
/// # Allocation
///
/// Length-delimited unknown fields allocate `vec![0u8; len]` where `len`
/// comes from wire data. The `buf.remaining() < len` check prevents
/// reading past the buffer, but callers processing untrusted input should
/// limit the input buffer size to bound maximum allocation.
pub fn decode_unknown_field(
    tag: Tag,
    buf: &mut impl Buf,
    depth: u32,
) -> Result<crate::unknown_fields::UnknownField, DecodeError> {
    use crate::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

    let data = match tag.wire_type() {
        WireType::Varint => UnknownFieldData::Varint(decode_varint(buf)?),
        WireType::Fixed64 => {
            if buf.remaining() < 8 {
                return Err(DecodeError::UnexpectedEof);
            }
            UnknownFieldData::Fixed64(buf.get_u64_le())
        }
        WireType::Fixed32 => {
            if buf.remaining() < 4 {
                return Err(DecodeError::UnexpectedEof);
            }
            UnknownFieldData::Fixed32(buf.get_u32_le())
        }
        WireType::LengthDelimited => {
            let len = decode_varint(buf)?;
            let len = usize::try_from(len).map_err(|_| DecodeError::MessageTooLarge)?;
            if buf.remaining() < len {
                return Err(DecodeError::UnexpectedEof);
            }
            let mut data = alloc::vec![0u8; len];
            buf.copy_to_slice(&mut data);
            UnknownFieldData::LengthDelimited(data)
        }
        WireType::StartGroup => {
            let depth = depth
                .checked_sub(1)
                .ok_or(DecodeError::RecursionLimitExceeded)?;
            let group_field_number = tag.field_number();
            // Read nested fields until the matching EndGroup tag.
            let mut nested = UnknownFields::new();
            loop {
                let nested_tag = Tag::decode(buf)?;
                if nested_tag.wire_type() == WireType::EndGroup {
                    // Per the protobuf spec the EndGroup tag must carry the same
                    // field number as the opening StartGroup tag.
                    if nested_tag.field_number() != group_field_number {
                        return Err(DecodeError::InvalidEndGroup(nested_tag.field_number()));
                    }
                    break;
                }
                nested.push(decode_unknown_field(nested_tag, buf, depth)?);
            }
            UnknownFieldData::Group(nested)
        }
        wt => return Err(DecodeError::InvalidWireType(wt as u8 as u32)),
    };
    Ok(UnknownField {
        number: tag.field_number(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        let test_values: &[u64] = &[0, 1, 127, 128, 255, 300, 16384, u64::MAX];
        for &v in test_values {
            let mut buf = Vec::new();
            encode_varint(v, &mut buf);
            assert_eq!(buf.len(), varint_len(v), "varint_len mismatch for {v}");
            let decoded = decode_varint(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_tag_roundtrip() {
        let tag = Tag::new(1, WireType::Varint);
        let mut buf = Vec::new();
        tag.encode(&mut buf);
        let decoded = Tag::decode(&mut buf.as_slice()).unwrap();
        assert_eq!(tag, decoded);
    }

    #[test]
    fn test_tag_high_field_number() {
        let tag = Tag::new(MAX_FIELD_NUMBER, WireType::LengthDelimited);
        let mut buf = Vec::new();
        tag.encode(&mut buf);
        let decoded = Tag::decode(&mut buf.as_slice()).unwrap();
        assert_eq!(tag, decoded);
    }

    #[test]
    fn test_zero_field_number_rejected() {
        // Field number 0 is invalid in protobuf
        let mut buf = Vec::new();
        encode_varint(0b0000_0000, &mut buf); // field 0, wire type 0
        let result = Tag::decode(&mut buf.as_slice());
        assert!(result.is_err());
    }

    #[test]
    fn test_oversize_field_number_rejected() {
        // Field number 2^29 = 536_870_912 encodes to a tag value of
        // 536_870_912 << 3 = 2^32, which exceeds u32::MAX and is rejected
        // by the overflow check in Tag::decode.
        let mut buf = Vec::new();
        encode_varint(536_870_912u64 << 3, &mut buf);
        assert_eq!(
            Tag::decode(&mut buf.as_slice()),
            Err(DecodeError::InvalidFieldNumber)
        );
    }

    #[test]
    fn test_tag_single_byte_fast_path() {
        // The fast path handles all one-byte tags: field numbers 1-15,
        // any wire type. Verify the boundary at both ends and that invalid
        // wire types / field 0 are still rejected through the fast path.
        #[rustfmt::skip]
        let cases: &[(u8, Option<(u32, WireType)>)] = &[
            (0x08, Some((1,  WireType::Varint))),          // field 1, wire 0 — smallest valid
            (0x0A, Some((1,  WireType::LengthDelimited))), // field 1, wire 2
            (0x7D, Some((15, WireType::Fixed32))),         // field 15, wire 5 — largest one-byte
            (0x78, Some((15, WireType::Varint))),          // field 15, wire 0
            (0x00, None),                                  // field 0 — invalid through fast path
            (0x07, None),                                  // field 0, wire 7 — invalid wire type also caught
            (0x0E, None),                                  // field 1, wire 6 — invalid wire type
        ];
        for &(byte, expected) in cases {
            let buf = [byte];
            let result = Tag::decode(&mut &buf[..]);
            match expected {
                Some((fn_, wt)) => {
                    let t = result.unwrap_or_else(|e| panic!("byte {byte:#04x}: {e:?}"));
                    assert_eq!(t.field_number(), fn_, "byte {byte:#04x}");
                    assert_eq!(t.wire_type(), wt, "byte {byte:#04x}");
                }
                None => assert!(result.is_err(), "byte {byte:#04x} should be rejected"),
            }
        }
    }

    #[test]
    fn test_tag_field_16_takes_slow_path() {
        // Field number 16 with wire type 0 encodes as [0x80, 0x01] — two
        // bytes, so it should take the multi-byte path via decode_varint.
        let tag = Tag::new(16, WireType::Varint);
        let mut buf = Vec::new();
        tag.encode(&mut buf);
        assert_eq!(buf, [0x80, 0x01]); // continuation bit set on first byte
        let decoded = Tag::decode(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded.field_number(), 16);
        assert_eq!(decoded.wire_type(), WireType::Varint);
    }

    #[test]
    fn test_varint_u64_max_roundtrip() {
        // u64::MAX requires all 10 bytes with the 10th byte == 0x01.
        let mut buf = Vec::new();
        encode_varint(u64::MAX, &mut buf);
        assert_eq!(buf.len(), 10);
        assert_eq!(buf[9], 0x01); // 10th byte must be exactly 0x01
        let decoded = decode_varint(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded, u64::MAX);
    }

    #[test]
    fn test_varint_10th_byte_overflow_rejected() {
        // 10th byte with overflow bits set (0x02–0x7F) must be rejected.
        // This encodes a value that would overflow u64.
        let mut buf: Vec<u8> = vec![0xFF; 9]; // 9 continuation bytes
        buf.push(0x02); // 10th byte: bit 1 set → overflow
        assert_eq!(
            decode_varint(&mut buf.as_slice()),
            Err(DecodeError::VarintTooLong)
        );
    }

    #[test]
    fn test_varint_11th_byte_rejected() {
        // 10th byte with continuation bit set implies an 11th byte → always malformed.
        let buf: Vec<u8> = vec![0xFF; 10]; // 10 continuation bytes
        assert_eq!(
            decode_varint(&mut buf.as_slice()),
            Err(DecodeError::VarintTooLong)
        );
    }

    #[test]
    fn test_skip_field_varint() {
        // Encode field 1 = varint 300, then check skip consumes it.
        let mut buf = Vec::new();
        let tag = Tag::new(1, WireType::Varint);
        tag.encode(&mut buf);
        encode_varint(300, &mut buf);
        // Prepend a second tag/value to verify skip stops at the right byte.
        let mut combined = buf.clone();
        let tag2 = Tag::new(2, WireType::Varint);
        tag2.encode(&mut combined);
        encode_varint(1, &mut combined);

        let slice = &mut combined.as_slice();
        let t = Tag::decode(slice).unwrap();
        skip_field(t, slice).unwrap();
        // After skipping field 1, we should see tag 2.
        let t2 = Tag::decode(slice).unwrap();
        assert_eq!(t2.field_number(), 2);
    }

    #[test]
    fn test_skip_field_fixed32() {
        let mut buf = Vec::new();
        Tag::new(1, WireType::Fixed32).encode(&mut buf);
        buf.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

        let slice = &mut buf.as_slice();
        let t = Tag::decode(slice).unwrap();
        skip_field(t, slice).unwrap();
        assert!(slice.is_empty());
    }

    #[test]
    fn test_skip_field_fixed64() {
        let mut buf = Vec::new();
        Tag::new(1, WireType::Fixed64).encode(&mut buf);
        buf.extend_from_slice(&[0u8; 8]);

        let slice = &mut buf.as_slice();
        let t = Tag::decode(slice).unwrap();
        skip_field(t, slice).unwrap();
        assert!(slice.is_empty());
    }

    #[test]
    fn test_skip_field_length_delimited() {
        let mut buf = Vec::new();
        Tag::new(1, WireType::LengthDelimited).encode(&mut buf);
        encode_varint(3, &mut buf); // length = 3
        buf.extend_from_slice(&[0xAA, 0xBB, 0xCC]);

        let slice = &mut buf.as_slice();
        let t = Tag::decode(slice).unwrap();
        skip_field(t, slice).unwrap();
        assert!(slice.is_empty());
    }

    #[test]
    fn test_skip_field_truncated_returns_error() {
        let mut buf = Vec::new();
        Tag::new(1, WireType::Fixed32).encode(&mut buf);
        buf.extend_from_slice(&[0x01, 0x02]); // only 2 bytes, not 4

        let slice = &mut buf.as_slice();
        let t = Tag::decode(slice).unwrap();
        assert_eq!(skip_field(t, slice), Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn test_skip_field_start_group_empty_buf() {
        let tag = Tag::new(1, WireType::StartGroup);
        let mut buf: &[u8] = &[];
        assert_eq!(skip_field(tag, &mut buf), Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn test_skip_field_start_group_with_end() {
        // Build: StartGroup(1) already consumed, then varint field 2 = 42,
        // then EndGroup(1).
        let mut data = Vec::new();
        Tag::new(2, WireType::Varint).encode(&mut data);
        encode_varint(42, &mut data);
        Tag::new(1, WireType::EndGroup).encode(&mut data);

        let tag = Tag::new(1, WireType::StartGroup);
        let mut buf: &[u8] = &data;
        assert_eq!(skip_field(tag, &mut buf), Ok(()));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_skip_field_start_group_wrong_end() {
        // EndGroup with mismatched field number.
        let mut data = Vec::new();
        Tag::new(99, WireType::EndGroup).encode(&mut data);

        let tag = Tag::new(1, WireType::StartGroup);
        let mut buf: &[u8] = &data;
        assert_eq!(
            skip_field(tag, &mut buf),
            Err(DecodeError::InvalidEndGroup(99))
        );
    }

    #[test]
    fn test_skip_field_end_group_returns_invalid_wire_type() {
        let tag = Tag::new(1, WireType::EndGroup);
        let mut buf: &[u8] = &[];
        assert_eq!(
            skip_field(tag, &mut buf),
            Err(DecodeError::InvalidWireType(4))
        );
    }

    // ---- decode_unknown_field: group recursion limit ----------------------

    /// Build the bytes that follow an already-decoded StartGroup tag for
    /// `field_number`: zero or more inner fields then the matching EndGroup.
    fn encode_group_payload(field_number: u32, inner: &[u8]) -> Vec<u8> {
        let mut buf = inner.to_vec();
        Tag::new(field_number, WireType::EndGroup).encode(&mut buf);
        buf
    }

    #[test]
    fn test_decode_unknown_field_group_at_depth_1_succeeds() {
        // depth = 1: checked_sub(1) = 0, which is the floor but still Ok.
        let payload = encode_group_payload(1, &[]);
        let tag = Tag::new(1, WireType::StartGroup);
        let result = decode_unknown_field(tag, &mut payload.as_slice(), 1);
        assert!(result.is_ok());
    }

    #[test]
    fn test_decode_unknown_field_group_at_depth_0_exceeds_limit() {
        // depth = 0: checked_sub(1) returns None → RecursionLimitExceeded.
        let payload = encode_group_payload(1, &[]);
        let tag = Tag::new(1, WireType::StartGroup);
        assert_eq!(
            decode_unknown_field(tag, &mut payload.as_slice(), 0),
            Err(DecodeError::RecursionLimitExceeded)
        );
    }

    #[test]
    fn test_decode_unknown_field_end_group_mismatched_field_number() {
        // Group opened as field 1 but closed with field 2's EndGroup tag.
        let mut payload = Vec::new();
        Tag::new(2, WireType::EndGroup).encode(&mut payload); // wrong field number
        let tag = Tag::new(1, WireType::StartGroup);
        assert_eq!(
            decode_unknown_field(tag, &mut payload.as_slice(), 1),
            Err(DecodeError::InvalidEndGroup(2))
        );
    }

    #[test]
    fn test_decode_unknown_field_all_wire_types() {
        // Table-driven: each wire type → expected UnknownFieldData variant.
        // Covers Fixed32, Fixed64, LengthDelimited, and StartGroup with
        // nested fields (the Varint and empty-group cases are above).
        use crate::unknown_fields::{UnknownField, UnknownFieldData};

        struct Case {
            tag: Tag,
            payload: Vec<u8>,
            expected: UnknownFieldData,
        }
        let cases = vec![
            // Varint: 300 encoded as [0xAC, 0x02].
            Case {
                tag: Tag::new(1, WireType::Varint),
                payload: {
                    let mut b = Vec::new();
                    encode_varint(300, &mut b);
                    b
                },
                expected: UnknownFieldData::Varint(300),
            },
            // Fixed32: 0xDEADBEEF little-endian.
            Case {
                tag: Tag::new(2, WireType::Fixed32),
                payload: 0xDEAD_BEEF_u32.to_le_bytes().to_vec(),
                expected: UnknownFieldData::Fixed32(0xDEAD_BEEF),
            },
            // Fixed64: full-width value little-endian.
            Case {
                tag: Tag::new(3, WireType::Fixed64),
                payload: 0x1234_5678_9ABC_DEF0_u64.to_le_bytes().to_vec(),
                expected: UnknownFieldData::Fixed64(0x1234_5678_9ABC_DEF0),
            },
            // LengthDelimited: len-prefix 3 + bytes "abc".
            Case {
                tag: Tag::new(4, WireType::LengthDelimited),
                payload: {
                    let mut b = Vec::new();
                    encode_varint(3, &mut b);
                    b.extend_from_slice(b"abc");
                    b
                },
                expected: UnknownFieldData::LengthDelimited(b"abc".to_vec()),
            },
            // StartGroup with nested varint field 1 = 42.
            Case {
                tag: Tag::new(5, WireType::StartGroup),
                payload: {
                    let mut inner = Vec::new();
                    Tag::new(1, WireType::Varint).encode(&mut inner);
                    encode_varint(42, &mut inner);
                    encode_group_payload(5, &inner)
                },
                expected: UnknownFieldData::Group({
                    let mut fields = crate::unknown_fields::UnknownFields::new();
                    fields.push(UnknownField {
                        number: 1,
                        data: UnknownFieldData::Varint(42),
                    });
                    fields
                }),
            },
        ];

        for case in cases {
            let got = decode_unknown_field(
                case.tag,
                &mut case.payload.as_slice(),
                crate::RECURSION_LIMIT,
            )
            .unwrap_or_else(|e| panic!("decode failed for tag {:?}: {e}", case.tag));
            assert_eq!(got.number, case.tag.field_number());
            assert_eq!(got.data, case.expected, "tag {:?}", case.tag);
        }
    }

    #[test]
    fn test_decode_unknown_field_eof_rejection() {
        // Each fixed-width / length-delimited wire type with truncated payload.
        #[rustfmt::skip]
        let cases: &[(Tag, &[u8])] = &[
            // Fixed32 needs 4 bytes; give 3.
            (Tag::new(1, WireType::Fixed32), &[0x01, 0x02, 0x03]),
            // Fixed64 needs 8 bytes; give 7.
            (Tag::new(2, WireType::Fixed64), &[0; 7]),
            // LengthDelimited: len=10 but only 2 payload bytes.
            (Tag::new(3, WireType::LengthDelimited), &[0x0A, 0xAA, 0xBB]),
        ];
        for &(tag, payload) in cases {
            assert_eq!(
                decode_unknown_field(tag, &mut &payload[..], crate::RECURSION_LIMIT),
                Err(DecodeError::UnexpectedEof),
                "tag {tag:?}"
            );
        }
    }

    #[test]
    fn test_decode_unknown_field_invalid_wire_type() {
        // EndGroup as a top-level tag is invalid (only valid inside a group).
        let tag = Tag::new(1, WireType::EndGroup);
        assert_eq!(
            decode_unknown_field(tag, &mut &[][..], crate::RECURSION_LIMIT),
            Err(DecodeError::InvalidWireType(4))
        );
    }

    #[test]
    fn test_decode_unknown_field_round_trip_via_unknown_fields() {
        // Encode an UnknownFields set → decode each back → compare.
        // Verifies decode_unknown_field ⇔ UnknownFields::write_to parity
        // across all wire types.
        use crate::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

        let mut original = UnknownFields::new();
        original.push(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(u64::MAX),
        });
        original.push(UnknownField {
            number: 2,
            data: UnknownFieldData::Fixed32(0xFFFF_FFFF),
        });
        original.push(UnknownField {
            number: 3,
            data: UnknownFieldData::Fixed64(0),
        });
        original.push(UnknownField {
            number: 4,
            data: UnknownFieldData::LengthDelimited(vec![]),
        });
        original.push(UnknownField {
            number: 5,
            data: UnknownFieldData::LengthDelimited(vec![0xFF; 200]),
        });

        let mut buf = Vec::new();
        original.write_to(&mut buf);
        assert_eq!(original.encoded_len(), buf.len());

        // Decode all fields back.
        let mut decoded = UnknownFields::new();
        let mut cur = buf.as_slice();
        while !cur.is_empty() {
            let tag = Tag::decode(&mut cur).unwrap();
            decoded.push(decode_unknown_field(tag, &mut cur, crate::RECURSION_LIMIT).unwrap());
        }
        assert_eq!(decoded, original);
    }

    // ---- decode_varint_slice: direct slice decode --------------------------

    #[test]
    fn test_decode_varint_slice_all_sizes() {
        // Exercise the unrolled slice decoder for every varint length (1–10 bytes).
        let test_values: &[u64] = &[
            0,        // 1 byte
            128,      // 2 bytes
            1 << 14,  // 3 bytes
            1 << 21,  // 4 bytes
            1 << 28,  // 5 bytes
            1 << 35,  // 6 bytes
            1 << 42,  // 7 bytes
            1 << 49,  // 8 bytes
            1 << 56,  // 9 bytes
            u64::MAX, // 10 bytes
        ];
        for &v in test_values {
            let mut buf = Vec::new();
            encode_varint(v, &mut buf);
            let (decoded, advance) = decode_varint_slice(&buf).unwrap();
            assert_eq!(v, decoded, "decode_varint_slice failed for {v}");
            assert_eq!(buf.len(), advance, "advance mismatch for {v}");
        }
    }

    #[test]
    fn test_decode_varint_slice_overflow_rejected() {
        // 10th byte with overflow bit: 9 continuation bytes + 0x02.
        let mut buf: Vec<u8> = vec![0xFF; 9];
        buf.push(0x02);
        assert_eq!(decode_varint_slice(&buf), Err(DecodeError::VarintTooLong));
    }

    // ---- decode_varint_slow: byte-at-a-time fallback -----------------------

    #[test]
    fn test_decode_varint_slow_roundtrip() {
        let test_values: &[u64] = &[
            0,
            1,
            127,
            128,
            300,
            16384,
            // Power-of-7 transition boundaries (2^7, 2^14, 2^21, ...).
            1 << 7,
            (1 << 7) - 1,
            1 << 14,
            (1 << 14) - 1,
            1 << 21,
            1 << 28,
            1 << 35,
            1 << 42,
            1 << 49,
            1 << 56,
            1 << 63,
            u64::MAX,
        ];
        for &v in test_values {
            let mut buf = Vec::new();
            encode_varint(v, &mut buf);
            let decoded = decode_varint_slow(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded, "slow path roundtrip failed for {v}");
        }
    }

    #[test]
    fn test_decode_varint_slow_overflow_rejected() {
        let mut buf: Vec<u8> = vec![0xFF; 9];
        buf.push(0x02);
        assert_eq!(
            decode_varint_slow(&mut buf.as_slice()),
            Err(DecodeError::VarintTooLong)
        );
    }

    #[test]
    fn test_decode_varint_empty_buffer() {
        let mut buf: &[u8] = &[];
        assert_eq!(decode_varint(&mut buf), Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn test_decode_varint_slow_path_via_fragmented_buffer() {
        // Exercise the slow path through `decode_varint` by using a
        // `Chain` buffer where the varint straddles two chunks. The
        // first chunk ends with a continuation byte, so `chunk()[last]`
        // has its high bit set, triggering the slow-path dispatch.
        use bytes::Buf;

        let test_values: &[u64] = &[128, 300, 16384, 1 << 28, u64::MAX];
        for &v in test_values {
            let mut encoded = Vec::new();
            encode_varint(v, &mut encoded);
            // Split in the middle so each chunk ends/starts on a
            // continuation byte.
            let mid = encoded.len() / 2;
            let first_half = bytes::Bytes::copy_from_slice(&encoded[..mid]);
            let second_half = bytes::Bytes::copy_from_slice(&encoded[mid..]);
            let mut chain = first_half.chain(second_half);
            let decoded = decode_varint(&mut chain).unwrap();
            assert_eq!(v, decoded, "fragmented buffer roundtrip failed for {v}");
            assert_eq!(chain.remaining(), 0);
        }
    }

    #[test]
    fn test_decode_varint_single_byte_fast_path() {
        // Values 0–127 should hit the single-byte fast path.
        for v in 0..=127u64 {
            let mut buf = Vec::new();
            encode_varint(v, &mut buf);
            assert_eq!(buf.len(), 1);
            let decoded = decode_varint(&mut buf.as_slice()).unwrap();
            assert_eq!(v, decoded);
        }
    }

    // --- 32-bit specific tests ---
    // These exercise the `MessageTooLarge` error path in `skip_field` and
    // `decode_unknown_field` when a length-delimited field has a varint length
    // prefix that exceeds `usize::MAX` on 32-bit targets.

    /// Varint encoding of 0x1_0000_0000 (u32::MAX + 1) — exceeds 32-bit usize.
    #[cfg(target_pointer_width = "32")]
    const OVERSIZED_VARINT: &[u8] = &[0x80, 0x80, 0x80, 0x80, 0x10];

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn skip_field_rejects_oversized_length_on_32bit() {
        let tag = Tag::new(1, WireType::LengthDelimited);
        let mut buf: &[u8] = OVERSIZED_VARINT;
        assert_eq!(skip_field(tag, &mut buf), Err(DecodeError::MessageTooLarge));
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn decode_unknown_field_rejects_oversized_length_on_32bit() {
        let tag = Tag::new(1, WireType::LengthDelimited);
        let mut buf: &[u8] = OVERSIZED_VARINT;
        assert_eq!(
            decode_unknown_field(tag, &mut buf, crate::RECURSION_LIMIT),
            Err(DecodeError::MessageTooLarge)
        );
    }
}
