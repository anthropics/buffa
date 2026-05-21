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

#[test]
fn json_rejects_duplicate_field_keys() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    // Exact duplicate key.
    assert!(
        DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"fInt32": 1, "fInt32": 2}"#).is_err(),
        "exact duplicate key must be rejected"
    );
    // Same field via its proto name and its JSON name — still a duplicate.
    assert!(
        DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"f_int32": 1, "fInt32": 2}"#).is_err(),
        "proto-name/json-name duplicate must be rejected"
    );
    // Two distinct fields are fine.
    assert!(
        DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"fInt32": 1, "fInt64": "2"}"#).is_ok()
    );
}

#[test]
fn json_unknown_fields_error_by_default_and_skip_when_lenient() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let input = r#"{"fInt32": 5, "noSuchField": {"nested": [1, 2, {"deep": true}]}}"#;
    // Strict mode: unknown field is an error.
    assert!(DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err());
    // Lenient mode: unknown field (and its arbitrarily nested value) is
    // skipped; known fields still parse.
    let m = DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), idx, input)
        .expect("lenient parse succeeds");
    assert_eq!(m.field_by_number(3), Some(&Value::I32(5)));
    // Lenient mode still rejects malformed values on *known* fields.
    assert!(DynamicMessage::from_json_ignoring_unknown(
        Arc::clone(&p),
        idx,
        r#"{"fInt32": "not a number"}"#
    )
    .is_err());
}

/// Assert `input` fails a strict parse of `Containers` but succeeds a
/// lenient one, returning the lenient result.
fn assert_strict_rejects_lenient_accepts(input: &str) -> DynamicMessage {
    let p = pool();
    let idx = p.message_index("reflect.test.Containers").unwrap();
    assert!(
        DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err(),
        "strict parse must reject: {input}"
    );
    DynamicMessage::from_json_ignoring_unknown(Arc::clone(&p), idx, input)
        .unwrap_or_else(|e| panic!("lenient parse must accept {input}: {e}"))
}

#[test]
fn json_lenient_mode_propagates_to_nested_messages() {
    // `nested` (field 5) is a singular Inner; the unknown field inside it is
    // skipped and the known `id` field still parses.
    let m = assert_strict_rejects_lenient_accepts(r#"{"nested": {"id": "x", "futureField": 1}}"#);
    let Some(Value::Message(inner)) = m.field_by_number(5) else {
        panic!("nested message not set");
    };
    assert_eq!(inner.field_by_number(1), Some(&Value::String("x".into())));
}

#[test]
fn json_lenient_mode_propagates_to_repeated_message_elements() {
    // `inners` (field 8) is a repeated Inner — exercises the
    // ListVisitor → SingularSeed → nested-message path.
    let m = assert_strict_rejects_lenient_accepts(
        r#"{"inners": [{"id": "a"}, {"id": "b", "futureField": 1}]}"#,
    );
    let Some(Value::List(items)) = m.field_by_number(8) else {
        panic!("repeated message field not set");
    };
    assert_eq!(items.len(), 2);
    let Value::Message(second) = &items[1] else {
        panic!("element is not a message");
    };
    assert_eq!(second.field_by_number(1), Some(&Value::String("b".into())));
}

#[test]
fn json_lenient_mode_propagates_to_map_values() {
    // `children` (field 4) is a map<int32, Inner> — exercises the
    // MapFieldVisitor → SingularSeed → nested-message path.
    let m = assert_strict_rejects_lenient_accepts(
        r#"{"children": {"1": {"id": "c", "futureField": true}}}"#,
    );
    let Some(Value::Map(entries)) = m.field_by_number(4) else {
        panic!("map field not set");
    };
    assert_eq!(entries.len(), 1);
}
