//! JSON tests for custom string-key map dispatch.

use crate::string_map::{MapStr, Maps};
use buffa_types::google::protobuf::UInt32Value;

#[test]
fn custom_string_key_wrapper_map_roundtrip() {
    let msg = Maps {
        wrapped: [(MapStr::from("priority"), UInt32Value::from(7))]
            .into_iter()
            .collect(),
        ..Default::default()
    };

    let json = serde_json::to_value(&msg).expect("serialize");
    assert_eq!(json["wrapped"]["priority"], serde_json::json!(7));

    let decoded: Maps = serde_json::from_value(json).expect("deserialize");
    assert_eq!(decoded.wrapped[&MapStr::from("priority")].value, 7);
}

#[test]
fn custom_string_key_wrapper_map_rejects_null_value() {
    let result = serde_json::from_value::<Maps>(serde_json::json!({
        "wrapped": {"priority": null}
    }));
    assert!(result.is_err(), "null wrapper map values must be rejected");
}

#[test]
fn custom_string_key_32bit_integer_maps_numeric_forms() {
    let decoded: Maps = serde_json::from_str(
        r#"{"si32":{"a":"-1","b":2.0,"c":3e1},"su32":{"a":"4294967295","b":"4.0","c":"5e1"}}"#,
    )
    .unwrap();
    assert_eq!(decoded.si32[&MapStr::from("a")], -1);
    assert_eq!(decoded.si32[&MapStr::from("b")], 2);
    assert_eq!(decoded.si32[&MapStr::from("c")], 30);
    assert_eq!(decoded.su32[&MapStr::from("a")], u32::MAX);
    assert_eq!(decoded.su32[&MapStr::from("b")], 4);
    assert_eq!(decoded.su32[&MapStr::from("c")], 50);

    let json = serde_json::to_value(&decoded).unwrap();
    assert_eq!(json["si32"], serde_json::json!({"a": -1, "b": 2, "c": 30}));
    assert_eq!(
        json["su32"],
        serde_json::json!({"a": u32::MAX, "b": 4, "c": 50})
    );
    assert_eq!(serde_json::from_value::<Maps>(json).unwrap(), decoded);
}

#[test]
fn custom_string_key_32bit_integer_maps_reject_invalid_values() {
    for json in [
        r#"{"si32":{"a":null}}"#,
        r#"{"su32":{"a":null}}"#,
        r#"{"si32":{"a":"1.5"}}"#,
        r#"{"su32":{"a":1.5}}"#,
        r#"{"si32":{"a":"2147483648"}}"#,
        r#"{"su32":{"a":"4294967296"}}"#,
        r#"{"su32":{"a":"-1"}}"#,
    ] {
        assert!(
            serde_json::from_str::<Maps>(json).is_err(),
            "accepted {json}"
        );
    }
}
