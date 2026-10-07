//! Reflective JSON for wrapper messages against a protoc-compiled schema.
//!
//! Regenerate the fixture from `tests/protos/` with:
//! `protoc --include_imports --descriptor_set_out=json_wrappers_test.fds json_wrappers_test.proto`.

#![cfg(all(feature = "reflect", feature = "json"))]

use std::sync::Arc;

use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage};
use buffa_descriptor::DescriptorPool;

const FDS_BYTES: &[u8] = include_bytes!("protos/json_wrappers_test.fds");

const WRAPPERS: &[(&str, &str, &str, &str)] = &[
    ("DoubleValue", "0.0", "1.5", "1.5"),
    ("FloatValue", "0.0", "1.5", "1.5"),
    ("Int64Value", r#""0""#, "-42", r#""-42""#),
    ("UInt64Value", r#""0""#, "42", r#""42""#),
    ("Int32Value", "0", "-42", "-42"),
    ("UInt32Value", "0", "42", "42"),
    ("BoolValue", "false", "true", "true"),
    ("StringValue", r#""""#, r#""hello""#, r#""hello""#),
    ("BytesValue", r#""""#, r#""AQID""#, r#""AQID""#),
];

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

fn parse(p: &Arc<DescriptorPool>, name: &str, json: &str) -> DynamicMessage {
    let idx = p.message_index(name).unwrap();
    DynamicMessage::from_json(Arc::clone(p), idx, json)
        .unwrap_or_else(|err| panic!("{name}: {json} must parse: {err}"))
}

#[test]
fn wrappers_accept_null_as_the_default_message() {
    let p = pool();
    for &(name, default_json, _, _) in WRAPPERS {
        let name = format!("google.protobuf.{name}");
        for input in ["null", " \n null \t "] {
            let msg = parse(&p, &name, input);
            let field = msg.message_descriptor().field(1).unwrap();
            assert!(!msg.has(field), "{name}: null must leave value unset");
            assert!(
                msg.encode_to_vec().is_empty(),
                "{name}: null has no wire value"
            );
            assert_eq!(msg.to_json().unwrap(), default_json, "{name}");
        }
    }
}

#[test]
fn wrappers_preserve_non_null_values() {
    let p = pool();
    for &(name, _, input, output) in WRAPPERS {
        let name = format!("google.protobuf.{name}");
        let msg = parse(&p, &name, input);
        assert_eq!(msg.to_json().unwrap(), output, "{name}");
        assert!(
            msg.has(msg.message_descriptor().field(1).unwrap()),
            "{name}"
        );
    }
}

#[test]
fn wrappers_reject_objects_and_arrays() {
    let p = pool();
    for &(name, _, _, _) in WRAPPERS {
        let name = format!("google.protobuf.{name}");
        let idx = p.message_index(&name).unwrap();
        for input in ["{}", "[]", "[null]", r#"{"value":null}"#] {
            assert!(
                DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err(),
                "{name}: {input} must be rejected"
            );
        }
    }
}

#[test]
fn wrappers_reject_invalid_scalar_values() {
    let p = pool();
    for (name, input) in [
        ("DoubleValue", r#""1e999""#),
        ("FloatValue", r#""3.5e38""#),
        ("Int64Value", r#""9223372036854775808""#),
        ("UInt64Value", "-1"),
        ("Int32Value", "1.5"),
        ("UInt32Value", "4294967296"),
        ("BoolValue", r#""true""#),
        ("StringValue", "42"),
        ("BytesValue", r#""?""#),
    ] {
        let name = format!("google.protobuf.{name}");
        let idx = p.message_index(&name).unwrap();
        assert!(
            DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err(),
            "{name}: {input} must be rejected"
        );
    }
}

#[cfg(feature = "std")]
#[test]
fn any_wrappers_accept_null_as_an_empty_payload() {
    let p = pool();
    for &(name, default_json, _, _) in WRAPPERS {
        let url = format!("type.googleapis.com/google.protobuf.{name}");
        let input = format!(r#"{{"@type":"{url}","value":null}}"#);
        let msg = parse(&p, "google.protobuf.Any", &input);
        let inner = msg.unpack_any().unwrap();
        assert!(inner.encode_to_vec().is_empty(), "{name}");
        assert_eq!(inner.to_json().unwrap(), default_json, "{name}");
        assert_eq!(
            msg.to_json().unwrap(),
            format!(r#"{{"@type":"{url}","value":{default_json}}}"#),
            "{name}"
        );
    }
}

#[cfg(feature = "std")]
#[test]
fn any_wrappers_still_require_a_value_key() {
    let p = pool();
    let idx = p.message_index("google.protobuf.Any").unwrap();
    for &(name, _, _, _) in WRAPPERS {
        let input = format!(r#"{{"@type":"type.googleapis.com/google.protobuf.{name}"}}"#);
        assert!(
            DynamicMessage::from_json(Arc::clone(&p), idx, &input).is_err(),
            "{name}: the value key must be present"
        );
    }
}

#[test]
fn null_wrapper_fields_remain_absent() {
    let p = pool();
    let msg = parse(
        &p,
        "reflect.json.Wrappers",
        r#"{"optionalValue":null,"repeatedValues":null,"mapValues":null,"choice":null}"#,
    );
    assert!(msg.encode_to_vec().is_empty());
    assert_eq!(msg.to_json().unwrap(), "{}");
    for field in msg.message_descriptor().fields() {
        assert!(!msg.has(field), "{}", field.name());
    }
}

#[test]
fn default_wrapper_values_keep_message_presence() {
    let p = pool();
    let input = r#"{"optionalValue":0,"repeatedValues":[0,1],"mapValues":{"key":0},"choice":0}"#;
    let msg = parse(&p, "reflect.json.Wrappers", input);
    assert_eq!(msg.to_json().unwrap(), input);
    for number in 1..=4 {
        let field = msg.message_descriptor().field(number).unwrap();
        assert!(msg.has(field), "{}", field.name());
    }
    let decoded =
        DynamicMessage::decode(Arc::clone(&p), msg.message_index(), &msg.encode_to_vec()).unwrap();
    assert_eq!(decoded.to_json().unwrap(), input);
}

#[test]
fn null_wrapper_elements_remain_invalid() {
    let p = pool();
    let idx = p.message_index("reflect.json.Wrappers").unwrap();
    for input in [
        r#"{"repeatedValues":[null]}"#,
        r#"{"repeatedValues":[1,null]}"#,
        r#"{"mapValues":{"key":null}}"#,
        r#"{"optionalValue":[null]}"#,
    ] {
        assert!(
            DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err(),
            "{input} must be rejected"
        );
    }
}
