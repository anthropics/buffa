//! JSON serialize/deserialize round-trips via generated serde impls.

#[test]
fn test_json_scalar_round_trip() {
    use crate::json_types::Scalar;

    let msg = Scalar {
        int32_val: -42,
        int64_val: i64::MAX,
        uint32_val: u32::MAX,
        uint64_val: u64::MAX,
        float_val: 1.5,
        double_val: std::f64::consts::PI,
        bool_val: true,
        string_val: "hello".into(),
        bytes_val: vec![0xDE, 0xAD],
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: Scalar = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.int32_val, -42);
    assert_eq!(decoded.int64_val, i64::MAX);
    assert_eq!(decoded.uint64_val, u64::MAX);
    assert!(decoded.bool_val);
    assert_eq!(decoded.string_val, "hello");
}

#[test]
fn test_json_enum_round_trip() {
    use crate::json_types::{Color, WithEnum};

    let msg = WithEnum {
        color: buffa::EnumValue::Known(Color::RED),
        colors: vec![
            buffa::EnumValue::Known(Color::GREEN),
            buffa::EnumValue::Known(Color::BLUE),
        ],
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(
        json.contains("\"RED\""),
        "enum should serialize as name: {json}"
    );
    let decoded: WithEnum = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.color, Color::RED);
}

#[test]
fn test_json_oneof_round_trip() {
    use crate::json_types::WithOneof;

    let msg = WithOneof {
        value: Some(crate::json_types::__buffa::oneof::with_oneof::Value::Text(
            "hello".into(),
        )),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: WithOneof = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.value, msg.value);
}

