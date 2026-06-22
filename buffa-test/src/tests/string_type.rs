//! string_type(): configurable owned string representation.
//!
//! `string_types.proto` is compiled with per-field custom `ProtoString` types:
//! the `buffa_smolstr::SmolStr` preset crate (`singular`/`maybe`/`named`) and
//! the crate-local `reprs::CompactStr` (`compact`) and `reprs::EcoStr` (`eco`)
//! newtypes (the foreign types no longer implement `ProtoString` directly — they
//! must be wrapped). The repeated `many` field stays `String` (a custom repeated
//! element must be crate-local; that case is covered by `vtable_string_repr`).
//! Compiling `crate::string_types` is itself most of the test — if any decode,
//! clear, view→owned, JSON, or arbitrary path emitted the wrong type these
//! would not build. The runtime checks below verify behavior and pin the field
//! types to the configured representations.

use crate::string_types::__buffa::oneof::string_contexts::Choice;
use crate::string_types::StringContexts;
use buffa::Message;

#[test]
fn test_string_type_field_types_are_configured() {
    // Type assertions: the broad default and the per-field overrides must
    // produce exactly the configured representations. These fail to compile if
    // the wrong type was emitted.
    let m = StringContexts::default();
    let _: &::buffa_smolstr::SmolStr = &m.singular;
    let _: &::core::option::Option<::buffa_smolstr::SmolStr> = &m.maybe;
    // `many` is repeated → stays default `String` (see module docs).
    let _: &::buffa::alloc::vec::Vec<::buffa::alloc::string::String> = &m.many;
    let _: &crate::reprs::CompactStr = &m.compact;
    let _: &crate::reprs::EcoStr = &m.eco;
    // Map keys/values are unaffected — always String.
    let _: &buffa::Map<String, String> = &m.by_key;
    // The oneof string variant payload must also honor the configured repr.
    let _: ::buffa_smolstr::SmolStr = match Choice::Named("x".into()) {
        Choice::Named(s) => s,
        Choice::Count(_) => unreachable!(),
    };
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
    let owned: StringContexts = view.to_owned_message().unwrap();
    assert_eq!(owned, msg);
    // to_owned built the configured types, not String.
    let _: ::buffa_smolstr::SmolStr = owned.singular.clone();
    let _: crate::reprs::CompactStr = owned.compact.clone();
    let _: crate::reprs::EcoStr = owned.eco.clone();
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
fn test_string_type_text_roundtrip() {
    // generate_text(true) is enabled for this module; the text decoder builds
    // each configured string type via From<String> rather than a bare String.
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    let back: StringContexts = buffa::text::decode_from_str(&text).expect("parse text");
    assert_eq!(back, msg);
    assert_eq!(back.singular.as_str(), "hello");
    assert_eq!(back.eco.as_str(), "eco-value");
    match &back.choice {
        Some(Choice::Named(s)) => assert_eq!(s.as_str(), "chosen"),
        other => panic!("expected Choice::Named, got {other:?}"),
    }
}

#[test]
fn test_string_type_proto2_default() {
    // proto2 `[default = "anonymous"]` on a required (bare) string field, with
    // string_type(SmolStr): both Default and clear() must yield the literal as
    // a SmolStr, not a String.
    use crate::string_proto2::Defaults;
    let d = Defaults::default();
    assert_eq!(d.name.as_str(), "anonymous");
    let _: ::buffa_smolstr::SmolStr = d.name.clone();

    let mut m = Defaults {
        name: "custom".into(),
        ..Default::default()
    };
    m.clear();
    assert_eq!(m.name.as_str(), "anonymous", "clear() restores the default");
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
