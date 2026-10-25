//! WKT integration: extern_path auto-mapping to buffa-types for
//! Timestamp/Duration/Any/Struct/wrappers/FieldMask/Api/Type/SourceContext,
//! including views.

use super::round_trip;
use buffa::Message;

#[test]
fn test_message_with_timestamp_derives_eq_and_hash() {
    use crate::wkt::TimestampKey;
    use buffa_types::google::protobuf::Timestamp;
    use std::collections::HashSet;

    let key = TimestampKey {
        at: buffa::MessageField::some(Timestamp {
            seconds: 42,
            nanos: 999,
            ..Default::default()
        }),
        id: "event-1".into(),
        ..Default::default()
    };
    let decoded = round_trip(&key);
    assert_eq!(key, decoded);
    let mut set = HashSet::new();
    assert!(set.insert(key));
    assert!(!set.insert(decoded));
}

#[test]
fn numeric_token_reflected_wkt_matches_typed_shapes() {
    use buffa_descriptor::reflect::DynamicMessage;
    use buffa_types::google::protobuf::{descriptor_pool, ListValue, Struct, Value};

    fn compare<T: Message + serde::de::DeserializeOwned>(name: &str, input: &str) {
        let typed: T = serde_json::from_str(input).unwrap();
        let pool = descriptor_pool();
        let index = pool.message_index(name).unwrap();
        let reflected =
            DynamicMessage::from_json(std::sync::Arc::clone(pool), index, input).unwrap();
        assert_eq!(
            reflected.encode_to_vec(),
            typed.encode_to_vec(),
            "{name}: {input}"
        );
    }

    for input in [
        "1e1",
        "12.5",
        r#"{"values":[1e1,12.5]}"#,
        r#"{"$serde_json::private::Number":"1e1"}"#,
        r#"{"nested":{"$serde_json::private::Number":"1e1"}}"#,
        r#"[1e1,12.5,{"$serde_json::private::Number":"1e1"}]"#,
    ] {
        compare::<Value>("google.protobuf.Value", input);
    }
    assert_eq!(
        serde_json::from_str::<Value>("1e1").unwrap().as_number(),
        Some(10.0)
    );
    assert_eq!(
        serde_json::from_str::<Value>("12.5").unwrap().as_number(),
        Some(12.5)
    );
    for input in [
        r#"{"value":1e1}"#,
        r#"{"value":[12.5,{"$serde_json::private::Number":"1e1"}]}"#,
    ] {
        compare::<Struct>("google.protobuf.Struct", input);
    }
    for input in ["[1e1,12.5]", r#"[{"$serde_json::private::Number":"1e1"}]"#] {
        compare::<ListValue>("google.protobuf.ListValue", input);
    }
    let pool = descriptor_pool();
    for (name, input) in [
        ("google.protobuf.Value", "1e400"),
        ("google.protobuf.Struct", r#"{"value":1e400}"#),
        ("google.protobuf.ListValue", "[1e400]"),
    ] {
        let index = pool.message_index(name).unwrap();
        assert!(DynamicMessage::from_json(std::sync::Arc::clone(pool), index, input).is_err());
    }
    assert!(serde_json::from_str::<Value>("1e400").is_err());
    assert!(serde_json::from_str::<Struct>(r#"{"value":1e400}"#).is_err());
    assert!(serde_json::from_str::<ListValue>("[1e400]").is_err());
}

#[test]
fn test_wkt_in_oneof_from_impls() {
    // Orphan-rule regression: From<T> for Option<Oneof> must only be generated
    // for local T. Extern-crate T (WKTs via ::buffa_types) would be E0117
    // because Option is foreign and not fundamental — no local type in the
    // impl header. From<T> for Oneof is fine either way (Oneof is local).
    //
    // The fact that this test compiles IS the test: wkt_usage.proto has
    // Envelope.content with Any/Timestamp (extern) + Event (local) variants.
    use crate::wkt::__buffa::oneof::envelope;
    use crate::wkt::{Envelope, Event};
    use buffa_types::google::protobuf::Any;

    // Local variant: both From impls exist.
    let env = Envelope {
        content: Event::default().into(),
        ..Default::default()
    };
    assert!(matches!(
        env.content,
        Some(envelope::Content::EventContent(_))
    ));

    // Extern variant: only From<T> for Content exists — Some() is explicit.
    let env = Envelope {
        content: Some(envelope::Content::from(Any::default())),
        ..Default::default()
    };
    assert!(matches!(
        env.content,
        Some(envelope::Content::AnyContent(_))
    ));

    // The From<T> for Option<Content> impl for extern T does NOT exist.
    // The following would not compile (uncomment to verify):
    // let _: Option<envelope::Content> = Any::default().into();

    let decoded = round_trip(&env);
    assert!(matches!(
        decoded.content,
        Some(envelope::Content::AnyContent(_))
    ));
}

#[test]
fn test_wkt_event_round_trip() {
    use crate::wkt::Event;
    let msg = Event {
        created_at: buffa::MessageField::some(buffa_types::google::protobuf::Timestamp {
            seconds: 1_700_000_000,
            nanos: 500_000_000,
            ..Default::default()
        }),
        ttl: buffa::MessageField::some(buffa_types::google::protobuf::Duration {
            seconds: 3600,
            ..Default::default()
        }),
        priority: buffa::MessageField::some(buffa_types::google::protobuf::Int32Value {
            value: 5,
            ..Default::default()
        }),
        description: buffa::MessageField::some(buffa_types::google::protobuf::StringValue {
            value: "test event".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.created_at.seconds, 1_700_000_000);
    assert_eq!(decoded.created_at.nanos, 500_000_000);
    assert_eq!(decoded.ttl.seconds, 3600);
    assert_eq!(decoded.priority.value, 5);
    assert_eq!(decoded.description.value, "test event");
}

#[test]
fn test_wkt_view_with_extern_path() {
    // Views for messages with WKT fields (extern_path'd to buffa-types)
    // must resolve to buffa_types::XxxView types. owned_to_view_ty_tokens
    // appends "View" to the last path segment: crate::wkt::Timestamp
    // → crate::wkt::TimestampView (which is re-exported from buffa-types'
    // generated code).
    use crate::wkt::__buffa::view::EventView;
    use crate::wkt::Event;
    use buffa::MessageView;
    let msg = Event {
        created_at: buffa::MessageField::some(buffa_types::google::protobuf::Timestamp {
            seconds: 42,
            nanos: 999,
            ..Default::default()
        }),
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    let view = EventView::decode_view(&wire).expect("decode_view");
    // Deref chain through MessageFieldView<TimestampView<'_>>.
    assert_eq!(view.created_at.seconds, 42);
    assert_eq!(view.created_at.nanos, 999);
    // Full round-trip.
    assert_eq!(view.to_owned_message().unwrap().encode_to_vec(), wire);
}

#[test]
fn test_wkt_api_types_round_trip() {
    use crate::wkt_api::Catalog;
    let msg = Catalog {
        api: buffa::MessageField::some(buffa_types::google::protobuf::Api {
            name: "google.example.v1.Example".into(),
            version: "1.0".into(),
            ..Default::default()
        }),
        type_info: buffa::MessageField::some(buffa_types::google::protobuf::Type {
            name: "google.example.v1.Msg".into(),
            ..Default::default()
        }),
        source_context: buffa::MessageField::some(buffa_types::google::protobuf::SourceContext {
            file_name: "google/example/v1/example.proto".into(),
            ..Default::default()
        }),
        enum_type: buffa::MessageField::some(buffa_types::google::protobuf::Enum {
            name: "google.example.v1.Color".into(),
            enumvalue: vec![buffa_types::google::protobuf::EnumValue {
                name: "COLOR_RED".into(),
                number: 1,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.api.name, "google.example.v1.Example");
    assert_eq!(decoded.type_info.name, "google.example.v1.Msg");
    assert_eq!(decoded.enum_type.name, "google.example.v1.Color");
    assert_eq!(decoded.enum_type.enumvalue[0].number, 1);
    assert_eq!(
        decoded.source_context.file_name,
        "google/example/v1/example.proto"
    );
}
