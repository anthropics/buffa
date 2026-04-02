//! MessageSet wire format helpers (legacy Google-internal encoding).
//!
//! Enabled per-message via `option message_set_wire_format = true` on a
//! proto2 message with no fields, only `extensions N to M;`. Each extension
//! is encoded as a group at field 1 containing `type_id = 2` (the extension's
//! field number) and `message = 3` (the extension's payload bytes):
//!
//! ```text
//! tag(1, SGROUP) | tag(2, Varint) | <type_id> | tag(3, LD) | len | <payload> | tag(1, EGROUP)
//! ```
//!
//! instead of the regular `tag(type_id, LD) | len | <payload>`.
//!
//! Storage is **non-MessageSet**: an unknown item with `type_id = T` and
//! payload `V` is stored in [`UnknownFields`](crate::UnknownFields) as
//! `{ number: T, data: LengthDelimited(V) }`, not as a group. Decode unwraps
//! the group into that form; encode rewraps it. This mirrors protobuf-go.

use crate::encoding::{decode_varint, skip_field_depth, varint_len, Tag, WireType};
use crate::DecodeError;
use alloc::vec::Vec;
use bytes::Buf;

/// Field 1, wire type `StartGroup` (3): `(1 << 3) | 3`.
pub const ITEM_START_TAG: u64 = (1 << 3) | 3;
/// Field 1, wire type `EndGroup` (4): `(1 << 3) | 4`.
pub const ITEM_END_TAG: u64 = (1 << 3) | 4;
/// Field 2, wire type `Varint` (0): `(2 << 3) | 0`.
pub const TYPE_ID_TAG: u64 = 2 << 3;
/// Field 3, wire type `LengthDelimited` (2): `(3 << 3) | 2`.
pub const MESSAGE_TAG: u64 = (3 << 3) | 2;

/// Parse one MessageSet `Item` group body (between `SGROUP` and `EGROUP`).
///
/// The caller must have already consumed the `ITEM_START_TAG`; this function
/// reads until the matching `EGROUP` (field 1). The `type_id` and `message`
/// fields may arrive in either order. Unknown fields inside the group are
/// skipped. Repeated `message` fields are concatenated (proto merge
/// semantics); repeated `type_id` fields take the last value.
///
/// Returns `(type_id, message_bytes)`. `message_bytes` is empty if no
/// `message` field was present (valid: an empty sub-message).
///
/// `depth` is the remaining recursion budget for skipping unknown group fields
/// **inside** the Item group. The caller should pass `caller_depth - 1` (the
/// Item group itself consumes one level).
///
/// # Errors
///
/// Returns [`DecodeError::InvalidMessageSet`] if `type_id` is missing or out
/// of the valid range `[1, i32::MAX]`. Returns other decode errors on
/// malformed input (truncated varint, buffer underrun, mismatched end-group).
pub fn merge_item(buf: &mut impl Buf, depth: u32) -> Result<(u32, Vec<u8>), DecodeError> {
    let mut type_id: Option<u32> = None;
    let mut message: Vec<u8> = Vec::new();

    loop {
        let tag = Tag::decode(buf)?;
        // Item group terminator: field 1, EndGroup.
        if tag.field_number() == 1 && tag.wire_type() == WireType::EndGroup {
            break;
        }
        match (tag.field_number(), tag.wire_type()) {
            (2, WireType::Varint) => {
                let v = decode_varint(buf)?;
                // type_id is `required int32`; valid protobuf field numbers
                // are [1, 2^29-1], but MessageSet historically allows the
                // full positive int32 range.
                if v < 1 || v > i32::MAX as u64 {
                    return Err(DecodeError::InvalidMessageSet("type_id out of range"));
                }
                type_id = Some(v as u32);
            }
            (3, WireType::LengthDelimited) => {
                let len = decode_varint(buf)?;
                // Compare as u64 so a 32-bit build rejects a length that
                // overflows usize before the cast truncates it.
                if len > buf.remaining() as u64 {
                    return Err(DecodeError::UnexpectedEof);
                }
                let len = len as usize;
                let start = message.len();
                message.resize(start + len, 0);
                buf.copy_to_slice(&mut message[start..]);
            }
            (_, WireType::EndGroup) => {
                // EndGroup for a different field number — the stream is
                // malformed (we never opened that group).
                return Err(DecodeError::InvalidEndGroup(tag.field_number()));
            }
            _ => {
                // Unknown field inside the Item group: skip. Generated code
                // passes `depth - 1` into `merge_item`, so nested groups here
                // share the caller's recursion budget.
                skip_field_depth(tag, buf, depth)?;
            }
        }
    }

    let type_id = type_id.ok_or(DecodeError::InvalidMessageSet("missing type_id"))?;
    Ok((type_id, message))
}

