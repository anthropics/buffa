//! Custom-owned-types example: demonstrates pluggable owned-type support.
//!
//! This example shows how to use custom owned types like SmolStr (for efficient
//! short-string storage) in place of the default String/Vec/HashMap.
//!
//! While this example uses well-known types from buffa-types, the same pattern
//! applies to custom protocols: configure buffa_build in build.rs with:
//!
//! ```ignore
//! buffa_build::Config::new()
//!     .files(&["proto/my_message.proto"])
//!     .string_type_custom("::buffa_smolstr::SmolStr")  // Use SmolStr instead of String
//!     .compile()?;
//! ```
//!
//! Then your generated message types automatically use SmolStr for all string
//! fields—no other changes needed.
//!
//! Usage:
//!   cargo run -p example-custom-types

use buffa::Message;
use buffa_types::{Struct, Value};

fn main() {
    println!("=== Custom-Owned-Types Example ===\n");

    // Demonstrate using buffa-types (which are pre-generated with JSON support).
    // The same pattern applies to custom-compiled protos with custom types.

    // Build a Struct with string keys (demonstrating dynamic field handling).
    let mut config = Struct::default();

    // Values are dynamically typed — demonstrate various types.
    config.fields.insert(
        "service_name".into(),
        Value {
            kind: Some(buffa_types::google::protobuf::__buffa::oneof::value::Kind::StringValue(
                "my-service".into(),
            )),
            ..Default::default()
        },
    );

    config.fields.insert(
        "port".into(),
        Value {
            kind: Some(buffa_types::google::protobuf::__buffa::oneof::value::Kind::NumberValue(
                8080.0,
            )),
            ..Default::default()
        },
    );

    config.fields.insert(
        "enabled".into(),
        Value {
            kind: Some(buffa_types::google::protobuf::__buffa::oneof::value::Kind::BoolValue(
                true,
            )),
            ..Default::default()
        },
    );

    println!("=== Original Struct ===");
    println!("Fields: {}", config.fields.len());
    for (key, value) in &config.fields {
        println!("  {}: {:?}", key, value.kind);
    }

    // Encode to binary.
    let encoded = config.encode_to_vec();
    println!("\n=== Encoded (binary) ===");
    println!("Size: {} bytes", encoded.len());

    // Decode from binary — demonstrates round-trip with owned types.
    let decoded: Struct = buffa::Message::decode_from_slice(&encoded)
        .expect("decode from binary failed");
    assert_eq!(decoded.fields.len(), config.fields.len());
    println!("\n=== Decoded (binary) ===");
    println!("Fields: {}", decoded.fields.len());

    // Serialize to JSON.
    let json_str = serde_json::to_string_pretty(&config)
        .expect("JSON serialization failed");
    println!("\n=== Serialized (JSON) ===");
    println!("{}", json_str);

    // Deserialize from JSON — demonstrates serde round-trip.
    let from_json: Struct = serde_json::from_str(&json_str)
        .expect("JSON deserialization failed");
    assert_eq!(from_json.fields.len(), config.fields.len());
    println!("\n=== Deserialized (JSON) ===");
    println!("Fields: {}", from_json.fields.len());

    println!("\n✓ Round-trips (binary and JSON) successful!");
    println!("\nNote: This example uses pre-generated buffa-types.");
    println!("For custom protocols with custom types, use buffa_build");
    println!("to specify custom type choices in build.rs.");
}
