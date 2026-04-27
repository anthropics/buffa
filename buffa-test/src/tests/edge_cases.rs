//! Edge cases not covered by conformance: reserved, large field numbers,
//! packed override, json_name, non-string map keys, sub-message merge,
//! negative enums, repeated/map proto3-JSON encoding.

use crate::edge::*;
use crate::proto2::{Proto2PackedOverride, Signed, WithSignedEnum};
use buffa::Message;

fn round_trip<T: Message + Default + PartialEq + core::fmt::Debug>(msg: &T) -> T {
    let bytes = msg.encode_to_vec();
    T::decode(&mut bytes.as_slice()).expect("decode")
}

// ── #11 reserved ───────────────────────────────────────────────────────

#[test]
fn test_reserved_compiles_and_roundtrips() {
    // If this compiles, reserved numbers/names were correctly ignored.
    let msg = WithReserved {
        id: 1,
        name: "x".into(),
        active: true,
        after_gap: 99,
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

// ── #6 large/sparse field numbers ──────────────────────────────────────

#[test]
fn test_large_field_numbers_roundtrip() {
    let msg = LargeFieldNumbers {
        small: 1,
        medium: 2,
        large: 3,
        max_field: "max".into(),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn test_max_field_number_tag_is_5_bytes() {
    // Field 536870911 (2^29 - 1) with wire type 2 (length-delimited):
    // tag = (536870911 << 3) | 2 = 4294967290 = 0xFFFF_FFFA
    // As a varint: 0xFA 0xFF 0xFF 0xFF 0x0F (5 bytes).
    let msg = LargeFieldNumbers {
        max_field: "x".into(),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    assert_eq!(&bytes[..5], &[0xFA, 0xFF, 0xFF, 0xFF, 0x0F]);
}

// ── #3 packed override ────────────────────────────────────────────────

#[test]
fn test_proto3_packed_override_wire_format() {
    // default_packed: proto3 default is packed (wire type 2).
    // explicit_unpacked: [packed=false] forces wire type 0 per element.
    let msg = PackedOverride {
        default_packed: vec![1, 2, 3],
        explicit_unpacked: vec![4, 5, 6],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Field 1 packed: tag 0x0A (field 1, wire type 2), then len + payload.
    assert_eq!(bytes[0], 0x0A, "default_packed must use wire type 2");
    // After the packed run: 3 separate tags 0x10 (field 2, wire type 0).
    let unpacked_tags: Vec<_> = bytes.iter().filter(|&&b| b == 0x10).collect();
    assert_eq!(unpacked_tags.len(), 3, "explicit_unpacked must emit 3 tags");
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn test_proto2_packed_override_wire_format() {
    let msg = Proto2PackedOverride {
        default_unpacked: vec![1, 2, 3],
        explicit_packed: vec![4, 5, 6],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Field 1 unpacked: 3 separate tags 0x08 (field 1, wire type 0).
    let unpacked_tags: Vec<_> = bytes.iter().filter(|&&b| b == 0x08).collect();
    assert_eq!(unpacked_tags.len(), 3, "proto2 default must emit 3 tags");
    // Field 2 packed: tag 0x12 (field 2, wire type 2).
    assert!(
        bytes.contains(&0x12),
        "explicit_packed must use wire type 2"
    );
    assert_eq!(round_trip(&msg), msg);
}

// ── #4 explicit json_name ─────────────────────────────────────────────

#[test]
fn test_custom_json_name_serialize() {
    let msg = CustomJsonName {
        original_name: "hello".into(),
        another_field: 42,
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""customName":"hello""#), "got: {json}");
    assert!(json.contains(r#""renamedInt":42"#), "got: {json}");
    // Must NOT use the original proto name.
    assert!(!json.contains("original_name"), "got: {json}");
}

#[test]
fn test_custom_json_name_deserialize_both() {
    // Proto3 JSON spec: parsers must accept both the custom json_name
    // and the original proto field name.
    let from_custom: CustomJsonName =
        serde_json::from_str(r#"{"customName":"a","renamedInt":1}"#).unwrap();
    assert_eq!(from_custom.original_name, "a");
    assert_eq!(from_custom.another_field, 1);

    let from_proto: CustomJsonName =
        serde_json::from_str(r#"{"original_name":"b","another_field":2}"#).unwrap();
    assert_eq!(from_proto.original_name, "b");
    assert_eq!(from_proto.another_field, 2);
}

// ── #2, #8 map key types + enum value ─────────────────────────────────

#[test]
fn test_map_int32_key_roundtrip() {
    let msg = MapKeyTypes {
        int_key: [(1, "a".into()), (-5, "b".into())].into_iter().collect(),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.int_key[&1], "a");
    assert_eq!(decoded.int_key[&-5], "b");
}

#[test]
fn test_map_int64_key_roundtrip() {
    let msg = MapKeyTypes {
        long_key: [(i64::MAX, "max".into()), (i64::MIN, "min".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.long_key[&i64::MAX], "max");
    assert_eq!(decoded.long_key[&i64::MIN], "min");
}

#[test]
fn test_map_bool_key_roundtrip() {
    let msg = MapKeyTypes {
        bool_key: [(true, "yes".into()), (false, "no".into())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.bool_key[&true], "yes");
    assert_eq!(decoded.bool_key[&false], "no");
}

#[test]
fn test_map_enum_value_roundtrip() {
    let msg = MapKeyTypes {
        enum_value: [
            ("r".into(), buffa::EnumValue::Known(Color::RED)),
            ("g".into(), buffa::EnumValue::Known(Color::GREEN)),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.enum_value["r"], Color::RED);
    assert_eq!(decoded.enum_value["g"], Color::GREEN);
}

#[test]
fn test_map_key_types_json_roundtrip() {
    // Proto3 JSON: all map keys are stringified.
    let msg = MapKeyTypes {
        int_key: [(42, "x".into())].into_iter().collect(),
        long_key: [(i64::MAX, "y".into())].into_iter().collect(),
        bool_key: [(true, "z".into())].into_iter().collect(),
        enum_value: [("c".into(), buffa::EnumValue::Known(Color::RED))]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    // Keys are strings in JSON.
    assert!(json.contains(r#""42":"x""#), "int key: {json}");
    assert!(json.contains(r#""true":"z""#), "bool key: {json}");
    // int64 key is a quoted string (proto3 JSON spec for int64).
    assert!(
        json.contains(&format!(r#""{}":"y""#, i64::MAX)),
        "int64 key: {json}"
    );
    // Enum value is the string name.
    assert!(json.contains(r#""c":"RED""#), "enum value: {json}");
    // Round-trip.
    let decoded: MapKeyTypes = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.int_key[&42], "x");
    assert_eq!(decoded.bool_key[&true], "z");
    assert_eq!(decoded.enum_value["c"], Color::RED);
}

// ── repeated/map proto3-JSON per-element encoding ────────────────────

#[test]
fn test_repeated_int64_json_quoted_strings() {
    let msg = RepeatedJsonSpecial {
        longs: vec![1, -2, i64::MAX],
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    // Proto3 JSON: int64 values are quoted decimal strings.
    assert!(
        json.contains(r#""longs":["1","-2","9223372036854775807"]"#),
        "repeated int64 must serialize as quoted strings: {json}"
    );
}

#[test]
fn test_repeated_int64_json_accepts_quoted_and_unquoted() {
    // Proto3 JSON parsers must accept both canonical quoted form and bare numbers.
    let quoted: RepeatedJsonSpecial = serde_json::from_str(r#"{"longs":["1","-2","42"]}"#).unwrap();
    assert_eq!(quoted.longs, vec![1, -2, 42]);
    let bare: RepeatedJsonSpecial = serde_json::from_str(r#"{"longs":[1,-2,42]}"#).unwrap();
    assert_eq!(bare.longs, vec![1, -2, 42]);
}

#[test]
fn test_repeated_float_json_nan_inf() {
    let msg = RepeatedJsonSpecial {
        floats: vec![1.5, f32::NAN, f32::INFINITY],
        doubles: vec![2.5, f64::NEG_INFINITY],
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    // Proto3 JSON spec: NaN/Inf as string tokens.
    assert!(
        json.contains(r#""floats":[1.5,"NaN","Infinity"]"#),
        "{json}"
    );
    assert!(json.contains(r#""doubles":[2.5,"-Infinity"]"#), "{json}");
    let back: RepeatedJsonSpecial = serde_json::from_str(&json).unwrap();
    assert_eq!(back.floats[0], 1.5);
    assert!(back.floats[1].is_nan());
    assert_eq!(back.floats[2], f32::INFINITY);
    assert_eq!(back.doubles[1], f64::NEG_INFINITY);
}

#[test]
fn test_map_int64_value_json_quoted() {
    // map<int32, int64>: both key (stringified int32) and value
    // (quoted int64) must be JSON strings per proto3 JSON spec.
    let msg = RepeatedJsonSpecial {
        long_values: [(1i32, 42i64), (2i32, i64::MAX)].into_iter().collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(
        json.contains(r#""1":"42""#) && json.contains(r#""2":"9223372036854775807""#),
        "map int64 values must be quoted: {json}"
    );
    let back: RepeatedJsonSpecial = serde_json::from_str(&json).unwrap();
    assert_eq!(back.long_values[&1], 42);
    assert_eq!(back.long_values[&2], i64::MAX);
}

// ── #1 sub-message merge semantics ─────────────────────────────────────

#[test]
fn test_submessage_merge_not_replace() {
    // When a singular message field appears twice on the wire, the
    // second occurrence MERGES into the first (proto spec). This is
    // the most common interop pitfall — naive decoders replace.
    let first = Whole {
        part: buffa::MessageField::some(Part {
            a: "from_first".into(),
            c: 1,
            ..Default::default()
        }),
        ..Default::default()
    };
    let second = Whole {
        part: buffa::MessageField::some(Part {
            b: "from_second".into(),
            c: 2,
            ..Default::default()
        }),
        ..Default::default()
    };
    // Concatenated wire bytes = both messages on same stream.
    let mut wire = first.encode_to_vec();
    wire.extend(second.encode_to_vec());
    let merged = Whole::decode(&mut wire.as_slice()).expect("decode");
    // Both `a` (from first) and `b` (from second) must be present.
    assert_eq!(merged.part.a, "from_first", "field a from first lost");
    assert_eq!(merged.part.b, "from_second", "field b from second lost");
    // Scalar last-wins: c=2 from second overwrites c=1 from first.
    assert_eq!(merged.part.c, 2, "scalar last-wins not applied");
}

// ── #5 negative enum values (proto2) ───────────────────────────────────

#[test]
fn test_negative_enum_roundtrip() {
    let msg = WithSignedEnum {
        value: Some(Signed::NEG_ONE),
        values: vec![Signed::NEG_BIG, Signed::ZERO, Signed::POS_ONE],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.value, Some(Signed::NEG_ONE));
    assert_eq!(
        decoded.values,
        vec![Signed::NEG_BIG, Signed::ZERO, Signed::POS_ONE]
    );
}

#[test]
fn test_negative_enum_default_is_none() {
    // Proto2 optional fields default to None (unset). The [default = X]
    // annotation would affect a getter accessor, but buffa exposes raw
    // Option<T> — see test_proto2_defaults_and_round_trip for precedent.
    let msg = WithSignedEnum::default();
    assert_eq!(msg.value, None);
}

#[test]
fn test_negative_enum_wire_is_10_byte_varint() {
    // Negative enum values are encoded as sign-extended 64-bit varints:
    // -1 = 0xFFFFFFFFFFFFFFFF = 10 bytes: 9x 0xFF + 0x01.
    let msg = WithSignedEnum {
        value: Some(Signed::NEG_ONE),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Tag for field 1 (varint) = 0x08, then 10-byte varint for -1.
    assert_eq!(bytes.len(), 11, "tag + 10-byte varint");
    assert_eq!(bytes[0], 0x08);
    assert_eq!(&bytes[1..10], &[0xFF; 9]);
    assert_eq!(bytes[10], 0x01);
}

// ── Unpacked repeated fixed-width types ──────────────────────────────
// [packed=false] for float/double/fixed*/bool — per-element size is
// constant, so compute_size uses len()*const instead of a loop.

#[test]
fn test_unpacked_fixed_width_round_trip() {
    let msg = UnpackedFixedWidth {
        floats: vec![1.0, 2.0, 3.0],
        doubles: vec![-1.5, 2.5],
        fx32: vec![0xDEAD_BEEF, 0],
        fx64: vec![u64::MAX, 0, 42],
        bools: vec![true, false, true, true],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.floats, vec![1.0, 2.0, 3.0]);
    assert_eq!(decoded.doubles, vec![-1.5, 2.5]);
    assert_eq!(decoded.fx32, vec![0xDEAD_BEEF, 0]);
    assert_eq!(decoded.fx64, vec![u64::MAX, 0, 42]);
    assert_eq!(decoded.bools, vec![true, false, true, true]);
}

#[test]
fn test_unpacked_fixed_width_wire_format() {
    // Verify each element has its own tag (unpacked, not a single LD blob).
    let msg = UnpackedFixedWidth {
        floats: vec![1.0, 2.0],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Each float: tag(1,Fixed32)=0x0D + 4 bytes = 5 bytes/elem.
    // 2 elements → 10 bytes.
    assert_eq!(bytes.len(), 10);
    assert_eq!(bytes[0], 0x0D); // tag for element 0
    assert_eq!(bytes[5], 0x0D); // tag for element 1
                                // compute_size must match encode_to_vec length.
    assert_eq!(msg.encoded_len() as usize, bytes.len());
}

#[test]
fn test_unpacked_fixed_width_empty() {
    let msg = UnpackedFixedWidth::default();
    assert_eq!(msg.encode_to_vec().len(), 0);
    assert_eq!(msg.encoded_len(), 0);
}

// ── Map with fixed-width key/value types ─────────────────────────────
// map<fixed32, bool>, map<fixed64, double> — exercises map_element_size_expr
// for constant-size types.

#[test]
fn test_map_fixed_width_round_trip() {
    let mut msg = MapFixedWidth::default();
    msg.fx32_to_bool.insert(0xDEAD_BEEF, true);
    msg.fx32_to_bool.insert(0, false);
    msg.fx64_to_double.insert(u64::MAX, 3.14);
    msg.fx64_to_double.insert(42, -1.5);
    msg.sfx32_to_float.insert(-100, 2.5);

    let decoded = round_trip(&msg);
    assert_eq!(decoded.fx32_to_bool.get(&0xDEAD_BEEF), Some(&true));
    assert_eq!(decoded.fx32_to_bool.get(&0), Some(&false));
    assert_eq!(decoded.fx64_to_double.get(&u64::MAX), Some(&3.14));
    assert_eq!(decoded.fx64_to_double.get(&42), Some(&-1.5));
    assert_eq!(decoded.sfx32_to_float.get(&-100), Some(&2.5));
}

#[test]
fn test_map_fixed_width_compute_size_matches_encode() {
    // compute_size() must equal encode_to_vec().len() — a mismatch would
    // indicate the map_element_size_expr constants are wrong.
    let mut msg = MapFixedWidth::default();
    msg.fx32_to_bool.insert(1, true);
    msg.fx32_to_bool.insert(2, false);
    msg.fx64_to_double.insert(100, f64::INFINITY);
    msg.sfx32_to_float.insert(-1, f32::NEG_INFINITY);

    let size = msg.encoded_len() as usize;
    let bytes = msg.encode_to_vec();
    assert_eq!(size, bytes.len(), "compute_size mismatch");
}