/// Encoded length of one MessageSet `Item` wrapping `payload_len` bytes at
/// extension field `number`.
///
/// Four single-byte tags (fields 1, 2, 3 all fit in one byte) + the `type_id`
/// varint + the length-prefix varint + the payload.
#[inline]
pub const fn item_encoded_len(number: u32, payload_len: usize) -> usize {
    // ITEM_START_TAG (1) + TYPE_ID_TAG (1) + varint(number)
    //   + MESSAGE_TAG (1) + varint(payload_len) + payload_len
    //   + ITEM_END_TAG (1)
    4 + varint_len(number as u64) + varint_len(payload_len as u64) + payload_len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::encode_varint;

    /// Build a MessageSet Item group body (no SGROUP tag — `merge_item`
    /// assumes the caller consumed it).
    fn item_body(parts: &[&[u8]]) -> Vec<u8> {
        let mut buf = Vec::new();
        for p in parts {
            buf.extend_from_slice(p);
        }
        buf
    }

    fn type_id_field(id: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_varint(TYPE_ID_TAG, &mut buf);
        encode_varint(id, &mut buf);
        buf
    }

    fn message_field(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_varint(MESSAGE_TAG, &mut buf);
        encode_varint(payload.len() as u64, &mut buf);
        buf.extend_from_slice(payload);
        buf
    }

    fn end_group() -> Vec<u8> {
        let mut buf = Vec::new();
        encode_varint(ITEM_END_TAG, &mut buf);
        buf
    }

    #[test]
    fn tag_constants_match_wire_bytes() {
        // Field 1-3 all produce single-byte tags.
        assert_eq!(ITEM_START_TAG, 0x0B);
        assert_eq!(ITEM_END_TAG, 0x0C);
        assert_eq!(TYPE_ID_TAG, 0x10);
        assert_eq!(MESSAGE_TAG, 0x1A);
    }

    #[test]
    fn merge_item_type_id_then_message() {
        let body = item_body(&[&type_id_field(1000), &message_field(b"hello"), &end_group()]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 1000);
        assert_eq!(msg, b"hello");
    }

    #[test]
    fn merge_item_message_then_type_id() {
        let body = item_body(&[&message_field(b"world"), &type_id_field(42), &end_group()]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 42);
        assert_eq!(msg, b"world");
    }

    #[test]
    #[allow(clippy::identity_op)] // `| 0` makes the Varint wire type explicit
    fn merge_item_skips_unknown_fields() {
        // Junk field 99 (varint) between type_id and message.
        let mut junk = Vec::new();
        encode_varint((99 << 3) | 0, &mut junk); // tag(99, Varint)
        encode_varint(12345, &mut junk);
        let body = item_body(&[
            &type_id_field(7),
            &junk,
            &message_field(b"ok"),
            &end_group(),
        ]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 7);
        assert_eq!(msg, b"ok");
    }

    #[test]
    #[allow(clippy::identity_op)] // `| 0` makes the Varint wire type explicit
    fn merge_item_skips_nested_group_respecting_depth() {
        // Junk field 50, wire type StartGroup, containing a varint then EndGroup.
        let mut junk = Vec::new();
        encode_varint((50 << 3) | 3, &mut junk); // tag(50, SGROUP)
        encode_varint((8 << 3) | 0, &mut junk); // tag(8, Varint)
        encode_varint(1, &mut junk);
        encode_varint((50 << 3) | 4, &mut junk); // tag(50, EGROUP)

        let body = item_body(&[&type_id_field(5), &junk, &message_field(b"x"), &end_group()]);

        // With depth budget: succeeds.
        let (tid, msg) = merge_item(&mut body.as_slice(), 10).expect("merge");
        assert_eq!(tid, 5);
        assert_eq!(msg, b"x");

        // With depth exhausted: fails.
        let err = merge_item(&mut body.as_slice(), 0).unwrap_err();
        assert_eq!(err, DecodeError::RecursionLimitExceeded);
    }

    #[test]
    fn merge_item_missing_type_id_errors() {
        let body = item_body(&[&message_field(b"orphan"), &end_group()]);
        let err = merge_item(&mut body.as_slice(), 50).unwrap_err();
        assert_eq!(err, DecodeError::InvalidMessageSet("missing type_id"));
    }

    #[test]
    fn merge_item_missing_message_yields_empty() {
        // Missing `message` is valid — it's an empty sub-message.
        let body = item_body(&[&type_id_field(3), &end_group()]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 3);
        assert_eq!(msg, b"");
    }

    #[test]
    fn merge_item_multiple_messages_concatenate() {
        let body = item_body(&[
            &type_id_field(9),
            &message_field(b"ab"),
            &message_field(b"cd"),
            &end_group(),
        ]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 9);
        assert_eq!(msg, b"abcd");
    }

    #[test]
    fn merge_item_repeated_type_id_last_wins() {
        // Two type_id fields in one Item: the second overwrites the first.
        let body = item_body(&[
            &type_id_field(5),
            &type_id_field(99),
            &message_field(b"x"),
            &end_group(),
        ]);
        let (tid, msg) = merge_item(&mut body.as_slice(), 50).expect("merge");
        assert_eq!(tid, 99);
        assert_eq!(msg, b"x");
    }

    #[test]
    fn merge_item_type_id_out_of_range() {
        #[rustfmt::skip]
        let cases: &[(u64, bool)] = &[
            (0,                     false), // zero → error
            (1,                     true),  // minimum valid
            (i32::MAX as u64,       true),  // maximum valid
            (i32::MAX as u64 + 1,   false), // overflows int32
        ];
        for &(id, ok) in cases {
            let body = item_body(&[&type_id_field(id), &message_field(b""), &end_group()]);
            let result = merge_item(&mut body.as_slice(), 50);
            assert_eq!(result.is_ok(), ok, "type_id = {id}");
        }
    }

    #[test]
    fn merge_item_mismatched_end_group_errors() {
        // EndGroup for field 7 instead of field 1.
        let mut bad_end = Vec::new();
        encode_varint((7 << 3) | 4, &mut bad_end);
        let body = item_body(&[&type_id_field(1), &bad_end]);
        let err = merge_item(&mut body.as_slice(), 50).unwrap_err();
        assert_eq!(err, DecodeError::InvalidEndGroup(7));
    }

    #[test]
    fn merge_item_truncated_message_errors() {
        // Length prefix claims 100 bytes, only 2 available.
        let mut body = Vec::new();
        encode_varint(TYPE_ID_TAG, &mut body);
        encode_varint(5, &mut body);
        encode_varint(MESSAGE_TAG, &mut body);
        encode_varint(100, &mut body);
        body.extend_from_slice(b"xy");
        let err = merge_item(&mut body.as_slice(), 50).unwrap_err();
        assert_eq!(err, DecodeError::UnexpectedEof);
    }

    #[test]
    #[allow(clippy::identity_op)] // `+ 0` keeps the byte-count breakdown columns aligned
    fn item_encoded_len_matches_manual_count() {
        #[rustfmt::skip]
        let cases: &[(u32, usize, usize)] = &[
            // (number, payload_len, expected)
            (1,       0,   4 + 1 + 1 + 0),  // 1-byte number, 1-byte len
            (127,     5,   4 + 1 + 1 + 5),  // 127 = 1-byte varint
            (128,     5,   4 + 2 + 1 + 5),  // 128 = 2-byte varint
            (1000,    10,  4 + 2 + 1 + 10), // 1000 = 2-byte varint
            (1000,    200, 4 + 2 + 2 + 200),// 200 = 2-byte varint
        ];
        for &(number, payload_len, expected) in cases {
            assert_eq!(
                item_encoded_len(number, payload_len),
                expected,
                "number={number} payload_len={payload_len}"
            );
        }
    }

    #[test]
    fn item_encoded_len_matches_actual_encoding() {
        // Cross-check: hand-build an Item and compare its length.
        let number = 1000u32;
        let payload = b"hello world";
        let mut buf = Vec::new();
        encode_varint(ITEM_START_TAG, &mut buf);
        encode_varint(TYPE_ID_TAG, &mut buf);
        encode_varint(number as u64, &mut buf);
        encode_varint(MESSAGE_TAG, &mut buf);
        encode_varint(payload.len() as u64, &mut buf);
        buf.extend_from_slice(payload);
        encode_varint(ITEM_END_TAG, &mut buf);

        assert_eq!(item_encoded_len(number, payload.len()), buf.len());
    }
}
