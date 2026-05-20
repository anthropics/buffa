//! End-to-end tests for [`DynamicMessage`]'s descriptor-driven JSON codec.

#![cfg(all(feature = "reflect", feature = "json"))]

use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, MapKey, MapValue, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test.fds");

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

#[test]
fn json_scalar_round_trip() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let md = p.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    msg.set(md.field(3).unwrap(), Value::I32(-42));
    msg.set(md.field(4).unwrap(), Value::I64(i64::MAX));
    msg.set(md.field(13).unwrap(), Value::Bool(true));
    msg.set(md.field(14).unwrap(), Value::String("hi".into()));
    msg.set(md.field(15).unwrap(), Value::Bytes(vec![1, 2, 3]));

    let json = msg.to_json().unwrap();
    // 64-bit integers serialize as quoted strings.
    assert!(json.contains(&format!("\"{}\"", i64::MAX)));
    // bytes serialize as base64.
    assert!(json.contains("\"AQID\""));

    let parsed = DynamicMessage::from_json(Arc::clone(&p), idx, &json).unwrap();
    assert_eq!(msg, parsed);
}

#[test]
fn json_containers_round_trip() {
    let p = pool();
    let containers_idx = p.message_index("reflect.test.Containers").unwrap();
    let inner_idx = p.message_index("reflect.test.Inner").unwrap();
    let md = p.message_by_name("reflect.test.Containers").unwrap();
    let inner_md = p.message_by_name("reflect.test.Inner").unwrap();

    let mut inner = DynamicMessage::new(Arc::clone(&p), inner_idx);
    inner.set(inner_md.field(1).unwrap(), Value::String("c1".into()));
    inner.set(inner_md.field(2).unwrap(), Value::I32(7));

    let mut msg = DynamicMessage::new(Arc::clone(&p), containers_idx);
    msg.set(
        md.field(1).unwrap(),
        Value::List(vec![Value::I32(1), Value::I32(2)]),
    );
    let mut tags = MapValue::new();
    tags.insert(MapKey::String("a".into()), Value::I32(1));
    msg.set(md.field(3).unwrap(), Value::Map(tags));
    msg.set(md.field(5).unwrap(), Value::Message(inner));
    msg.set(md.field(6).unwrap(), Value::EnumNumber(2)); // GREEN

    let json = msg.to_json().unwrap();
    // Enum serializes as a string name.
    assert!(json.contains("\"GREEN\""));
    // json_name camelCase.
    assert!(json.contains("\"packedInts\""));

    let parsed = DynamicMessage::from_json(Arc::clone(&p), containers_idx, &json).unwrap();
    assert_eq!(msg, parsed);
}

#[test]
fn json_default_omitted() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let msg = DynamicMessage::new(Arc::clone(&p), idx);
    assert_eq!(msg.to_json().unwrap(), "{}");
}

#[test]
fn json_accepts_proto_field_names() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    // Both camelCase json_name and snake_case proto name accepted.
    let m1 = DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"fInt32": 5}"#).unwrap();
    let m2 = DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"f_int32": 5}"#).unwrap();
    assert_eq!(m1, m2);
    assert_eq!(m1.field_by_number(3), Some(&Value::I32(5)));
}
