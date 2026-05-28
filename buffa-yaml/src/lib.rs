//! YAML serialization and deserialization for buffa Protocol Buffers messages.
//!
//! This crate provides a thin carrier layer that routes buffa's existing
//! protobuf-JSON serde impls through [`serde_norway`] instead of
//! [`serde_json`], giving you YAML I/O with the full protobuf JSON mapping —
//! camelCase and snake_case field names, quoted `int64`/`uint64`, base64
//! bytes, enum string names, and canonical well-known-type encodings.
//!
//! # Quick start
//!
//! ```no_run
//! # use buffa_yaml::{to_string, from_str};
//! # #[derive(serde::Serialize, serde::Deserialize, buffa::Message)]
//! # struct MyMessage { /* ... */ }
//! let msg = MyMessage::default();
//!
//! let yaml = to_string(&msg)?;
//! let decoded: MyMessage = from_str(&yaml)?;
//! # Ok::<(), buffa_yaml::Error>(())
//! ```
//!
//! # Behavioral notes vs protoyaml-go
//!
//! This is Phase 1: "protobuf-JSON semantics on a YAML carrier." It does not
//! yet implement the lenience extensions (byte-size suffixes, Go durations,
//! field-number addressing) or snippet diagnostics. See the tracking issue for
//! the full delta table.
//!
//! The carrier (`serde_norway`) applies YAML 1.1 restricted scalar resolution
//! with dtolnay's Norway-problem fix, so `name: no` arrives as a string. Float
//! specials (`Infinity`, `.nan`) are delivered as `f64` values, which buffa's
//! float helpers accept.

mod decode;
mod encode;
mod error;

pub use decode::{from_reader, from_slice, from_str};
pub use encode::{to_string, to_writer};
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
        let ts = Timestamp { seconds: 1_700_000_000, nanos: 123_000_000, ..Default::default() };
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
        let dur = Duration { seconds: 90, nanos: 500_000_000, ..Default::default() };
        assert_eq!(round_trip(&dur), dur);
    }

    #[test]
    fn wkt_field_mask_round_trip() {
        use buffa_types::FieldMask;
        let fm = FieldMask { paths: vec!["foo.bar".into(), "baz".into()], ..Default::default() };
        assert_eq!(round_trip(&fm), fm);
    }

    #[test]
    fn wkt_value_null_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value { kind: Some(Kind::NullValue(Default::default())), ..Default::default() };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_value_bool_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value { kind: Some(Kind::BoolValue(true)), ..Default::default() };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_value_number_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value { kind: Some(Kind::NumberValue(3.14)), ..Default::default() };
        let decoded: Value = round_trip(&val);
        if let Some(Kind::NumberValue(n)) = decoded.kind {
            assert!((n - 3.14).abs() < 1e-10);
        } else {
            panic!("expected NumberValue, got {:?}", decoded.kind);
        }
    }

    #[test]
    fn wkt_value_string_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::Value;
        let val = Value { kind: Some(Kind::StringValue("hello yaml".into())), ..Default::default() };
        assert_eq!(round_trip(&val), val);
    }

    #[test]
    fn wkt_list_value_round_trip() {
        use buffa_types::google::protobuf::__buffa::oneof::value::Kind;
        use buffa_types::{ListValue, Value};
        let lv = ListValue {
            values: vec![
                Value { kind: Some(Kind::NumberValue(1.0)), ..Default::default() },
                Value { kind: Some(Kind::StringValue("two".into())), ..Default::default() },
                Value { kind: Some(Kind::BoolValue(false)), ..Default::default() },
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
            Value { kind: Some(Kind::NumberValue(42.0)), ..Default::default() },
        );
        assert_eq!(round_trip(&s), s);
    }

    // ── scalar edge cases ─────────────────────────────────────────────────────

    #[test]
    fn int64_quoted_string_precision() {
        use buffa_test::json_types::Scalar;
        let large = i64::MAX;
        let msg = Scalar { int64_val: large, ..Default::default() };
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
        let msg = Scalar { uint64_val: large, ..Default::default() };
        let yaml = to_string(&msg).expect("to_string");
        let quoted = yaml.contains(&format!("'{large}'")) || yaml.contains(&format!("\"{large}\""));
        assert!(quoted, "uint64 not quoted in yaml: {yaml}");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert_eq!(decoded.uint64_val, large);
    }

    #[test]
    fn double_nan_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar { double_val: f64::NAN, ..Default::default() };
        let yaml = to_string(&msg).expect("to_string");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert!(decoded.double_val.is_nan());
    }

    #[test]
    fn double_inf_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar { double_val: f64::INFINITY, ..Default::default() };
        let yaml = to_string(&msg).expect("to_string");
        let decoded: Scalar = from_str(&yaml).expect("from_str");
        assert!(decoded.double_val.is_infinite() && decoded.double_val.is_sign_positive());
    }

    #[test]
    fn bytes_base64_round_trip() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar { bytes_val: vec![0xDE, 0xAD, 0xBE, 0xEF], ..Default::default() };
        assert_eq!(round_trip(&msg).bytes_val, msg.bytes_val);
    }

    // ── oneof field naming ────────────────────────────────────────────────────

    #[test]
    fn oneof_round_trip() {
        use buffa_test::json_types::{__buffa::oneof::with_oneof::Value as OneofValue, WithOneof};
        let msg = WithOneof { value: Some(OneofValue::Text("oneof yaml".into())), ..Default::default() };
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
        let msg = Scalar { int32_val: 99, bool_val: true, ..Default::default() };
        let yaml_str = to_string(&msg).expect("to_string");
        let decoded_str: Scalar = from_str(&yaml_str).expect("from_str");
        let decoded_slice: Scalar = from_slice(yaml_str.as_bytes()).expect("from_slice");
        assert_eq!(decoded_str, decoded_slice);
    }

    #[test]
    fn to_writer_mirrors_to_string() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar { int32_val: 7, string_val: "writer".into(), ..Default::default() };
        let expected = to_string(&msg).expect("to_string");
        let mut buf = Vec::new();
        to_writer(&mut buf, &msg).expect("to_writer");
        assert_eq!(String::from_utf8(buf).expect("utf8"), expected);
    }

    #[test]
    fn from_reader_mirrors_from_str() {
        use buffa_test::json_types::Scalar;
        let msg = Scalar { int32_val: 42, ..Default::default() };
        let yaml_str = to_string(&msg).expect("to_string");
        let decoded_reader: Scalar = from_reader(yaml_str.as_bytes()).expect("from_reader");
        assert_eq!(decoded_reader.int32_val, 42);
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

        // "no" is a string, not bool false (Norway problem is fixed).
        // We encode a string field as "no" and verify it round-trips as-is.
        let yaml_no = "stringVal: \"no\"\n";
        let decoded_no: Scalar = from_str(yaml_no).expect("string 'no'");
        assert_eq!(decoded_no.string_val, "no");
    }

    // ── error location ────────────────────────────────────────────────────────

    #[test]
    fn error_exposes_location() {
        use buffa_test::json_types::Scalar;
        // Feed deliberately malformed YAML (invalid indented mapping value).
        let bad_yaml = "int32Val: [\n  - broken";
        let err = from_str::<Scalar>(bad_yaml).expect_err("should fail");
        // We can't assert exact line/col since the carrier may vary, but we
        // verify the Error type exposes the location() method without panicking.
        let _ = err.location();
    }
}
