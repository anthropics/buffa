//! WKT integration: extern_path auto-mapping to buffa-types for
//! Timestamp/Duration/Any/Struct/wrappers/FieldMask/Api/Type/SourceContext,
//! including views.

use super::round_trip;
use buffa::Message;

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
