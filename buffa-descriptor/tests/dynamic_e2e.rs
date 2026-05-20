//! End-to-end tests for [`DynamicMessage`] encode/decode and the
//! [`ReflectMessage`] trait surface against a `protoc`-compiled
//! `FileDescriptorSet`.

#![cfg(feature = "reflect")]

use std::sync::Arc;

use buffa_descriptor::reflect::{
    DynamicMessage, MapKey, MapValue, ReflectMessage, ReflectMessageMut, Value,
};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

#[test]
fn dynamic_message_scalar_round_trip() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    let md = p.message_by_name("reflect.test.Scalars").unwrap();

    // Set every field through the descriptor-keyed API.
    msg.set(md.field(1).unwrap(), Value::F64(1.5));
    msg.set(md.field(2).unwrap(), Value::F32(2.5));
    msg.set(md.field(3).unwrap(), Value::I32(-3));
    msg.set(md.field(4).unwrap(), Value::I64(-4));
    msg.set(md.field(5).unwrap(), Value::U32(5));
    msg.set(md.field(6).unwrap(), Value::U64(6));
    msg.set(md.field(7).unwrap(), Value::I32(-7));
    msg.set(md.field(8).unwrap(), Value::I64(-8));
    msg.set(md.field(9).unwrap(), Value::U32(9));
    msg.set(md.field(10).unwrap(), Value::U64(10));
    msg.set(md.field(11).unwrap(), Value::I32(-11));
    msg.set(md.field(12).unwrap(), Value::I64(-12));
    msg.set(md.field(13).unwrap(), Value::Bool(true));
    msg.set(md.field(14).unwrap(), Value::String("hello".into()));
    msg.set(md.field(15).unwrap(), Value::Bytes(vec![1, 2, 3]));
    msg.set(md.field(16).unwrap(), Value::I32(99));

    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &bytes).unwrap();
    assert_eq!(msg, decoded);

    // Spot-check a few values.
    assert_eq!(decoded.field_by_number(3), Some(&Value::I32(-3)));
    assert_eq!(
        decoded.field_by_number(14),
        Some(&Value::String("hello".into()))
    );
    assert_eq!(decoded.field_by_number(16), Some(&Value::I32(99)));
}

#[test]
fn dynamic_message_containers_round_trip() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    // Repeated packed ints.
    msg.set(
        md.field(1).unwrap(),
        Value::List(vec![Value::I32(1), Value::I32(2), Value::I32(300)]),
    );

    // Repeated strings (unpacked).
    msg.set(
        md.field(2).unwrap(),
        Value::List(vec![Value::String("a".into()), Value::String("b".into())]),
    );

    // map<string, int32>.
    let mut tags = MapValue::new();
    tags.insert(MapKey::String("k1".into()), Value::I32(10));
    tags.insert(MapKey::String("k2".into()), Value::I32(20));
    msg.set(md.field(3).unwrap(), Value::Map(tags));

    // map<int32, Inner>.
    let inner_md = p.message_by_name("reflect.test.Inner").unwrap();
    let mut child = DynamicMessage::new(Arc::clone(&p), inner_idx);
    child.set(inner_md.field(1).unwrap(), Value::String("c1".into()));
    child.set(inner_md.field(2).unwrap(), Value::I32(42));
    let mut children = MapValue::new();
    children.insert(MapKey::I32(1), Value::Message(child.clone()));
    msg.set(md.field(4).unwrap(), Value::Map(children));

    // Nested singular message.
    msg.set(md.field(5).unwrap(), Value::Message(child));

    // Enum.
    msg.set(md.field(6).unwrap(), Value::EnumNumber(2));

    // Repeated enum (packed).
    msg.set(
        md.field(7).unwrap(),
        Value::List(vec![Value::EnumNumber(1), Value::EnumNumber(3)]),
    );

    // Round-trip.
    let bytes = msg.encode_to_vec();
    let decoded = DynamicMessage::decode(Arc::clone(&p), containers_idx, &bytes).unwrap();
    assert_eq!(msg, decoded);

    // The encoded length should match the actual bytes written.
    assert_eq!(msg.encoded_len(), bytes.len());
}

