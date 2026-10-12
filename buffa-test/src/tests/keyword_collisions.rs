//! Members that share a name after keyword escaping: the keyword member
//! takes a second underscore. That `protos/keyword_collisions.proto` compiles
//! is half the test; `build.rs` says which impls it is generated with.

use super::round_trip;
use crate::keyword_collisions::{
    field_beside_oneof, keyword_beside_oneof, required_and_defaults, FieldBesideOneof, FieldPair,
    FieldPairOwnedView, KeywordBesideOneof, RequiredAndDefaults, RequiredAndDefaultsView,
    SameField,
};
use buffa::MessageView;

fn field_pair() -> FieldPair {
    let mut msg = FieldPair {
        self__: Some(1),
        self_: Some("two".into()),
        super__: vec![3, 4],
        super_: Some(vec![5]),
        crate_: Some(true),
        Self__: buffa::MessageField::some(FieldPair {
            self__: Some(7),
            ..Default::default()
        }),
        Self_: Some(8),
        ..Default::default()
    };
    msg.crate__.insert("k".into(), 6);
    msg
}

#[test]
fn each_member_of_a_pair_keeps_its_own_value() {
    let decoded = round_trip(&field_pair());
    assert_eq!(decoded, field_pair());
    assert_eq!(decoded.self__, Some(1));
    assert_eq!(decoded.self_.as_deref(), Some("two"));
    assert_eq!(decoded.Self__.self__, Some(7));
    assert_eq!(decoded.Self_, Some(8));
}

#[test]
fn wire_numbers_follow_the_proto_fields() {
    // `self = 1` alone: tag 0x08, value 1.
    let only_keyword = FieldPair {
        self__: Some(1),
        ..Default::default()
    };
    assert_eq!(buffa::Message::encode_to_vec(&only_keyword), [0x08, 0x01]);
    // `self_ = 2` alone: tag 0x12, length 1.
    let only_authored = FieldPair {
        self_: Some("x".into()),
        ..Default::default()
    };
    assert_eq!(
        buffa::Message::encode_to_vec(&only_authored),
        [0x12, 0x01, b'x']
    );
}

#[test]
fn json_and_text_keep_the_proto_names() {
    let msg = FieldPair {
        self__: Some(1),
        self_: Some("two".into()),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"self":1,"selfUnderscore":"two"}"#);
    assert_eq!(serde_json::from_str::<FieldPair>(&json).unwrap(), msg);

    let text = buffa::text::encode_to_string(&msg);
    assert_eq!(text, r#"self: 1 self_: "two""#);
    assert_eq!(
        buffa::text::decode_from_str::<FieldPair>(&text).unwrap(),
        msg
    );
}

#[test]
fn views_read_the_renamed_members() {
    let bytes = buffa::Message::encode_to_vec(&field_pair());
    let view = crate::keyword_collisions::FieldPairView::decode_view(&bytes).unwrap();
    assert_eq!(view.self__, Some(1));
    assert_eq!(view.self_, Some("two"));
    assert_eq!(view.to_owned_message().unwrap(), field_pair());
}

#[test]
fn oneof_next_to_a_field_with_its_escaped_name() {
    let msg = FieldBesideOneof {
        self_: Some("field".into()),
        self__: Some(field_beside_oneof::Self_::Text("oneof".into())),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.self_.as_deref(), Some("field"));
    assert_eq!(
        decoded.self__,
        Some(field_beside_oneof::Self_::Text("oneof".into()))
    );
}

#[test]
fn oneof_member_and_field_both_survive_json() {
    let msg = FieldBesideOneof {
        self_: Some("field".into()),
        self__: Some(field_beside_oneof::Self_::Number(4)),
        ..Default::default()
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"self":"field","number":4}"#);
    assert_eq!(
        serde_json::from_str::<FieldBesideOneof>(&json).unwrap(),
        msg
    );
}

#[test]
fn field_next_to_a_oneof_named_with_its_escaped_name() {
    let msg = KeywordBesideOneof {
        self__: Some(9),
        self_: Some(keyword_beside_oneof::Self_::Text("oneof".into())),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.self__, Some(9));
    assert_eq!(
        decoded.self_,
        Some(keyword_beside_oneof::Self_::Text("oneof".into()))
    );
}

#[test]
fn owned_view_accessors_use_the_renamed_members() {
    let bytes = buffa::bytes::Bytes::from(buffa::Message::encode_to_vec(&field_pair()));
    let owned = FieldPairOwnedView::decode(bytes).unwrap();
    assert_eq!(owned.self__(), Some(1));
    assert_eq!(owned.self_(), Some("two"));
}

#[test]
fn generated_default_and_presence_check_name_the_renamed_members() {
    let msg = RequiredAndDefaults::default();
    assert_eq!(msg.crate__, 5);
    assert_eq!(msg.crate_, None);
    assert_eq!(msg.super__, None);

    let bytes = buffa::Message::encode_to_vec(&RequiredAndDefaults {
        crate__: 3,
        super_: Some("field".into()),
        super__: Some(required_and_defaults::Super::Pick(8)),
        ..Default::default()
    });
    let view = RequiredAndDefaultsView::decode_view(&bytes).unwrap();
    assert!(view.has_crate__());
    assert_eq!(view.crate__, 3);
    assert_eq!(view.super_, Some("field"));
}

#[test]
fn message_with_the_same_field_gets_the_same_name() {
    let msg = SameField {
        self__: Some(9),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg).self__, Some(9));
}
