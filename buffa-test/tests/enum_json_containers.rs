//! Malformed enum containers must not expose their scalar input in diagnostics.

use buffa_descriptor::reflect::{DynamicMessage, ReflectMessage};
use buffa_test::json_types::WithEnum;
use buffa_test::open_enums::OpenEnumContexts;
use buffa_test::p2json::ClosedEnumJson;
use serde_json::{json, Value};
use std::sync::Arc;

#[test]
fn malformed_enum_container_errors_omit_input() {
    let reflected = OpenEnumContexts::default();
    let pool = reflected.pool();
    let index = pool
        .message_index("test.openenums.OpenEnumContexts")
        .unwrap();
    for ignore_unknown in [false, true] {
        let options =
            buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore_unknown);
        buffa::json::with_json_parse_options(&options, || {
            for scalar in [
                json!("private-marker"),
                json!(-123456),
                json!(123456),
                json!(1.25),
                json!(true),
            ] {
                let marker = scalar
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| scalar.to_string());
                let list_errors = [
                    serde_json::from_value::<WithEnum>(json!({"colors": scalar})).unwrap_err(),
                    serde_json::from_value::<ClosedEnumJson>(json!({"tiers": scalar})).unwrap_err(),
                    DynamicMessage::from_json(
                        Arc::clone(pool),
                        index,
                        &json!({"rep": scalar}).to_string(),
                    )
                    .unwrap_err(),
                ];
                for error in list_errors {
                    assert!(!error.to_string().contains(&marker), "{error}");
                    assert!(
                        error
                            .to_string()
                            .starts_with("expected a protobuf list or null"),
                        "{error}"
                    );
                }
                let map_errors = [
                    serde_json::from_value::<OpenEnumContexts>(json!({"labels": scalar}))
                        .unwrap_err(),
                    serde_json::from_value::<ClosedEnumJson>(json!({"byName": scalar}))
                        .unwrap_err(),
                    DynamicMessage::from_json(
                        Arc::clone(pool),
                        index,
                        &json!({"labels": scalar}).to_string(),
                    )
                    .unwrap_err(),
                ];
                for error in map_errors {
                    assert!(!error.to_string().contains(&marker), "{error}");
                    assert!(
                        error
                            .to_string()
                            .starts_with("expected a protobuf map or null"),
                        "{error}"
                    );
                }
            }
        });
    }
    // Container null/empty handling and successful values keep their grammar.
    for input in [Value::Null, json!([])] {
        let decoded: WithEnum = serde_json::from_value(json!({"colors":input})).unwrap();
        assert!(decoded.colors.is_empty());
    }
    for input in [Value::Null, json!({})] {
        let decoded: ClosedEnumJson = serde_json::from_value(json!({"byName":input})).unwrap();
        assert!(decoded.by_name.is_empty());
    }
    let decoded: ClosedEnumJson =
        serde_json::from_value(json!({"tiers":["PRO"], "byName":{"ok":"ENTERPRISE"}})).unwrap();
    assert_eq!(decoded.tiers, [buffa_test::p2json::Tier::PRO]);
    assert_eq!(decoded.by_name["ok"], buffa_test::p2json::Tier::ENTERPRISE);
}

#[test]
fn enum_container_128_bit_errors_omit_input() {
    use buffa::json_helpers::{map_closed_enum, map_enum, repeated_closed_enum, repeated_enum};
    use buffa::EnumValue;
    use buffa_test::json_types::Color;
    use buffa_test::p2json::Tier;
    use serde::de::value::{Error, I128Deserializer, U128Deserializer};

    let unsigned = || U128Deserializer::<Error>::new(u128::MAX);
    let signed = || I128Deserializer::<Error>::new(i128::MIN);
    let list_errors = [
        repeated_enum::deserialize::<Color, Vec<_>, _>(unsigned()).unwrap_err(),
        repeated_enum::deserialize::<Color, Vec<_>, _>(signed()).unwrap_err(),
        repeated_closed_enum::deserialize::<Tier, Vec<_>, _>(unsigned()).unwrap_err(),
        repeated_closed_enum::deserialize::<Tier, Vec<_>, _>(signed()).unwrap_err(),
    ];
    for error in list_errors {
        assert_eq!(error.to_string(), "expected a protobuf list or null");
    }
    let map_errors = [
        map_enum::deserialize::<buffa::Map<String, EnumValue<Color>>, _>(unsigned()).unwrap_err(),
        map_enum::deserialize::<buffa::Map<String, EnumValue<Color>>, _>(signed()).unwrap_err(),
        map_closed_enum::deserialize::<buffa::Map<String, Tier>, _>(unsigned()).unwrap_err(),
        map_closed_enum::deserialize::<buffa::Map<String, Tier>, _>(signed()).unwrap_err(),
    ];
    for error in map_errors {
        assert_eq!(error.to_string(), "expected a protobuf map or null");
    }
}
