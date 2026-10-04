//! `(buffa.ext.field).name`: the option sets the Rust name of a field in
//! `protos/ext_name.proto`. Compiling these field accesses is half the test.
//! The other half checks that the wire format, JSON, text format and
//! reflection still use the proto names.

use super::round_trip;
use crate::ext_name::renamed::{Body, Inner};
use crate::ext_name::{Renamed, RenamedView};
use buffa::{ExtensionSet, Message, MessageView};
use buffa_descriptor::reflect::{ReflectMessage, ValueRef};

fn sample() -> Renamed {
    let mut msg = Renamed {
        kind: Some("note".into()),
        this: 7,
        labels: vec!["a".into(), "b".into()],
        child: buffa::MessageField::some(Inner {
            r#type: Some("inner".into()),
            ..Default::default()
        }),
        plain: Some("untouched".into()),
        body: Some(Body::PlainText("hello".into())),
        ..Default::default()
    };
    msg.tallies.insert("x".into(), 3);
    msg
}

#[test]
fn renamed_fields_round_trip() {
    let decoded = round_trip(&sample());
    assert_eq!(decoded, sample());
    assert_eq!(decoded.kind.as_deref(), Some("note"));
    assert_eq!(decoded.this, 7);
    assert_eq!(decoded.labels, ["a", "b"]);
    assert_eq!(decoded.tallies["x"], 3);
}

#[test]
fn same_name_and_number_in_another_message_keeps_its_name() {
    // `Inner.type = 1` has no option, so it is still the raw identifier.
    let decoded = round_trip(&sample());
    assert_eq!(decoded.child.r#type.as_deref(), Some("inner"));
}

#[test]
fn setter_follows_the_option() {
    let msg = Renamed::default().with_kind("memo");
    assert_eq!(msg.kind.as_deref(), Some("memo"));
}

#[test]
fn variants_follow_the_option() {
    let mut msg = sample();
    assert_eq!(round_trip(&msg).body, Some(Body::PlainText("hello".into())));
    // The option's value is `raw_bytes`, and the variant keeps that case.
    msg.body = Some(Body::raw_bytes(vec![1, 2]));
    assert_eq!(round_trip(&msg).body, Some(Body::raw_bytes(vec![1, 2])));
    // A member without the option keeps the variant from its proto name.
    msg.body = Some(Body::Nested(Box::default()));
    assert_eq!(round_trip(&msg).body, msg.body);
}

#[test]
fn from_impls_resolve_beside_a_variant_named_from() {
    // `Body::from(..)` is the variant here, so the conversions go through
    // `Into`.
    let inner = Inner {
        r#type: Some("inner".into()),
        ..Default::default()
    };
    let body: Body = inner.clone().into();
    assert_eq!(body, Body::Nested(Box::new(inner.clone())));
    let body: Option<Body> = inner.into();
    assert!(matches!(body, Some(Body::Nested(_))));

    let mut msg = sample();
    msg.body = Some(Body::from("here".into()));
    assert_eq!(round_trip(&msg).body, msg.body);
}

#[test]
fn snake_case_variant_works_in_views_json_and_text() {
    use crate::ext_name::__buffa::view::oneof::renamed::Body as BodyView;

    let mut msg = sample();
    msg.body = Some(Body::raw_bytes(vec![1, 2]));

    let bytes = msg.encode_to_vec();
    let view = RenamedView::decode_view(&bytes).unwrap();
    assert!(matches!(view.body, Some(BodyView::raw_bytes([1, 2]))));

    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(json["raw"], serde_json::json!("AQI="));
    let back: Renamed = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);

    let text = buffa::text::encode_to_string(&msg);
    assert!(text.contains("raw: "), "{text}");
    let back: Renamed = buffa::text::decode_from_str(&text).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn view_fields_follow_the_option() {
    use crate::ext_name::__buffa::view::oneof::renamed::Body as BodyView;

    let bytes = sample().encode_to_vec();
    let view = RenamedView::decode_view(&bytes).unwrap();
    assert_eq!(view.kind, Some("note"));
    assert_eq!(view.this, 7);
    assert!(view.has_this());
    assert_eq!(view.labels.first().copied(), Some("a"));
    assert_eq!(view.child.r#type, Some("inner"));
    match &view.body {
        Some(BodyView::PlainText(text)) => assert_eq!(*text, "hello"),
        other => panic!("unexpected oneof: {other:?}"),
    }
}

#[test]
fn lazy_view_fields_follow_the_option() {
    use crate::ext_name::__buffa::lazy_view::RenamedLazyView;
    use buffa::view::LazyMessageView;

    let bytes = sample().encode_to_vec();
    let view = RenamedLazyView::decode_lazy(&bytes).unwrap();
    assert_eq!(view.kind, Some("note"));
    assert_eq!(view.this, 7);
    let child = view.child.get().unwrap().expect("child is set");
    assert_eq!(child.r#type, Some("inner"));
    assert_eq!(view.to_owned_message().unwrap(), sample());
}

#[cfg(has_table_codec)]
#[test]
fn table_codec_fields_follow_the_option() {
    use crate::ext_name_table::{Flat, Leaf};

    let msg = Flat {
        kind: Some("note".into()),
        hits: vec![1, 2, 3],
        child: buffa::MessageField::some(Leaf {
            value: Some(9),
            ..Default::default()
        }),
        plain: Some("untouched".into()),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded, msg);
    assert_eq!(decoded.kind.as_deref(), Some("note"));
    assert_eq!(decoded.hits, [1, 2, 3]);
    assert_eq!(decoded.child.value, Some(9));
}

#[test]
fn json_keeps_the_proto_names() {
    let msg = sample();
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "type": "note",
            "self": 7,
            "tags": ["a", "b"],
            "counts": {"x": 3},
            "inner": {"type": "inner"},
            "plain": "untouched",
            "text": "hello",
        })
    );
    let back: Renamed = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn text_format_keeps_the_proto_names() {
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    for line in [
        "type: \"note\"",
        "self: 7",
        "tags: \"a\"",
        "text: \"hello\"",
    ] {
        assert!(text.contains(line), "missing `{line}` in {text}");
    }
    assert!(!text.contains("kind"), "{text}");
    let back: Renamed = buffa::text::decode_from_str(&text).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn reflection_keeps_the_proto_names() {
    let msg = sample();
    let reflected: &dyn ReflectMessage = &msg;
    let descriptor = reflected.message_descriptor();
    assert!(descriptor.field_by_name("kind").is_none());
    let field = descriptor.field_by_name("type").unwrap();
    assert!(matches!(reflected.get(field), ValueRef::String("note")));
    // The option is in the embedded descriptor, so a tool that reads the
    // descriptor can find the Rust name.
    let option = field
        .options()
        .unwrap()
        .extension(&buffa_proto_options::FIELD)
        .unwrap();
    assert_eq!(option.name.as_deref(), Some("kind"));
}
