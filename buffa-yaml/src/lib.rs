//! YAML serialization and deserialization for buffa Protocol Buffers messages.
//!
//! This crate provides a thin carrier layer that routes buffa's existing
//! protobuf-JSON serde impls through [`serde_norway`] instead of
//! `serde_json`, giving you YAML I/O with the full protobuf JSON mapping —
//! `camelCase` and `snake_case` field names, quoted `int64`/`uint64`, base64
//! bytes, enum string names, and canonical well-known-type encodings.
//!
//! The serde impls these functions route through are the protobuf-JSON impls
//! emitted by buffa codegen, so message types must be generated with JSON
//! support enabled (`json = true`). Types generated without it do not
//! implement `Serialize`/`Deserialize` and will fail the trait bounds here.
//!
//! # Quick start
//!
//! ```
//! # fn main() -> Result<(), buffa_yaml::Error> {
//! let ts = buffa_types::Timestamp {
//!     seconds: 1_700_000_000,
//!     ..Default::default()
//! };
//! let yaml = buffa_yaml::to_string(&ts)?;
//! let decoded: buffa_types::Timestamp = buffa_yaml::from_str(&yaml)?;
//! assert_eq!(decoded, ts);
//! # Ok(())
//! # }
//! ```
//!
//! # Behavioral notes vs protoyaml-go
//!
//! This is Phase 1: "protobuf-JSON semantics on a YAML carrier." It does not
//! yet implement the lenience extensions (byte-size suffixes, Go durations,
//! field-number addressing) or snippet diagnostics. See the tracking issue for
//! the full delta table.
//!
//! Zero-copy views are supported on the encode side: [`to_string_view`] and
//! [`to_writer_view`] accept any generated `FooView<'_>` (and an
//! `OwnedView<V>` handle via `handle.reborrow()`), producing the same YAML as
//! the owned message. Decoding always targets owned message types — YAML
//! input cannot be borrowed from.
//!
//! The carrier (`serde_norway`) applies YAML 1.1 restricted scalar resolution
//! with the Norway-problem fix, so an unquoted `name: no` arrives as the
//! string `"no"`, not boolean `false`. Float specials follow the protobuf JSON
//! mapping on output (`NaN`/`Infinity`/`-Infinity` as quoted strings); on
//! input, both those strings and YAML-native `.nan`/`.inf` forms are accepted.
//!
//! # Security
//!
//! The YAML carrier expands anchors and aliases during parsing, so a small,
//! deeply-aliased document can consume disproportionate memory and CPU
//! ("billion laughs"). Bound the size of untrusted input before passing it to
//! [`from_str`], [`from_slice`], or [`from_reader`].

mod decode;
mod encode;
mod error;

