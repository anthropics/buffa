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
