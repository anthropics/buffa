//! repeated_type(): configurable owned collection for `repeated` fields.
//!
//! `repeated_type.proto` is compiled with
//! `repeated_type_custom("crate::repeated_type::CustomList<*>")`, so every
//! `repeated` field is a `CustomList<T>` (a crate-local `ProtoList<T>` impl)
//! instead of `Vec<T>`. Compiling `crate::repeated_type` is most of the test —
//! the merge (`push`/`reserve`), encode, clear, view→owned and text paths must
//! all emit the generic `ProtoList` surface. The runtime checks below pin the
//! field types and verify binary, view→owned and text round-trips across the
//! element kinds whose codegen differs (packed varint, packed fixed-width,
//! unpacked string, repeated message).

use crate::repeated_type::{CustomList, Inner, Lists};
use buffa::Message;

fn inner(id: i32) -> Inner {
    Inner {
        id,
        ..Default::default()
    }
}

fn sample() -> Lists {
    // `vec![..].into()` exercises the `From<Vec<T>>` ProtoList supertrait — the
    // ergonomic constructor a connectrpc handler would use when building a
    // response by hand.
    Lists {
        numbers: ::buffa::alloc::vec![1, -2, 300, 0, 42].into(),
        names: ::buffa::alloc::vec!["alpha".into(), "beta".into()].into(),
        items: ::buffa::alloc::vec![inner(7), inner(9)].into(),
        fixed: ::buffa::alloc::vec![10, -20, 30].into(),
        labels: ::buffa::alloc::vec!["gamma".into()].into(),
        blobs: ::buffa::alloc::vec![bytes::Bytes::from_static(b"\x00raw")].into(),
        ..Default::default()
    }
}

#[test]
fn field_types_are_custom_list() {
    // Fails to compile if codegen emitted the wrong collection type for any
    // element kind (scalar, string, or message).
    let m = Lists::default();
    let _: &CustomList<i32> = &m.numbers;
    let _: &CustomList<::buffa::alloc::string::String> = &m.names;
    let _: &CustomList<Inner> = &m.items;
    let _: &CustomList<i32> = &m.fixed;
    let _: &CustomList<crate::reprs::EcoStr> = &m.labels;
    let _: &CustomList<bytes::Bytes> = &m.blobs;
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = Lists::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    // Spot-check the merge actually populated the custom collection.
    assert_eq!(&*decoded.numbers, &[1, -2, 300, 0, 42]);
    assert_eq!(decoded.names.len(), 2);
    assert_eq!(decoded.items[1].id, 9);
    assert_eq!(&*decoded.fixed, &[10, -20, 30]);
}

#[test]
fn empty_round_trips_clean() {
    let msg = Lists::default();
    let bytes = msg.encode_to_vec();
    assert!(bytes.is_empty(), "empty repeated fields encode to nothing");
    let decoded = Lists::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
}

#[test]
fn view_to_owned_round_trip() {
    // Exercises the view→owned path: scalar elements collect via FromIterator
    // (not `Vec::to_vec`), strings/messages collect into the custom collection.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: Lists = crate::repeated_type::ListsOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message();
    assert_eq!(owned, sample());
}

/// `EcoStr` has no `Arbitrary` impl, so `Lists: Arbitrary` compiling shows that
/// `labels` got a builder that makes each element from a `String` and returns
/// the custom collection.
#[cfg(feature = "arbitrary")]
#[test]
fn arbitrary_builds_custom_list_of_custom_strings() {
    use arbitrary::{Arbitrary, Unstructured};

    let raw: [u8; 256] = core::array::from_fn(|i| (i as u8) | 1);
    let msg = Lists::arbitrary(&mut Unstructured::new(&raw)).unwrap();
    let _: &CustomList<crate::reprs::EcoStr> = &msg.labels;
}

#[test]
fn text_round_trip() {
    // The text encoder iterates the `Deref` slice and the decoder appends
    // through `ProtoList::push`.
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    assert_eq!(
        text,
        r#"numbers: 1 numbers: -2 numbers: 300 numbers: 0 numbers: 42 names: "alpha" names: "beta" items {id: 7} items {id: 9} fixed: 10 fixed: -20 fixed: 30 labels: "gamma" blobs: "\000raw""#
    );
    let decoded: Lists = buffa::text::decode_from_str(&text).expect("decode text");
    assert_eq!(decoded, msg);
}

#[test]
fn text_merge_appends_in_order() {
    // Both repeated forms append after the elements already in the collection.
    let mut msg = sample();
    buffa::text::merge_from_str(
        &mut msg,
        r#"numbers: [5, 6] numbers: 7 items: [{id: 1}, {id: 2}] names: [] labels: ["delta"]"#,
    )
    .expect("merge text");
    assert_eq!(&*msg.numbers, &[1, -2, 300, 0, 42, 5, 6, 7]);
    assert_eq!(
        msg.items.iter().map(|i| i.id).collect::<Vec<_>>(),
        [7, 9, 1, 2]
    );
    assert_eq!(msg.names.len(), 2);
    assert_eq!(
        msg.labels.iter().map(|l| l.as_str()).collect::<Vec<_>>(),
        ["gamma", "delta"]
    );
}