#[test]
fn dynamic_message_unknown_fields_preserved() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    // Build wire bytes with a known field (int32 #3) and an unknown field
    // (#17, varint). Use buffa's own Tag encoder so the wire bytes are
    // correct by construction.
    use buffa::encoding::{Tag, WireType};
    let mut wire = Vec::new();
    Tag::new(3, WireType::Varint).encode(&mut wire);
    wire.push(7u8); // f_int32 = 7
    Tag::new(17, WireType::Varint).encode(&mut wire);
    wire.push(0x05u8); // unknown field 17 = 5

    let decoded = DynamicMessage::decode(Arc::clone(&p), idx, &wire).unwrap();
    assert_eq!(decoded.field_by_number(3), Some(&Value::I32(7)));
    assert_eq!(decoded.unknown_fields().len(), 1);

    // Round-trip preserves the unknown field.
    let re_encoded = decoded.encode_to_vec();
    assert_eq!(re_encoded.len(), wire.len());
}

#[test]
fn reflect_message_get_has_for_each() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    let md = p.message_by_name("reflect.test.Scalars").unwrap();

    msg.set(md.field(3).unwrap(), Value::I32(42));
    msg.set(md.field(14).unwrap(), Value::String("abc".into()));

    // get returns the set value.
    let v = msg.get(md.field(3).unwrap());
    assert!(matches!(v, buffa_descriptor::reflect::ValueRef::I32(42)));

    // get on an absent field returns the default.
    let v = msg.get(md.field(13).unwrap());
    assert!(matches!(
        v,
        buffa_descriptor::reflect::ValueRef::Bool(false)
    ));

    // has reflects presence.
    assert!(msg.has(md.field(3).unwrap()));
    assert!(!msg.has(md.field(13).unwrap()));

    // for_each_set visits exactly the set fields.
    let mut seen = Vec::new();
    msg.for_each_set(&mut |fd, _| seen.push(fd.number()));
    seen.sort();
    assert_eq!(seen, vec![3, 14]);
}

#[test]
fn dynamic_message_empty_containers_have_returns_false() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    let md = p.message_by_name("reflect.test.Containers").unwrap();

    // Empty list and map — has() should be false, for_each_set should skip.
    msg.set(md.field(1).unwrap(), Value::List(Vec::new()));
    msg.set(md.field(3).unwrap(), Value::Map(MapValue::new()));

    assert!(!msg.has(md.field(1).unwrap()));
    assert!(!msg.has(md.field(3).unwrap()));

    let mut count = 0;
    msg.for_each_set(&mut |_, _| count += 1);
    assert_eq!(count, 0);
}

#[test]
fn which_oneof_resolves_set_member() {
    let p = pool();
    let oneof_idx = p.message_index("reflect.test.OneOf").unwrap();
    let md = p.message_by_name("reflect.test.OneOf").unwrap();
    let oneof = &md.oneofs()[0];

    // Empty message — no oneof member set.
    let empty = DynamicMessage::new(Arc::clone(&p), oneof_idx);
    assert!(empty.which_oneof(oneof).is_none());

    // Set one member.
    let mut msg = DynamicMessage::new(Arc::clone(&p), oneof_idx);
    msg.set(md.field(2).unwrap(), Value::String("hello".into()));
    let active = msg.which_oneof(oneof).expect("a member is set");
    assert_eq!(active.number(), 2);
    assert_eq!(active.name(), "text");

    // Switch to a different member — last write wins.
    msg.set(md.field(1).unwrap(), Value::I32(42));
    let active = msg.which_oneof(oneof).expect("a member is set");
    assert_eq!(active.number(), 1);
    assert_eq!(active.name(), "num");
}
