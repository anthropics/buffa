//! `PointerRepr::Inline` default: built-in inline storage for singular message
//! fields.
//!
//! `inline_field.proto` is compiled with **no pointer config**, so every
//! non-recursive singular message field is `MessageField<T, ::buffa::Inline<T>>`
//! (laid out as `Option<T>` in the parent struct — no heap). The recursive
//! `self_ref` field stays on `Box`. Compiling `crate::inline_field` is most of
//! the test: an inlined `self_ref` would E0072, so the recursion guard is proven
//! on the default config by `cargo build`. The runtime checks below pin the
//! field types, assert the inline layout, and verify binary + view→owned
//! round-trips.

use crate::inline_field::{Inner, Outer};
use buffa::{Inline, Message, MessageField};

fn inner(id: i32, name: &str) -> Inner {
    Inner {
        id,
        name: name.into(),
        ..Default::default()
    }
}

fn sample() -> Outer {
    Outer {
        inner: MessageField::some(inner(7, "alpha")),
        maybe: MessageField::some(inner(9, "beta")),
        count: 42,
        ..Default::default()
    }
}

#[test]
fn field_types_are_inline_except_recursive() {
    // Fails to compile if codegen emitted the wrong pointer.
    let m = Outer::default();
    let _: &MessageField<Inner, Inline<Inner>> = &m.inner;
    let _: &MessageField<Inner, Inline<Inner>> = &m.maybe;
    let _: i32 = m.count;
    // self_ref is recursive: must stay on Box (default pointer param).
    let _: &MessageField<Outer> = &m.self_ref;
}

#[test]
fn inline_field_layout_is_option_t() {
    // The whole point of #248: no per-field heap allocation. `Inline<T>` is
    // `repr(transparent)`, so `MessageField<T, Inline<T>>` is `Option<T>`.
    use core::mem::size_of;
    assert_eq!(
        size_of::<MessageField<Inner, Inline<Inner>>>(),
        size_of::<Option<Inner>>()
    );
    // And the parent struct holds at least two `Inner` worth (the boxed default
    // would be two pointers worth instead).
    assert!(size_of::<Outer>() >= 2 * size_of::<Inner>());
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = Outer::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    assert_eq!(decoded.inner.id, 7);
    assert_eq!(decoded.inner.name, "alpha");
    assert!(decoded.self_ref.is_unset());
}

#[test]
fn self_referential_nesting_round_trips() {
    // The recursive field is heap-backed, so a chain works.
    let mut msg = Outer::default();
    msg.self_ref = MessageField::some(sample());
    let bytes = msg.encode_to_vec();
    let decoded = Outer::decode(&mut bytes.as_slice()).expect("decode");
    assert!(decoded.self_ref.is_set());
    assert_eq!(decoded.self_ref.inner.id, 7);
}

#[test]
fn view_to_owned_round_trip() {
    // Exercises the view→owned `some_path` emitting the inline pointer.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: Outer = crate::inline_field::OuterOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message()
        .expect("to_owned");
    assert_eq!(owned, sample());
}
