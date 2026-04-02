use super::*;
use alloc::vec;
use alloc::vec::Vec;

// ── clamp_size_hint ───────────────────────────────────────────────────

#[test]
fn clamp_size_hint_none_is_zero() {
    assert_eq!(clamp_size_hint(None), 0);
}

#[test]
fn clamp_size_hint_small_passes_through() {
    assert_eq!(clamp_size_hint(Some(10)), 10);
    assert_eq!(clamp_size_hint(Some(MAX_PREALLOC_HINT)), MAX_PREALLOC_HINT);
}

#[test]
fn clamp_size_hint_caps_large_values() {
    assert_eq!(clamp_size_hint(Some(usize::MAX)), MAX_PREALLOC_HINT);
    assert_eq!(
        clamp_size_hint(Some(MAX_PREALLOC_HINT + 1)),
        MAX_PREALLOC_HINT
    );
}

// ── proto_seq ───────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SeqI64Holder(#[serde(with = "proto_seq")] Vec<i64>);

#[test]
fn proto_seq_int64_serializes_as_quoted_strings() {
    let h = SeqI64Holder(vec![1, -2, i64::MAX]);
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"["1","-2","9223372036854775807"]"#);
}

#[test]
fn proto_seq_int64_deserializes_quoted_and_unquoted() {
    // Canonical proto3 JSON (quoted):
    let h: SeqI64Holder = serde_json::from_str(r#"["1","-2","42"]"#).unwrap();
    assert_eq!(h.0, vec![1, -2, 42]);
    // Also accept bare numbers (proto3 JSON parsers must accept both):
    let h: SeqI64Holder = serde_json::from_str(r#"[1,-2,42]"#).unwrap();
    assert_eq!(h.0, vec![1, -2, 42]);
}

#[test]
fn proto_seq_int64_null_is_empty() {
    let h: SeqI64Holder = serde_json::from_str("null").unwrap();
    assert_eq!(h.0, Vec::<i64>::new());
}

#[test]
fn proto_seq_rejects_null_element() {
    // Proto3 JSON spec: null as an ELEMENT of a repeated field is invalid
    // (only the CONTAINER may be null = empty). The singular int32 helper
    // accepts null → 0 for singular fields; proto_seq must not.
    let result: Result<SeqI64Holder, _> = serde_json::from_str(r#"[1,null,2]"#);
    assert!(result.is_err(), "null element must be rejected");
}

#[test]
fn proto_map_rejects_null_value() {
    // Same for map values.
    let result: Result<MapI32I64Holder, _> = serde_json::from_str(r#"{"1":null}"#);
    assert!(result.is_err(), "null map value must be rejected");
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SeqF64Holder(#[serde(with = "proto_seq")] Vec<f64>);

#[test]
fn proto_seq_float_nan_inf_as_strings() {
    let h = SeqF64Holder(vec![1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY]);
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"[1.5,"NaN","Infinity","-Infinity"]"#);
    // Roundtrip:
    let back: SeqF64Holder = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0[0], 1.5);
    assert!(back.0[1].is_nan());
    assert_eq!(back.0[2], f64::INFINITY);
    assert_eq!(back.0[3], f64::NEG_INFINITY);
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SeqBytesHolder(#[serde(with = "proto_seq")] Vec<Vec<u8>>);

#[test]
fn proto_seq_bytes_as_base64() {
    let h = SeqBytesHolder(vec![vec![0xDE, 0xAD], vec![]]);
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"["3q0=",""]"#);
    let back: SeqBytesHolder = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0, vec![vec![0xDE, 0xAD], vec![]]);
}

// ── proto_map ───────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct MapI32I64Holder(#[serde(with = "proto_map")] crate::__private::HashMap<i32, i64>);

#[test]
fn proto_map_int64_value_quoted() {
    let mut m = crate::__private::HashMap::new();
    m.insert(1i32, 42i64);
    m.insert(2i32, i64::MAX);
    let h = MapI32I64Holder(m);
    let json = serde_json::to_string(&h).unwrap();
    // Keys are stringified, values are quoted decimal strings.
    // HashMap order is non-deterministic, so check both possible orders.
    assert!(
        json == r#"{"1":"42","2":"9223372036854775807"}"#
            || json == r#"{"2":"9223372036854775807","1":"42"}"#,
        "got: {json}"
    );
}

#[test]
fn proto_map_int64_value_deserializes_quoted() {
    let h: MapI32I64Holder = serde_json::from_str(r#"{"1":"42","2":"-7"}"#).unwrap();
    assert_eq!(h.0[&1], 42);
    assert_eq!(h.0[&2], -7);
}

#[test]
fn proto_map_null_is_empty() {
    let h: MapI32I64Holder = serde_json::from_str("null").unwrap();
    assert!(h.0.is_empty());
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MapBoolF64Holder(#[serde(with = "proto_map")] crate::__private::HashMap<bool, f64>);

#[test]
fn proto_map_bool_key_float_nan_value() {
    let mut m = crate::__private::HashMap::new();
    m.insert(true, f64::NAN);
    let h = MapBoolF64Holder(m);
    let json = serde_json::to_string(&h).unwrap();
    assert_eq!(json, r#"{"true":"NaN"}"#);
    let back: MapBoolF64Holder = serde_json::from_str(&json).unwrap();
    assert!(back.0[&true].is_nan());
}

// ── proto_seq / proto_map with EnumValue<E> (open enum) ──────────────
//
// Uses the `Color` test enum defined further down in this module.

#[derive(serde::Serialize, serde::Deserialize)]
struct SeqEnumHolder(#[serde(with = "proto_seq")] Vec<crate::EnumValue<Color>>);

#[test]
fn proto_seq_enum_value_roundtrip() {
    let h = SeqEnumHolder(vec![
        crate::EnumValue::Known(Color::Red),
        crate::EnumValue::Known(Color::Blue),
        crate::EnumValue::Unknown(99),
    ]);
    let json = serde_json::to_string(&h).unwrap();
    // Known values serialize as proto name strings; unknown as int.
    assert_eq!(json, r#"["RED","BLUE",99]"#);
    let back: SeqEnumHolder = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0, h.0);
}

#[test]
fn proto_seq_enum_value_accepts_names_and_ints() {
    // Proto3 JSON parsers accept enum names OR integer values.
    let h: SeqEnumHolder = serde_json::from_str(r#"["GREEN",0,42]"#).unwrap();
    assert_eq!(h.0[0], crate::EnumValue::Known(Color::Green));
    assert_eq!(h.0[1], crate::EnumValue::Known(Color::Red)); // 0 → Red
    assert_eq!(h.0[2], crate::EnumValue::Unknown(42));
}

#[test]
fn proto_seq_enum_value_rejects_null_element() {
    let result: Result<SeqEnumHolder, _> = serde_json::from_str(r#"["RED",null,"BLUE"]"#);
    assert!(result.is_err(), "null element must be rejected");
}

#[derive(serde::Serialize, serde::Deserialize)]
struct MapStrEnumHolder(
    #[serde(with = "proto_map")]
    crate::__private::HashMap<alloc::string::String, crate::EnumValue<Color>>,
);

#[test]
fn proto_map_enum_value_roundtrip() {
    let mut m = crate::__private::HashMap::new();
    m.insert("a".into(), crate::EnumValue::Known(Color::Green));
    m.insert("b".into(), crate::EnumValue::Unknown(-1));
    let h = MapStrEnumHolder(m);
    let json = serde_json::to_string(&h).unwrap();
    let back: MapStrEnumHolder = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0, h.0);
}

#[test]
fn proto_map_enum_value_rejects_null_value() {
    let result: Result<MapStrEnumHolder, _> = serde_json::from_str(r#"{"a":null}"#);
    assert!(result.is_err(), "null map value must be rejected");
}

// Helper: round-trip via serde_json
fn json_roundtrip_int64(original: i64) -> i64 {
    let serialized = serde_json::to_string(&SerdeInt64(original)).unwrap();
    let recovered: SerdeInt64 = serde_json::from_str(&serialized).unwrap();
    recovered.0
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeInt64(#[serde(with = "int64")] i64);

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeUint64(#[serde(with = "uint64")] u64);

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeFloat(#[serde(with = "float")] f32);

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeDouble(#[serde(with = "double")] f64);

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeBytes(#[serde(with = "bytes")] Vec<u8>);

// ── int64 ──────────────────────────────────────────────────────────────

#[test]
fn int64_serializes_as_quoted_string() {
    let json = serde_json::to_string(&SerdeInt64(9007199254740993)).unwrap();
    assert_eq!(json, r#""9007199254740993""#);
}

#[test]
fn int64_roundtrip_boundary_values() {
    for v in [i64::MAX, i64::MIN, 0, -1, 9007199254740993] {
        assert_eq!(json_roundtrip_int64(v), v, "roundtrip failed for {v}");
    }
}

#[test]
fn int64_deserializes_from_quoted_string() {
    let val: SerdeInt64 = serde_json::from_str(r#""-9007199254740993""#).unwrap();
    assert_eq!(val.0, -9007199254740993i64);
}

// ── uint64 ──────────────────────────────────────────────────────────────

#[test]
fn uint64_serializes_as_quoted_string() {
    let json = serde_json::to_string(&SerdeUint64(u64::MAX)).unwrap();
    assert_eq!(json, r#""18446744073709551615""#);
}

#[test]
fn uint64_deserializes_from_quoted_string() {
    let val: SerdeUint64 = serde_json::from_str(r#""18446744073709551615""#).unwrap();
    assert_eq!(val.0, u64::MAX);
}

// ── float ───────────────────────────────────────────────────────────────

#[test]
fn float_serializes_special_values_as_strings() {
    #[rustfmt::skip]
    let cases: &[(f32, &str)] = &[
        (f32::NAN,          r#""NaN""#),
        (f32::INFINITY,     r#""Infinity""#),
        (f32::NEG_INFINITY, r#""-Infinity""#),
    ];
    for &(value, expected_json) in cases {
        let json = serde_json::to_string(&SerdeFloat(value)).unwrap();
        assert_eq!(json, expected_json, "value: {value}");
    }
}

#[test]
fn float_serializes_finite_as_number() {
    let json = serde_json::to_string(&SerdeFloat(1.5)).unwrap();
    // serde_json serializes f32 as a JSON number
    assert!(!json.starts_with('"'));
}

#[test]
fn float_deserializes_nan_string() {
    let val: SerdeFloat = serde_json::from_str(r#""NaN""#).unwrap();
    assert!(val.0.is_nan());
}

#[test]
fn float_deserializes_infinity_string() {
    let val: SerdeFloat = serde_json::from_str(r#""Infinity""#).unwrap();
    assert_eq!(val.0, f32::INFINITY);
}

#[test]
fn float_deserializes_neg_infinity_string() {
    let val: SerdeFloat = serde_json::from_str(r#""-Infinity""#).unwrap();
    assert_eq!(val.0, f32::NEG_INFINITY);
}

// ── double ──────────────────────────────────────────────────────────────

#[test]
fn double_serializes_nan_as_string() {
    let json = serde_json::to_string(&SerdeDouble(f64::NAN)).unwrap();
    assert_eq!(json, r#""NaN""#);
}

#[test]
fn double_serializes_infinity_as_string() {
    let json = serde_json::to_string(&SerdeDouble(f64::INFINITY)).unwrap();
    assert_eq!(json, r#""Infinity""#);
}

#[test]
fn double_finite_roundtrip() {
    let val: SerdeDouble = serde_json::from_str("2.5").unwrap();
    assert!((val.0 - 2.5).abs() < 1e-10);
}

// ── bytes ────────────────────────────────────────────────────────────────

#[test]
fn bytes_serializes_as_base64() {
    let json = serde_json::to_string(&SerdeBytes(vec![0xde, 0xad, 0xbe, 0xef])).unwrap();
    assert_eq!(json, r#""3q2+7w==""#);
}

#[test]
fn bytes_deserializes_all_base64_dialects() {
    // All three dialects decode to the same bytes.
    const EXPECTED: &[u8] = &[0xde, 0xad, 0xbe, 0xef];
    #[rustfmt::skip]
    let inputs: &[&str] = &[
        r#""3q2+7w==""#,  // standard, padded
        r#""3q2-7w==""#,  // URL-safe (- for +, _ for /)
        r#""3q2+7w""#,    // standard, unpadded
    ];
    for json in inputs {
        let val: SerdeBytes = serde_json::from_str(json).unwrap();
        assert_eq!(val.0, EXPECTED, "input: {json}");
    }
}

#[test]
fn bytes_roundtrip_empty() {
    let json = serde_json::to_string(&SerdeBytes(vec![])).unwrap();
    let val: SerdeBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(val.0, Vec::<u8>::new());
}

#[test]
fn bytes_rejects_invalid_base64() {
    assert!(serde_json::from_str::<SerdeBytes>(r#""!!! not base64 !!!""#).is_err());
}

#[test]
fn bytes_url_safe_with_trailing_bits() {
    // "-_" is URL-safe base64 for byte 0xFB. The pre-built engines reject
    // this because it has non-zero trailing bits; our lenient engines accept it.
    let val: SerdeBytes = serde_json::from_str(r#""-_""#).unwrap();
    assert_eq!(val.0, vec![0xFB]);
}

#[test]
fn bytes_url_safe_padded_with_trailing_bits() {
    // "-_==" is the padded form of URL-safe base64 for byte 0xFB.
    let val: SerdeBytes = serde_json::from_str(r#""-_==""#).unwrap();
    assert_eq!(val.0, vec![0xFB]);
}

// ── error paths ──────────────────────────────────────────────────────────

#[test]
fn int64_rejects_non_numeric_string() {
    assert!(serde_json::from_str::<SerdeInt64>(r#""abc""#).is_err());
}

#[test]
fn int64_rejects_overflow_string() {
    assert!(serde_json::from_str::<SerdeInt64>(r#""99999999999999999999""#).is_err());
}

#[test]
fn uint64_rejects_negative_string() {
    // "-1" parses as i64 but fails u64::try_from
    assert!(serde_json::from_str::<SerdeUint64>(r#""-1""#).is_err());
}

#[test]
fn float_rejects_unknown_string() {
    assert!(serde_json::from_str::<SerdeFloat>(r#""not-a-float""#).is_err());
}

#[test]
fn double_rejects_unknown_string() {
    assert!(serde_json::from_str::<SerdeDouble>(r#""not-a-double""#).is_err());
}

// ── skip_if ──────────────────────────────────────────────────────────────

#[test]
fn skip_if_numeric_predicates() {
    assert!(skip_if::is_zero_i32(&0));
    assert!(!skip_if::is_zero_i32(&1));
    assert!(skip_if::is_zero_i64(&0));
    assert!(!skip_if::is_zero_i64(&-1));
    assert!(skip_if::is_zero_u32(&0));
    assert!(skip_if::is_zero_u64(&0));
    assert!(skip_if::is_zero_f32(&0.0));
    assert!(!skip_if::is_zero_f32(&1.0));
    assert!(skip_if::is_zero_f64(&0.0));
}

#[test]
fn skip_if_bool_predicates() {
    assert!(skip_if::is_false(&false));
    assert!(!skip_if::is_false(&true));
}

#[test]
fn skip_if_string_and_bytes_predicates() {
    assert!(skip_if::is_empty_str(""));
    assert!(!skip_if::is_empty_str("x"));
    assert!(skip_if::is_empty_bytes(&[]));
    assert!(!skip_if::is_empty_bytes(&[1u8]));
    assert!(skip_if::is_empty_vec::<i32>(&[]));
    assert!(!skip_if::is_empty_vec(&[1]));
}

// ── NullableDeserializeSeed / DefaultDeserializeSeed ──────────────────

#[test]
fn nullable_seed_returns_none_for_null() {
    let seed = NullableDeserializeSeed(DefaultDeserializeSeed::<i32>::new());
    let result: Option<i32> =
        serde::de::DeserializeSeed::deserialize(seed, serde_json::Value::Null).unwrap();
    assert_eq!(result, None);
}

#[test]
fn nullable_seed_returns_some_for_value() {
    let seed = NullableDeserializeSeed(DefaultDeserializeSeed::<i32>::new());
    let result: Option<i32> =
        serde::de::DeserializeSeed::deserialize(seed, serde_json::Value::Number(42.into()))
            .unwrap();
    assert_eq!(result, Some(42));
}

#[test]
fn nullable_seed_with_custom_seed_returns_none_for_null() {
    // Use a custom DeserializeSeed that wraps int64 deserialization
    struct Int64Seed;
    impl<'de> serde::de::DeserializeSeed<'de> for Int64Seed {
        type Value = i64;
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<i64, D::Error> {
            int64::deserialize(d)
        }
    }
    let seed = NullableDeserializeSeed(Int64Seed);
    let result: Option<i64> =
        serde::de::DeserializeSeed::deserialize(seed, serde_json::Value::Null).unwrap();
    assert_eq!(result, None);
}

#[test]
fn nullable_seed_with_custom_seed_returns_some_for_string() {
    struct Int64Seed;
    impl<'de> serde::de::DeserializeSeed<'de> for Int64Seed {
        type Value = i64;
        fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<i64, D::Error> {
            int64::deserialize(d)
        }
    }
    let seed = NullableDeserializeSeed(Int64Seed);
    let result: Option<i64> =
        serde::de::DeserializeSeed::deserialize(seed, serde_json::Value::String("123".to_string()))
            .unwrap();
    assert_eq!(result, Some(123));
}

#[test]
fn default_seed_deserializes_string() {
    let seed = DefaultDeserializeSeed::<alloc::string::String>::new();
    let result: alloc::string::String = serde::de::DeserializeSeed::deserialize(
        seed,
        serde_json::Value::String("hello".to_string()),
    )
    .unwrap();
    assert_eq!(result, "hello");
}

// ── message_field_always_present ──────────────────────────────────────

/// A type whose custom Deserialize handles null by producing a sentinel,
/// mimicking `google.protobuf.Value` behavior.
#[derive(Debug, Default, PartialEq)]
struct NullAccepting(i32);

impl<'de> serde::Deserialize<'de> for NullAccepting {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = NullAccepting;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("integer or null")
            }
            fn visit_unit<E>(self) -> Result<NullAccepting, E> {
                Ok(NullAccepting(-1))
            }
            fn visit_i64<E>(self, v: i64) -> Result<NullAccepting, E> {
                Ok(NullAccepting(v as i32))
            }
            fn visit_u64<E>(self, v: u64) -> Result<NullAccepting, E> {
                Ok(NullAccepting(v as i32))
            }
        }
        d.deserialize_any(V)
    }
}

#[test]
fn message_field_always_present_forwards_null() {
    // Default MessageField<T> deserialization: null → unset
    let mf: crate::MessageField<NullAccepting> = serde_json::from_str("null").unwrap();
    assert!(
        mf.is_unset(),
        "default MessageField should be unset for null"
    );

    // message_field_always_present: null → set (T::deserialize handles null)
    #[derive(Debug, serde::Deserialize)]
    struct Wrapper {
        #[serde(deserialize_with = "message_field_always_present")]
        #[serde(default)]
        val: crate::MessageField<NullAccepting>,
    }
    let w: Wrapper = serde_json::from_str(r#"{"val": null}"#).unwrap();
    assert!(
        !w.val.is_unset(),
        "always_present should set the field for null"
    );
    assert_eq!(
        w.val.as_option(),
        Some(&NullAccepting(-1)),
        "null should produce sentinel value"
    );
}

// ── repeated_enum / map_enum ─────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
enum Color {
    #[default]
    Red,
    Green,
    Blue,
}

// Custom serde impls matching what codegen produces (proto names).
impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(crate::Enumeration::proto_name(self))
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = Color;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a string, integer, or null for Color")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Color, E> {
                <Color as crate::Enumeration>::from_proto_name(v)
                    .ok_or_else(|| serde::de::Error::unknown_variant(v, &[]))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Color, E> {
                <Color as crate::Enumeration>::from_i32(v as i32)
                    .ok_or_else(|| serde::de::Error::custom("unknown"))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Color, E> {
                <Color as crate::Enumeration>::from_i32(v as i32)
                    .ok_or_else(|| serde::de::Error::custom("unknown"))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Color, E> {
                Ok(Color::default())
            }
        }
        d.deserialize_any(V)
    }
}

impl crate::Enumeration for Color {
    fn from_i32(v: i32) -> Option<Self> {
        match v {
            0 => Some(Color::Red),
            1 => Some(Color::Green),
            2 => Some(Color::Blue),
            _ => None,
        }
    }
    fn to_i32(&self) -> i32 {
        *self as i32
    }
    fn proto_name(&self) -> &'static str {
        match self {
            Color::Red => "RED",
            Color::Green => "GREEN",
            Color::Blue => "BLUE",
        }
    }
    fn from_proto_name(name: &str) -> Option<Self> {
        match name {
            "RED" => Some(Color::Red),
            "GREEN" => Some(Color::Green),
            "BLUE" => Some(Color::Blue),
            _ => None,
        }
    }
}

#[test]
fn repeated_enum_deserializes_known_values() {
    let json = r#"["RED","GREEN","BLUE"]"#;
    let result: Vec<crate::EnumValue<Color>> =
        repeated_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], crate::EnumValue::Known(Color::Red));
    assert_eq!(result[1], crate::EnumValue::Known(Color::Green));
    assert_eq!(result[2], crate::EnumValue::Known(Color::Blue));
}

#[test]
fn repeated_enum_rejects_unknown_by_default() {
    let json = r#"["RED","PURPLE","BLUE"]"#;
    let result: Result<Vec<crate::EnumValue<Color>>, _> =
        repeated_enum::deserialize(&mut serde_json::Deserializer::from_str(json));
    assert!(result.is_err());
}

#[test]
fn repeated_enum_skips_unknown_when_lenient() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"["RED","PURPLE","BLUE"]"#;
    let result: Vec<crate::EnumValue<Color>> = with_json_parse_options(&opts, || {
        repeated_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap()
    });
    // PURPLE is skipped, not defaulted.
    assert_eq!(result.len(), 2);
    assert_eq!(result[0], crate::EnumValue::Known(Color::Red));
    assert_eq!(result[1], crate::EnumValue::Known(Color::Blue));
}

#[test]
fn repeated_enum_null_returns_empty_vec() {
    let result: Vec<crate::EnumValue<Color>> =
        repeated_enum::deserialize(&mut serde_json::Deserializer::from_str("null")).unwrap();
    assert!(result.is_empty());
}

#[test]
fn map_enum_deserializes_known_values() {
    let json = r#"{"a":"RED","b":"GREEN"}"#;
    let result: crate::__private::HashMap<alloc::string::String, crate::EnumValue<Color>> =
        map_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result["a"], crate::EnumValue::Known(Color::Red));
    assert_eq!(result["b"], crate::EnumValue::Known(Color::Green));
}

#[test]
fn map_enum_rejects_unknown_by_default() {
    let json = r#"{"a":"RED","b":"PURPLE"}"#;
    let result: Result<
        crate::__private::HashMap<alloc::string::String, crate::EnumValue<Color>>,
        _,
    > = map_enum::deserialize(&mut serde_json::Deserializer::from_str(json));
    assert!(result.is_err());
}

#[test]
fn map_enum_drops_unknown_entries_when_lenient() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"a":"RED","b":"PURPLE","c":"BLUE"}"#;
    let result: crate::__private::HashMap<alloc::string::String, crate::EnumValue<Color>> =
        with_json_parse_options(&opts, || {
            map_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap()
        });
    // "b" entry dropped because PURPLE is unknown.
    assert_eq!(result.len(), 2);
    assert_eq!(result["a"], crate::EnumValue::Known(Color::Red));
    assert_eq!(result["c"], crate::EnumValue::Known(Color::Blue));
}

#[test]
fn map_enum_null_returns_empty_map() {
    let result: crate::__private::HashMap<alloc::string::String, crate::EnumValue<Color>> =
        map_enum::deserialize(&mut serde_json::Deserializer::from_str("null")).unwrap();
    assert!(result.is_empty());
}

// ── Integration tests mirroring conformance suite scenarios ──────────
//
// These use full structs with serde attributes matching how generated
// code uses the helpers, exercising the same code paths as the
// conformance suite's JSON_IGNORE_UNKNOWN_PARSING_TEST tests.

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TestMsg {
    #[serde(
        rename = "optionalNestedEnum",
        with = "proto_enum",
        skip_serializing_if = "skip_if::is_default_enum_value",
        default
    )]
    optional_nested_enum: crate::EnumValue<Color>,

    #[serde(
        rename = "repeatedNestedEnum",
        with = "repeated_enum",
        skip_serializing_if = "skip_if::is_empty_vec",
        default
    )]
    repeated_nested_enum: Vec<crate::EnumValue<Color>>,

    #[serde(
        rename = "mapStringNestedEnum",
        with = "map_enum",
        skip_serializing_if = "std::collections::HashMap::is_empty",
        default
    )]
    map_string_nested_enum:
        std::collections::HashMap<alloc::string::String, crate::EnumValue<Color>>,
}

#[test]
fn conformance_singular_unknown_enum_strict_rejects() {
    let json = r#"{"optionalNestedEnum": "UNKNOWN_VALUE"}"#;
    let result: Result<TestMsg, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "strict mode should reject unknown enum strings"
    );
}

#[test]
fn conformance_singular_unknown_enum_lenient_defaults() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"optionalNestedEnum": "UNKNOWN_VALUE"}"#;
    let msg: TestMsg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    // Unknown string → default (0 = RED). Field is set but at default value.
    assert_eq!(
        msg.optional_nested_enum,
        crate::EnumValue::Known(Color::Red)
    );
    // Re-serializing: default value is skipped, so the field is absent.
    let out = serde_json::to_string(&msg).unwrap();
    assert_eq!(out, "{}");
}

#[test]
fn conformance_repeated_all_unknown_lenient_produces_empty() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"repeatedNestedEnum": ["UNKNOWN_VALUE"]}"#;
    let msg: TestMsg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    assert!(msg.repeated_nested_enum.is_empty());
    let out = serde_json::to_string(&msg).unwrap();
    assert_eq!(out, "{}");
}

#[test]
fn conformance_repeated_mixed_unknown_lenient_skips() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"repeatedNestedEnum": ["RED", "UNKNOWN_VALUE", "RED"]}"#;
    let msg: TestMsg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    // Middle element skipped.
    assert_eq!(msg.repeated_nested_enum.len(), 2);
    assert_eq!(
        msg.repeated_nested_enum[0],
        crate::EnumValue::Known(Color::Red)
    );
    assert_eq!(
        msg.repeated_nested_enum[1],
        crate::EnumValue::Known(Color::Red)
    );
}

#[test]
fn conformance_map_all_unknown_lenient_produces_empty() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"mapStringNestedEnum": {"key": "UNKNOWN_VALUE"}}"#;
    let msg: TestMsg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    assert!(msg.map_string_nested_enum.is_empty());
    let out = serde_json::to_string(&msg).unwrap();
    assert_eq!(out, "{}");
}

#[test]
fn conformance_map_mixed_unknown_lenient_drops_entry() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#"{"mapStringNestedEnum": {"key1": "RED", "key2": "UNKNOWN_VALUE"}}"#;
    let msg: TestMsg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    // key2 dropped.
    assert_eq!(msg.map_string_nested_enum.len(), 1);
    assert_eq!(
        msg.map_string_nested_enum["key1"],
        crate::EnumValue::Known(Color::Red)
    );
}

// ── opt_enum ─────────────────────────────────────────────────────────

#[test]
fn opt_enum_deserializes_known_value() {
    let json = r#""GREEN""#;
    let result: Option<crate::EnumValue<Color>> =
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap();
    assert_eq!(result, Some(crate::EnumValue::Known(Color::Green)));
}

#[test]
fn opt_enum_deserializes_null_as_none() {
    let result: Option<crate::EnumValue<Color>> =
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str("null")).unwrap();
    assert_eq!(result, None);
}

#[test]
fn opt_enum_deserializes_integer_value() {
    let result: Option<crate::EnumValue<Color>> =
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str("2")).unwrap();
    assert_eq!(result, Some(crate::EnumValue::Known(Color::Blue)));
}

#[test]
fn opt_enum_rejects_unknown_by_default() {
    let json = r#""UNKNOWN_VALUE""#;
    let result: Result<Option<crate::EnumValue<Color>>, _> =
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str(json));
    assert!(result.is_err());
}

#[test]
fn opt_enum_returns_none_for_unknown_when_lenient() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#""UNKNOWN_VALUE""#;
    let result: Option<crate::EnumValue<Color>> = with_json_parse_options(&opts, || {
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap()
    });
    assert_eq!(result, None);
}

#[test]
fn opt_enum_known_value_when_lenient() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };
    let json = r#""GREEN""#;
    let result: Option<crate::EnumValue<Color>> = with_json_parse_options(&opts, || {
        opt_enum::deserialize(&mut serde_json::Deserializer::from_str(json)).unwrap()
    });
    assert_eq!(result, Some(crate::EnumValue::Known(Color::Green)));
}

#[test]
fn opt_enum_roundtrip_some() {
    let val = Some(crate::EnumValue::Known(Color::Blue));
    let json = serde_json::to_value(SerdeOptEnum(val)).unwrap();
    assert_eq!(json, serde_json::json!("BLUE"));
}

#[test]
fn opt_enum_roundtrip_none() {
    let val: Option<crate::EnumValue<Color>> = None;
    let json = serde_json::to_value(SerdeOptEnum(val)).unwrap();
    assert_eq!(json, serde_json::Value::Null);
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeOptEnum(#[serde(with = "opt_enum")] Option<crate::EnumValue<Color>>);

#[test]
fn conformance_proto2_optional_unknown_enum_lenient_absent() {
    use crate::json::{with_json_parse_options, JsonParseOptions};
    let opts = JsonParseOptions {
        ignore_unknown_enum_values: true,
        ..Default::default()
    };

    #[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
    struct Proto2Msg {
        #[serde(
            rename = "optionalNestedEnum",
            with = "opt_enum",
            skip_serializing_if = "Option::is_none",
            default
        )]
        optional_nested_enum: Option<crate::EnumValue<Color>>,
    }

    let json = r#"{"optionalNestedEnum": "UNKNOWN_VALUE"}"#;
    let msg: Proto2Msg = with_json_parse_options(&opts, || serde_json::from_str(json).unwrap());
    // Unknown string → None (field absent), not Some(default).
    assert_eq!(msg.optional_nested_enum, None);
    // Re-serializing: field is absent → empty object.
    let out = serde_json::to_string(&msg).unwrap();
    assert_eq!(out, "{}");
}

// ── int32 ────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeInt32(#[serde(with = "int32")] i32);

#[test]
fn int32_serializes_as_number() {
    assert_eq!(serde_json::to_string(&SerdeInt32(42)).unwrap(), "42");
}

#[test]
fn int32_deserialize_table() {
    #[rustfmt::skip]
    let cases: &[(&str, Option<i32>)] = &[
        ("42",               Some(42)),       // bare number
        (r#""42""#,          Some(42)),       // quoted string
        (r#""1e2""#,         Some(100)),      // exponential string notation
        ("null",             Some(0)),        // null → 0
        ("1.0",              Some(1)),        // integer-valued f64 (visit_f64)
        ("-2147483648",      Some(i32::MIN)), // boundary
        ("2147483647",       Some(i32::MAX)), // boundary
        ("9223372036854775807", None),        // i64::MAX overflow → error
        (r#""1.5""#,         None),           // fractional string → error
        ("1.5",              None),           // fractional f64 → error
    ];
    for &(json, expected) in cases {
        let result = serde_json::from_str::<SerdeInt32>(json).map(|v| v.0);
        assert_eq!(result.ok(), expected, "input: {json}");
    }
}

// ── uint32 ───────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeUint32(#[serde(with = "uint32")] u32);

#[test]
fn uint32_serializes_as_number() {
    assert_eq!(serde_json::to_string(&SerdeUint32(42)).unwrap(), "42");
}

#[test]
fn uint32_deserialize_table() {
    #[rustfmt::skip]
    let cases: &[(&str, Option<u32>)] = &[
        ("42",           Some(42)),       // bare number
        (r#""42""#,      Some(42)),       // quoted string
        ("null",         Some(0)),        // null → 0
        ("1.0",          Some(1)),        // integer-valued f64 (visit_f64)
        ("4294967295",   Some(u32::MAX)), // boundary
        ("-1",           None),           // negative → error
        ("18446744073709551615", None),   // u64::MAX overflow → error
        ("1.5",          None),           // fractional f64 → error
    ];
    for &(json, expected) in cases {
        let result = serde_json::from_str::<SerdeUint32>(json).map(|v| v.0);
        assert_eq!(result.ok(), expected, "input: {json}");
    }
}

// ── string_key_map ───────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeStringKeyMap(
    #[serde(with = "string_key_map")] std::collections::HashMap<i32, alloc::string::String>,
);

#[test]
fn string_key_map_serializes_keys_as_strings() {
    let mut m = std::collections::HashMap::new();
    m.insert(42, "hello".into());
    let json = serde_json::to_string(&SerdeStringKeyMap(m)).unwrap();
    assert!(json.contains(r#""42""#));
}

#[test]
fn string_key_map_deserializes_string_keys() {
    let v: SerdeStringKeyMap = serde_json::from_str(r#"{"42":"hello"}"#).unwrap();
    assert_eq!(v.0[&42], "hello");
}

#[test]
fn string_key_map_null_returns_empty() {
    let v: SerdeStringKeyMap = serde_json::from_str("null").unwrap();
    assert!(v.0.is_empty());
}

#[test]
fn string_key_map_rejects_invalid_key() {
    let result = serde_json::from_str::<SerdeStringKeyMap>(r#"{"not_a_number":"v"}"#);
    assert!(result.is_err());
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeBoolKeyMap(#[serde(with = "string_key_map")] std::collections::HashMap<bool, i32>);

#[test]
fn string_key_map_bool_keys_roundtrip() {
    let mut m = std::collections::HashMap::new();
    m.insert(true, 1);
    m.insert(false, 0);
    let json = serde_json::to_string(&SerdeBoolKeyMap(m)).unwrap();
    let recovered: SerdeBoolKeyMap = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0[&true], 1);
    assert_eq!(recovered.0[&false], 0);
}

// ── opt_int64 (representative of all opt_* macro modules) ────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeOptInt64(#[serde(with = "opt_int64")] Option<i64>);

#[test]
fn opt_int64_some_roundtrip() {
    let v = SerdeOptInt64(Some(9007199254740993));
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""9007199254740993""#);
    let recovered: SerdeOptInt64 = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, Some(9007199254740993));
}

#[test]
fn opt_int64_none_roundtrip() {
    let v = SerdeOptInt64(None);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "null");
    let recovered: SerdeOptInt64 = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, None);
}

// ── opt_bytes (manual implementation, not macro) ─────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeOptBytes(#[serde(with = "opt_bytes")] Option<Vec<u8>>);

#[test]
fn opt_bytes_some_roundtrip() {
    let v = SerdeOptBytes(Some(vec![0xde, 0xad]));
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""3q0=""#);
    let recovered: SerdeOptBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, Some(vec![0xde, 0xad]));
}

#[test]
fn opt_bytes_none_roundtrip() {
    let v = SerdeOptBytes(None);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "null");
    let recovered: SerdeOptBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, None);
}

// ── bytes/opt_bytes/ProtoElemJson generic over bytes::Bytes ──────────
//
// Codegen's use_bytes_type() types fields as bytes::Bytes. The helpers
// are generic over From<Vec<u8>> (deserialize) / AsRef<[u8]> (opt_bytes
// serialize) / &[u8] deref-coerce (bytes serialize) so no codegen shim
// is needed. These tests exercise the Bytes instantiation directly.

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeBytesBytes(#[serde(with = "bytes")] ::bytes::Bytes);

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeOptBytesBytes(#[serde(with = "opt_bytes")] Option<::bytes::Bytes>);

#[test]
fn bytes_module_generic_over_bytes_type() {
    // Same wire format as Vec<u8> — deserialize is generic, serialize
    // takes &[u8] (Bytes deref-coerces).
    #[rustfmt::skip]
    let cases: &[(&[u8], &str)] = &[
        (&[0xde, 0xad],             r#""3q0=""#),
        (&[],                       r#""""#),
        (&[0xCA, 0xFE, 0xBA, 0xBE], r#""yv66vg==""#),
    ];
    for &(raw, json) in cases {
        let v = SerdeBytesBytes(::bytes::Bytes::copy_from_slice(raw));
        assert_eq!(serde_json::to_string(&v).unwrap(), json, "ser {raw:?}");
        let back: SerdeBytesBytes = serde_json::from_str(json).unwrap();
        assert_eq!(&back.0[..], raw, "deser {json}");
    }
    // null → empty (same as Vec<u8>).
    let back: SerdeBytesBytes = serde_json::from_str("null").unwrap();
    assert!(back.0.is_empty());
}

#[test]
fn opt_bytes_generic_over_bytes_type() {
    // Some → base64 string (AsRef<[u8]> path).
    let v = SerdeOptBytesBytes(Some(::bytes::Bytes::from_static(&[0x01, 0x02])));
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""AQI=""#);
    let back: SerdeOptBytesBytes = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0.as_deref(), Some(&[0x01, 0x02][..]));

    // None → null and back (no Vec<u8> in sight).
    let v = SerdeOptBytesBytes(None);
    assert_eq!(serde_json::to_string(&v).unwrap(), "null");
    let back: SerdeOptBytesBytes = serde_json::from_str("null").unwrap();
    assert!(back.0.is_none());
}

#[test]
fn proto_elem_json_for_bytes_via_proto_seq() {
    // repeated bytes → proto_seq<T: ProtoElemJson>. The Bytes impl delegates
    // to the bytes module (generic deserialize, deref-coerce serialize).
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Seq(#[serde(with = "proto_seq")] Vec<::bytes::Bytes>);

    let v = Seq(vec![
        ::bytes::Bytes::from_static(b"a"),
        ::bytes::Bytes::from_static(b""),
        ::bytes::Bytes::from_static(&[0xff]),
    ]);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#"["YQ==","","/w=="]"#);
    let back: Seq = serde_json::from_str(&json).unwrap();
    assert_eq!(back.0, v.0);

    // null → empty vec.
    let back: Seq = serde_json::from_str("null").unwrap();
    assert!(back.0.is_empty());
}

#[test]
fn bytes_and_vec_produce_identical_json() {
    // Regression guard: the whole point of the genericization is that
    // Vec<u8> and Bytes produce byte-identical JSON. If this breaks,
    // a use_bytes_type() sender and a default receiver stop interoperating.
    let raw: &[u8] = &[0, 127, 128, 255];
    let as_vec = SerdeBytes(raw.to_vec());
    let as_bytes = SerdeBytesBytes(::bytes::Bytes::copy_from_slice(raw));
    assert_eq!(
        serde_json::to_string(&as_vec).unwrap(),
        serde_json::to_string(&as_bytes).unwrap()
    );
}

// ── proto_bool ───────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeBool(#[serde(with = "proto_bool")] bool);

#[test]
fn proto_bool_serializes_as_bool() {
    assert_eq!(serde_json::to_string(&SerdeBool(true)).unwrap(), "true");
    assert_eq!(serde_json::to_string(&SerdeBool(false)).unwrap(), "false");
}

#[test]
fn proto_bool_deserializes_from_bool() {
    assert!(serde_json::from_str::<SerdeBool>("true").unwrap().0);
    assert!(!serde_json::from_str::<SerdeBool>("false").unwrap().0);
}

#[test]
fn proto_bool_null_is_false() {
    assert!(!serde_json::from_str::<SerdeBool>("null").unwrap().0);
}

// ── proto_string ─────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeString(#[serde(with = "proto_string")] alloc::string::String);

#[test]
fn proto_string_roundtrip() {
    let v = SerdeString("hello".into());
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""hello""#);
    let recovered: SerdeString = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, "hello");
}

#[test]
fn proto_string_null_is_empty() {
    let v: SerdeString = serde_json::from_str("null").unwrap();
    assert_eq!(v.0, "");
}

// ── closed_enum tests ─────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeClosedEnum(#[serde(with = "closed_enum")] Color);

#[test]
fn closed_enum_serializes_as_proto_name() {
    let v = SerdeClosedEnum(Color::Green);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""GREEN""#);
}

#[test]
fn closed_enum_deserialize_table() {
    #[rustfmt::skip]
    let cases: &[(&str, Option<Color>)] = &[
        (r#""GREEN""#, Some(Color::Green)),  // proto name string
        ("1",          Some(Color::Green)),  // integer value
        ("null",       Some(Color::Red)),    // null → default (Red = 0)
        (r#""UNKNOWN_VALUE""#, None),        // unknown string → error
        ("99",                  None),        // unknown integer → error
    ];
    for &(json, expected) in cases {
        let result = serde_json::from_str::<SerdeClosedEnum>(json).map(|v| v.0);
        assert_eq!(result.ok(), expected, "input: {json}");
    }
}

// ── opt_closed_enum tests ─────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeOptClosedEnum(#[serde(with = "opt_closed_enum")] Option<Color>);

#[test]
fn opt_closed_enum_some_roundtrip() {
    let v = SerdeOptClosedEnum(Some(Color::Blue));
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#""BLUE""#);
    let recovered: SerdeOptClosedEnum = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, Some(Color::Blue));
}

#[test]
fn opt_closed_enum_none_roundtrip() {
    let v = SerdeOptClosedEnum(None);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, "null");
    let recovered: SerdeOptClosedEnum = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, None);
}

// ── repeated_closed_enum tests ────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
struct SerdeRepeatedClosedEnum(#[serde(with = "repeated_closed_enum")] Vec<Color>);

#[test]
fn repeated_closed_enum_roundtrip() {
    let v = SerdeRepeatedClosedEnum(vec![Color::Red, Color::Blue]);
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(json, r#"["RED","BLUE"]"#);
    let recovered: SerdeRepeatedClosedEnum = serde_json::from_str(&json).unwrap();
    assert_eq!(recovered.0, vec![Color::Red, Color::Blue]);
}

#[test]
fn repeated_closed_enum_null_is_empty() {
    let v: SerdeRepeatedClosedEnum = serde_json::from_str("null").unwrap();
    assert!(v.0.is_empty());
}

// ── skip_if::is_default_closed_enum tests ─────────────────────────────

#[test]
fn is_default_closed_enum_zero_is_default() {
    assert!(skip_if::is_default_closed_enum(&Color::Red));
}

#[test]
fn is_default_closed_enum_nonzero_is_not_default() {
    assert!(!skip_if::is_default_closed_enum(&Color::Green));
}

// ── bytes_key_map / bytes_key_bytes_val_map ─────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct BytesKeyWrapper {
    #[serde(with = "bytes_key_map")]
    m: crate::__private::HashMap<Vec<u8>, i32>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct BytesKeyBytesValWrapper {
    #[serde(with = "bytes_key_bytes_val_map")]
    m: crate::__private::HashMap<Vec<u8>, Vec<u8>>,
}

#[test]
fn bytes_key_map_roundtrip() {
    let mut m = crate::__private::HashMap::new();
    m.insert(b"key1".to_vec(), 42);
    let w = BytesKeyWrapper { m };
    let json = serde_json::to_string(&w).unwrap();
    // "key1" base64 = "a2V5MQ=="
    assert_eq!(json, r#"{"m":{"a2V5MQ==":42}}"#);
    let back: BytesKeyWrapper = serde_json::from_str(&json).unwrap();
    assert_eq!(back.m.get(b"key1".as_slice()), Some(&42));
}

#[test]
fn bytes_key_bytes_val_map_roundtrip() {
    let mut m = crate::__private::HashMap::new();
    m.insert(b"k".to_vec(), b"v".to_vec());
    let w = BytesKeyBytesValWrapper { m };
    let json = serde_json::to_string(&w).unwrap();
    // "k" = "aw==", "v" = "dg=="
    assert_eq!(json, r#"{"m":{"aw==":"dg=="}}"#);
    let back: BytesKeyBytesValWrapper = serde_json::from_str(&json).unwrap();
    assert_eq!(back.m.get(b"k".as_slice()), Some(&b"v".to_vec()));
}

#[test]
fn bytes_key_maps_deserialize_null() {
    let w: BytesKeyWrapper = serde_json::from_str(r#"{"m":null}"#).unwrap();
    assert!(w.m.is_empty());
    let w: BytesKeyBytesValWrapper = serde_json::from_str(r#"{"m":null}"#).unwrap();
    assert!(w.m.is_empty());
}

// ── float / double deserialize from JSON numbers ────────────────────
// Existing float/double tests above only use string inputs ("NaN",
// "Infinity", etc.) which exercise visit_str. These tables add coverage
// for visit_f64 (JSON number → f32/f64), visit_i64/visit_u64 (JSON
// integer → f32/f64), visit_unit (null → 0.0), and the f32 overflow
// check in visit_f64. Reuses SerdeFloat/SerdeDouble from above.

#[test]
fn float_serialize_table() {
    #[rustfmt::skip]
    let cases: &[(f32, &str)] = &[
        (0.0,               "0.0"),
        (1.5,               "1.5"),
        (-3.25,             "-3.25"),
        (f32::INFINITY,     r#""Infinity""#),
        (f32::NEG_INFINITY, r#""-Infinity""#),
        (f32::NAN,          r#""NaN""#),
    ];
    for &(v, expected) in cases {
        let json = serde_json::to_string(&SerdeFloat(v)).unwrap();
        assert_eq!(json, expected, "f32 {v}");
    }
}

#[test]
fn float_deserialize_table() {
    // Some(v) = succeeds with v; None = error expected.
    #[rustfmt::skip]
    let cases: &[(&str, Option<f32>)] = &[
        // Finite JSON number → f32.
        ("1.5",              Some(1.5)),
        ("-3.25",            Some(-3.25)),
        // Integer JSON number → f32 (proto3-JSON accepts both).
        ("42",               Some(42.0)),
        ("-7",               Some(-7.0)),
        // Special-value string tokens (NaN handled after loop, NaN != NaN).
        (r#""Infinity""#,    Some(f32::INFINITY)),
        (r#""-Infinity""#,   Some(f32::NEG_INFINITY)),
        // Decimal-as-string accepted.
        (r#""1.5""#,         Some(1.5)),
        // null → 0.0 (proto3-JSON default).
        ("null",             Some(0.0)),
        // Finite f64 value that overflows f32 → error.
        ("1e300",            None),
        // Garbage string → error.
        (r#""not-a-number""#,None),
    ];
    for &(json, expected) in cases {
        let result = serde_json::from_str::<SerdeFloat>(json).map(|h| h.0);
        match expected {
            Some(v) => assert_eq!(result.ok(), Some(v), "input: {json}"),
            None => assert!(
                result.is_err(),
                "input: {json}, expected error got {result:?}"
            ),
        }
    }
    // NaN case: equality is reflexively false.
    let h: SerdeFloat = serde_json::from_str(r#""NaN""#).unwrap();
    assert!(h.0.is_nan());
}

#[test]
fn double_serialize_table() {
    #[rustfmt::skip]
    let cases: &[(f64, &str)] = &[
        (0.0,               "0.0"),
        (1.5,               "1.5"),
        (-3.25,             "-3.25"),
        (f64::INFINITY,     r#""Infinity""#),
        (f64::NEG_INFINITY, r#""-Infinity""#),
        (f64::NAN,          r#""NaN""#),
    ];
    for &(v, expected) in cases {
        let json = serde_json::to_string(&SerdeDouble(v)).unwrap();
        assert_eq!(json, expected, "f64 {v}");
    }
}

#[test]
fn double_deserialize_table() {
    #[rustfmt::skip]
    let cases: &[(&str, Option<f64>)] = &[
        ("1.5",              Some(1.5)),
        ("42",               Some(42.0)),
        ("-7",               Some(-7.0)),
        (r#""Infinity""#,    Some(f64::INFINITY)),
        (r#""-Infinity""#,   Some(f64::NEG_INFINITY)),
        (r#""2.5""#,         Some(2.5)),
        ("null",             Some(0.0)),
        // f64 has no overflow check (all JSON numbers fit in f64 domain).
        ("1e308",            Some(1e308)),
        (r#""garbage""#,     None),
    ];
    for &(json, expected) in cases {
        let result = serde_json::from_str::<SerdeDouble>(json).map(|h| h.0);
        match expected {
            Some(v) => assert_eq!(result.ok(), Some(v), "input: {json}"),
            None => assert!(
                result.is_err(),
                "input: {json}, expected error got {result:?}"
            ),
        }
    }
    let h: SerdeDouble = serde_json::from_str(r#""NaN""#).unwrap();
    assert!(h.0.is_nan());
}
