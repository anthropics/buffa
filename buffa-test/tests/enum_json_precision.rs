//! Privacy regressions for serde_json's additive arbitrary-precision feature.
//!
//! These tests require `--features serde_json/arbitrary_precision -- --ignored`.
//! They check rejection privacy, not general arbitrary-precision JSON support.

use buffa::json_helpers::{closed_enum, proto_enum};
use buffa::EnumValue;
use buffa_descriptor::reflect::DynamicMessageSeed;
use buffa_descriptor::DescriptorPool;
use buffa_test::json_types::Color;
use buffa_test::p2json::Tier;
use serde::de::DeserializeSeed;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

const WIDE_INTEGERS: [&str; 2] = [
    "314159265358979323846264338327950288419",
    "-141421356237309504880168872420969807856",
];
const OUT_OF_RANGE: &str = "enum number out of i32 range";

fn precise_value(text: &str) -> Value {
    let value: Value = serde_json::from_str(text).unwrap();
    let number = value.as_number().unwrap();
    // A float would exercise visit_f64 instead of the vulnerable 128-bit path.
    assert!(!number.is_f64(), "enable serde_json/arbitrary_precision");
    assert_eq!(number.to_string(), text);
    value
}

#[test]
#[ignore = "requires serde_json/arbitrary_precision; exercised separately in CI"]
fn enum_json_precision_value_errors_omit_input() {
    for text in WIDE_INTEGERS {
        let value = precise_value(text);
        for ignore_unknown in [false, true] {
            let options =
                buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(ignore_unknown);
            buffa::json::with_json_parse_options(&options, || {
                let errors = [
                    serde_json::from_value::<Color>(value.clone()).unwrap_err(),
                    serde_json::from_value::<Tier>(value.clone()).unwrap_err(),
                    serde_json::from_value::<EnumValue<Color>>(value.clone()).unwrap_err(),
                    Color::deserialize(&value).unwrap_err(),
                    proto_enum::deserialize::<Color, _>(value.clone()).unwrap_err(),
                    closed_enum::deserialize::<Tier, _>(value.clone()).unwrap_err(),
                    closed_enum::deserialize_strict::<Tier, _>(value.clone()).unwrap_err(),
                ];
                for error in errors {
                    assert_eq!(error.to_string(), OUT_OF_RANGE);
                }
                // Text takes a different Serde path from an owned/borrowed Value.
                // It must also reject without including the numeric token.
                let error = serde_json::from_str::<Color>(text).unwrap_err();
                assert!(!error.to_string().contains(text));
            });
        }
    }
}

#[test]
#[ignore = "requires serde_json/arbitrary_precision; exercised separately in CI"]
fn reflective_enum_json_precision_value_errors_omit_input() {
    let pool = Arc::new(
        DescriptorPool::decode(include_bytes!(
            "../../buffa-descriptor/tests/protos/reflect_test.fds"
        ))
        .unwrap(),
    );
    let index = pool.message_index("reflect.test.Containers").unwrap();
    for text in WIDE_INTEGERS {
        let value = precise_value(text);
        for input in [json!({"color": value}), json!({"colors": [value]})] {
            let error = DynamicMessageSeed::new(Arc::clone(&pool), index)
                .deserialize(input)
                .unwrap_err();
            assert_eq!(error.to_string(), OUT_OF_RANGE);
        }
    }
}

#[test]
#[ignore = "requires serde_json/arbitrary_precision; exercised separately in CI"]
fn enum_json_precision_container_errors_omit_input() {
    use buffa_descriptor::reflect::ReflectMessage;
    use buffa_test::open_enums::OpenEnumContexts;
    use buffa_test::p2json::ClosedEnumJson;

    let reflected = OpenEnumContexts::default();
    let pool = reflected.pool();
    let index = pool
        .message_index("test.openenums.OpenEnumContexts")
        .unwrap();
    for text in WIDE_INTEGERS {
        let value = precise_value(text);
        let errors = [
            (
                serde_json::from_value::<ClosedEnumJson>(json!({"tiers":value})).unwrap_err(),
                "expected a protobuf list or null",
            ),
            (
                serde_json::from_value::<OpenEnumContexts>(json!({"labels":value})).unwrap_err(),
                "expected a protobuf map or null",
            ),
            (
                DynamicMessageSeed::new(Arc::clone(pool), index)
                    .deserialize(json!({"rep":value}))
                    .unwrap_err(),
                "expected a protobuf list or null",
            ),
            (
                DynamicMessageSeed::new(Arc::clone(pool), index)
                    .deserialize(json!({"labels":value}))
                    .unwrap_err(),
                "expected a protobuf map or null",
            ),
        ];
        for (error, expected) in errors {
            assert_eq!(error.to_string(), expected);
        }
    }
}
