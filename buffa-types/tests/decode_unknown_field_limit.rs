//! Regression tests for the unknown-field decode limit.
//!
//! Unknown wire data can occupy far more memory decoded than encoded — a
//! 2-byte varint field inflates to a ~40-byte `UnknownField`, so a 64 MiB
//! payload of minimal unknown fields used to force >1 GiB of heap. These
//! tests use `google.protobuf.Empty` as the receiving type (it has no
//! declared fields, so every payload byte routes through the unknown-field
//! path) and assert that the field-count limit bounds decoder memory
//! amplification independent of input size.

use buffa::{DecodeError, DecodeOptions, Message, DEFAULT_UNKNOWN_FIELD_LIMIT};
use buffa_types::Empty;

/// `n` minimal (2-byte) varint unknown fields: tag 0x08 (field 1, varint),
/// value 0.
fn flat_varint_flood(n: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 * n);
    for _ in 0..n {
        payload.extend_from_slice(&[0x08, 0x00]);
    }
    payload
}

/// The reported group-amplification payload: one unknown group (field 1)
/// holding `n` minimal varint fields.
fn group_amp(n: usize) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 * n + 2);
    payload.push(0x0b); // StartGroup, field 1
    for _ in 0..n {
        payload.extend_from_slice(&[0x08, 0x00]);
    }
    payload.push(0x0c); // EndGroup, field 1
    payload
}

/// More fields than the default limit, while the wire payload stays small
/// (~2 MiB).
const OVER_DEFAULT_LIMIT: usize = DEFAULT_UNKNOWN_FIELD_LIMIT + 1;

#[test]
fn group_amplification_rejected_by_default_limit() {
    let payload = group_amp(OVER_DEFAULT_LIMIT);
    assert_eq!(
        Empty::decode_from_slice(&payload),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}

#[test]
fn flat_varint_flood_rejected_by_default_limit() {
    // The report demonstrates the group vector, but a flat run of top-level
    // unknown varints amplifies identically — the limit must catch both.
    let payload = flat_varint_flood(OVER_DEFAULT_LIMIT);
    assert_eq!(
        Empty::decode_from_slice(&payload),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}

#[test]
fn small_unknown_payloads_still_decode() {
    // Forward-compatibility must keep working: a modest unknown payload
    // decodes and round-trips under the default limit.
    let payload = group_amp(100);
    let msg = Empty::decode_from_slice(&payload).expect("within limit");
    assert_eq!(msg.encode_to_vec(), payload);
}

#[test]
fn lowered_limit_rejects_what_default_accepts() {
    let payload = flat_varint_flood(100);
    Empty::decode_from_slice(&payload).expect("default limit accepts");
    assert_eq!(
        DecodeOptions::new()
            .with_unknown_field_limit(99)
            .decode_from_slice::<Empty>(&payload),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}

#[test]
fn raised_limit_accepts_what_default_rejects() {
    let payload = group_amp(OVER_DEFAULT_LIMIT);
    let msg = DecodeOptions::new()
        .with_unknown_field_limit(2 * DEFAULT_UNKNOWN_FIELD_LIMIT)
        .decode_from_slice::<Empty>(&payload)
        .expect("raised limit accepts");
    assert_eq!(msg.encode_to_vec(), payload);
}

#[test]
fn limit_is_exact() {
    // The group field itself plus its nested fields each consume one slot,
    // so a group of N fields needs exactly N + 1 slots.
    let payload = group_amp(100);
    DecodeOptions::new()
        .with_unknown_field_limit(101)
        .decode_from_slice::<Empty>(&payload)
        .expect("exactly enough slots");
    assert_eq!(
        DecodeOptions::new()
            .with_unknown_field_limit(100)
            .decode_from_slice::<Empty>(&payload),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}

#[test]
fn length_delimited_payload_not_counted_against_limit() {
    // One unknown LengthDelimited field with an 8 KiB payload consumes one
    // slot regardless of payload size — the payload bytes are bounded by
    // the input (and `with_max_message_size`), not by the field limit.
    let inner_len = 8 * 1024;
    let mut payload = vec![0x0a, 0x80, 0x40]; // tag (field 1, LD) + varint 8192
    payload.extend_from_slice(&vec![0u8; inner_len]);
    let msg = DecodeOptions::new()
        .with_unknown_field_limit(1)
        .decode_from_slice::<Empty>(&payload)
        .expect("one slot suffices for one field");
    assert_eq!(msg.encode_to_vec(), payload);
}

#[test]
fn limit_spans_nested_groups() {
    // Splitting the flood across two sibling groups must not reset the
    // limit: two groups of N/2 cost the same as one group of N (plus the
    // two group fields themselves).
    let n = OVER_DEFAULT_LIMIT / 2;
    let mut payload = Vec::with_capacity(2 * OVER_DEFAULT_LIMIT + 4);
    for _ in 0..2 {
        payload.push(0x0b);
        for _ in 0..n {
            payload.extend_from_slice(&[0x08, 0x00]);
        }
        payload.push(0x0c);
    }
    assert_eq!(
        Empty::decode_from_slice(&payload),
        Err(DecodeError::UnknownFieldLimitExceeded)
    );
}
