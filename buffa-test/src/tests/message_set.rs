//! Integration tests for `option message_set_wire_format = true`.
//!
//! MessageSet encodes each extension as a group-at-field-1 Item:
//!   `0x0B | 0x10 | varint(type_id) | 0x1A | varint(len) | payload | 0x0C`
//! instead of the regular `tag(type_id, LD) | len | payload`.
//!
//! Storage is flat: `__buffa_unknown_fields` holds
//! `{number: type_id, data: LengthDelimited(payload)}`. Decode unwraps the
//! group; encode rewraps it.

use crate::msgset::__buffa::ext::{MARKER_EXT, PAYLOAD_EXT};
use crate::msgset::{Container, Marker, Payload};
use buffa::{ExtensionSet, Message};

#[test]
fn roundtrip_single_extension() {
    let payload = Payload {
        name: Some("hello".to_string()),
        value: Some(42),
        ..Default::default()
    };

    let mut container = Container::default();
    container.set_extension(&PAYLOAD_EXT, payload.clone());

    let bytes = container.encode_to_vec();
    let decoded = Container::decode_from_slice(&bytes).expect("decode");

    let got = decoded.extension(&PAYLOAD_EXT).expect("payload present");
    assert_eq!(got.name, Some("hello".to_string()));
    assert_eq!(got.value, Some(42));
}

#[test]
fn roundtrip_two_extensions() {
    let payload = Payload {
        name: Some("x".to_string()),
        ..Default::default()
    };
    let marker = Marker {
        flag: Some(true),
        ..Default::default()
    };

    let mut container = Container::default();
    container.set_extension(&PAYLOAD_EXT, payload);
    container.set_extension(&MARKER_EXT, marker);

    let decoded = super::round_trip(&container);
    assert_eq!(
        decoded.extension(&PAYLOAD_EXT).unwrap().name,
        Some("x".to_string())
    );
    assert_eq!(decoded.extension(&MARKER_EXT).unwrap().flag, Some(true));
}

/// Wire-level check: the encoded bytes use the MessageSet Item group framing,
/// not regular extension tags.
#[test]
fn encoded_bytes_use_item_group_framing() {
    let mut container = Container::default();
    container.set_extension(
        &PAYLOAD_EXT,
        Payload {
            name: Some("a".to_string()),
            ..Default::default()
        },
    );

    let bytes = container.encode_to_vec();

    // 0x0B = tag(1, SGROUP), 0x10 = tag(2, Varint), 0xE8 0x07 = varint(1000),
    // 0x1A = tag(3, LD). The payload encodes `name = "a"` (field 1, LD, len 1)
    // → `0x0A 0x01 0x61`, so the Item's message field is len=3. Final byte is
    // 0x0C = tag(1, EGROUP).
    assert_eq!(
        bytes,
        &[
            0x0B, // ITEM_START_TAG
            0x10, 0xE8, 0x07, // TYPE_ID_TAG, varint(1000)
            0x1A, 0x03, 0x0A, 0x01, 0x61, // MESSAGE_TAG, len=3, (tag=0x0A, len=1, "a")
            0x0C, // ITEM_END_TAG
        ],
    );

    // Sanity: the *wrong* format (regular extension tag at field 1000) would
    // start with tag((1000 << 3) | 2) = 8002 = varint 0xD2 0x3E. Assert that's
    // not what we produced.
    assert_ne!(&bytes[0..2], &[0xD2, 0x3E]);
}

