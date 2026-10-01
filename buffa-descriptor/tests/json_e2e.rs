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
        r#"{"fFieldMask":"foo/bar"}"#,
        r#"{"fFieldMask":"3d"}"#,
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
    for path in [
        " ", "foo bar", "foo-bar", "foo/bar", "3d", "", ".foo", "foo.", "foo..bar",
    ] {
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

/// Quoted floats go through the same `json_helpers` modules as generated
/// messages, so out-of-range strings are rejected rather than saturating to
/// infinity, and only the three exact tokens spell a non-finite value.
#[test]
fn json_quoted_float_bounds_match_generated_messages() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    let parsed = DynamicMessage::from_json(
        Arc::clone(&p),
        idx,
        r#"{"fFloat": "-1.5e38", "fDouble": "1.7976931348623157e308"}"#,
    )
    .expect("quoted in-range floats parse");
    assert_eq!(parsed.field_by_number(2), Some(&Value::F32(-1.5e38)));
    assert_eq!(parsed.field_by_number(1), Some(&Value::F64(f64::MAX)));

    let inf = DynamicMessage::from_json(
        Arc::clone(&p),
        idx,
        r#"{"fFloat": "Infinity", "fDouble": "-Infinity"}"#,
    )
    .expect("the exact infinity tokens parse");
    assert_eq!(inf.field_by_number(2), Some(&Value::F32(f32::INFINITY)));
    assert_eq!(inf.field_by_number(1), Some(&Value::F64(f64::NEG_INFINITY)));

    for input in [
        r#"{"fFloat": "3.5e38"}"#,
        r#"{"fFloat": "-3.5e38"}"#,
        r#"{"fFloat": "1e400"}"#,
        r#"{"fDouble": "1e400"}"#,
        r#"{"fDouble": "-1e400"}"#,
        r#"{"fFloat": "inf"}"#,
        r#"{"fDouble": "-INF"}"#,
        r#"{"fDouble": "nan"}"#,
    ] {
        let err = DynamicMessage::from_json(Arc::clone(&p), idx, input)
            .expect_err("out-of-range or non-token quoted float must be rejected");
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

/// The `float` text the two paths must agree on: `(value, canonical form)`.
///
/// The canonical form is the shortest decimal that round-trips as an **`f32`**,
/// which is what the shared `json_helpers::float` serializer produces (asserted
/// case by case below, so a change there shows up here too). Each case is one
/// where the widened `f64` spelling differs: `0.1`/`1.6` gain the `f64` tail
/// digits, `f32::MAX` changes both digits and exponent form, `1e-45` (the
/// smallest subnormal) and the integral value differ in digits an `f32` cannot
/// represent at all.
const FLOAT_CASES: &[(f32, &str)] = &[
    (0.1, "0.1"),
    (1.6, "1.6"),
    (f32::MAX, "3.4028235e+38"),
    (f32::MIN, "-3.4028235e+38"),
    (1e-45, "1e-45"),
    (1079984100.0, "1079984100.0"),
];

/// The typed path's `float` serialization is codegen's
/// `#[serde(with = "::buffa::json_helpers::float")]` on the generated field
/// (`buffa-codegen/src/message.rs`, `buffa-codegen/src/oneof.rs`), which
/// lowers to exactly this call. `buffa-types`' `FloatValue` wrapper
/// (`wrapper_ext.rs`) is the same function on the same serializer.
fn typed_float_text(value: f32) -> String {
    struct TypedFloat(f32);

    impl serde::Serialize for TypedFloat {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            buffa::json_helpers::float::serialize(&self.0, s)
        }
    }

    serde_json::to_string(&TypedFloat(value)).expect("f32 always serializes")
}

fn scalars_with_float(pool: &Arc<DescriptorPool>, value: f32) -> DynamicMessage {
    let idx = pool.message_index("reflect.test.Scalars").unwrap();
    let md = pool.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(pool), idx);
    // `f_float` is a plain proto3 `float` field (field 2), not a wrapper.
    msg.set(md.field(2).unwrap(), Value::F32(value));
    msg
}

