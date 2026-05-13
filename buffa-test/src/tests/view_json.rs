//! View JSON round-trip tests (issue #83 / PR #106).
//!
//! Each test encodes an owned message → decodes as a view → serializes both
//! to JSON → asserts identical output. The proto sources are
//! `protos/view_json.proto` (proto3) and `protos/view_json_proto2.proto`
//! (proto2), both built with `generate_views(true)` + `generate_json(true)`,
//! and together they exercise every distinct codegen branch in
//! `generate_view_serialize`: implicit/explicit/required presence, repeated,
//! maps with each key/value family, oneof variants, nested messages, enums
//! (open and closed), `NullValue`, and WKT view fields.

use buffa::{Message, MessageView};

/// Encode `$owned`, decode as `$view`, serialize both to JSON, and assert
/// they are byte-identical. Returns the JSON string for follow-on assertions.
macro_rules! assert_view_json_parity {
    ($view:ty, $owned:expr) => {{
        let owned = $owned;
        let bytes = Message::encode_to_vec(&owned);
        let view = <$view>::decode_view(&bytes).expect("decode_view");
        let json_owned = serde_json::to_string(&owned).expect("serialize owned");
        let json_view = serde_json::to_string(&view).expect("serialize view");
        assert_eq!(
            json_view, json_owned,
            "view JSON must match owned JSON (left = view)"
        );
        json_view
    }};
}

/// Like `assert_view_json_parity!` but compares as `serde_json::Value` so map
/// key ordering doesn't matter (owned `HashMap` iteration order is unstable).
macro_rules! assert_view_json_value_parity {
    ($view:ty, $owned:expr) => {{
        let owned = $owned;
        let bytes = Message::encode_to_vec(&owned);
        let view = <$view>::decode_view(&bytes).expect("decode_view");
        let json_owned = serde_json::to_string(&owned).expect("serialize owned");
        let json_view = serde_json::to_string(&view).expect("serialize view");
        let v_owned: serde_json::Value = serde_json::from_str(&json_owned).unwrap();
        let v_view: serde_json::Value = serde_json::from_str(&json_view).unwrap();
        assert_eq!(v_view, v_owned, "view JSON must match owned JSON as Value");
        json_view
    }};
}

// ── Proto3: singular (implicit-presence) scalars ───────────────────────────

mod scalars {
    use super::*;
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;

