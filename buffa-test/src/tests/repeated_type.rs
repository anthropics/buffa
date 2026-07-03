//! repeated_type(): configurable owned collection for `repeated` fields.
//!
//! `repeated_type.proto` is compiled with
//! `repeated_type_custom("crate::repeated_type::CustomList<*>")`, so every
//! `repeated` field is a `CustomList<T>` (a crate-local `ProtoList<T>` impl)
//! instead of `Vec<T>`. Compiling `crate::repeated_type` is most of the test —
//! the merge (`push`/`reserve`), encode, clear, and view→owned paths must all
//! emit the generic `ProtoList` surface. The runtime checks below pin the field
//! types and verify binary and view→owned round-trips across the element kinds
//! whose codegen differs (packed varint, packed fixed-width, unpacked string,
//! repeated message).

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
