//! Messages/fields named after Rust types and generated-method names.

use super::round_trip;

#[test]
fn test_rust_type_named_messages_round_trip() {
    // Messages named Vec, String, Option, Result, Box, Default
    // must not shadow Rust prelude types in generated code.
    use crate::collisions;

    let vec_msg = collisions::Vec {
        items: vec![1, 2, 3],
        ..core::default::Default::default()
    };
    let decoded = round_trip(&vec_msg);
    assert_eq!(decoded.items, vec![1, 2, 3]);

    let string_msg = collisions::String {
        value: "hello".into(),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&string_msg);
    assert_eq!(decoded.value, "hello");

    let option_msg = collisions::Option {
        present: true,
        ..core::default::Default::default()
    };
    let decoded = round_trip(&option_msg);
    assert!(decoded.present);

    let result_msg = collisions::Result {
        ok: true,
        error: "none".into(),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&result_msg);
    assert!(decoded.ok);
    assert_eq!(decoded.error, "none");

    let box_msg = collisions::Box {
        content: vec![0xFF],
        ..core::default::Default::default()
    };
    let decoded = round_trip(&box_msg);
    assert_eq!(decoded.content, vec![0xFF]);

    let default_msg = collisions::Default {
        value: 42,
        ..core::default::Default::default()
    };
    let decoded = round_trip(&default_msg);
    assert_eq!(decoded.value, 42);
}

#[test]
fn test_method_named_fields_round_trip() {
    use crate::collisions::MethodNames;

    let msg = MethodNames {
        compute_size: 100,
        write_to: "file.txt".into(),
        encode: vec![1, 2],
        decode: true,
        merge: "strategy".into(),
        clear: 0,
        cached_size: 999,
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.compute_size, 100);
    assert_eq!(decoded.write_to, "file.txt");
    assert_eq!(decoded.encode, vec![1, 2]);
    assert!(decoded.decode);
    assert_eq!(decoded.merge, "strategy");
    assert_eq!(decoded.cached_size, 999);
}

#[test]
fn test_oneof_name_matching_parent_message() {
    use crate::collisions;

    let msg = collisions::Status {
        status: Some(collisions::__buffa::oneof::status::Status::Code(42)),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.status,
        Some(collisions::__buffa::oneof::status::Status::Code(42))
    );

    let msg2 = collisions::Status {
        status: Some(collisions::__buffa::oneof::status::Status::Message(
            "error".into(),
        )),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg2);
    assert_eq!(
        decoded.status,
        Some(collisions::__buffa::oneof::status::Status::Message(
            "error".into()
        ))
    );
}

#[test]
fn test_container_references_collision_types() {
    use crate::collisions;

    let msg = collisions::Container {
        vec_field: buffa::MessageField::some(collisions::Vec {
            items: vec![1],
            ..core::default::Default::default()
        }),
        string_field: buffa::MessageField::some(collisions::String {
            value: "v".into(),
            ..core::default::Default::default()
        }),
        status: buffa::MessageField::some(collisions::Status {
            status: Some(collisions::__buffa::oneof::status::Status::Code(1)),
            ..core::default::Default::default()
        }),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.vec_field.items, vec![1]);
    assert_eq!(decoded.string_field.value, "v");
}

