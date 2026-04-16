//! Comprehensive proto3-specific semantics verification.
//!
//! Proto3 is the most common syntax in new definitions. This module tests
//! the behaviours that make proto3 distinct from proto2:
//!
//! 1. Implicit presence: zero values NOT encoded (wire efficiency).
//! 2. `optional` keyword: zero values ARE encoded (explicit presence).
//! 3. Open enums: unknown values preserved, not dropped.
//! 4. Packed repeated scalars by default.
//! 5. Synthetic oneofs for `optional` fields filtered from user-visible API.

use crate::proto3sem::*;
use buffa::{EnumValue, Message, MessageView};

fn round_trip<T: Message>(msg: &T) -> T {
    T::decode(&mut msg.encode_to_vec().as_slice()).expect("decode")
}

// ═══════════════════════════════════════════════════════════════════════════
// IMPLICIT PRESENCE: zero values are not encoded
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn implicit_all_zero_encodes_empty() {
    // Every field at its zero value → nothing on the wire.
    let msg = ImplicitScalars::default();
    assert_eq!(msg.encode_to_vec().len(), 0);
}

#[test]
fn implicit_partial_zero_encodes_only_nonzero() {
    // Fields at zero are suppressed; only non-zero fields encode.
    // This is THE core proto3 wire-efficiency guarantee.
    let msg = ImplicitScalars {
        i32: 42,
        // i64, u32, etc. all remain 0 — must not encode.
        s: "hello".into(),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();

    // Only 2 fields encoded: i32 (tag=0x08, varint=0x2A) + s (tag=0x72, len=5, "hello").
    // Tag 0x08: field 1, wire type 0 (varint).
    // Tag 0x72: field 14, wire type 2 (length-delimited).
    assert_eq!(bytes[0], 0x08);
    assert_eq!(bytes[1], 42);
    // s: tag 0x72 + len 5 + "hello" = 7 bytes.
    assert_eq!(bytes[2], 0x72);
    assert_eq!(bytes[3], 5);
    assert_eq!(&bytes[4..9], b"hello");
    assert_eq!(bytes.len(), 9, "expected exactly 2 fields encoded");
}

#[test]
fn implicit_decode_absent_as_zero() {
    // Decoding an empty buffer → all fields at zero value (not error).
    let msg = ImplicitScalars::decode(&mut &[][..]).unwrap();
    assert_eq!(msg.i32, 0);
    assert_eq!(msg.i64, 0);
    assert_eq!(msg.u64, 0);
    assert_eq!(msg.f64, 0.0);
    assert!(!msg.b);
    assert_eq!(msg.s, "");
    assert!(msg.by.is_empty());
}

#[test]
fn implicit_zero_value_round_trip_table() {
    // Each scalar type: set to zero → encodes as nothing → decodes as zero.
    // This catches bugs where a type's zero-check is wrong (e.g. -0.0 for float).
    let zero = ImplicitScalars::default();
    let reencoded = round_trip(&zero);
    assert_eq!(reencoded, zero);
    // Explicitly verify each field decodes as the zero value.
    assert_eq!(reencoded.i32, 0);
    assert_eq!(reencoded.si32, 0);
    assert_eq!(reencoded.fx32, 0);
    assert_eq!(reencoded.sfx32, 0);
    assert_eq!(reencoded.f32, 0.0);
    assert!(!reencoded.b);
}

#[test]
fn implicit_nonzero_value_round_trip_table() {
    // Every scalar type set to a non-default value must round-trip.
    let msg = ImplicitScalars {
        i32: -1,
        i64: i64::MIN,
        u32: u32::MAX,
        u64: u64::MAX,
        si32: -100,
        si64: -200,
        fx32: 0xDEAD_BEEF,
        fx64: 0xCAFE_BABE_DEAD_BEEF,
        sfx32: -42,
        sfx64: -84,
        f32: 1.5,
        f64: -3.25,
        b: true,
        s: "hello".into(),
        by: vec![0xFF, 0x00, 0x42],
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn implicit_negative_zero_float_emitted() {
    // -0.0 is NOT the proto3 default: only +0.0 is. The presence check
    // compares bit patterns (`to_bits() != 0`), so -0.0 (sign bit set) is
    // serialized. The conformance suite verifies this round-trips.
    //
    // Earlier versions of buffa used `!= 0.0` (IEEE equality) and suppressed
    // -0.0. That was wrong — see `TextFormatInput.FloatFieldNegativeZero.*`.
    let msg = ImplicitScalars {
        f32: -0.0,
        f64: -0.0,
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Each field: 1 tag byte + 4 or 8 payload bytes.
    assert_eq!(
        bytes.len(),
        (1 + 4) + (1 + 8),
        "-0.0 should be emitted (bit pattern is non-zero); got {bytes:02X?}"
    );
    // Round-trip: decode and check the sign bit survived.
    let back = ImplicitScalars::decode_from_slice(&bytes).unwrap();
    assert!(back.f32.is_sign_negative() && back.f32 == 0.0);
    assert!(back.f64.is_sign_negative() && back.f64 == 0.0);
}

// ═══════════════════════════════════════════════════════════════════════════
// EXPLICIT PRESENCE (`optional`): zero values ARE encoded
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn optional_all_none_encodes_empty() {
    // All None → nothing on the wire.
    let msg = OptionalAllTypes::default();
    assert_eq!(msg.encode_to_vec().len(), 0);
    // And all fields are None (not Some(0)).
    assert_eq!(msg.i32, None);
    assert_eq!(msg.s, None);
    assert_eq!(msg.e, None);
}

#[test]
fn optional_some_zero_is_encoded() {
    // Some(0) is DIFFERENT from None — it must encode. This is the key
    // semantic distinction between proto3 implicit and explicit presence.
    let msg = OptionalAllTypes {
        i32: Some(0),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Must encode: tag=0x08 + varint(0)=0x00 = 2 bytes.
    assert_eq!(bytes, vec![0x08, 0x00], "Some(0) must encode on the wire");

    // Round-trip: Some(0) stays Some(0), not None.
    let decoded = OptionalAllTypes::decode(&mut bytes.as_slice()).unwrap();
    assert_eq!(decoded.i32, Some(0), "Some(0) must survive round-trip");
}

#[test]
fn optional_some_zero_all_types_table() {
    // Every scalar type: Some(zero) → encodes → decodes as Some(zero).
    // Regression test for any type where the zero-value check and the
    // optional-encode check got confused.
    let msg = OptionalAllTypes {
        i32: Some(0),
        i64: Some(0),
        u32: Some(0),
        u64: Some(0),
        si32: Some(0),
        si64: Some(0),
        fx32: Some(0),
        fx64: Some(0),
        sfx32: Some(0),
        sfx64: Some(0),
        f32: Some(0.0),
        f64: Some(0.0),
        b: Some(false),
        s: Some("".into()),
        by: Some(vec![]),
        e: Some(EnumValue::Known(Color::COLOR_UNSPECIFIED)),
        nested: buffa::MessageField::some(ImplicitScalars::default()),
        ..Default::default()
    };

    let bytes = msg.encode_to_vec();
    assert!(
        !bytes.is_empty(),
        "Some(zero) for all types must encode something"
    );

    let decoded = round_trip(&msg);
    // Every field should still be Some(zero), not None.
    assert_eq!(decoded.i32, Some(0));
    assert_eq!(decoded.i64, Some(0));
    assert_eq!(decoded.u32, Some(0));
    assert_eq!(decoded.u64, Some(0));
    assert_eq!(decoded.si32, Some(0));
    assert_eq!(decoded.si64, Some(0));
    assert_eq!(decoded.fx32, Some(0));
    assert_eq!(decoded.fx64, Some(0));
    assert_eq!(decoded.sfx32, Some(0));
    assert_eq!(decoded.sfx64, Some(0));
    assert_eq!(decoded.f32, Some(0.0));
    assert_eq!(decoded.f64, Some(0.0));
    assert_eq!(decoded.b, Some(false));
    assert_eq!(decoded.s, Some("".into()));
    assert_eq!(decoded.by, Some(vec![]));
    assert_eq!(decoded.e, Some(EnumValue::Known(Color::COLOR_UNSPECIFIED)));
    assert!(decoded.nested.is_set());
}

#[test]
fn optional_none_vs_some_zero_wire_distinguishable() {
    // The KEY invariant: None and Some(0) produce different wire bytes.
    // A receiver can always tell them apart.
    let none_msg = OptionalAllTypes::default();
    let zero_msg = OptionalAllTypes {
        i32: Some(0),
        ..Default::default()
    };
    assert_ne!(
        none_msg.encode_to_vec(),
        zero_msg.encode_to_vec(),
        "None and Some(0) must encode differently"
    );
}

#[test]
fn optional_nonzero_round_trip_all_types() {
    let msg = OptionalAllTypes {
        i32: Some(-1),
        i64: Some(i64::MAX),
        u32: Some(100),
        u64: Some(200),
        si32: Some(-50),
        si64: Some(-60),
        fx32: Some(0xAAAA_BBBB),
        fx64: Some(0xCCCC_DDDD_EEEE_FFFF),
        sfx32: Some(-1),
        sfx64: Some(-2),
        f32: Some(f32::INFINITY),
        f64: Some(f64::NEG_INFINITY),
        b: Some(true),
        s: Some("non-empty".into()),
        by: Some(vec![0x01, 0x02]),
        e: Some(EnumValue::Known(Color::BLUE)),
        nested: buffa::MessageField::some(ImplicitScalars {
            i32: 99,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn optional_message_redundant_but_works() {
    // `optional ImplicitScalars nested` — message fields already have explicit
    // presence (MessageField is Option<Box<T>>). The `optional` keyword is
    // redundant but protoc accepts it. Codegen should produce identical
    // behaviour to a bare message field.
    let with = OptionalAllTypes {
        nested: buffa::MessageField::some(ImplicitScalars {
            i32: 7,
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&with);
    assert!(decoded.nested.is_set());
    assert_eq!(decoded.nested.i32, 7);

    let without = OptionalAllTypes::default();
    let decoded = round_trip(&without);
    assert!(!decoded.nested.is_set());
}

// ═══════════════════════════════════════════════════════════════════════════
// OPEN ENUMS: unknown values preserved as EnumValue::Unknown
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn enum_zero_value_suppressed() {
    // Proto3 enum default is the zero-variant (COLOR_UNSPECIFIED = 0).
    // Singular implicit enum at zero → not encoded.
    let msg = EnumContexts {
        singular: EnumValue::Known(Color::COLOR_UNSPECIFIED),
        ..Default::default()
    };
    // Only `singular` at zero — repeated/map/oneof are empty.
    assert_eq!(msg.encode_to_vec().len(), 0);
}

#[test]
fn enum_known_value_round_trip() {
    let msg = EnumContexts {
        singular: EnumValue::Known(Color::BLUE),
        rep: vec![EnumValue::Known(Color::RED), EnumValue::Known(Color::GREEN)],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.singular, EnumValue::Known(Color::BLUE));
    assert_eq!(
        decoded.rep,
        vec![EnumValue::Known(Color::RED), EnumValue::Known(Color::GREEN)]
    );
}

#[test]
fn enum_unknown_value_preserved_singular() {
    // Wire carries value 99 (not a defined Color variant).
    // Proto3 open enum → decode as Unknown(99), NOT error, NOT drop.
    let msg = EnumContexts {
        singular: EnumValue::Unknown(99),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.singular, EnumValue::Unknown(99));
}

#[test]
fn enum_unknown_value_preserved_repeated() {
    // Mix of known and unknown in a packed repeated.
    let msg = EnumContexts {
        rep: vec![
            EnumValue::Known(Color::RED),
            EnumValue::Unknown(99),
            EnumValue::Known(Color::BLUE),
            EnumValue::Unknown(-1), // Negative! Unusual but wire-legal (10-byte varint).
        ],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.rep,
        vec![
            EnumValue::Known(Color::RED),
            EnumValue::Unknown(99),
            EnumValue::Known(Color::BLUE),
            EnumValue::Unknown(-1),
        ]
    );
}

#[test]
fn enum_unknown_value_preserved_map() {
    let mut msg = EnumContexts::default();
    msg.by_key
        .insert("known".into(), EnumValue::Known(Color::GREEN));
    msg.by_key.insert("unknown".into(), EnumValue::Unknown(42));
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.by_key.get("known"),
        Some(&EnumValue::Known(Color::GREEN))
    );
    assert_eq!(decoded.by_key.get("unknown"), Some(&EnumValue::Unknown(42)));
}

#[test]
fn enum_unknown_value_preserved_oneof() {
    let msg = EnumContexts {
        choice: Some(enum_contexts::ChoiceOneof::Picked(EnumValue::Unknown(77))),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(
        decoded.choice,
        Some(enum_contexts::ChoiceOneof::Picked(EnumValue::Unknown(77)))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// PACKED REPEATED SCALARS (default)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn repeated_packed_by_default_wire_format() {
    // proto3 packs all packable scalars by default: single LD tag + blob.
    let msg = RepeatedPacking {
        ints: vec![1, 2, 3],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Packed: tag(1, LengthDelimited)=0x0A + len(3) + [0x01, 0x02, 0x03].
    assert_eq!(bytes, vec![0x0A, 0x03, 0x01, 0x02, 0x03]);
}

#[test]
fn repeated_accepts_both_packed_and_unpacked_on_decode() {
    // proto3 encoders use packed, but the decoder must accept BOTH forms
    // (proto2-era encoders may send unpacked).
    use buffa::encoding::{encode_varint, Tag, WireType};
    // Unpacked: three separate tag+value pairs.
    let mut wire = Vec::new();
    for v in [10u64, 20, 30] {
        Tag::new(1, WireType::Varint).encode(&mut wire);
        encode_varint(v, &mut wire);
    }
    let decoded = RepeatedPacking::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(decoded.ints, vec![10, 20, 30]);
}

#[test]
fn repeated_mixed_packed_and_unpacked_on_wire() {
    // Extreme case: same field has BOTH packed and unpacked occurrences.
    // Decoder must accumulate all elements in order.
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    // Packed blob: [1, 2]
    Tag::new(1, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(2, &mut wire); // payload len
    encode_varint(1, &mut wire);
    encode_varint(2, &mut wire);
    // Unpacked element: 3
    Tag::new(1, WireType::Varint).encode(&mut wire);
    encode_varint(3, &mut wire);
    // Another packed blob: [4, 5]
    Tag::new(1, WireType::LengthDelimited).encode(&mut wire);
    encode_varint(2, &mut wire);
    encode_varint(4, &mut wire);
    encode_varint(5, &mut wire);

    let decoded = RepeatedPacking::decode(&mut wire.as_slice()).unwrap();
    assert_eq!(decoded.ints, vec![1, 2, 3, 4, 5]);
}

#[test]
fn repeated_empty_encodes_nothing() {
    let msg = RepeatedPacking {
        ints: vec![],
        strings: vec![],
        ..Default::default()
    };
    assert_eq!(msg.encode_to_vec().len(), 0);
}

#[test]
fn repeated_strings_never_packed() {
    // Strings are length-delimited by nature — each element gets its own tag.
    let msg = RepeatedPacking {
        strings: vec!["a".into(), "b".into()],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Tag(6, LD)=0x32 + len(1) + "a" + Tag(6, LD)=0x32 + len(1) + "b".
    assert_eq!(bytes, vec![0x32, 0x01, b'a', 0x32, 0x01, b'b']);
}

#[test]
fn repeated_all_types_round_trip() {
    let msg = RepeatedPacking {
        ints: vec![-1, 0, 1, i32::MAX, i32::MIN],
        fixeds: vec![0, u32::MAX],
        bools: vec![true, false, true],
        doubles: vec![1.5, f64::NEG_INFINITY],
        colors: vec![EnumValue::Known(Color::RED), EnumValue::Unknown(99)],
        strings: vec!["".into(), "nonempty".into()],
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.ints, msg.ints);
    assert_eq!(decoded.fixeds, msg.fixeds);
    assert_eq!(decoded.bools, msg.bools);
    // NaN-safe comparison (no NaN here, but good practice).
    assert_eq!(decoded.doubles, msg.doubles);
    assert_eq!(decoded.colors, msg.colors);
    assert_eq!(decoded.strings, msg.strings);
}

// ═══════════════════════════════════════════════════════════════════════════
// MIXED PRESENCE: implicit and optional in same message
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn mixed_presence_encodes_correctly() {
    // Implicit fields at zero → suppressed.
    // Optional fields at Some(0) → encoded.
    let msg = MixedPresence {
        implicit_id: 0,                                                   // suppressed
        optional_id: Some(0),                                             // encoded
        implicit_name: "".into(),                                         // suppressed
        optional_name: Some("".into()),                                   // encoded
        implicit_color: EnumValue::Known(Color::COLOR_UNSPECIFIED),       // suppressed
        optional_color: Some(EnumValue::Known(Color::COLOR_UNSPECIFIED)), // encoded
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    assert!(
        !bytes.is_empty(),
        "Some(0) optional fields must encode even when implicit neighbours are suppressed"
    );

    let decoded = round_trip(&msg);
    // Implicit zero comes back as zero (absence-as-default).
    assert_eq!(decoded.implicit_id, 0);
    assert_eq!(decoded.implicit_name, "");
    // Optional Some(0) comes back as Some(0), not None.
    assert_eq!(decoded.optional_id, Some(0));
    assert_eq!(decoded.optional_name, Some("".into()));
    assert_eq!(
        decoded.optional_color,
        Some(EnumValue::Known(Color::COLOR_UNSPECIFIED))
    );
}

#[test]
fn mixed_presence_both_nonzero() {
    let msg = MixedPresence {
        implicit_id: 42,
        optional_id: Some(99),
        implicit_name: "implicit".into(),
        optional_name: Some("optional".into()),
        implicit_color: EnumValue::Known(Color::RED),
        optional_color: Some(EnumValue::Known(Color::BLUE)),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn mixed_presence_optional_unset_implicit_set() {
    // Implicit set, optional None.
    let msg = MixedPresence {
        implicit_id: 42,
        optional_id: None,
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.implicit_id, 42);
    assert_eq!(decoded.optional_id, None);
}

// ═══════════════════════════════════════════════════════════════════════════
// SYNTHETIC ONEOFS: `optional` creates a hidden single-variant oneof
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn synthetic_oneofs_not_user_visible() {
    // proto3 `optional int32 i32 = 1` creates a synthetic oneof internally
    // (FieldDescriptorProto.proto3_optional=true). Codegen must emit
    // `Option<i32>`, NOT `Option<SomeOneofEnum>`. This is a compile-time
    // assertion — if synthetic oneofs leak, the type shape is wrong.
    let _: Option<i32> = OptionalAllTypes::default().i32;
    let _: Option<String> = OptionalAllTypes::default().s;
    let _: Option<EnumValue<Color>> = OptionalAllTypes::default().e;
    // The optional message field uses MessageField, not Option<Box<T>>.
    let _: buffa::MessageField<ImplicitScalars> = OptionalAllTypes::default().nested;
}

// ═══════════════════════════════════════════════════════════════════════════
// VIEW PARITY: view decoder must match owned decoder for all proto3 semantics
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn view_implicit_presence_matches_owned() {
    let msg = ImplicitScalars {
        i32: 42,
        s: "view".into(),
        by: vec![0xAA, 0xBB],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = ImplicitScalarsView::decode_view(&bytes).unwrap();
    assert_eq!(view.i32, 42);
    assert_eq!(view.s, "view");
    assert_eq!(view.by, &[0xAA, 0xBB]);
    // Zero fields stay zero.
    assert_eq!(view.i64, 0);
    assert!(!view.b);

    let owned = view.to_owned_message();
    assert_eq!(owned, msg);
}

#[test]
fn view_optional_some_zero_matches_owned() {
    let msg = OptionalAllTypes {
        i32: Some(0),
        s: Some("".into()),
        b: Some(false),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = OptionalAllTypesView::decode_view(&bytes).unwrap();
    assert_eq!(view.i32, Some(0));
    assert_eq!(view.s, Some(""));
    assert_eq!(view.b, Some(false));

    // View → owned → encode must match original bytes.
    assert_eq!(view.to_owned_message().encode_to_vec(), bytes);
}

#[test]
fn view_open_enum_unknown_preserved() {
    let msg = EnumContexts {
        singular: EnumValue::Unknown(55),
        rep: vec![EnumValue::Known(Color::RED), EnumValue::Unknown(99)],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    let view = EnumContextsView::decode_view(&bytes).unwrap();
    assert_eq!(view.singular, EnumValue::Unknown(55));
    let rep_collected: Vec<_> = view.rep.iter().copied().collect();
    assert_eq!(
        rep_collected,
        vec![EnumValue::Known(Color::RED), EnumValue::Unknown(99)]
    );

    assert_eq!(view.to_owned_message(), msg);
}
