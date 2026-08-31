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
fn json_field_mask_round_trip_and_rejects_invalid_paths() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    let wildcard_input = r#"{"fFieldMask":"*"}"#;
    let wildcard = DynamicMessage::from_json(Arc::clone(&p), idx, wildcard_input).unwrap();
    assert_eq!(wildcard.to_json().unwrap(), wildcard_input);

    let valid_input = r#"{"fFieldMask":"fooBar,foo.barBaz,Foo,foo.Bar"}"#;
    let valid = DynamicMessage::from_json(Arc::clone(&p), idx, valid_input).unwrap();
    assert_eq!(valid.to_json().unwrap(), valid_input);

    for input in [
        r#"{"fFieldMask":" "}"#,
        r#"{"fFieldMask":"foo, barBaz"}"#,
        r#"{"fFieldMask":"foo,bar-baz"}"#,
        r#"{"fFieldMask":"foo,"}"#,
        r#"{"fFieldMask":".foo"}"#,
        r#"{"fFieldMask":"foo."}"#,
        r#"{"fFieldMask":"foo..bar"}"#,
    ] {
        assert!(
            DynamicMessage::from_json(Arc::clone(&p), idx, input).is_err(),
            "JSON {input} must be rejected"
        );
    }

    let field_mask_idx = p.message_index("google.protobuf.FieldMask").unwrap();
    let field_mask_md = p.message_by_name("google.protobuf.FieldMask").unwrap();
    let scalars_md = p.message_by_name("reflect.test.Scalars").unwrap();
    for path in [" ", "foo bar", "foo-bar", "", ".foo", "foo.", "foo..bar"] {
        let mut mask = DynamicMessage::new(Arc::clone(&p), field_mask_idx);
        mask.set(
            field_mask_md.field(1).unwrap(),
            Value::List(vec![Value::String(path.into())]),
        );
        let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
        msg.set(scalars_md.field(17).unwrap(), Value::Message(mask));
        assert!(msg.to_json().is_err(), "path {path:?} must be rejected");
    }
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
fn json_integer_parsing_matches_generated_messages() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    let parsed = DynamicMessage::from_json(
        Arc::clone(&p),
        idx,
        r#"{
            "fInt32": "1.5e3",
            "fInt64": "9007199254740993.0",
            "fUint32": "1200e-2",
            "fUint64": "18446744073709551615.0"
        }"#,
    )
    .expect("exact quoted decimal and exponent forms must parse");
    assert_eq!(parsed.field_by_number(3), Some(&Value::I32(1_500)));
    assert_eq!(
        parsed.field_by_number(4),
        Some(&Value::I64(9_007_199_254_740_993))
    );
    assert_eq!(parsed.field_by_number(5), Some(&Value::U32(12)));
    assert_eq!(parsed.field_by_number(6), Some(&Value::U64(u64::MAX)));

    let safe_unquoted =
        DynamicMessage::from_json(Arc::clone(&p), idx, r#"{"fInt64": 4503599627370495.0}"#)
            .expect("unquoted integer floats below 2^52 must still parse");
    assert_eq!(
        safe_unquoted.field_by_number(4),
        Some(&Value::I64(4_503_599_627_370_495))
    );

    // serde_json can round integer-valued float tokens at this magnitude
    // before the visitor sees them. Generated decoders reject them, and the
    // reflective path must do the same instead of accepting an adjacent value.
    for input in [
        r#"{"fInt64": 4503599627370496.0}"#,
        r#"{"fInt64": 9007199254740991.0}"#,
        r#"{"fInt64": -9007199254740991.0}"#,
        r#"{"fUint64": 9007199254740991.0}"#,
    ] {
        let err = DynamicMessage::from_json(Arc::clone(&p), idx, input)
            .expect_err("unsafe unquoted integer float must be rejected");
        assert!(
            err.to_string().contains("invalid value"),
            "rejection should name the offending value, got: {err}"
        );
    }
}

/// The quoted-string path covers the full range exactly and rejects the
/// same inputs the generated decoders reject: overflow, negative values for
/// unsigned fields, and non-integral forms.
#[test]
fn json_quoted_integer_bounds_match_generated_messages() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    let parsed = DynamicMessage::from_json(
        Arc::clone(&p),
        idx,
        &format!(
            r#"{{"fInt32": "{}", "fInt64": "{}", "fUint32": "{}", "fUint64": "{}"}}"#,
            i32::MIN,
            i64::MIN,
            u32::MAX,
            u64::MAX
        ),
    )
    .expect("quoted extremes parse exactly");
    assert_eq!(parsed.field_by_number(3), Some(&Value::I32(i32::MIN)));
    assert_eq!(parsed.field_by_number(4), Some(&Value::I64(i64::MIN)));
    assert_eq!(parsed.field_by_number(5), Some(&Value::U32(u32::MAX)));
    assert_eq!(parsed.field_by_number(6), Some(&Value::U64(u64::MAX)));

    let max = DynamicMessage::from_json(
        Arc::clone(&p),
        idx,
        &format!(r#"{{"fInt32": "{}", "fInt64": "{}"}}"#, i32::MAX, i64::MAX),
    )
    .expect("quoted signed maxima parse exactly");
    assert_eq!(max.field_by_number(3), Some(&Value::I32(i32::MAX)));
    assert_eq!(max.field_by_number(4), Some(&Value::I64(i64::MAX)));

    for input in [
        r#"{"fInt32": "2147483648"}"#,
        r#"{"fInt64": "9223372036854775808"}"#,
        r#"{"fUint32": "4294967296"}"#,
        r#"{"fUint64": "18446744073709551616"}"#,
        r#"{"fUint64": "-1"}"#,
        r#"{"fUint32": "-0.5e1"}"#,
        r#"{"fInt64": "1.5"}"#,
        r#"{"fInt32": "1e-1"}"#,
        r#"{"fInt64": "abc"}"#,
    ] {
        let err = DynamicMessage::from_json(Arc::clone(&p), idx, input)
            .expect_err("out-of-range or non-integral quoted integer must be rejected");
        assert!(
            err.to_string().contains("invalid value"),
            "rejection should name the offending value for {input}, got: {err}"
        );
    }
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