#[test]
fn test_nested_option_message_round_trip() {
    // gh#36: nested `message Option` shadows core::option::Option in the
    // message's `pub mod { use super::*; }` scope. The proto is built with
    // views + JSON enabled so all Option<...> emission paths compile.
    use crate::prelude_shadow::{self, picker};

    let msg = prelude_shadow::Picker {
        options: vec![picker::Option {
            title: Some("a".into()),
            value: Some(prelude_shadow::__buffa::oneof::picker::option::Value::IntValue(7)),
            ..core::default::Default::default()
        }],
        label: Some("L".into()),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.options[0].title.as_deref(), Some("a"));
    assert_eq!(decoded.label.as_deref(), Some("L"));

    // JSON round-trip exercises skip_serializing_if + custom-deser temporaries.
    let json = serde_json::to_string(&msg).expect("serialize");
    let back: prelude_shadow::Picker = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn test_nested_option_sibling_propagation_compiles() {
    // `Outer.Option` shadows the prelude in `mod outer` AND in
    // `mod outer::middle` (via `use super::*`). Verifies the child
    // resolver propagates the blocked set through >1 hop.
    use crate::prelude_shadow::outer::{self, middle};

    let msg = outer::Middle {
        inner: buffa::MessageField::some(middle::Inner {
            x: Some(1),
            ..core::default::Default::default()
        }),
        note: Some("n".into()),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.inner.x, Some(1));
    assert_eq!(decoded.note.as_deref(), Some("n"));
}

#[test]
fn test_escaped_type_names_round_trip_binary_and_json() {
    // Fixture: `type_name_escapes.proto`.
    use crate::type_name_escapes::{self as esc, __buffa::oneof};
    use buffa::{EnumValue, MessageField};

    let msg = esc::Holder {
        flag: MessageField::some(esc::bool_ {
            label: Some("on".into()),
            inner: MessageField::some(esc::bool::Inner {
                n: 7,
                ..Default::default()
            }),
            pick: Some(oneof::bool::Pick::Number(3)),
            ..Default::default()
        }),
        strs: vec![esc::str_ {
            data: vec![1, 2],
            ..Default::default()
        }],
        bytes_by_name: [(
            "k".to_string(),
            esc::u8_ {
                v: 255,
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        big: MessageField::some(esc::u64_ {
            v: u64::MAX,
            inner: MessageField::some(esc::u64::Inner {
                n: 1,
                ..Default::default()
            }),
            ..Default::default()
        }),
        size: MessageField::some(esc::usize_ {
            v: 9,
            ..Default::default()
        }),
        small: MessageField::some(esc::i32_ {
            pick: Some(oneof::i32::Pick::Number(-1)),
            ..Default::default()
        }),
        unsigned: EnumValue::Known(esc::u32_::U32_ONE),
        floats: vec![EnumValue::Known(esc::f64_::F64_ONE)],
        maybe_unsigned: Some(EnumValue::Known(esc::u32_::U32_ONE)),
        me: MessageField::some(esc::Self_ {
            v: "me".into(),
            ..Default::default()
        }),
        kind: MessageField::some(esc::type_ {
            v: "kind".into(),
            inner: MessageField::some(esc::r#type::Inner {
                n: 2,
                ..Default::default()
            }),
            pick: Some(oneof::r#type::Pick::Number(5)),
            ..Default::default()
        }),
        kind_inner: MessageField::some(esc::r#type::Inner {
            n: 3,
            ..Default::default()
        }),
        arm: EnumValue::Known(esc::match_::MATCH_ONE),
        // `char` and `i8` are not escaped: generated code does not name them.
        letter: MessageField::some(esc::char {
            v: "c".into(),
            ..Default::default()
        }),
        tiny: EnumValue::Known(esc::i8::I8_ONE),
        pick: Some(oneof::holder::Pick::PickFloat(EnumValue::Known(
            esc::f64_::F64_ONE,
        ))),
        scalar_bool: true,
        scalar_i32: -1,
        scalar_u64: u64::MAX,
        scalar_f64: 1.5,
        scalar_string: "s".into(),
        bools: vec![true, false],
        doubles: [(1, 2.5)].into_iter().collect(),
        maybe_u64: Some(1),
        ..Default::default()
    };

    assert_eq!(round_trip(&msg), msg);

    let json = serde_json::to_string(&msg).expect("serialize");
    let back: esc::Holder = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn test_escaped_type_names_keep_their_proto_names() {
    use crate::type_name_escapes as esc;
    use buffa::MessageName;

    assert_eq!(esc::bool_::FULL_NAME, "test.type_name_escapes.bool");
    assert_eq!(esc::Self_::FULL_NAME, "test.type_name_escapes.Self");
    assert_eq!(esc::type_::FULL_NAME, "test.type_name_escapes.type");
    assert_eq!(
        esc::bool::Inner::FULL_NAME,
        "test.type_name_escapes.bool.Inner"
    );
    assert_eq!(
        esc::r#type::Inner::FULL_NAME,
        "test.type_name_escapes.type.Inner"
    );
    assert_eq!(
        esc::bool_::TYPE_URL,
        "type.googleapis.com/test.type_name_escapes.bool"
    );
}