/// Cross-validation: hand-encode a MessageSet Item (bytes literal), decode
/// with buffa, verify extension value. Confirms our decoder accepts what
/// other implementations produce.
#[test]
fn decodes_hand_built_item() {
    // Item wrapping a Payload { value: 7 } at type_id 1000.
    // Payload.value is field 2, varint → `0x10 0x07`, so the Item's message
    // field has len=2.
    let wire = &[
        0x0B, // ITEM_START_TAG
        0x10, 0xE8, 0x07, // TYPE_ID_TAG, varint(1000)
        0x1A, 0x02, 0x10, 0x07, // MESSAGE_TAG, len=2, (tag=0x10, varint 7)
        0x0C, // ITEM_END_TAG
    ];

    let decoded = Container::decode_from_slice(wire).expect("decode");
    let payload = decoded.extension(&PAYLOAD_EXT).expect("payload present");
    assert_eq!(payload.value, Some(7));
    assert_eq!(payload.name, None);
}

/// The `type_id` and `message` fields can arrive in either order inside the
/// Item group — protobuf-go handles both, so must we.
#[test]
fn decodes_message_before_type_id() {
    // Same as above but with message field before type_id.
    let wire = &[
        0x0B, // ITEM_START_TAG
        0x1A, 0x02, 0x10, 0x07, // MESSAGE_TAG, len=2, (tag=0x10, varint 7)
        0x10, 0xE8, 0x07, // TYPE_ID_TAG, varint(1000)
        0x0C, // ITEM_END_TAG
    ];

    let decoded = Container::decode_from_slice(wire).expect("decode");
    assert_eq!(decoded.extension(&PAYLOAD_EXT).unwrap().value, Some(7));
}

/// Encoded size must match the actual byte count — `compute_size` feeds
/// length prefixes when a MessageSet is nested inside another message.
#[test]
fn compute_size_matches_encoded_length() {
    let mut container = Container::default();
    container.set_extension(
        &PAYLOAD_EXT,
        Payload {
            name: Some("hello world".to_string()),
            value: Some(-1),
            ..Default::default()
        },
    );
    container.set_extension(
        &MARKER_EXT,
        Marker {
            flag: Some(true),
            ..Default::default()
        },
    );

    let size = container.encoded_len();
    let bytes = container.encode_to_vec();
    assert_eq!(size as usize, bytes.len());
}

/// Non-Item data on a MessageSet wire (stray tag outside the group) is
/// preserved as a regular unknown field and re-emitted as-is.
#[test]
fn stray_varint_preserved_through_roundtrip() {
    // A valid Item followed by a stray varint at field 5.
    let wire = &[
        0x0B, 0x10, 0xE8, 0x07, 0x1A, 0x00, 0x0C, // Item: type_id=1000, message=empty
        0x28, 0x2A, // tag(5, Varint), varint(42)
    ];

    let decoded = Container::decode_from_slice(wire).expect("decode");
    // Extension is present (empty message).
    assert!(decoded.extension(&PAYLOAD_EXT).is_some());

    // Re-encode: the Item comes back in group form, the stray varint is
    // re-emitted as-is. Total length preserved.
    let reencoded = decoded.encode_to_vec();
    assert_eq!(reencoded.len(), wire.len());
    assert_eq!(decoded.encoded_len() as usize, reencoded.len());

    // Decode again to verify the stray varint survived.
    let redecoded = Container::decode_from_slice(&reencoded).expect("redecode");
    let stray: Vec<_> = redecoded
        .unknown_fields()
        .iter()
        .filter(|f| f.number == 5)
        .collect();
    assert_eq!(stray.len(), 1);
    assert_eq!(stray[0].data, buffa::UnknownFieldData::Varint(42));
}

/// Empty container encodes to zero bytes (no unknown fields → no Items).
#[test]
fn empty_container_encodes_empty() {
    let container = Container::default();
    assert_eq!(container.encode_to_vec(), Vec::<u8>::new());
    assert_eq!(container.encoded_len(), 0);
}

/// Clearing after setting an extension yields an empty encode.
#[test]
fn clear_resets_to_empty() {
    let mut container = Container::default();
    container.set_extension(&PAYLOAD_EXT, Payload::default());
    assert!(!container.encode_to_vec().is_empty());

    container.clear();
    assert_eq!(container.encode_to_vec(), Vec::<u8>::new());
}
