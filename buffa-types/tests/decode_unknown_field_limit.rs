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

// ── Zero-copy view path ────────────────────────────────────────────────────
//
// Views store unknown fields as borrowed spans rather than decoded values,
// and adjacent unknown records coalesce into a single span — so a contiguous
// flood costs one slot regardless of field count, while interleaved runs
// are counted per span against the same unknown-field limit.

mod view_path {
    use super::*;
    use buffa::view::MessageView;
    use buffa_types::google::protobuf::__buffa::view::{DurationView, EmptyView};

    #[test]
    fn contiguous_unknown_flood_coalesces_to_one_span() {
        // 100k unknown varint fields, fully contiguous: one span, one slot.
        let payload = flat_varint_flood(100_000);
        let view: EmptyView = DecodeOptions::new()
            .with_unknown_field_limit(1)
            .decode_view(&payload)
            .expect("a contiguous flood coalesces into a single span");
        // Converting to owned materializes one UnknownField per record, so
        // the decode-time allowance (1) carries through and is enforced.
        assert_eq!(
            view.to_owned_message(),
            Err(DecodeError::UnknownFieldLimitExceeded)
        );
        // With an adequate allowance the round-trip is byte-identical.
        let view: EmptyView = EmptyView::decode_view(&payload).expect("default limit");
        let owned = view.to_owned_message().expect("within default allowance");
        assert_eq!(owned.encode_to_vec(), payload);
    }

    #[test]
    fn interleaved_unknown_runs_counted_against_limit() {
        // Alternate a known Duration field (seconds = field 1) with an
        // unknown field (field 99): every unknown run needs its own span.
        let mut payload = Vec::new();
        for _ in 0..10 {
            payload.extend_from_slice(&[0x08, 0x01]); // seconds = 1 (known)
            payload.extend_from_slice(&[0x98, 0x06, 0x00]); // field 99 varint (unknown)
        }
        let view = DecodeOptions::new()
            .with_unknown_field_limit(10)
            .decode_view::<DurationView>(&payload)
            .expect("10 spans fit a limit of 10");
        view.to_owned_message()
            .expect("10 fields fit the same allowance");
        assert!(matches!(
            DecodeOptions::new()
                .with_unknown_field_limit(9)
                .decode_view::<DurationView>(&payload),
            Err(DecodeError::UnknownFieldLimitExceeded)
        ));
    }

    #[test]
    fn coalesced_spans_convert_to_owned() {
        // to_owned parses every record inside a coalesced span.
        let payload = flat_varint_flood(50);
        let view = EmptyView::decode_view(&payload).expect("decodes");
        let owned = view.to_owned_message().expect("within allowance");
        assert_eq!(owned.encode_to_vec(), payload);
    }

    #[test]
    fn group_flood_through_views_is_bounded() {
        // The group payload through the view path: the group is skipped
        // (not recursed into) and captured as one contiguous span. The
        // decode-time allowance carries into conversion, where each nested
        // record materializes as an owned field.
        let payload = group_amp(100_000);
        let view: EmptyView = DecodeOptions::new()
            .with_unknown_field_limit(1)
            .decode_view(&payload)
            .expect("one span for the whole group record");
        assert_eq!(
            view.to_owned_message(),
            Err(DecodeError::UnknownFieldLimitExceeded)
        );
        let view: EmptyView = EmptyView::decode_view(&payload).expect("default limit");
        assert_eq!(
            view.to_owned_message()
                .expect("default allowance")
                .encode_to_vec(),
            payload
        );
    }
}
