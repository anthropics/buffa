//! Reflective enum defaults against a protoc-compiled schema.
//!
//! Regenerate the fixture from `tests/protos/` with:
//! `protoc --descriptor_set_out=enum_defaults.fds enum_defaults.proto`.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage, ReflectMessageMut, ValueRef};
use buffa_descriptor::{DescriptorPool, FieldDescriptor};

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(include_bytes!("protos/enum_defaults.fds")).unwrap())
}

fn decode(pool: &Arc<DescriptorPool>, wire: &[u8]) -> DynamicMessage {
    let index = pool
        .message_index("reflect.enum_defaults.Defaults")
        .unwrap();
    DynamicMessage::decode(Arc::clone(pool), index, wire).unwrap()
}

fn assert_enum_number(message: &dyn ReflectMessage, field: &FieldDescriptor, expected: i32) {
    let ValueRef::EnumNumber(actual) = message.get(field) else {
        panic!("expected enum field {}", field.name());
    };
    assert_eq!(actual, expected, "field {}", field.name());
}

#[test]
fn absent_enum_fields_return_the_first_declared_number() {
    let pool = pool();
    let message = decode(&pool, &[]);
    let descriptor = message.message_descriptor();
    for (number, expected) in [(1, 7), (2, -7), (3, i32::MIN), (4, 0), (5, 7)] {
        let field = descriptor.field(number).unwrap();
        assert_enum_number(&message, field, expected);
        assert!(!message.has(field));
        assert!(message.field_by_number(number).is_none());
    }
    assert!(message.encode_to_vec().is_empty());
}

#[test]
fn absent_enum_extension_returns_the_first_declared_number() {
    let pool = pool();
    let message = decode(&pool, &[]);
    let index = pool
        .extension_index("reflect.enum_defaults.extension")
        .unwrap();
    let field = pool.extension(index).field();
    assert_enum_number(&message, field, 7);
    assert!(!message.has(field));
    assert!(message.encode_to_vec().is_empty());
}

#[test]
fn absent_nested_message_returns_the_same_enum_defaults() {
    let pool = pool();
    let message = decode(&pool, &[]);
    let nested_field = message.message_descriptor().field(6).unwrap();
    let ValueRef::Message(nested) = message.get(nested_field) else {
        panic!("expected nested message");
    };
    for (number, expected) in [(1, 7), (2, -7), (3, i32::MIN), (4, 0), (5, 7)] {
        let field = nested.message_descriptor().field(number).unwrap();
        assert_enum_number(&*nested, field, expected);
        assert!(!nested.has(field));
    }
    assert!(!message.has(nested_field));
    assert!(message.encode_to_vec().is_empty());
}

#[test]
fn explicit_enum_values_override_the_default() {
    let pool = pool();
    for (number, value) in [
        (1, 0),
        (1, 7),
        (1, 9),
        (2, -7),
        (2, -9),
        (3, i32::MIN),
        (3, i32::MAX),
        (4, 0),
        (5, 0),
        (5, 7),
        (100, 7),
        (100, 9),
    ] {
        let mut wire = Vec::new();
        Tag::new(number, WireType::Varint).encode(&mut wire);
        encode_varint(value as u64, &mut wire);
        let message = decode(&pool, &wire);
        let field = if number == 100 {
            pool.extensions_of(
                pool.message_index("reflect.enum_defaults.Defaults")
                    .unwrap(),
            )
            .next()
            .unwrap()
            .field()
        } else {
            message.message_descriptor().field(number).unwrap()
        };
        assert_enum_number(&message, field, value);
        assert!(message.has(field));
        assert_eq!(message.encode_to_vec(), wire);
    }
}

#[test]
fn unknown_closed_enum_value_leaves_the_field_at_its_default() {
    let pool = pool();
    let wire = [0x08, 123];
    let message = decode(&pool, &wire);
    let field = message.message_descriptor().field(1).unwrap();
    assert_enum_number(&message, field, 7);
    assert!(!message.has(field));
    assert_eq!(message.unknown_fields().len(), 1);
    assert_eq!(message.encode_to_vec(), wire);
}

#[test]
fn clearing_an_enum_field_restores_its_declared_default() {
    let pool = pool();
    let mut message = decode(&pool, &[0x08, 9]);
    let field = pool
        .message_by_name("reflect.enum_defaults.Defaults")
        .unwrap()
        .field(1)
        .unwrap();
    assert_enum_number(&message, field, 9);
    message.clear(field);
    assert_enum_number(&message, field, 7);
    assert!(!message.has(field));
    assert!(message.encode_to_vec().is_empty());
}

#[test]
fn absent_open_enum_field_keeps_its_zero_default() {
    let pool = Arc::new(DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap());
    let index = pool.message_index("reflect.test.Containers").unwrap();
    let message = DynamicMessage::decode(Arc::clone(&pool), index, &[]).unwrap();
    let field = message.message_descriptor().field(6).unwrap();
    assert_enum_number(&message, field, 0);
    assert!(!message.has(field));
}

#[test]
fn enum_containers_keep_their_empty_default() {
    let pool = pool();
    let message = decode(&pool, &[]);
    let descriptor = message.message_descriptor();
    let ValueRef::List(list) = message.get(descriptor.field(7).unwrap()) else {
        panic!("expected list");
    };
    assert!(list.is_empty());
    let ValueRef::Map(map) = message.get(descriptor.field(8).unwrap()) else {
        panic!("expected map");
    };
    assert!(map.is_empty());
}
