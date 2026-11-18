//! The merge failure policy must cover every destination container and limit.
#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa::encoding::{encode_varint, Tag, WireType};
use buffa::{DecodeError, DecodeOptions};
use buffa_descriptor::reflect::{DynamicMessage, MapKey, MapValue, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

const UNKNOWN_FIELD: u32 = 99;
const EXTENSION_GROUP: u32 = 120;

#[derive(Clone, Copy)]
enum Fixture {
    Containers,
    Oneof,
    Extensions,
}
impl Fixture {
    fn name(self) -> &'static str {
        match self {
            Self::Containers => "reflect.test.Containers",
            Self::Oneof => "reflect.test.OneOf",
            Self::Extensions => "reflect.ext.Extendable",
        }
    }
}

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(include_bytes!("protos/reflect_test.fds")).unwrap())
}

fn varint(number: u32, value: u64) -> Vec<u8> {
    let mut wire = Vec::new();
    Tag::new(number, WireType::Varint).encode(&mut wire);
    encode_varint(value, &mut wire);
    wire
}

fn framed(number: u32, payload: &[u8]) -> Vec<u8> {
    let mut wire = Vec::new();
    Tag::new(number, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(u64::try_from(payload.len()).unwrap(), &mut wire);
    wire.extend_from_slice(payload);
    wire
}

fn child(pool: &Arc<DescriptorPool>) -> DynamicMessage {
    DynamicMessage::decode(
        Arc::clone(pool),
        pool.message_index("reflect.test.Inner").unwrap(),
        &framed(1, b"old-child"),
    )
    .unwrap()
}

fn populated(pool: &Arc<DescriptorPool>, fixture: Fixture) -> DynamicMessage {
    let mut message = DynamicMessage::new_by_name(Arc::clone(pool), fixture.name()).unwrap();
    let descriptor = pool.message_by_name(fixture.name()).unwrap();
    match fixture {
        Fixture::Containers => {
            for (number, value) in [
                (1, Value::List(vec![Value::I32(17)])),
                (2, Value::List(vec![Value::String("old-list".into())])),
                (
                    3,
                    Value::Map(MapValue::from_entries(vec![(
                        MapKey::String("old-key".into()),
                        Value::I32(7),
                    )])),
                ),
                (5, Value::Message(child(pool))),
                (8, Value::List(vec![Value::Message(child(pool))])),
            ] {
                message
                    .try_set(descriptor.field(number).unwrap(), value)
                    .unwrap();
            }
        }
        Fixture::Oneof => {
            message
                .try_set(descriptor.field(3).unwrap(), Value::Message(child(pool)))
                .unwrap();
        }
        Fixture::Extensions => {
            let mut wire = varint(1, 17);
            wire.extend(varint(100, 7));
            wire.extend(framed(101, b"old-extension"));
            wire.extend(varint(102, 3));
            wire.extend(framed(103, &framed(1, b"old-payload")));
            Tag::new(EXTENSION_GROUP, WireType::StartGroup).encode(&mut wire);
            wire.extend(varint(1, 77));
            Tag::new(EXTENSION_GROUP, WireType::EndGroup).encode(&mut wire);
            message.merge(&wire).unwrap();
        }
    }
    // Old unknown data must disappear too, not just records read by the
    // failing call. The per-call budget does not charge pre-existing fields.
    message.merge(&varint(UNKNOWN_FIELD, 7)).unwrap();
    message
}

fn assert_failure_policy(
    initial: &DynamicMessage,
    wire: &[u8],
    options: DecodeOptions,
    expected: DecodeError,
) -> DynamicMessage {
    let before = initial.clone();
    let mut retained = initial.clone();
    let disabled = options.clone().with_clear_on_merge_error(false);
    assert_eq!(
        retained.merge_with_options(wire, &disabled),
        Err(expected.clone())
    );
    assert!(retained.iter_set_fields().next().is_some());
    assert!(!retained.unknown_fields().is_empty());
    if expected == DecodeError::MessageTooLarge {
        assert_eq!(retained, before, "size rejection happens before parsing");
    }

    let mut cleared = initial.clone();
    let empty = DynamicMessage::new(Arc::clone(initial.pool()), initial.message_index());
    let enabled = options.with_clear_on_merge_error(true);
    assert_eq!(cleared.merge_with_options(wire, &enabled), Err(expected));
    assert_eq!(cleared, empty);
    assert!(cleared.unknown_fields().is_empty());
    assert!(cleared.encode_to_vec().is_empty());
    assert!(Arc::ptr_eq(cleared.pool(), initial.pool()));
    assert_eq!(cleared.message_index(), initial.message_index());
    assert_eq!(*initial, before, "a caller-owned clone remains independent");
    retained
}

#[test]
fn nested_list_and_map_errors_clear_the_entire_destination() {
    let pool = pool();
    let initial = populated(&pool, Fixture::Containers);
    let mut invalid_child = framed(1, b"new-child");
    invalid_child.push(0); // Invalid field after a successful nested update.
    let mut map_entry = varint(1, 1);
    map_entry.extend(framed(2, &invalid_child));
    for (wire, expected) in [
        (framed(5, &invalid_child), DecodeError::InvalidFieldNumber),
        (framed(8, &invalid_child), DecodeError::InvalidFieldNumber),
        (framed(4, &map_entry), DecodeError::InvalidFieldNumber),
        (framed(3, &framed(1, &[0xff])), DecodeError::InvalidUtf8),
        (vec![0x2a, 5, 1], DecodeError::UnexpectedEof),
    ] {
        assert_failure_policy(&initial, &wire, DecodeOptions::new(), expected);
    }

    // Sorting still completes on failure when partial results are retained.
    let mut wire = Vec::new();
    for (key, value) in [(b'z', 1), (b'a', 2)] {
        let mut entry = framed(1, &[key]);
        entry.extend(varint(2, value));
        wire.extend(framed(3, &entry));
    }
    wire.push(0);
    let retained = assert_failure_policy(
        &initial,
        &wire,
        DecodeOptions::new(),
        DecodeError::InvalidFieldNumber,
    );
    let Some(Value::Map(map)) = retained.field_by_number(3) else {
        panic!("map was lost without the clearing policy");
    };
    assert_eq!(map.get(&MapKey::String("a".into())), Some(&Value::I32(2)));
    assert_eq!(map.get(&MapKey::String("z".into())), Some(&Value::I32(1)));
}

#[test]
fn every_resource_limit_applies_the_same_failure_policy() {
    let pool = pool();
    let initial = populated(&pool, Fixture::Containers);
    for (wire, options, expected) in [
        (
            varint(UNKNOWN_FIELD, 9),
            DecodeOptions::new().with_unknown_field_limit(0),
            DecodeError::UnknownFieldLimitExceeded,
        ),
        (
            framed(1, &[1, 2]),
            DecodeOptions::new().with_element_memory_limit(0),
            DecodeError::ElementMemoryLimitExceeded,
        ),
        (
            framed(5, &framed(1, b"new-child")),
            DecodeOptions::new().with_recursion_limit(0),
            DecodeError::RecursionLimitExceeded,
        ),
        (
            varint(1, 9),
            DecodeOptions::new().with_max_message_size(0),
            DecodeError::MessageTooLarge,
        ),
    ] {
        assert_failure_policy(&initial, &wire, options, expected);
    }
}

#[test]
fn oneof_failures_cover_existing_children_and_failed_replacements() {
    let pool = pool();
    let initial = populated(&pool, Fixture::Oneof);
    let mut invalid_child = framed(1, b"new-child");
    invalid_child.push(0);
    let wire = framed(3, &invalid_child);
    assert_failure_policy(
        &initial,
        &wire,
        DecodeOptions::new(),
        DecodeError::InvalidFieldNumber,
    );

    let mut initial = initial;
    initial
        .try_set(
            pool.message_by_name(Fixture::Oneof.name())
                .unwrap()
                .field(1)
                .unwrap(),
            Value::I32(99),
        )
        .unwrap();
    let retained = assert_failure_policy(
        &initial,
        &wire,
        DecodeOptions::new(),
        DecodeError::InvalidFieldNumber,
    );
    assert_eq!(
        retained, initial,
        "a failed replacement keeps the old oneof without opt-in"
    );
}

#[test]
fn registered_extensions_and_unknown_groups_are_removed_on_failure() {
    let pool = pool();
    let initial = populated(&pool, Fixture::Extensions);
    let mut message = framed(1, b"new-payload");
    message.push(0);
    let mut group = Vec::new();
    Tag::new(EXTENSION_GROUP, WireType::StartGroup).encode(&mut group);
    group.extend(varint(1, 9));
    Tag::new(121, WireType::EndGroup).encode(&mut group);
    let mut unknown_group = Vec::new();
    Tag::new(UNKNOWN_FIELD, WireType::StartGroup).encode(&mut unknown_group);
    unknown_group.extend(varint(1, 9));
    for (wire, expected) in [
        (framed(101, &[0xff]), DecodeError::InvalidUtf8),
        (framed(103, &message), DecodeError::InvalidFieldNumber),
        (group, DecodeError::InvalidEndGroup(121)),
        (unknown_group, DecodeError::UnexpectedEof),
    ] {
        assert_failure_policy(&initial, &wire, DecodeOptions::new(), expected);
    }
}

#[test]
fn success_empty_input_and_reuse_keep_normal_merge_semantics() {
    let pool = pool();
    let mut message = populated(&pool, Fixture::Containers);
    let initial = message.clone();
    let enabled = DecodeOptions::new().clear_on_merge_error();
    message.merge_with_options(&[], &enabled).unwrap();
    assert_eq!(message, initial);
    let wire = framed(5, &framed(1, b"new-child"));
    let mut expected = initial.clone();
    expected.merge(&wire).unwrap();
    message.merge_with_options(&wire, &enabled).unwrap();
    assert_eq!(message, expected);

    assert_eq!(
        message.merge_with_options(&[0], &enabled),
        Err(DecodeError::InvalidFieldNumber)
    );
    message.merge_with_options(&wire, &enabled).unwrap();
    assert_eq!(
        message,
        DynamicMessage::decode(Arc::clone(&pool), message.message_index(), &wire).unwrap()
    );
}
