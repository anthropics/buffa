//! Per-enum `enum_type` overrides in JSON map contexts (editions).
//!
//! Regression: `map_serde_module` read `enum_type` from the map FIELD's
//! features. The map field is `TYPE_MESSAGE`, so `resolve_field` skips the
//! enum_type overlay → stale file-level default. For editions protos where
//! a per-enum `CLOSED` override differs from the file default `OPEN`, type
//! resolution produced bare `E` (correct) but the serde module was
//! `map_enum` (expects `EnumValue<E>`) → E0308.
//!
//! proto2/proto3 never triggered this — file default always matches enum
//! default (CLOSED/OPEN respectively). Editions-only.
//!
//! Compilation of the `edenumjson` module is the primary assertion.

use crate::edenumjson::{ClosedFlavour, EnumJsonContexts, OpenFlavour};
use buffa::{EnumValue, Message};

#[test]
fn closed_enum_containers_reject_null_elements() {
    for ignore_unknown_enum_values in [false, true] {
        let opts = buffa::json::JsonParseOptions::new()
            .ignore_unknown_enum_values(ignore_unknown_enum_values);
        buffa::json::with_json_parse_options(&opts, || {
            for json in [
                r#"{"many":[null]}"#,
                r#"{"many":["FLAVOUR_VANILLA",null,2]}"#,
                r#"{"byKey":{"a":null}}"#,
                r#"{"byKey":{"a":"FLAVOUR_VANILLA","b":null}}"#,
                r#"{"byId":{"-1":null}}"#,
                r#"{"byId":{"1":2,"-1":null}}"#,
            ] {
                let result = serde_json::from_str::<EnumJsonContexts>(json);
                assert!(
                    result.is_err(),
                    "accepted {json}, lenient={ignore_unknown_enum_values}"
                );
            }

            let empty: EnumJsonContexts =
                serde_json::from_str(r#"{"singular":null,"many":null,"byKey":null,"byId":null}"#)
                    .unwrap();
            assert_eq!(empty.singular, None);
            assert!(empty.many.is_empty());
            assert!(empty.by_key.is_empty());
            assert!(empty.by_id.is_empty());
        });
    }
}

#[test]
fn closed_enum_map_value_json_roundtrip() {
    let msg = EnumJsonContexts {
        by_key: [
            ("a".to_string(), ClosedFlavour::FLAVOUR_VANILLA),
            ("b".to_string(), ClosedFlavour::FLAVOUR_CHOCOLATE),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    // Closed enums serialize as name strings.
    assert!(json.contains(r#""a":"FLAVOUR_VANILLA""#), "got: {json}");
    assert!(json.contains(r#""b":"FLAVOUR_CHOCOLATE""#), "got: {json}");

    let back: EnumJsonContexts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.by_key, msg.by_key);
}

#[test]
fn closed_enum_map_int_key_json_roundtrip() {
    // Also verifies bughunter's Issue 1 is a false positive: serde_json
    // auto-stringifies int keys ("1":"FLAVOUR_VANILLA") and parses them back.
    let msg = EnumJsonContexts {
        by_id: [
            (1, ClosedFlavour::FLAVOUR_VANILLA),
            (-42, ClosedFlavour::FLAVOUR_CHOCOLATE),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(json.contains(r#""1":"FLAVOUR_VANILLA""#), "got: {json}");
    assert!(json.contains(r#""-42":"FLAVOUR_CHOCOLATE""#), "got: {json}");

    let back: EnumJsonContexts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.by_id, msg.by_id);
}

#[test]
fn open_enum_map_value_json_roundtrip_control() {
    // Control: file-default OPEN enum → EnumValue<E> wrapper, map_enum module.
    // This path was always correct; pinned here so a regression would be caught.
    let msg = EnumJsonContexts {
        open_by_key: [(
            "k".to_string(),
            EnumValue::Known(OpenFlavour::OPEN_STRAWBERRY),
        )]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(json.contains(r#""k":"OPEN_STRAWBERRY""#), "got: {json}");

    let back: EnumJsonContexts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.open_by_key, msg.open_by_key);
}

#[test]
fn closed_enum_map_binary_roundtrip() {
    // Binary path was already correct (55ca457 fixed map_merge_arm).
    // Included for completeness — full binary+JSON parity.
    let msg = EnumJsonContexts {
        by_key: [("x".to_string(), ClosedFlavour::FLAVOUR_VANILLA)]
            .into_iter()
            .collect(),
        by_id: [(7, ClosedFlavour::FLAVOUR_CHOCOLATE)]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let back = EnumJsonContexts::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(back.by_key, msg.by_key);
    assert_eq!(back.by_id, msg.by_id);
}
