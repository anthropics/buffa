//! string_type(): configurable owned string representation.
//!
//! `string_types.proto` is compiled with a broad `SmolStr` default plus
//! per-field `CompactString` (`compact`) and `EcoString` (`eco`) overrides.
//! Compiling `crate::string_types` is itself most of the test — if any decode,
//! clear, view→owned, JSON, or arbitrary path emitted the wrong type these
//! would not build. The runtime checks below verify behavior and pin the field
//! types to the configured representations.

use crate::string_types::StringContexts;
use crate::string_types::__buffa::oneof::string_contexts::Choice;
use buffa::Message;

#[test]
fn test_string_type_field_types_are_configured() {
    // Type assertions: the broad default and the per-field overrides must
    // produce exactly the configured representations. These fail to compile if
    // the wrong type was emitted.
    let m = StringContexts::default();
    let _: &::buffa::smol_str::SmolStr = &m.singular;
    let _: &::core::option::Option<::buffa::smol_str::SmolStr> = &m.maybe;
    let _: &::buffa::alloc::vec::Vec<::buffa::smol_str::SmolStr> = &m.many;
    let _: &::buffa::compact_str::CompactString = &m.compact;
    let _: &::buffa::ecow::EcoString = &m.eco;
    // Map keys/values are unaffected — always String.
    let _: &std::collections::HashMap<String, String> = &m.by_key;
}

fn sample() -> StringContexts {
    StringContexts {
        singular: "hello".into(),
        maybe: Some("nick".into()),
        many: vec!["a".into(), "b".into(), "".into()],
        compact: "compact-value".into(),
        eco: "eco-value".into(),
        by_key: [("k".to_string(), "v".to_string())].into_iter().collect(),
        choice: Some(Choice::Named("chosen".into())),
        ..Default::default()
    }
}

#[test]
fn test_string_type_binary_roundtrip() {
    let msg = sample();
    let wire = msg.encode_to_vec();
    let decoded = StringContexts::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(decoded, msg);
    assert_eq!(decoded.singular.as_str(), "hello");
    assert_eq!(decoded.compact.as_str(), "compact-value");
    assert_eq!(decoded.eco.as_str(), "eco-value");
    assert_eq!(decoded.many.len(), 3);
    match &decoded.choice {
        Some(Choice::Named(s)) => assert_eq!(s.as_str(), "chosen"),
        other => panic!("expected Choice::Named, got {other:?}"),
    }
}

#[test]
fn test_string_type_wire_compatible_with_string() {
    // Wire format must be identical to the default String representation. Build
    // a message via the configured types, decode it back, and confirm the
    // bytes round-trip exactly.
    let msg = sample();
    let wire = msg.encode_to_vec();
    let back = StringContexts::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(back.encode_to_vec(), wire);
}

#[test]
fn test_string_type_clear_resets_immutable_types() {
    // SmolStr / EcoString are immutable (no `clear()`); clear() must reset them
    // to the default value rather than calling a String-specific method.
    let mut msg = sample();
    msg.clear();
    assert!(msg.singular.is_empty());
    assert!(msg.maybe.is_none());
    assert!(msg.many.is_empty());
    assert!(msg.compact.is_empty());
    assert!(msg.eco.is_empty());
    assert!(msg.choice.is_none());
}

#[test]
fn test_string_type_view_to_owned() {
    use crate::string_types::__buffa::view::StringContextsView;
    use buffa::MessageView;
    let msg = sample();
    let wire = msg.encode_to_vec();
    let view = StringContextsView::decode_view(&wire).expect("decode_view");
    // Views always borrow &str regardless of the owned representation.
    assert_eq!(view.singular, "hello");
    assert_eq!(view.compact, "compact-value");
    let owned: StringContexts = view.to_owned_message();
    assert_eq!(owned, msg);
    // to_owned built the configured types, not String.
    let _: ::buffa::smol_str::SmolStr = owned.singular.clone();
    let _: ::buffa::compact_str::CompactString = owned.compact.clone();
    let _: ::buffa::ecow::EcoString = owned.eco.clone();
}

#[test]
fn test_string_type_json_roundtrip() {
    let msg = sample();
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(json.contains(r#""singular":"hello""#), "{json}");
    assert!(json.contains(r#""many":["a","b",""]"#), "{json}");
    assert!(json.contains(r#""named":"chosen""#), "{json}");
    let back: StringContexts = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, msg);
}

#[test]
fn test_string_type_json_null_is_empty() {
    let json = r#"{"singular":null,"maybe":null,"many":null,"compact":null,"eco":null}"#;
    let back: StringContexts = serde_json::from_str(json).expect("deserialize nulls");
    assert!(back.singular.is_empty());
    assert!(back.maybe.is_none());
    assert!(back.many.is_empty());
    assert!(back.compact.is_empty());
    assert!(back.eco.is_empty());
}