pub use decode::{from_reader, from_slice, from_str};
pub use encode::{to_string, to_string_view, to_writer, to_writer_view};
pub use error::{Error, Location};

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn round_trip<M>(msg: &M) -> M
    where
        M: buffa::Message + serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let yaml = to_string(msg).expect("to_string");
        from_str(&yaml).expect("from_str")
    }

    // ── well-known types ──────────────────────────────────────────────────────

    #[test]
    fn wkt_empty_round_trip() {
        let msg = buffa_types::Empty::default();
        assert_eq!(round_trip(&msg), msg);
    }

    #[test]
    fn wkt_timestamp_round_trip() {
        use buffa_types::Timestamp;
        let ts = Timestamp {
            seconds: 1_700_000_000,
            nanos: 123_000_000,
            ..Default::default()
        };
        let yaml = to_string(&ts).expect("to_string");
        assert!(
            yaml.contains("2023-") || yaml.contains("1700"),
            "timestamp yaml: {yaml}"
        );
        let decoded: Timestamp = from_str(&yaml).expect("from_str");
        assert_eq!(decoded.seconds, ts.seconds);
        assert_eq!(decoded.nanos, ts.nanos);
    }

    #[test]
    fn wkt_duration_round_trip() {
        use buffa_types::Duration;
        let dur = Duration {
            seconds: 90,
            nanos: 500_000_000,
            ..Default::default()
        };
        assert_eq!(round_trip(&dur), dur);
    }

    #[test]
    fn wkt_field_mask_round_trip() {
        use buffa_types::FieldMask;
        let fm = FieldMask {
            paths: vec!["foo.bar".into(), "baz".into()],
            ..Default::default()
        };
        assert_eq!(round_trip(&fm), fm);
    }

    #[test]
    fn wkt_value_null_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value {
            kind: Some(Kind::NullValue(Default::default())),
            ..Default::default()
        };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_value_bool_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value {
            kind: Some(Kind::BoolValue(true)),
            ..Default::default()
        };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_value_number_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value {
            kind: Some(Kind::NumberValue(1.5)),
            ..Default::default()
        };
        let decoded: Value = round_trip(&val);
        if let Some(Kind::NumberValue(n)) = decoded.kind {
            assert!((n - 1.5).abs() < 1e-10);
        } else {
            panic!("expected NumberValue, got {:?}", decoded.kind);
        }
    }

    #[test]
    fn wkt_value_string_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value {
            kind: Some(Kind::StringValue("hello yaml".into())),
            ..Default::default()
        };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_list_value_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::{ListValue, Value};
        let lv = ListValue {
            values: vec![
                Value {
                    kind: Some(Kind::NumberValue(1.0)),
                    ..Default::default()
                },
                Value {
                    kind: Some(Kind::StringValue("two".into())),
                    ..Default::default()
                },
                Value {
                    kind: Some(Kind::BoolValue(false)),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(round_trip(&lv), lv);
    }

    #[test]
    fn wkt_struct_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::{Struct, Value};
        let mut s = Struct::default();
        s.fields.insert(
            "key".into(),
            Value {
                kind: Some(Kind::NumberValue(42.0)),
                ..Default::default()
            },
        );
        assert_eq!(round_trip(&s), s);
    }

    // ── scalar edge cases ─────────────────────────────────────────────────────

    #[test]
    fn int64_quoted_string_precision() {
        use buffa_test::json_types::Scalar;
        let large = i64::MAX;
        let msg = Scalar {
            int64_val: large,
            ..Default::default()
        };
        let yaml = to_string(&msg).expect("to_string");
        // int64 must be serialized as a quoted string per proto JSON spec so
        // that the value is not lost as a YAML float. The carrier may use
        // single or double quotes — both are valid YAML string scalars.
        let quoted = yaml.contains(&format!("'{large}'")) || yaml.contains(&format!("\"{large}\""));
        assert!(quoted, "int64 not quoted in yaml: {yaml}");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert_eq!(decoded.int64_val, large);
    }

    #[test]
    fn uint64_quoted_string_precision() {
        use buffa_test::json_types::Scalar;
        let large = u64::MAX;
        let msg = Scalar {
            uint64_val: large,
            ..Default::default()
        };
        let yaml = to_string(&msg).expect("to_string");
        let quoted = yaml.contains(&format!("'{large}'")) || yaml.contains(&format!("\"{large}\""));
        assert!(quoted, "uint64 not quoted in yaml: {yaml}");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert_eq!(decoded.uint64_val, large);
    }

    #[test]
    fn double_nan_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            double_val: f64::NAN,
            ..Default::default()
        };
        let yaml = to_string(&msg).expect("to_string");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert!(decoded.double_val.is_nan());
    }

    #[test]
    fn double_inf_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            double_val: f64::INFINITY,
            ..Default::default()
        };
        let yaml = to_string(&msg).expect("to_string");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert!(decoded.double_val.is_infinite() && decoded.double_val.is_sign_positive());
    }

    #[test]
    fn bytes_base64_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            bytes_val: vec![0xDE, 0xAD, 0xBE, 0xEF],
            ..Default::default()
        };
        assert_eq!(round_trip(&msg).bytes_val, msg.bytes_val);
    }

    // ── oneof field naming ────────────────────────────────────────────────────

    #[test]
    fn oneof_round_trip() {
        use buffa_test::json_types::{__buffa::oneof::with_oneof::Value as OneofValue, WithOneof};
        let msg = WithOneof {
            value: Some(OneofValue::Text("oneof yaml".into())),
            ..Default::default()
        };
        assert_eq!(round_trip(&msg), msg);
    }

    // ── maps ──────────────────────────────────────────────────────────────────

    #[test]
    fn map_string_string_round_trip() {
        use buffa_test::json_types::WithMap;
        let mut msg = WithMap::default();
        msg.labels.insert("env".into(), "prod".into());
        msg.labels.insert("region".into(), "us-east".into());
        assert_eq!(round_trip(&msg).labels, msg.labels);
    }

    #[test]
    fn map_string_int_round_trip() {
        use buffa_test::json_types::WithMap;
        let mut msg = WithMap::default();
        msg.counts.insert("hits".into(), 42);
        msg.counts.insert("misses".into(), 7);
        assert_eq!(round_trip(&msg).counts, msg.counts);
    }

    // ── from_slice / to_writer ────────────────────────────────────────────────

    #[test]
    fn from_slice_mirrors_from_str() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            int32_val: 99,
            bool_val: true,
            ..Default::default()
        };
        let yaml_str = to_string(&msg).expect("to_string");
        let decoded_str: Scalar = from_str(&yaml_str).expect("from_str");
        let decoded_slice: Scalar = from_slice(yaml_str.as_bytes()).expect("from_slice");
        assert_eq!(decoded_str, decoded_slice);
    }

    #[test]
    fn to_writer_mirrors_to_string() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            int32_val: 7,
            string_val: "writer".into(),
            ..Default::default()
        };
        let expected = to_string(&msg).expect("to_string");
        let mut buf = Vec::new();
        to_writer(&mut buf, &msg).expect("to_writer");
        assert_eq!(String::from_utf8(buf).expect("utf8"), expected);
    }

    #[test]
    fn from_reader_mirrors_from_str() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar {
            int32_val: 42,
            ..Default::default()
        };
        let yaml_str = to_string(&msg).expect("to_string");
        let decoded_reader: Scalar = from_reader(yaml_str.as_bytes()).expect("from_reader");
        assert_eq!(decoded_reader.int32_val, 42);
    }

    // ── zero-copy views ───────────────────────────────────────────────────────

    #[test]
    fn view_to_string_matches_owned() {
        use buffa::{Message as _, MessageView as _};
        use buffa_test::view_json::{Scalars, ScalarsView};
        let msg = Scalars {
            i32: -5,
            i64: 1 << 40,
            u64: u64::MAX,
            f64: 2.5,
            b: true,
            s: "no".into(),
            by: vec![0xDE, 0xAD],
            ..Default::default()
        };
        let bytes = msg.encode_to_vec();
        let view = ScalarsView::decode_view(&bytes).expect("decode_view");
        let from_view = to_string_view(&view).expect("to_string_view");
        let from_owned = to_string(&msg).expect("to_string");
        assert_eq!(from_view, from_owned);
        // And the view's YAML decodes back to the original owned message.
        let decoded: Scalars = from_str(&from_view).expect("from_str");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn view_to_string_wkt_fields() {
        use buffa::{Message as _, MessageView as _};
        use buffa_test::view_json::{WithWkt, WithWktView};
        let msg = WithWkt {
            ts: Some(buffa_types::Timestamp {
                seconds: 1_700_000_000,
                nanos: 5,
                ..Default::default()
            })
            .into(),
            dur: Some(buffa_types::Duration {
                seconds: 90,
                ..Default::default()
            })
            .into(),
            count: Some(buffa_types::google::protobuf::Int64Value {
                value: i64::MAX,
                ..Default::default()
            })
            .into(),
            label: Some(buffa_types::google::protobuf::StringValue {
                value: "hello".into(),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        let bytes = msg.encode_to_vec();
        let view = WithWktView::decode_view(&bytes).expect("decode_view");
        let from_view = to_string_view(&view).expect("to_string_view");
        assert_eq!(from_view, to_string(&msg).expect("to_string"));
        let decoded: WithWkt = from_str(&from_view).expect("from_str");
        assert_eq!(decoded, msg);
    }

    #[test]
    fn owned_view_handle_via_reborrow() {
        use buffa::OwnedView;
        use buffa_test::view_json::{Scalars, ScalarsView};
        let msg = Scalars {
            i64: -1,
            s: "owned view".into(),
            ..Default::default()
        };
        let handle: OwnedView<ScalarsView<'static>> =
            OwnedView::from_owned(&msg).expect("from_owned");
        let yaml = to_string_view(handle.reborrow()).expect("to_string_view");
        assert_eq!(yaml, to_string(&msg).expect("to_string"));
    }

    #[test]
    fn to_writer_view_mirrors_to_string_view() {
        use buffa::{Message as _, MessageView as _};
        use buffa_test::view_json::{Scalars, ScalarsView};
        let msg = Scalars {
            u32: 7,
            ..Default::default()
        };
        let bytes = msg.encode_to_vec();
        let view = ScalarsView::decode_view(&bytes).expect("decode_view");
        let expected = to_string_view(&view).expect("to_string_view");
        let mut buf = Vec::new();
        to_writer_view(&mut buf, &view).expect("to_writer_view");
        assert_eq!(String::from_utf8(buf).expect("utf8"), expected);
    }

    // ── YAML-specific scalar resolution ──────────────────────────────────────

    #[test]
    fn yaml_carrier_scalar_resolution() {
        // Exercises serde_norway's scalar resolution for YAML-specific inputs:
        //   - "no" should arrive as a string, not bool false (Norway fix)
        //   - 0x1F should arrive as integer 31 (hex literal)
        //   - ~ should arrive as null / default
        // We use the plain Scalar message and check what survives a full round-trip.
        use buffa_test::json_types::Scalar;

        // Hex integer literals are accepted by the YAML carrier and resolve to
        // their decimal equivalents.
        let yaml = "int32Val: 0x1F\n";
        let decoded: Scalar = from_str(yaml).expect("hex int literal");
        assert_eq!(decoded.int32_val, 0x1F);

        // Null (~) produces the default value for the field.
        let yaml_null = "int32Val: ~\n";
        let decoded_null: Scalar = from_str(yaml_null).expect("null field");
        assert_eq!(decoded_null.int32_val, 0);

        // The Norway problem: an *unquoted* `no` must arrive as the string
        // "no", not be resolved to boolean false.
        let yaml_no = "stringVal: no\n";
        let decoded_no: Scalar = from_str(yaml_no).expect("unquoted 'no'");
        assert_eq!(decoded_no.string_val, "no");

        // And the serializer must emit a form that survives the round trip
        // (i.e. quote the string so a YAML 1.1 reader can't see a bool).
        let msg = Scalar {
            string_val: "no".into(),
            ..Default::default()
        };
        let yaml_out = to_string(&msg).expect("to_string");
        let round: Scalar = from_str(&yaml_out).expect("round trip");
        assert_eq!(round.string_val, "no");
    }

    #[test]
    fn yaml_native_float_specials_accepted() {
        // The protobuf JSON mapping spells float specials as quoted strings,
        // but the YAML carrier also resolves native `.nan` / `.inf` scalars to
        // f64 values, which buffa's float deserialization accepts.
        use buffa_test::json_types::Scalar;
        let decoded: Scalar = from_str("doubleVal: .nan\n").expect(".nan");
        assert!(decoded.double_val.is_nan());
        let decoded: Scalar = from_str("doubleVal: .inf\n").expect(".inf");
        assert_eq!(decoded.double_val, f64::INFINITY);
        let decoded: Scalar = from_str("doubleVal: -.inf\n").expect("-.inf");
        assert_eq!(decoded.double_val, f64::NEG_INFINITY);
    }

    // ── error location ────────────────────────────────────────────────────────

    #[test]
    fn error_exposes_location() {
        use buffa_test::json_types::Scalar;
        // Feed deliberately malformed YAML (invalid indented mapping value).
        let bad_yaml = "int32Val: [\n  - broken";
        let err = from_str::<Scalar>(bad_yaml).expect_err("should fail");
        // The exact line/col may vary with the carrier, but a parse error in
        // the input must produce *a* location with 1-based coordinates.
        let loc = err.location().expect("parse error should carry a location");
        assert!(loc.line >= 1, "line is 1-based: {loc:?}");
        assert!(loc.column >= 1, "column is 1-based: {loc:?}");
    }
}
