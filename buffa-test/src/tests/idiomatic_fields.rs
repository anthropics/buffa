//! Idiomatic field names (#256): `idiomatic_field_names(true)` converts the
//! camelCase proto names in `protos/idiomatic_fields.proto` to snake_case
//! Rust identifiers, while the wire format and JSON names keep the
//! originals. Compiling these field accesses is half the test; the runtime
//! half checks the encode/decode round-trip and the JSON name surface.

use super::round_trip;
use crate::idiomatic_fields::web_message_info::QuotedMessage;
use crate::idiomatic_fields::WebMessageInfo;
use buffa::MessageView;

type MessageContent = crate::idiomatic_fields::__buffa::oneof::web_message_info::MessageContent;

fn sample() -> WebMessageInfo {
    let mut msg = WebMessageInfo {
        remote_jid: "alice@s.whatsapp.net".into(),
        message_timestamp: 1_700_000_000,
        mentioned_jid: vec!["bob@s.whatsapp.net".into()],
        quoted_message: buffa::MessageField::some(QuotedMessage {
            stanza_id: "ABCD1234".into(),
            ..Default::default()
        }),
        r#type: "chat".into(),
        plain_field: "untouched".into(),
        ip_v6_address: "::1".into(),
        ..Default::default()
    };
    msg.reaction_counts.insert("👍".into(), 3);
    msg.message_content = Some(MessageContent::TextBody("hello".into()));
    msg
}

#[test]
fn snake_case_round_trip() {
    let msg = sample();
    let decoded = round_trip(&msg);
    assert_eq!(decoded.remote_jid, "alice@s.whatsapp.net");
    assert_eq!(decoded.message_timestamp, 1_700_000_000);
    assert_eq!(decoded.plain_field, "untouched");
    assert_eq!(decoded.ip_v6_address, "::1");
    assert_eq!(
        decoded.quoted_message.as_option().unwrap().stanza_id,
        "ABCD1234"
    );
    assert_eq!(
        decoded.message_content,
        Some(MessageContent::TextBody("hello".into()))
    );
}

#[test]
fn with_setter_uses_snake_name() {
    // `pushName` is proto3 optional → Option field with a `with_*` setter.
    let msg = WebMessageInfo::default().with_push_name("Alice");
    assert_eq!(msg.push_name.as_deref(), Some("Alice"));
}

#[test]
fn view_accessors_use_snake_names() {
    let msg = sample();
    let bytes = buffa::Message::encode_to_vec(&msg);
    let view = crate::idiomatic_fields::WebMessageInfoView::decode_view(&bytes).unwrap();
    assert_eq!(view.remote_jid, "alice@s.whatsapp.net");
    assert_eq!(view.message_timestamp, 1_700_000_000);
    assert_eq!(
        view.mentioned_jid.first().copied(),
        Some("bob@s.whatsapp.net")
    );
    match &view.message_content {
        Some(
            crate::idiomatic_fields::__buffa::view::oneof::web_message_info::MessageContent::TextBody(
                s,
            ),
        ) => assert_eq!(*s, "hello"),
        other => panic!("unexpected oneof: {other:?}"),
    }
}

#[test]
fn json_uses_original_proto_names() {
    let msg = sample();
    let json = serde_json::to_string(&msg).unwrap();
    // JSON keys are the descriptor names (here the json_name defaults derived
    // from the camelCase proto names), not the renamed Rust idents.
    assert!(json.contains("\"remoteJid\""), "{json}");
    assert!(json.contains("\"messageTimestamp\""), "{json}");
    assert!(json.contains("\"textBody\""), "{json}");
    assert!(!json.contains("remote_jid"), "{json}");

    // Round-trip back from JSON: camelCase keys land in snake_case fields.
    let back: WebMessageInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(back.remote_jid, msg.remote_jid);
    assert_eq!(back.message_content, msg.message_content);
}

#[test]
fn keyword_field_is_raw_ident() {
    let msg = sample();
    assert_eq!(msg.r#type, "chat");
    let decoded = round_trip(&msg);
    assert_eq!(decoded.r#type, "chat");
}
