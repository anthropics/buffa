//! Segmented ("rope") encode: generated code + [`buffa::Rope`] end to end.
//!
//! The contract under test: encoding into a `Rope` produces byte-identical
//! wire output to a contiguous sink, while `bytes::Bytes`-typed fields above
//! the segment threshold are captured by reference count (same allocation,
//! no copy) — and view re-encoding through a backing-attached rope is
//! zero-copy for large borrowed fields.

use buffa::{Message, Rope, ViewEncode};
use bytes::Bytes;

use crate::basic_bytes::{BytesContexts, Person};

/// A payload comfortably above the segment threshold used in these tests.
fn big(byte: u8) -> Bytes {
    Bytes::from(vec![byte; 8 * 1024])
}

const MIN_SEGMENT: usize = 1024;

fn rope_bytes(rope: Rope) -> Vec<u8> {
    let mut out = Vec::new();
    for segment in rope.into_segments() {
        out.extend_from_slice(&segment);
    }
    out
}

#[test]
fn bytes_field_is_captured_by_refcount() {
    let msg = Person {
        id: 7,
        name: "holder".into(),
        avatar: big(0xAB),
        ..Default::default()
    };
    let payload_ptr = msg.avatar.as_ptr();

    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);

    let segments = rope.into_segments();
    assert!(
        segments
            .iter()
            .any(|s| core::ptr::eq(s.as_ptr(), payload_ptr)),
        "avatar payload must appear as a refcount-shared segment"
    );
}

#[test]
fn rope_output_is_byte_identical_to_contiguous() {
    let msg = BytesContexts {
        singular: big(0x01),
        maybe: Some(big(0x02)),
        many: vec![big(0x03), Bytes::from_static(b"small"), big(0x04)],
        choice: Some(crate::basic_bytes::bytes_contexts::Choice::Raw(big(0x05))),
        ..Default::default()
    };

    let contiguous = msg.encode_to_vec();
    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);

    assert_eq!(rope_bytes(rope), contiguous);
}

#[test]
fn repeated_and_oneof_bytes_share_segments() {
    let msg = BytesContexts {
        many: vec![big(0x11), big(0x22)],
        choice: Some(crate::basic_bytes::bytes_contexts::Choice::Raw(big(0x33))),
        ..Default::default()
    };
    let ptrs = [
        msg.many[0].as_ptr(),
        msg.many[1].as_ptr(),
        match &msg.choice {
            Some(crate::basic_bytes::bytes_contexts::Choice::Raw(b)) => b.as_ptr(),
            _ => unreachable!(),
        },
    ];

    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);
    let segments = rope.into_segments();

    for (i, ptr) in ptrs.iter().enumerate() {
        assert!(
            segments.iter().any(|s| core::ptr::eq(s.as_ptr(), *ptr)),
            "payload {i} must be refcount-shared"
        );
    }
}

#[test]
fn map_bytes_values_share_segments() {
    let mut msg = BytesContexts::default();
    msg.by_key.insert("k".to_string(), big(0x44));
    let ptr = msg.by_key["k"].as_ptr();

    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);
    let segments = rope.into_segments();

    assert!(
        segments.iter().any(|s| core::ptr::eq(s.as_ptr(), ptr)),
        "map bytes value must be refcount-shared"
    );
    let mut reassembled = Vec::new();
    for s in &segments {
        reassembled.extend_from_slice(s);
    }
    assert_eq!(reassembled, msg.encode_to_vec());
}

#[test]
fn small_fields_do_not_fragment() {
    let msg = Person {
        id: 1,
        name: "tiny".into(),
        avatar: Bytes::from_static(b"below threshold"),
        ..Default::default()
    };
    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);
    assert_eq!(rope.segment_count(), 1, "small message stays contiguous");
    assert_eq!(rope_bytes(rope), msg.encode_to_vec());
}

#[test]
fn view_reencode_through_backed_rope_is_zero_copy() {
    use crate::basic_bytes::__buffa::view::PersonView;

    let original = Person {
        id: 9,
        name: "viewed".into(),
        avatar: big(0x5C),
        ..Default::default()
    };
    let wire = Bytes::from(original.encode_to_vec());
    let wire_range = wire.as_ptr() as usize..wire.as_ptr() as usize + wire.len();

    let view = buffa::view::OwnedView::<PersonView>::decode(wire.clone()).expect("decode view");

    let mut rope = Rope::with_min_segment(MIN_SEGMENT).with_backing(wire.clone());
    view.reborrow().encode(&mut rope);
    let segments = rope.into_segments();

    // The avatar segment must point INTO the original wire buffer.
    assert!(
        segments
            .iter()
            .any(|s| s.len() >= MIN_SEGMENT && wire_range.contains(&(s.as_ptr() as usize))),
        "large view field must be sliced from the backing buffer, not copied"
    );

    // And the reassembled bytes decode back to the original message.
    let mut reassembled = Vec::new();
    for s in &segments {
        reassembled.extend_from_slice(s);
    }
    let decoded = Person::decode_from_slice(&reassembled).expect("reassembled decode");
    assert_eq!(decoded, original);
}

#[test]
fn vec_repr_still_copies_but_matches_wire() {
    // The default Vec<u8> repr has no shareable handle: same wire bytes,
    // payload copied into the tail (no aliasing with the message field).
    let msg = crate::basic::Person {
        id: 3,
        avatar: vec![0x77; 8 * 1024],
        ..Default::default()
    };
    let payload_ptr = msg.avatar.as_ptr();

    let mut rope = Rope::with_min_segment(MIN_SEGMENT);
    msg.encode(&mut rope);
    let segments = rope.into_segments();

    assert!(
        !segments
            .iter()
            .any(|s| core::ptr::eq(s.as_ptr(), payload_ptr)),
        "Vec<u8> payload cannot be shared"
    );
    let mut reassembled = Vec::new();
    for s in &segments {
        reassembled.extend_from_slice(s);
    }
    assert_eq!(reassembled, msg.encode_to_vec());
}