    #[test]
    fn matches_owned() {
        let json = assert_view_json_parity!(
            ScalarsView,
            Scalars {
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
            }
        );
        assert!(
            json.contains(r#""i64":"9007199254740993""#),
            "int64 >2^53 must be quoted: {json}"
        );
        assert!(
            json.contains(r#""by":"3q2+7w==""#),
            "bytes must be base64: {json}"
        );
    }

    #[test]
    fn double_special_values() {
        let cases: &[(f64, &str)] = &[
            (f64::NAN, r#""f64":"NaN""#),
            (f64::INFINITY, r#""f64":"Infinity""#),
            (f64::NEG_INFINITY, r#""f64":"-Infinity""#),
        ];
        for (val, expected_fragment) in cases {
            let json = assert_view_json_parity!(
                ScalarsView,
                Scalars {
                    f64: *val,
                    ..Default::default()
                }
            );
            assert!(
                json.contains(expected_fragment),
                "double {val:?} must serialize as {expected_fragment}: {json}"
            );
        }
    }

    #[test]
    fn proto3_defaults_omitted() {
        let json = assert_view_json_parity!(ScalarsView, Scalars::default());
        assert_eq!(json, "{}", "default view must serialize as empty object");
    }
}

// ── Proto3: explicit-presence (`optional`) scalars ─────────────────────────

mod optional_scalars {
    use super::*;
    use crate::view_json::__buffa::view::OptionalScalarsView;
    use crate::view_json::Color;
    use crate::view_json::OptionalScalars;

    #[test]
    fn all_set_matches_owned() {
        let json = assert_view_json_parity!(
            OptionalScalarsView,
            OptionalScalars {
                i32: Some(-1),
                i64: Some(9007199254740993),
                f64: Some(f64::NAN),
                b: Some(true),
                s: Some("opt".into()),
                by: Some(vec![0x01, 0x02]),
                color: Some(buffa::EnumValue::Known(Color::GREEN)),
                ..Default::default()
            }
        );
        assert!(
            json.contains(r#""i64":"9007199254740993""#),
            "optional int64 must be quoted: {json}"
        );
        assert!(json.contains(r#""f64":"NaN""#), "optional NaN: {json}");
        assert!(
            json.contains(r#""color":"GREEN""#),
            "optional enum as name: {json}"
        );
    }

    #[test]
    fn explicit_zero_is_emitted() {
        // Explicit-presence fields with default-equivalent values must still
        // appear (unlike implicit presence, where they're omitted).
        let json = assert_view_json_parity!(
            OptionalScalarsView,
            OptionalScalars {
                i32: Some(0),
                b: Some(false),
                s: Some(String::new()),
                ..Default::default()
            }
        );
        assert!(json.contains(r#""i32":0"#), "explicit 0 emitted: {json}");
        assert!(
            json.contains(r#""b":false"#),
            "explicit false emitted: {json}"
        );
        assert!(json.contains(r#""s":"""#), "explicit empty string: {json}");
    }

    #[test]
    fn unset_omitted() {
        let json = assert_view_json_parity!(OptionalScalarsView, OptionalScalars::default());
        assert_eq!(json, "{}");
    }
}

// ── Proto3: repeated fields ────────────────────────────────────────────────

mod repeats {
    use super::*;
    use crate::view_json::__buffa::view::RepeatsView;
    use crate::view_json::Repeats;

    #[test]
    fn matches_owned() {
        let json = assert_view_json_parity!(
            RepeatsView,
            Repeats {
                i64s: vec![1, -1, 9007199254740993],
                u64s: vec![0, u64::MAX],
                f64s: vec![1.0, f64::NAN, f64::INFINITY],
                bys: vec![vec![0xAA], vec![0xBB, 0xCC]],
                strs: vec!["a".into(), "b".into()],
                bools: vec![true, false, true],
                ..Default::default()
            }
        );
        assert!(
            json.contains(r#""9007199254740993""#),
            "repeated int64 quoted: {json}"
        );
        assert!(json.contains(r#""NaN""#), "repeated NaN: {json}");
        assert!(json.contains(r#""qg==""#), "repeated bytes base64: {json}");
    }

    #[test]
    fn empty_omitted() {
        let json = assert_view_json_parity!(RepeatsView, Repeats::default());
        assert_eq!(json, "{}");
    }
}

// ── Proto3: enums ──────────────────────────────────────────────────────────

mod enums {
    use super::*;
    use crate::view_json::__buffa::view::WithEnumView;
    use crate::view_json::{Color, WithEnum};

    #[test]
    fn matches_owned() {
        let json = assert_view_json_parity!(
            WithEnumView,
            WithEnum {
                color: buffa::EnumValue::Known(Color::RED),
                colors: vec![
                    buffa::EnumValue::Known(Color::GREEN),
                    buffa::EnumValue::Known(Color::BLUE),
                ],
                ..Default::default()
            }
        );
        assert!(json.contains(r#""color":"RED""#), "enum as name: {json}");
        assert!(json.contains(r#""GREEN""#), "repeated enum: {json}");
    }
}

// ── Proto3: oneofs ─────────────────────────────────────────────────────────

mod oneofs {
    use super::*;
    use crate::view_json::__buffa::oneof::with_oneof::Value as ValueOneof;
    use crate::view_json::__buffa::view::WithOneofView;
    use crate::view_json::{Color, Inner, WithOneof};
    use buffa_types::google::protobuf::NullValue;

    #[test]
    fn variants_match_owned() {
        let cases: &[ValueOneof] = &[
            ValueOneof::Text("hello".into()),
            ValueOneof::Number(i64::MAX),
            ValueOneof::Data(vec![0xAB, 0xCD]),
            ValueOneof::Color(buffa::EnumValue::Known(Color::GREEN)),
            ValueOneof::Msg(Box::new(Inner {
                x: 7,
                name: "inner".into(),
                ..Default::default()
            })),
        ];
        for variant in cases {
            assert_view_json_parity!(
                WithOneofView,
                WithOneof {
                    value: Some(variant.clone()),
                    ..Default::default()
                }
            );
        }
    }

    #[test]
    fn unset_omitted() {
        let json = assert_view_json_parity!(WithOneofView, WithOneof::default());
        assert_eq!(json, "{}");
    }

    #[test]
    fn null_value_serializes_as_null() {
        // NullValue oneof variants must serialize as JSON `null`, not
        // "NULL_VALUE". Owned-path coverage is in `json.rs`.
        let json = assert_view_json_parity!(
            WithOneofView,
            WithOneof {
                value: Some(ValueOneof::NullVal(NullValue::NULL_VALUE.into())),
                ..Default::default()
            }
        );
        assert_eq!(json, r#"{"nullVal":null}"#);
    }
}

// ── Proto3: maps ───────────────────────────────────────────────────────────

mod maps {
    use super::*;
    use crate::view_json::__buffa::view::WithMapsView;
    use crate::view_json::{Color, Inner, WithMaps};

    #[test]
    fn matches_owned() {
        let json = assert_view_json_value_parity!(
            WithMapsView,
            WithMaps {
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
            }
        );
        // int64 map value must be a quoted string.
        assert!(
            json.contains(r#""9007199254740993""#),
            "int64 map value must be quoted: {json}"
        );
    }

    #[test]
    fn non_string_keys_match_owned() {
        // Proto3 JSON requires all map keys to be JSON strings. The view path
        // relies on serde_json's MapKeySerializer to stringify scalar keys;
        // verify it agrees with the owned-side `DisplayKey` wrapper.
        let json = assert_view_json_value_parity!(
            WithMapsView,
            WithMaps {
                by_i64: [(9007199254740993i64, "big".into()), (-1, "neg".into())]
                    .into_iter()
                    .collect(),
                by_u64: [(u64::MAX, "max".into())].into_iter().collect(),
                by_bool: [(true, "yes".into()), (false, "no".into())]
                    .into_iter()
                    .collect(),
                ..Default::default()
            }
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        // Keys must be string-typed in the JSON.
        assert!(parsed["byI64"]["9007199254740993"].is_string());
        assert!(parsed["byI64"]["-1"].is_string());
        assert!(parsed["byU64"][u64::MAX.to_string()].is_string());
        assert!(parsed["byBool"]["true"].is_string());
        assert!(parsed["byBool"]["false"].is_string());
    }

    #[test]
    fn duplicate_keys_dedup_last_write_wins() {
        // `MapView` preserves all wire entries (it's zero-copy), but a JSON
        // object cannot have duplicate keys. The view `Serialize` impl must
        // deduplicate (last-write-wins), matching the owned `HashMap`
        // decode. Construct wire bytes with a duplicate `labels` entry by
        // concatenating two single-entry encodings, and verify the view
        // serializes valid JSON identical to what the owned decoder yields.
        use crate::view_json::__buffa::view::WithMapsView;
        use crate::view_json::WithMaps;
        use buffa::Message;

        let entry = |k: &str, v: &str| -> Vec<u8> {
            Message::encode_to_vec(&WithMaps {
                labels: [(k.to_string(), v.to_string())].into_iter().collect(),
                ..Default::default()
            })
        };
        let mut wire = entry("env", "stage");
        wire.extend(entry("env", "prod")); // duplicate key — last wins
        wire.extend(entry("region", "us"));

        let view = WithMapsView::decode_view(&wire).expect("decode_view");
        let owned = WithMaps::decode(&mut wire.as_slice()).expect("decode owned");

        let json_view = serde_json::to_string(&view).expect("serialize view");
        let json_owned = serde_json::to_string(&owned).expect("serialize owned");
        let v_view: serde_json::Value = serde_json::from_str(&json_view).unwrap();
        let v_owned: serde_json::Value = serde_json::from_str(&json_owned).unwrap();
        assert_eq!(v_view, v_owned, "view dedup must match owned: {json_view}");
        assert_eq!(v_view["labels"]["env"], "prod", "last write wins");
        assert_eq!(v_view["labels"]["region"], "us");
        // Only two distinct keys, even though the wire has three entries.
        assert_eq!(v_view["labels"].as_object().unwrap().len(), 2);
    }

    #[test]
    fn complex_values_match_owned() {
        assert_view_json_value_parity!(
            WithMapsView,
            WithMaps {
                blobs: [("a".into(), vec![0xDE, 0xAD])].into_iter().collect(),
                nested: [(
                    "k".into(),
                    Inner {
                        x: 5,
                        name: "v".into(),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ratios: [
                    ("pi".into(), std::f64::consts::PI),
                    ("nan".into(), f64::NAN),
                ]
                .into_iter()
                .collect(),
                ..Default::default()
            }
        );
    }
}

// ── Proto3: nested messages ────────────────────────────────────────────────

mod nested {
    use super::*;
    use crate::view_json::__buffa::view::OuterView;
    use crate::view_json::{Inner, Outer};

    #[test]
    fn matches_owned() {
        assert_view_json_parity!(
            OuterView,
            Outer {
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
            }
        );
    }
}

// ── Proto3: WKT fields ─────────────────────────────────────────────────────

mod wkt {
    use super::*;
    use crate::view_json::__buffa::view::WithWktView;
    use crate::view_json::WithWkt;
    use buffa::MessageField;
    use buffa_types::google::protobuf::{BoolValue, Duration, Int64Value, StringValue, Timestamp};

    #[test]
    fn matches_owned() {
        let json = assert_view_json_parity!(
            WithWktView,
            WithWkt {
                ts: MessageField::some(Timestamp {
                    seconds: 1_700_000_000,
                    nanos: 500_000_000,
                    ..Default::default()
                }),
                dur: MessageField::some(Duration {
                    seconds: 90,
                    nanos: 0,
                    ..Default::default()
                }),
                count: MessageField::some(Int64Value {
                    value: 9007199254740993,
                    ..Default::default()
                }),
                label: MessageField::some(StringValue {
                    value: "tag".into(),
                    ..Default::default()
                }),
                history: vec![
                    Timestamp {
                        seconds: 1,
                        ..Default::default()
                    },
                    Timestamp {
                        seconds: 2,
                        ..Default::default()
                    },
                ],
                flag: MessageField::some(BoolValue {
                    value: true,
                    ..Default::default()
                }),
                ..Default::default()
            }
        );
        // Timestamp must be RFC 3339, not a struct.
        assert!(
            json.contains(r#""ts":"2023-11-14T22:13:20.500Z""#),
            "Timestamp must be RFC 3339: {json}"
        );
        // Duration must be the proto3 JSON "<seconds>s" string.
        assert!(json.contains(r#""dur":"90s""#), "Duration string: {json}");
        // Int64Value wrapper must be a quoted string.
        assert!(
            json.contains(r#""count":"9007199254740993""#),
            "Int64Value must be quoted: {json}"
        );
        // StringValue/BoolValue wrappers unwrap to their inner value.
        assert!(
            json.contains(r#""label":"tag""#),
            "StringValue unwrap: {json}"
        );
        assert!(json.contains(r#""flag":true"#), "BoolValue unwrap: {json}");
        // Repeated Timestamp uses RFC 3339 elements.
        assert!(
            json.contains(r#""1970-01-01T00:00:01Z""#),
            "repeated Timestamp: {json}"
        );
    }

    #[test]
    fn unset_omitted() {
        let json = assert_view_json_parity!(WithWktView, WithWkt::default());
        assert_eq!(json, "{}");
    }
}

// ── OwnedView<V> blanket impl ──────────────────────────────────────────────

mod owned_view {
    use super::*;
    use crate::view_json::__buffa::view::ScalarsView;
    use crate::view_json::Scalars;
    use buffa::view::OwnedView;

    #[test]
    fn blanket_impl_matches_owned() {
        // `OwnedView<V>` must implement `Serialize` via the blanket impl so
        // that `serde_json::to_string(&owned_view)` works without `&*`.
        let owned = Scalars {
            i32: 99,
            s: "owned_view".into(),
            by: vec![0x01, 0x02],
            ..Default::default()
        };
        let bytes = bytes::Bytes::from(Message::encode_to_vec(&owned));
        let owned_view =
            OwnedView::<ScalarsView<'static>>::decode(bytes).expect("decode OwnedView");

        let json_owned_view = serde_json::to_string(&owned_view).expect("serialize OwnedView");
        let json_view = serde_json::to_string(&*owned_view).expect("serialize &view");
        let json_owned = serde_json::to_string(&owned).expect("serialize owned");

        assert_eq!(json_owned_view, json_owned);
        assert_eq!(json_owned_view, json_view);
    }
}

// ── Proto2 ─────────────────────────────────────────────────────────────────

mod proto2 {
    use super::*;
    use crate::view_json_p2::__buffa::view::{AllRequiredView, OptionalsView, WithCollectionsView};
    use crate::view_json_p2::{AllRequired, Optionals, Status, Tier, WithCollections};

    #[test]
    fn required_fields_always_emitted() {
        // `required` fields are unconditionally serialized — even when the
        // value is default-equivalent.
        let json = assert_view_json_parity!(
            AllRequiredView,
            AllRequired {
                a: 0,
                b: vec![],
                c: 0,
                d: 0.0,
                e: String::new(),
                tier: Tier::BRONZE,
                ..Default::default()
            }
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.as_object().unwrap().len(),
            6,
            "all 6 required: {json}"
        );
        assert!(json.contains(r#""a":"0""#), "required int64 quoted: {json}");
        assert!(
            json.contains(r#""tier":"BRONZE""#),
            "closed enum name: {json}"
        );
    }

    #[test]
    fn required_special_values_match_owned() {
        let json = assert_view_json_parity!(
            AllRequiredView,
            AllRequired {
                a: 9007199254740993,
                b: vec![0xDE, 0xAD],
                c: u64::MAX,
                d: f64::NAN,
                e: "x".into(),
                tier: Tier::GOLD,
                ..Default::default()
            }
        );
        assert!(json.contains(r#""a":"9007199254740993""#), "int64: {json}");
        assert!(json.contains(r#""d":"NaN""#), "NaN: {json}");
    }

    #[test]
    fn optional_fields_match_owned() {
        // Proto2 `optional` is explicit presence: unset → omitted, set →
        // emitted even if default-equivalent.
        let json = assert_view_json_parity!(OptionalsView, Optionals::default());
        assert_eq!(json, "{}");

        let json = assert_view_json_parity!(
            OptionalsView,
            Optionals {
                i64: Some(0),
                by: Some(vec![]),
                f64: Some(0.0),
                s: Some(String::new()),
                tier: Some(Tier::SILVER),
                ..Default::default()
            }
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.as_object().unwrap().len(),
            5,
            "all 5 optional: {json}"
        );
    }

    #[test]
    fn collections_match_owned() {
        assert_view_json_value_parity!(
            WithCollectionsView,
            WithCollections {
                tiers: vec![Tier::GOLD, Tier::BRONZE],
                nums: vec![1, 9007199254740993],
                by_name: [("a".into(), Status::ACTIVE)].into_iter().collect(),
                blobs: [(1, vec![0xAA]), (2, vec![0xBB])].into_iter().collect(),
                ..Default::default()
            }
        );
    }
}
