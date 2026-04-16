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
        status: Some(collisions::status::StatusOneof::Code(42)),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.status,
        Some(collisions::status::StatusOneof::Code(42))
    );

    let msg2 = collisions::Status {
        status: Some(collisions::status::StatusOneof::Message("error".into())),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg2);
    assert_eq!(
        decoded.status,
        Some(collisions::status::StatusOneof::Message("error".into()))
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
            status: Some(collisions::status::StatusOneof::Code(1)),
            ..core::default::Default::default()
        }),
        ..core::default::Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.vec_field.items, vec![1]);
    assert_eq!(decoded.string_field.value, "v");
}
