//! box_type(): configurable owned pointer for singular message fields.
//!
//! `box_type.proto` is compiled with
//! `box_type_custom("crate::box_type::CustomBox<*>")`, so every singular message
//! field is a `MessageField<T, CustomBox<T>>` instead of `MessageField<T>`
//! (`= MessageField<T, Box<T>>`). Compiling `crate::box_type` is most of the
//! test — the field type, decode (`get_or_insert_default`), clear, and
//! view→owned (`some`) paths must all emit the custom pointer. The runtime
//! checks below pin the field type and verify binary + view→owned round-trips,
//! including a self-referential field.

use crate::box_type::{CustomBox, Inner, Outer};
use buffa::{Message, MessageField};

fn inner(id: i32, name: &str) -> Inner {
    Inner {
        id,
        name: name.into(),
        ..Default::default()
    }
}

fn sample() -> Outer {
    Outer {
        // The field type pins the pointer, so bare `some` infers `CustomBox`.
        inner: MessageField::some(inner(7, "alpha")),
        maybe: MessageField::some(inner(9, "beta")),
        count: 42,
        ..Default::default()
    }
}

#[test]
fn field_type_is_custom_box() {
    // Fails to compile if codegen emitted the wrong pointer for the singular
    // message fields (scalar `count` is unaffected).
    let m = Outer::default();
    let _: &MessageField<Inner, CustomBox<Inner>> = &m.inner;
    let _: &MessageField<Inner, CustomBox<Inner>> = &m.maybe;
    let _: i32 = m.count;
    // self_ref is `MessageField<Outer, CustomBox<Outer>>`.
    let _: &MessageField<Outer, CustomBox<Outer>> = &m.self_ref;
}

#[test]
fn binary_round_trip() {
    let msg = sample();
    let bytes = msg.encode_to_vec();
    let decoded = Outer::decode(&mut bytes.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    // Read through the custom pointer (MessageField -> CustomBox -> Inner).
    assert_eq!(decoded.inner.id, 7);
    assert_eq!(decoded.inner.name, "alpha");
    assert_eq!(decoded.maybe.id, 9);
    assert_eq!(decoded.count, 42);
    assert!(decoded.self_ref.is_unset());
}

#[test]
fn unset_message_field_decodes_default() {
    // Exercises the decode `get_or_insert_default` path on the custom pointer:
    // a nested message present on the wire but with no fields set.
    let msg = Outer {
        inner: MessageField::some(Inner::default()),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let decoded = Outer::decode(&mut bytes.as_slice()).expect("decode");
    assert!(decoded.inner.is_set());
    assert_eq!(decoded.inner.id, 0);
}

#[test]
fn self_referential_nesting_round_trips() {
    let mut msg = Outer::default();
    msg.self_ref = MessageField::some(sample());
    let bytes = msg.encode_to_vec();
    let decoded = Outer::decode(&mut bytes.as_slice()).expect("decode");
    assert!(decoded.self_ref.is_set());
    assert_eq!(decoded.self_ref.inner.id, 7);
}

#[test]
fn view_to_owned_round_trip() {
    // Exercises the view→owned `some` path emitting the custom pointer.
    let bytes = bytes::Bytes::from(sample().encode_to_vec());
    let owned: Outer = crate::box_type::OuterOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message()
        .expect("to_owned");
    assert_eq!(owned, sample());
}

#[test]
fn oneof_message_variant_uses_custom_box() {
    use crate::box_type::__buffa::oneof::with_oneof::Kind;
    use crate::box_type::WithOneof;
    use buffa::alloc::boxed::Box;

    // Type pin: a boxed message variant holds the custom pointer, not `Box`.
    let k = Kind::Msg(CustomBox(Box::new(inner(3, "x"))));
    let _: &CustomBox<Inner> = match &k {
        Kind::Msg(p) => p,
        _ => unreachable!(),
    };

    // Binary round-trip of a message variant (decode constructs via the custom
    // pointer's `ProtoBox::new`).
    let msg = WithOneof {
        kind: Some(Kind::Msg(CustomBox(Box::new(inner(7, "a"))))),
        ..Default::default()
    };
    let decoded = WithOneof::decode(&mut msg.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded, msg);

    // A *recursive* variant: must stay pointered, and the custom pointer (being
    // sized) works where inlining can't.
    let rec = WithOneof {
        kind: Some(Kind::Nested(CustomBox(Box::new(WithOneof {
            kind: Some(Kind::Scalar(42)),
            ..Default::default()
        })))),
        ..Default::default()
    };
    let rd = WithOneof::decode(&mut rec.encode_to_vec().as_slice()).expect("decode recursive");
    assert_eq!(rd, rec);
    match rd.kind {
        Some(Kind::Nested(inner)) => assert_eq!(inner.kind, Some(Kind::Scalar(42))),
        other => panic!("expected nested variant, got {other:?}"),
    }
}

#[test]
fn oneof_view_to_owned_round_trip() {
    // Exercises the oneof view→owned path emitting `ProtoBox::new` for the
    // custom pointer.
    use crate::box_type::__buffa::oneof::with_oneof::Kind;
    use crate::box_type::WithOneof;
    use buffa::alloc::boxed::Box;

    let msg = WithOneof {
        kind: Some(Kind::Msg(CustomBox(Box::new(inner(5, "v"))))),
        ..Default::default()
    };
    let bytes = bytes::Bytes::from(msg.encode_to_vec());
    let owned: WithOneof = crate::box_type::WithOneofOwnedView::decode(bytes)
        .expect("decode view")
        .to_owned_message()
        .expect("to_owned");
    assert_eq!(owned, msg);
}
