#![cfg(all(feature = "reflect", feature = "json"))]

use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, MapKey, MapValue, ReflectMessageMut, Value};
use buffa_descriptor::DescriptorPool;

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(include_bytes!("protos/json_struct_test.fds")).unwrap())
}

fn message(p: &Arc<DescriptorPool>, name: &str) -> DynamicMessage {
    DynamicMessage::new(Arc::clone(p), p.message_index(name).unwrap())
}

#[test]
fn unset_value_serialize_is_error() {
    let p = pool();
    let msg = message(&p, "google.protobuf.Value");
    assert!(msg.to_json().is_err());
    assert!(serde_json::to_string(&msg).is_err());
}

#[test]
fn unset_value_in_containers_serialize_is_error() {
    let p = pool();
    let unset = message(&p, "google.protobuf.Value");
    let value_md = p.message_by_name("google.protobuf.Value").unwrap();

    let mut st = message(&p, "google.protobuf.Struct");
    let st_md = p.message_by_name("google.protobuf.Struct").unwrap();
    let mut fields = MapValue::new();
    fields.insert(
        MapKey::String("unset".into()),
        Value::Message(unset.clone()),
    );
    st.set(st_md.field(1).unwrap(), Value::Map(fields));
    assert!(st.to_json().is_err());

    let mut list = message(&p, "google.protobuf.ListValue");
    let list_md = p.message_by_name("google.protobuf.ListValue").unwrap();
    list.set(
        list_md.field(1).unwrap(),
        Value::List(vec![Value::Message(unset.clone())]),
    );
    assert!(list.to_json().is_err());

    for (number, container) in [(5, st), (6, list)] {
        let mut value = message(&p, "google.protobuf.Value");
        value.set(value_md.field(number).unwrap(), Value::Message(container));
        assert!(value.to_json().is_err());
    }

    for (number, value) in [
        (1, Value::Message(unset.clone())),
        (3, Value::List(vec![Value::Message(unset)])),
    ] {
        let mut payload = message(&p, "reflect.json.Payload");
        let payload_md = p.message_by_name("reflect.json.Payload").unwrap();
        payload.set(payload_md.field(number).unwrap(), value);
        assert!(payload.to_json().is_err());
    }
}

#[test]
fn value_default_variants_roundtrip() {
    let p = pool();
    let idx = p.message_index("google.protobuf.Value").unwrap();
    let md = p.message_by_name("google.protobuf.Value").unwrap();
    for (number, value, expected) in [
        (1, Value::EnumNumber(0), "null"),
        (2, Value::F64(0.0), "0.0"),
        (3, Value::String(String::new()), r#""""#),
        (4, Value::Bool(false), "false"),
        (
            5,
            Value::Message(message(&p, "google.protobuf.Struct")),
            "{}",
        ),
        (
            6,
            Value::Message(message(&p, "google.protobuf.ListValue")),
            "[]",
        ),
    ] {
        let mut msg = message(&p, "google.protobuf.Value");
        msg.set(md.field(number).unwrap(), value);
        let json = msg.to_json().unwrap();
        assert_eq!(json, expected);
        let parsed = DynamicMessage::from_json(Arc::clone(&p), idx, &json).unwrap();
        assert!(parsed.field_by_number(number).is_some());
        assert_eq!(parsed.to_json().unwrap(), expected);
    }
}

#[test]
fn absent_value_field_serializes_normally() {
    let p = pool();
    let payload = message(&p, "reflect.json.Payload");
    assert_eq!(payload.to_json().unwrap(), "{}");
}