#[test]
fn test_json_oneof_all_scalar_types_round_trip() {
    // Exercises serde_helper_path dispatch for all proto3-JSON-special
    // scalar types in oneof position, and the corresponding runtime
    // json_helpers::{int64, uint32, uint64, float, double, bytes} paths.
    use crate::json_types::__buffa::oneof::with_oneof_types::Kind as KindOneof;
    use crate::json_types::WithOneofTypes;

    #[rustfmt::skip]
    let cases: &[(KindOneof, &str)] = &[
        // int64 → quoted decimal string.
        (KindOneof::I64(i64::MAX),       r#"{"i64":"9223372036854775807"}"#),
        (KindOneof::I64(-1),             r#"{"i64":"-1"}"#),
        // uint32 → unquoted integer.
        (KindOneof::U32(u32::MAX),       r#"{"u32":4294967295}"#),
        // uint64 → quoted decimal string.
        (KindOneof::U64(u64::MAX),       r#"{"u64":"18446744073709551615"}"#),
        // float → JSON number.
        (KindOneof::F32(1.5),            r#"{"f32":1.5}"#),
        // double → JSON number.
        (KindOneof::F64(3.25),           r#"{"f64":3.25}"#),
        // bytes → base64-encoded string.
        (KindOneof::B(vec![0xDE, 0xAD]), r#"{"b":"3q0="}"#),
    ];

    for (kind, expected_json) in cases {
        let msg = WithOneofTypes {
            kind: Some(kind.clone()),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert_eq!(json, *expected_json, "serialize mismatch for {kind:?}");

        let decoded: WithOneofTypes = serde_json::from_str(&json).expect("deserialize");
        match (&decoded.kind, kind) {
            (Some(KindOneof::F32(a)), KindOneof::F32(b)) => assert_eq!(a, b),
            (Some(KindOneof::F64(a)), KindOneof::F64(b)) => assert_eq!(a, b),
            (a, b) => assert_eq!(a, &Some(b.clone()), "deserialize mismatch"),
        }
    }
}

#[test]
fn test_json_oneof_float_special_values() {
    // NaN/Infinity/-Infinity serialize as string tokens per proto3-JSON spec.
    use crate::json_types::__buffa::oneof::with_oneof_types::Kind as KindOneof;
    use crate::json_types::WithOneofTypes;

    #[rustfmt::skip]
    let cases: &[(KindOneof, &str)] = &[
        (KindOneof::F32(f32::NAN),          r#"{"f32":"NaN"}"#),
        (KindOneof::F32(f32::INFINITY),     r#"{"f32":"Infinity"}"#),
        (KindOneof::F32(f32::NEG_INFINITY), r#"{"f32":"-Infinity"}"#),
        (KindOneof::F64(f64::NAN),          r#"{"f64":"NaN"}"#),
        (KindOneof::F64(f64::INFINITY),     r#"{"f64":"Infinity"}"#),
        (KindOneof::F64(f64::NEG_INFINITY), r#"{"f64":"-Infinity"}"#),
    ];

    for (kind, expected_json) in cases {
        let msg = WithOneofTypes {
            kind: Some(kind.clone()),
            ..Default::default()
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert_eq!(json, *expected_json, "serialize mismatch for {kind:?}");

        let decoded: WithOneofTypes = serde_json::from_str(&json).expect("deserialize");
        // Check class equality for NaN (NaN != NaN).
        match (decoded.kind.unwrap(), kind) {
            (KindOneof::F32(a), KindOneof::F32(b)) => {
                assert_eq!(a.is_nan(), b.is_nan(), "{kind:?}");
                if !b.is_nan() {
                    assert_eq!(a, *b, "{kind:?}");
                }
            }
            (KindOneof::F64(a), KindOneof::F64(b)) => {
                assert_eq!(a.is_nan(), b.is_nan(), "{kind:?}");
                if !b.is_nan() {
                    assert_eq!(a, *b, "{kind:?}");
                }
            }
            _ => panic!("variant mismatch"),
        }
    }
}

#[test]
fn test_json_oneof_null_value() {
    // google.protobuf.NullValue in a oneof serializes as JSON null.
    // On deserialize, JSON null populates the NullValue variant (not unset).
    use crate::json_types::__buffa::oneof::with_oneof_types::Kind as KindOneof;
    use crate::json_types::WithOneofTypes;
    use buffa_types::google::protobuf::NullValue;

    let msg = WithOneofTypes {
        kind: Some(KindOneof::Nv(NullValue::NULL_VALUE.into())),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert_eq!(json, r#"{"nv":null}"#);

    let decoded: WithOneofTypes = serde_json::from_str(&json).expect("deserialize");
    assert!(
        matches!(decoded.kind, Some(KindOneof::Nv(_))),
        "expected Nv variant, got {:?}",
        decoded.kind
    );
}

#[test]
fn test_json_oneof_float_deserialize_from_integer() {
    // proto3-JSON: float/double fields accept integer JSON values.
    // Exercises json_helpers::float::visit_i64/visit_u64.
    use crate::json_types::__buffa::oneof::with_oneof_types::Kind as KindOneof;
    use crate::json_types::WithOneofTypes;

    let decoded: WithOneofTypes = serde_json::from_str(r#"{"f32": 42}"#).unwrap();
    assert_eq!(decoded.kind, Some(KindOneof::F32(42.0)));

    let decoded: WithOneofTypes = serde_json::from_str(r#"{"f64": -7}"#).unwrap();
    assert_eq!(decoded.kind, Some(KindOneof::F64(-7.0)));
}

#[test]
fn test_json_oneof_float_deserialize_overflow_rejected() {
    // A finite f64 that overflows f32 range should be rejected.
    // Exercises json_helpers::float::visit_f64 overflow check.
    use crate::json_types::WithOneofTypes;
    let result: Result<WithOneofTypes, _> = serde_json::from_str(r#"{"f32": 1e300}"#);
    assert!(result.is_err(), "1e300 should overflow f32");
}

#[test]
fn test_json_map_round_trip() {
    use crate::json_types::WithMap;

    let msg = WithMap {
        labels: [("env".into(), "prod".into())].into_iter().collect(),
        counts: [("hits".into(), 42)].into_iter().collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: WithMap = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.labels["env"], "prod");
    assert_eq!(decoded.counts["hits"], 42);
}

#[test]
fn test_json_null_for_simple_containers_is_empty() {
    // proto3-JSON: null for repeated/map fields → empty, not error.
    // Regression guard: when map<string,string> and repeated string use
    // derive serde (for performance), null-handling must still work via
    // the deserialize_with = null_as_default fallback.
    use crate::json_types::{MixedOneofAndFields, WithMap};

    // map<string, string> null → empty.
    let decoded: WithMap = serde_json::from_str(r#"{"labels": null}"#).unwrap();
    assert!(decoded.labels.is_empty());

    // map<string, int32> null → empty.
    let decoded: WithMap = serde_json::from_str(r#"{"counts": null}"#).unwrap();
    assert!(decoded.counts.is_empty());

    // repeated string null → empty.
    let decoded: MixedOneofAndFields = serde_json::from_str(r#"{"tags": null}"#).unwrap();
    assert!(decoded.tags.is_empty());
}

#[test]
fn test_json_timestamp_round_trip() {
    use crate::json_types::WithTimestamp;

    let msg = WithTimestamp {
        created: buffa::MessageField::some(buffa_types::google::protobuf::Timestamp {
            seconds: 1_700_000_000,
            nanos: 500_000_000,
            ..Default::default()
        }),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: WithTimestamp = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.created.seconds, 1_700_000_000);
    assert_eq!(decoded.created.nanos, 500_000_000);
}

#[test]
fn test_json_nested_round_trip() {
    use crate::json_types::{Color, Nested, Scalar, WithEnum};

    let msg = Nested {
        scalar: buffa::MessageField::some(Scalar {
            int32_val: 42,
            string_val: "inner".into(),
            ..Default::default()
        }),
        items: vec![Scalar {
            bool_val: true,
            ..Default::default()
        }],
        with_enum: buffa::MessageField::some(WithEnum {
            color: buffa::EnumValue::Known(Color::BLUE),
            ..Default::default()
        }),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    let decoded: Nested = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.scalar.int32_val, 42);
    assert_eq!(decoded.scalar.string_val, "inner");
    assert_eq!(decoded.items.len(), 1);
    assert!(decoded.items[0].bool_val);
    assert_eq!(decoded.with_enum.color, Color::BLUE);
}

// ── Proto2 closed-enum JSON ────────────────────────────────────────
// Proto2 enums are closed (bare E, not EnumValue<E>). JSON codegen must
// route through closed_enum / repeated_closed_enum / map_closed_enum
// helpers rather than the open-enum variants. Coverage-driven: these
// helpers previously had zero call-site tests.

#[test]
fn test_proto2_closed_enum_json_round_trip() {
    use crate::p2json::{ClosedEnumJson, Tier};

    let mut by_name = std::collections::HashMap::new();
    by_name.insert("alice".to_string(), Tier::PRO);
    by_name.insert("bob".to_string(), Tier::FREE);

    let msg = ClosedEnumJson {
        tier: Some(Tier::ENTERPRISE),
        tiers: vec![Tier::FREE, Tier::PRO],
        by_name,
        ..Default::default()
    };

    let json = serde_json::to_string(&msg).expect("serialize");
    // Closed enum serializes as proto name string.
    assert!(json.contains(r#""tier":"ENTERPRISE""#), "json: {json}");
    assert!(json.contains(r#""tiers":["FREE","PRO"]"#), "json: {json}");
    assert!(json.contains(r#""alice":"PRO""#), "json: {json}");

    let decoded: ClosedEnumJson = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.tier, Some(Tier::ENTERPRISE));
    assert_eq!(decoded.tiers, vec![Tier::FREE, Tier::PRO]);
    assert_eq!(decoded.by_name.get("alice"), Some(&Tier::PRO));
    assert_eq!(decoded.by_name.get("bob"), Some(&Tier::FREE));
}

#[test]
fn test_proto2_closed_enum_json_deserialize_integer() {
    // Proto3-JSON allows enum values as integers.
    use crate::p2json::{ClosedEnumJson, Tier};
    let json = r#"{"tier": 1, "tiers": [0, 2], "byName": {"x": 1}}"#;
    let decoded: ClosedEnumJson = serde_json::from_str(json).expect("deserialize");
    assert_eq!(decoded.tier, Some(Tier::PRO));
    assert_eq!(decoded.tiers, vec![Tier::FREE, Tier::ENTERPRISE]);
    assert_eq!(decoded.by_name.get("x"), Some(&Tier::PRO));
}

#[test]
fn test_proto2_closed_enum_json_unknown_value_rejected() {
    // Closed enums reject unknown values by default (unlike open enums).
    use crate::p2json::ClosedEnumJson;
    let json = r#"{"tier": "UNKNOWN_TIER"}"#;
    let result: Result<ClosedEnumJson, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "expected error for unknown closed-enum value"
    );
}

#[test]
fn test_proto2_closed_enum_json_null_defaults() {
    // null for closed-enum fields → None / empty.
    use crate::p2json::ClosedEnumJson;
    let json = r#"{"tier": null, "tiers": null, "byName": null}"#;
    let decoded: ClosedEnumJson = serde_json::from_str(json).expect("deserialize");
    assert_eq!(decoded.tier, None);
    assert!(decoded.tiers.is_empty());
    assert!(decoded.by_name.is_empty());
}

// ── proto3 `optional` (explicit presence) for all scalar types ──────────
// Covers optional_serde_module type dispatch: each type uses a distinct
// opt_* helper (opt_int64, opt_uint64, opt_float, etc.). Previously only
// optional int32/string were tested.

#[test]
fn test_json_optional_scalars_round_trip() {
    use crate::json_types::{Color, OptionalScalars};

    let msg = OptionalScalars {
        o_i32: Some(-42),
        o_u32: Some(u32::MAX),
        o_i64: Some(i64::MIN),
        o_u64: Some(u64::MAX),
        o_f32: Some(1.5),
        o_f64: Some(-3.25),
        o_bytes: Some(vec![0xDE, 0xAD]),
        o_color: Some(buffa::EnumValue::Known(Color::BLUE)),
        ..Default::default()
    };

    let json = serde_json::to_string(&msg).expect("serialize");
    // int64/uint64 quoted; others unquoted; bytes base64; enum as name.
    assert!(
        json.contains(r#""oI64":"-9223372036854775808""#),
        "json: {json}"
    );
    assert!(
        json.contains(r#""oU64":"18446744073709551615""#),
        "json: {json}"
    );
    assert!(json.contains(r#""oU32":4294967295"#), "json: {json}");
    assert!(json.contains(r#""oF32":1.5"#), "json: {json}");
    assert!(json.contains(r#""oBytes":"3q0=""#), "json: {json}");
    assert!(json.contains(r#""oColor":"BLUE""#), "json: {json}");

    let decoded: OptionalScalars = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.o_i32, Some(-42));
    assert_eq!(decoded.o_u32, Some(u32::MAX));
    assert_eq!(decoded.o_i64, Some(i64::MIN));
    assert_eq!(decoded.o_u64, Some(u64::MAX));
    assert_eq!(decoded.o_f32, Some(1.5));
    assert_eq!(decoded.o_f64, Some(-3.25));
    assert_eq!(decoded.o_bytes, Some(vec![0xDE, 0xAD]));
    assert_eq!(decoded.o_color, Some(Color::BLUE.into()));
}

#[test]
fn test_json_optional_scalars_unset_omitted() {
    // All-unset (None) → empty JSON object (skip_serializing_if).
    use crate::json_types::OptionalScalars;
    let msg = OptionalScalars::default();
    let json = serde_json::to_string(&msg).expect("serialize");
    assert_eq!(json, "{}");

    // Empty JSON → all None.
    let decoded: OptionalScalars = serde_json::from_str("{}").expect("deserialize");
    assert_eq!(decoded.o_i64, None);
    assert_eq!(decoded.o_u64, None);
    assert_eq!(decoded.o_f32, None);
    assert_eq!(decoded.o_color, None);
}

#[test]
fn test_json_optional_scalars_null_is_unset() {
    // proto3-JSON: null for an optional field → field not set (None).
    use crate::json_types::OptionalScalars;
    let json = r#"{"oI64": null, "oU64": null, "oF32": null, "oColor": null}"#;
    let decoded: OptionalScalars = serde_json::from_str(json).expect("deserialize");
    assert_eq!(decoded.o_i64, None);
    assert_eq!(decoded.o_u64, None);
    assert_eq!(decoded.o_f32, None);
    assert_eq!(decoded.o_color, None);
}

#[test]
fn test_json_optional_open_enum_integer_deserialize() {
    // opt_enum: open enum accepts integer value, including unknown.
    use crate::json_types::{Color, OptionalScalars};
    let decoded: OptionalScalars = serde_json::from_str(r#"{"oColor": 2}"#).unwrap();
    assert_eq!(decoded.o_color, Some(Color::GREEN.into()));
    // Unknown integer → EnumValue::Unknown.
    let decoded: OptionalScalars = serde_json::from_str(r#"{"oColor": 99}"#).unwrap();
    assert_eq!(decoded.o_color, Some(buffa::EnumValue::Unknown(99)));
}

// ── Mixed oneof + non-oneof fields (custom Deserialize) ─────────────────
// When a message has oneofs, a hand-generated Deserialize impl is emitted
// that must handle ALL fields, not just the oneof. Previously no test message
// had both oneof and non-oneof fields.

#[test]
fn test_json_mixed_oneof_and_fields_round_trip() {
    use crate::json_types::__buffa::oneof::mixed_oneof_and_fields::Choice as ChoiceOneof;
    use crate::json_types::{MixedOneofAndFields, Scalar};

    let msg = MixedOneofAndFields {
        id: 12345,
        tags: vec!["a".into(), "b".into()],
        counts: [("x".into(), 10)].into_iter().collect(),
        scalar: buffa::MessageField::some(Scalar {
            int32_val: 42,
            ..Default::default()
        }),
        dynamic: buffa::MessageField::some(buffa_types::google::protobuf::Value::from(3.14)),
        snake_case_field: 7,
        choice: Some(ChoiceOneof::Text("hello".into())),
        ..Default::default()
    };

    let json = serde_json::to_string(&msg).expect("serialize");
    // id is int64 → quoted string.
    assert!(json.contains(r#""id":"12345""#), "json: {json}");
    // json_name override used for serialization.
    assert!(json.contains(r#""snakeCaseField":7"#), "json: {json}");
    assert!(json.contains(r#""text":"hello""#), "json: {json}");
    assert!(json.contains(r#""dynamic":3.14"#), "json: {json}");

    let decoded: MixedOneofAndFields = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(decoded.id, 12345);
    assert_eq!(decoded.tags, vec!["a", "b"]);
    assert_eq!(decoded.counts.get("x"), Some(&10));
    assert_eq!(decoded.scalar.int32_val, 42);
    assert_eq!(decoded.dynamic.as_number(), Some(3.14));
    assert_eq!(decoded.snake_case_field, 7);
    assert_eq!(decoded.choice, Some(ChoiceOneof::Text("hello".into())));
}

#[test]
fn test_json_mixed_oneof_deserialize_proto_name_alias() {
    // Non-oneof fields in a custom-Deserialize message accept both
    // json_name and proto_name (covers the `json_name | proto_name` match arm).
    use crate::json_types::MixedOneofAndFields;
    // snake_case_field has json_name="snakeCaseField"; both should work.
    let decoded: MixedOneofAndFields = serde_json::from_str(r#"{"snake_case_field": 99}"#).unwrap();
    assert_eq!(decoded.snake_case_field, 99);
    let decoded: MixedOneofAndFields = serde_json::from_str(r#"{"snakeCaseField": 88}"#).unwrap();
    assert_eq!(decoded.snake_case_field, 88);
}

#[test]
fn test_json_mixed_value_field_null_forwarding() {
    // google.protobuf.Value: JSON null is a VALID value (NullValue),
    // not "field absent". The custom Deserialize must forward null to
    // Value's own Deserialize rather than skipping the field.
    use crate::json_types::MixedOneofAndFields;
    use buffa_types::google::protobuf::__buffa::oneof::value::Kind as KindOneof;
    use buffa_types::google::protobuf::NullValue;

    let decoded: MixedOneofAndFields = serde_json::from_str(r#"{"dynamic": null}"#).unwrap();
    assert!(decoded.dynamic.is_set(), "null should set the Value field");
    assert!(
        matches!(decoded.dynamic.kind, Some(KindOneof::NullValue(_))),
        "expected NullValue, got {:?}",
        decoded.dynamic.kind
    );

    // vs. absent field → MessageField unset.
    let decoded: MixedOneofAndFields = serde_json::from_str("{}").unwrap();
    assert!(!decoded.dynamic.is_set());
    let _ = NullValue::NULL_VALUE; // silence unused import if not needed
}

// ── View JSON round-trip (issue #83) ────────────────────────────────────────
// Each test encodes an owned message → decodes as a view → serializes both to
// JSON → asserts they produce identical output.
//
// Uses `crate::view_json` (view_json.proto, built with generate_views=true +
// generate_json=true) to avoid WKT-view Serialize gaps — WKT view Serialize
// is a separate follow-up.

#[test]
fn test_view_json_scalar_matches_owned() {
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;
    use buffa::MessageView;

    let owned = Scalars {
        i32: -42,
        i64: 9007199254740993, // > 2^53 — must be quoted string
        u32: u32::MAX,
        u64: u64::MAX,
        f32: 1.5,
        f64: std::f64::consts::PI,
        b: true,
        s: "hello world".into(),
        by: vec![0xDE, 0xAD, 0xBE, 0xEF],
        ..Default::default()
    };
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = ScalarsView::decode_view(&bytes).expect("decode_view");

    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(json_view, json_owned, "view JSON must match owned JSON");

    // int64 >2^53 must be a quoted string, not a raw number.
    assert!(
        json_view.contains(r#""i64":"9007199254740993""#),
        "int64 >2^53 must be quoted: {json_view}"
    );
    // bytes must be base64-encoded.
    assert!(
        json_view.contains(r#""by":"3q2+7w==""#),
        "bytes must be base64: {json_view}"
    );
}

#[test]
fn test_view_json_double_special_values() {
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;
    use buffa::MessageView;

    let cases: &[(f64, &str)] = &[
        (f64::NAN, r#""f64":"NaN""#),
        (f64::INFINITY, r#""f64":"Infinity""#),
        (f64::NEG_INFINITY, r#""f64":"-Infinity""#),
    ];
    for (val, expected_fragment) in cases {
        let owned = Scalars {
            f64: *val,
            ..Default::default()
        };
        let bytes = buffa::Message::encode_to_vec(&owned);
        let view = ScalarsView::decode_view(&bytes).expect("decode_view");

        let json_view = serde_json::to_string(&view).expect("serialize view");
        assert!(
            json_view.contains(expected_fragment),
            "double {val:?} must serialize as {expected_fragment}: {json_view}"
        );
        let json_owned = serde_json::to_string(&owned).expect("serialize owned");
        assert_eq!(json_view, json_owned, "view must match owned for {val:?}");
    }
}

#[test]
fn test_view_json_proto3_defaults_omitted() {
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;
    use buffa::MessageView;

    let owned = Scalars::default();
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = ScalarsView::decode_view(&bytes).expect("decode_view");

    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(
        json_view, "{}",
        "default view must serialize as empty object"
    );
    assert_eq!(json_view, json_owned);
}

#[test]
fn test_view_json_enum_matches_owned() {
    use crate::view_json::__buffa::view::WithEnumView;
    use crate::view_json::{Color, WithEnum};
    use buffa::MessageView;

    let owned = WithEnum {
        color: buffa::EnumValue::Known(Color::RED),
        colors: vec![
            buffa::EnumValue::Known(Color::GREEN),
            buffa::EnumValue::Known(Color::BLUE),
        ],
        ..Default::default()
    };
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = WithEnumView::decode_view(&bytes).expect("decode_view");

    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(json_view, json_owned, "view JSON must match owned JSON");
    assert!(
        json_view.contains(r#""color":"RED""#),
        "enum as name: {json_view}"
    );
    assert!(
        json_view.contains(r#""GREEN""#),
        "repeated enum: {json_view}"
    );
}

#[test]
fn test_view_json_oneof_matches_owned() {
    use crate::view_json::__buffa::oneof::with_oneof::Value as ValueOneof;
    use crate::view_json::__buffa::view::WithOneofView;
    use crate::view_json::{Color, WithOneof};
    use buffa::MessageView;

    let cases: &[ValueOneof] = &[
        ValueOneof::Text("hello".into()),
        ValueOneof::Number(i64::MAX),
        ValueOneof::Data(vec![0xAB, 0xCD]),
        ValueOneof::Color(buffa::EnumValue::Known(Color::GREEN)),
    ];
    for variant in cases {
        let owned = WithOneof {
            value: Some(variant.clone()),
            ..Default::default()
        };
        let bytes = buffa::Message::encode_to_vec(&owned);
        let view = WithOneofView::decode_view(&bytes).expect("decode_view");

        let json_owned = serde_json::to_string(&owned).expect("serialize owned");
        let json_view = serde_json::to_string(&view).expect("serialize view");
        assert_eq!(
            json_view, json_owned,
            "view must match owned for {variant:?}"
        );
    }

    // Unset oneof → empty object.
    let owned = WithOneof::default();
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = WithOneofView::decode_view(&bytes).expect("decode_view");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(json_view, "{}");
}

#[test]
fn test_view_json_map_matches_owned() {
    use crate::view_json::__buffa::view::WithMapsView;
    use crate::view_json::{Color, WithMaps};
    use buffa::MessageView;

    let owned = WithMaps {
        labels: [
            ("env".into(), "prod".into()),
            ("region".into(), "us-east".into()),
        ]
        .into_iter()
        .collect(),
        by_id: [(1, "one".into()), (2, "two".into())].into_iter().collect(),
        counts: [("hits".into(), 9007199254740993i64)].into_iter().collect(),
        by_color: [("bg".into(), buffa::EnumValue::Known(Color::BLUE))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = WithMapsView::decode_view(&bytes).expect("decode_view");

    // Parse as serde_json::Value so key ordering doesn't matter.
    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    let v_owned: serde_json::Value = serde_json::from_str(&json_owned).unwrap();
    let v_view: serde_json::Value = serde_json::from_str(&json_view).unwrap();
    assert_eq!(v_view, v_owned, "view map JSON must match owned map JSON");

    // int64 map value must be a quoted string.
    assert!(
        json_view.contains(r#""9007199254740993""#),
        "int64 map value must be quoted: {json_view}"
    );
}

#[test]
fn test_view_json_nested_matches_owned() {
    use crate::view_json::__buffa::view::OuterView;
    use crate::view_json::{Inner, Outer};
    use buffa::MessageView;

    let owned = Outer {
        inner: buffa::MessageField::some(Inner {
            x: 7,
            name: "root".into(),
            ..Default::default()
        }),
        items: vec![
            Inner {
                x: 1,
                name: "a".into(),
                ..Default::default()
            },
            Inner {
                x: 2,
                name: "b".into(),
                ..Default::default()
            },
        ],
        id: i64::MAX,
        ..Default::default()
    };
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = OuterView::decode_view(&bytes).expect("decode_view");

    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(
        json_view, json_owned,
        "nested view JSON must match owned JSON"
    );
}

#[test]
fn test_view_json_owned_view_blanket_impl() {
    // OwnedView<V> must implement Serialize via the blanket impl so that
    // `serde_json::to_string(&owned_view)` works without an explicit deref.
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;
    use buffa::view::OwnedView;

    let owned = Scalars {
        i32: 99,
        s: "owned_view".into(),
        by: vec![0x01, 0x02],
        ..Default::default()
    };
    let bytes = bytes::Bytes::from(buffa::Message::encode_to_vec(&owned));
    let owned_view = OwnedView::<ScalarsView<'static>>::decode(bytes).expect("decode OwnedView");

    let json_owned_view = serde_json::to_string(&owned_view).expect("serialize OwnedView");
    let json_view = serde_json::to_string(&*owned_view).expect("serialize &view");
    let json_owned = serde_json::to_string(&owned).expect("serialize owned");

    assert_eq!(
        json_owned_view, json_owned,
        "OwnedView blanket impl must match owned"
    );
    assert_eq!(
        json_owned_view, json_view,
        "OwnedView blanket impl must match &*view"
    );
}

#[test]
fn test_view_json_null_value_oneof_serializes_as_null() {
    // NullValue oneof variants must serialize as JSON `null`, not "NULL_VALUE".
    // Regression guard for the view path — the owned path is covered by
    // test_json_oneof_null_value.
    use crate::view_json::__buffa::oneof::with_oneof::Value as ValueOneof;
    use crate::view_json::__buffa::view::WithOneofView;
    use crate::view_json::WithOneof;
    use buffa::MessageView;
    use buffa_types::google::protobuf::NullValue;

    let owned = WithOneof {
        value: Some(ValueOneof::NullVal(NullValue::NULL_VALUE.into())),
        ..Default::default()
    };
    let bytes = buffa::Message::encode_to_vec(&owned);
    let view = WithOneofView::decode_view(&bytes).expect("decode_view");

    let json_view = serde_json::to_string(&view).expect("serialize view");
    assert_eq!(
        json_view, r#"{"nullVal":null}"#,
        "NullValue oneof variant must serialize as JSON null: {json_view}"
    );

    // Also verify view matches owned.
    let json_owned = serde_json::to_string(&owned).expect("serialize owned");
    assert_eq!(json_view, json_owned, "view must match owned for NullValue");
}
