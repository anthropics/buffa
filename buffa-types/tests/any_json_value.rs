#![cfg(feature = "json")]

use buffa::type_registry::{set_type_registry, TypeRegistry};
use buffa::Message;
use buffa_types::google::protobuf::{value::Kind, Any, BoolValue, Empty, Value};
use serde_json::{json, Value as JsonValue};

fn register_wkts() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        let mut registry = TypeRegistry::new();
        buffa_types::register_wkt_types(&mut registry);
        set_type_registry(registry);
    });
}

fn wkt_cases() -> [(&'static str, JsonValue); 16] {
    [
        ("BoolValue", json!(false)),
        ("Int32Value", json!(0)),
        ("UInt32Value", json!(0)),
        ("Int64Value", json!("0")),
        ("UInt64Value", json!("0")),
        ("FloatValue", json!(0.0)),
        ("DoubleValue", json!(0.0)),
        ("StringValue", json!("")),
        ("BytesValue", json!("")),
        ("Duration", json!("0s")),
        ("Timestamp", json!("1970-01-01T00:00:00Z")),
        ("FieldMask", json!("")),
        ("Value", JsonValue::Null),
        ("Struct", json!({})),
        ("ListValue", json!([])),
        ("Any", json!({})),
    ]
}

#[test]
fn registered_wkts_require_a_value_key() {
    register_wkts();
    for (name, _) in wkt_cases() {
        for prefix in ["type.googleapis.com", "custom.example/v1", ""] {
            let type_url = format!("{prefix}/google.protobuf.{name}");
            let input = json!({ "@type": type_url });
            for result in [
                serde_json::from_str::<Any>(&input.to_string()),
                serde_json::from_value::<Any>(input),
            ] {
                let error = result.expect_err(&type_url).to_string();
                assert!(error.contains(&type_url), "{error}");
                assert!(error.contains("requires a \"value\" key"), "{error}");
            }
        }
    }
}

#[test]
fn registered_wkts_round_trip_with_a_value_key() {
    register_wkts();
    for (name, value) in wkt_cases() {
        let type_url = format!("type.googleapis.com/google.protobuf.{name}");
        let input = json!({ "@type": type_url, "value": value });
        let any: Any = serde_json::from_str(&input.to_string()).unwrap();
        assert_eq!(serde_json::to_value(&any).unwrap(), input, "{name}");
        let decoded = Any::decode_from_slice(&any.encode_to_vec()).unwrap();
        assert_eq!(decoded, any, "{name}");
    }
}

#[test]
fn an_explicit_null_value_remains_a_null_value() {
    register_wkts();
    let input = json!({ "@type": Value::TYPE_URL, "value": null });
    let any: Any = serde_json::from_value(input.clone()).unwrap();
    let value: Value = any.unpack_unchecked().unwrap();
    assert!(matches!(value.kind, Some(Kind::NullValue(_))));
    assert_eq!(serde_json::to_value(any).unwrap(), input);
}

#[test]
fn an_explicit_null_wrapper_value_remains_a_default_scalar() {
    register_wkts();
    let input = json!({ "@type": BoolValue::TYPE_URL, "value": null });
    let any: Any = serde_json::from_value(input).unwrap();
    let value: BoolValue = any.unpack_unchecked().unwrap();
    assert!(!value.value);
}

#[test]
fn a_nested_any_requires_its_wkt_value_key() {
    register_wkts();
    let input = json!({
        "@type": Any::TYPE_URL,
        "value": { "@type": BoolValue::TYPE_URL },
    });
    let error = serde_json::from_value::<Any>(input)
        .unwrap_err()
        .to_string();
    assert!(error.contains(BoolValue::TYPE_URL), "{error}");
    assert!(error.contains("requires a \"value\" key"), "{error}");
}

#[test]
fn a_value_before_the_type_tag_is_accepted() {
    register_wkts();
    let input = r#"{"value":false,"@type":"type.googleapis.com/google.protobuf.BoolValue"}"#;
    let any: Any = serde_json::from_str(input).unwrap();
    assert_eq!(any, Any::pack_message(&BoolValue::default()));
}

#[test]
fn a_regular_empty_message_needs_no_value_key() {
    register_wkts();
    let input = json!({ "@type": Empty::TYPE_URL });
    let any: Any = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(any, Any::pack_message(&Empty::default()));
    assert_eq!(serde_json::to_value(any).unwrap(), input);
}

#[test]
fn an_empty_any_needs_no_value_key() {
    register_wkts();
    let any: Any = serde_json::from_str("{}").unwrap();
    assert_eq!(any, Any::default());
}

#[test]
fn an_unregistered_type_keeps_its_opaque_empty_payload() {
    register_wkts();
    let type_url = "type.googleapis.com/test.Unregistered";
    let any: Any = serde_json::from_value(json!({ "@type": type_url })).unwrap();
    assert_eq!(any.type_url, type_url);
    assert!(any.value.is_empty());
}
