//! `DynamicMessage` reads `serde_json`'s private `RawValue` key as data when
//! it buffers a `google.protobuf.Any`. See `buffa::json_helpers::buffered`
//! for what the key does. `raw_value` is a dev-dependency feature of this
//! crate, so these tests run with it on.
//!
//! Run standalone with (the file compiles to nothing without the features):
//! `cargo test -p buffa-descriptor --features reflect,json --test json_private_key`

#![cfg(all(feature = "reflect", feature = "json", feature = "std"))]

use std::sync::Arc;

use buffa_descriptor::reflect::DynamicMessage;
use buffa_descriptor::DescriptorPool;
use serde_json::json;

/// `descriptor.proto` + `any.proto` + `reflect.opt.Envelope { Any payload = 1; }`
/// + `reflect.opt.Annotated { string email = 1; int32 id = 2; }`.
const FDS_BYTES: &[u8] = include_bytes!("protos/reflect_test_options.fds");

const RAW_VALUE_KEY: &str = "$serde_json::private::RawValue";
const ANNOTATED_URL: &str = "type.googleapis.com/reflect.opt.Annotated";

fn envelope_from_json(json: &serde_json::Value) -> Result<DynamicMessage, serde_json::Error> {
    let pool = Arc::new(DescriptorPool::decode(FDS_BYTES).expect("pool builds from protoc FDS"));
    let idx = pool.message_index("reflect.opt.Envelope").unwrap();
    DynamicMessage::from_json(pool, idx, &json.to_string())
}

#[test]
fn the_test_build_has_serde_json_raw_value_enabled() {
    let text = json!({ RAW_VALUE_KEY: "[1]" }).to_string();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value, json!([1]));
}

#[test]
fn a_field_of_an_any_payload_is_not_read_from_the_private_key() {
    let plain = json!({ "payload": { "@type": ANNOTATED_URL, "id": 7 } });
    envelope_from_json(&plain).expect("the plain payload decodes");

    let hidden = json!({ "payload": { "@type": ANNOTATED_URL, "id": { RAW_VALUE_KEY: "7" } } });
    envelope_from_json(&hidden)
        .map(|_| ())
        .expect_err("an object is not an int32");
}

#[test]
fn a_string_under_the_private_key_is_not_parsed() {
    // Parsing the string fails the recursion limit. As data it is an object
    // where `id` takes a number.
    let deep = format!("{}0{}", "[".repeat(200), "]".repeat(200));
    let json = json!({ "payload": { "@type": ANNOTATED_URL, "id": { RAW_VALUE_KEY: deep } } });
    let err = envelope_from_json(&json).map(|_| ()).unwrap_err();
    assert!(!err.to_string().contains("recursion limit"), "{err}");
    assert!(err.to_string().contains("invalid type: map"), "{err}");
}
