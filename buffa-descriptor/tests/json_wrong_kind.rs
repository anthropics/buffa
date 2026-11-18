//! Wrong-kind JSON input at each position the reflective codec parses is
//! rejected without echoing the input in the error.

#![cfg(all(feature = "reflect", feature = "json"))]

use std::sync::Arc;

use buffa_descriptor::reflect::DynamicMessage;
use buffa_descriptor::{DescriptorPool, DynamicMessageSeed};
use serde::de::{DeserializeSeed, IntoDeserializer};

/// `json_wrong_kind_test.proto` and the well-known type files it imports.
/// Regenerate with:
/// `protoc --descriptor_set_out=buffa-descriptor/tests/protos/json_wrong_kind_test.fds
/// --include_imports -I buffa-types/protos -I buffa-descriptor/tests/protos
/// json_wrong_kind_test.proto`
const FDS_BYTES: &[u8] = include_bytes!("protos/json_wrong_kind_test.fds");

/// Text that would appear in a diagnostic that echoes one of the inputs.
const MARKERS: [&str; 5] = [
    "123456789",
    "1.5e",
    "true",
    "marker",
    "340282366920938463463374607431768211455",
];

const INT: &str = "123456789";
const FLOAT: &str = "-1.5e300";
const BOOL: &str = "true";
const STR: &str = r#""marker-secret""#;
const SEQ: &str = "[]";
const MAP: &str = "{}";

fn pool() -> Arc<DescriptorPool> {
    Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"))
}

fn assert_redacted(input: &str, text: &str) {
    for marker in MARKERS {
        assert!(
            !text.contains(marker),
            "error for {input} echoes {marker:?}: {text}"
        );
    }
}

#[test]
fn field_errors_omit_wrong_kind_input() {
    let pool = pool();
    let idx = pool.message_index("reflect.wrongkind.Fields").unwrap();
    let not_string: &[&str] = &[INT, FLOAT, BOOL, SEQ, MAP];
    let not_bool: &[&str] = &[INT, FLOAT, STR, SEQ, MAP];
    let not_object: &[&str] = &[INT, FLOAT, BOOL, STR, SEQ];
    let not_array: &[&str] = &[INT, FLOAT, BOOL, STR, MAP];
    let cases: &[(&str, &[&str])] = &[
        ("flag", not_bool),
        ("maybeFlag", not_bool),
        ("text", not_string),
        ("texts", not_array),
        ("flags", not_object),
        ("child", not_object),
        ("timestamp", not_string),
        ("duration", not_string),
        ("fieldMask", not_string),
        ("struct", not_object),
        ("list", not_array),
        ("empty", not_object),
    ];
    let mut inputs = Vec::new();
    for (field, kinds) in cases {
        for kind in *kinds {
            inputs.push(format!(r#"{{"{field}":{kind}}}"#));
        }
    }
    // Repeated elements, map values, nested fields and the top level.
    inputs.extend(
        [
            r#"{"texts":[true]}"#,
            r#"{"flags":{"k":"marker-secret"}}"#,
            r#"{"child":{"text":123456789}}"#,
        ]
        .map(String::from),
    );
    inputs.extend(not_object.iter().map(|kind| String::from(*kind)));

    for input in inputs {
        let err = DynamicMessage::from_json(Arc::clone(&pool), idx, &input).err();
        let text = err.map_or_else(|| panic!("{input} decoded"), |e| e.to_string());
        assert_redacted(&input, &text);
    }
}

#[test]
fn u128_input_is_not_echoed() {
    let pool = pool();
    for name in [
        "reflect.wrongkind.Fields",
        "google.protobuf.Timestamp",
        "google.protobuf.Duration",
        "google.protobuf.FieldMask",
        "google.protobuf.Struct",
        "google.protobuf.ListValue",
        "google.protobuf.Empty",
        "google.protobuf.Value",
    ] {
        let idx = pool.message_index(name).unwrap();
        let de = IntoDeserializer::<serde::de::value::Error>::into_deserializer(u128::MAX);
        let err = DynamicMessageSeed::new(Arc::clone(&pool), idx)
            .deserialize(de)
            .err();
        let text = err.map_or_else(|| panic!("{name} decoded"), |e| e.to_string());
        assert_redacted(name, &text);
    }
}

#[test]
fn object_forms_reject_the_number_token_key() {
    // `serde_json`'s `arbitrary_precision` feature hands a number to an
    // object visitor under this key; it is rejected as a number even when
    // unknown fields are ignored.
    let pool = pool();
    let token = r#"{"$serde_json::private::Number":"1.5"}"#;
    for name in ["reflect.wrongkind.Fields", "google.protobuf.Empty"] {
        let idx = pool.message_index(name).unwrap();
        let mut de = serde_json::Deserializer::from_str(token);
        let err = DynamicMessageSeed::new(Arc::clone(&pool), idx)
            .ignore_unknown_fields(true)
            .deserialize(&mut de)
            .unwrap_err();
        assert!(
            err.to_string().starts_with("invalid type: number"),
            "{name}: {err}"
        );
    }
}
