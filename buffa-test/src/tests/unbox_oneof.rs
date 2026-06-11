//! unbox_oneof(): message-typed oneof variants stored inline instead of
//! behind a `Box`.
//!
//! `Envelope.body.small` is opted out (stored inline as `Small`); `large`
//! stays boxed (`Box<Large>`). These tests round-trip the inline variant
//! through binary, JSON, and text, exercise the `From` impl and merge
//! semantics, and confirm the boxed sibling still works.

use crate::unbox_oneof::__buffa::oneof::envelope::Body;
use crate::unbox_oneof::{Envelope, Large, Small};
use buffa::text::{decode_from_str, encode_to_string};
use buffa::{Message, MessageView};

fn small(value: i32) -> Small {
    Small {
        value,
        ..Default::default()
    }
}

fn envelope_small(value: i32) -> Envelope {
    Envelope {
        body: Some(Body::Small(small(value))),
        ..Default::default()
    }
}

#[test]
fn inline_variant_binary_roundtrip() {
    // The opted-out variant holds `Small` directly (no `Box::new`).
    let decoded = super::round_trip(&envelope_small(7));
    match decoded.body {
        Some(Body::Small(s)) => assert_eq!(s.value, 7),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn boxed_sibling_variant_roundtrip() {
    // A variant left alone is still boxed and round-trips unchanged.
    let msg = Envelope {
        body: Some(Body::Large(std::boxed::Box::new(Large {
            label: "hello".to_string(),
            ..Default::default()
        }))),
        ..Default::default()
    };
    let decoded = super::round_trip(&msg);
    match decoded.body {
        Some(Body::Large(large)) => assert_eq!(large.label, "hello"),
        other => panic!("expected Body::Large, got {other:?}"),
    }
}

#[test]
fn from_impl_stores_inline_value() {
    // `From<Small>` moves the message in without wrapping it in a `Box`.
    let body: Body = small(3).into();
    match body {
        Body::Small(s) => assert_eq!(s.value, 3),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn inline_variant_merges_existing() {
    // Two wire messages carrying the same message variant merge into one
    // (proto3 oneof merge semantics), even when stored inline.
    let mut concatenated = envelope_small(1).encode_to_vec();
    concatenated.extend_from_slice(&envelope_small(9).encode_to_vec());

    let decoded = Envelope::decode(&mut concatenated.as_slice()).expect("decode");
    match decoded.body {
        // Scalar fields take the last value on the wire after merge.
        Some(Body::Small(s)) => assert_eq!(s.value, 9),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn inline_variant_json_roundtrip() {
    let json = serde_json::to_string(&envelope_small(11)).expect("serialize");
    let decoded: Envelope = serde_json::from_str(&json).expect("deserialize");
    match decoded.body {
        Some(Body::Small(s)) => assert_eq!(s.value, 11),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn inline_variant_text_roundtrip() {
    let text = encode_to_string(&envelope_small(5));
    let decoded: Envelope = decode_from_str(&text).expect("decode_from_str");
    match decoded.body {
        Some(Body::Small(s)) => assert_eq!(s.value, 5),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn inline_variant_view_to_owned_roundtrip() {
    // The view-to-owned conversion stores the inline variant without a Box
    // (oneof_variant_to_owned in view.rs branches on variant_boxed).
    let bytes = envelope_small(13).encode_to_vec();
    let view = crate::unbox_oneof::EnvelopeView::decode_view(&bytes).expect("decode_view");
    let owned = view.to_owned_message().unwrap();
    match owned.body {
        Some(Body::Small(s)) => assert_eq!(s.value, 13),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}

#[test]
fn inline_variant_text_merge_into_existing() {
    // Text-format merge takes the merge-into-existing arm when the same
    // oneof variant is already set, mirroring binary merge semantics.
    let mut msg = envelope_small(1);
    buffa::text::merge_from_str(&mut msg, "small { value: 9 }").expect("merge_from_str");
    match msg.body {
        Some(Body::Small(s)) => assert_eq!(s.value, 9),
        other => panic!("expected Body::Small, got {other:?}"),
    }
}