/// A `float` field must print the same text whether the message is typed or
/// dynamic: the widened `f64` spelling is a different value's text, and
/// consumers that compare or cache JSON (ETags, canonical-form hashes, golden
/// fixtures) see two documents for one message.
#[test]
fn json_float_text_matches_typed_path() {
    let p = pool();

    for (value, canonical) in FLOAT_CASES {
        assert_eq!(
            typed_float_text(*value),
            *canonical,
            "expectation drift: the typed path no longer prints {canonical:?} for {value}"
        );
        assert_eq!(
            scalars_with_float(&p, *value).to_json().unwrap(),
            format!(r#"{{"fFloat":{canonical}}}"#),
            "reflective text for {value} must be the canonical f32-precision form"
        );
    }
}

/// The reflective decoder has to read what the encoder side writes — its own
/// output and, since the two paths now agree on text, the canonical form the
/// typed path emits. Both directions go through the shared `json_helpers`
/// range rule, so an in-range value near `f32::MAX` is never rejected.
#[test]
fn json_float_round_trips_and_reads_canonical_text() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    for (value, canonical) in FLOAT_CASES {
        let written = scalars_with_float(&p, *value).to_json().unwrap();
        let parsed = DynamicMessage::from_json(Arc::clone(&p), idx, &written)
            .unwrap_or_else(|e| panic!("reflective parse of its own output {written} failed: {e}"));
        assert_eq!(
            parsed.field_by_number(2),
            Some(&Value::F32(*value)),
            "reflective round trip of {value} changed the value"
        );

        let canonical_input = format!(r#"{{"fFloat":{canonical}}}"#);
        let read = DynamicMessage::from_json(Arc::clone(&p), idx, &canonical_input)
            .unwrap_or_else(|e| panic!("canonical text {canonical_input} must parse: {e}"));
        assert_eq!(
            read.field_by_number(2),
            Some(&Value::F32(*value)),
            "canonical text {canonical_input} must read back as {value}"
        );
    }

    // Sharing the helper's range rule must not turn overflow into saturation:
    // past the point where narrowing to `f32` yields infinity it is still an
    // error, exactly as on the typed path.
    for input in [r#"{"fFloat": 3.5e38}"#, r#"{"fFloat": -3.5e38}"#] {
        let err = DynamicMessage::from_json(Arc::clone(&p), idx, input)
            .expect_err("out-of-range unquoted float must be rejected");
        assert!(
            err.to_string().contains("invalid value"),
            "rejection should name the offending value for {input}, got: {err}"
        );
    }
}

/// The proto3 JSON special tokens are unchanged by routing `float` through the
/// shared helper: `NaN` / `Infinity` / `-Infinity` stay strings on the way out
/// and re-parse to the same non-finite value.
#[test]
fn json_float_special_values_keep_the_protojson_tokens() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();

    for (value, text) in [
        (f32::NAN, r#"{"fFloat":"NaN"}"#),
        (f32::INFINITY, r#"{"fFloat":"Infinity"}"#),
        (f32::NEG_INFINITY, r#"{"fFloat":"-Infinity"}"#),
    ] {
        let written = scalars_with_float(&p, value).to_json().unwrap();
        assert_eq!(written, text, "special-value text for {value}");
        let parsed = DynamicMessage::from_json(Arc::clone(&p), idx, text)
            .unwrap_or_else(|e| panic!("{text} must parse: {e}"));
        assert_eq!(parsed.to_json().unwrap(), text, "re-write of {text}");
    }
}

/// `double` keeps its own `f64`-precision text: the fix is `f32`-specific, so
/// the shared `float` helper must not be applied to the `double` branch.
#[test]
fn json_double_text_is_f64_precision() {
    let p = pool();
    let idx = p.message_index("reflect.test.Scalars").unwrap();
    let md = p.message_by_name("reflect.test.Scalars").unwrap();
    let mut msg = DynamicMessage::new(Arc::clone(&p), idx);
    // `f_double` is field 1.
    msg.set(md.field(1).unwrap(), Value::F64(0.1));
    assert_eq!(msg.to_json().unwrap(), r#"{"fDouble":0.1}"#);
}
