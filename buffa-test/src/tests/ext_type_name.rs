//! `(buffa.ext.message).name` and `(buffa.ext.enum).name`: the options set
//! the Rust names of the structs and enums in `protos/ext_type_name.proto`.
//! Compiling these paths is half the test. The other half checks that the
//! type URL, JSON, text format and reflection still use the proto names.

use super::round_trip;
use crate::ext_type_name::r#type::{Piece, Setting};
use crate::ext_type_name::user::User;
use crate::ext_type_name::{Holder, Kind, KindView, Severity};
use buffa::{ExtensionSet, Message, MessageName, MessageView};
use buffa_descriptor::reflect::ReflectMessage;

fn sample() -> Kind {
    let mut msg = Kind {
        label: "note".into(),
        level: Severity::LEVEL_HIGH.into(),
        part: buffa::MessageField::some(Piece {
            size: 3,
            ..Default::default()
        }),
        mode: Setting::MODE_ON.into(),
        children: vec![Kind {
            label: "child".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    msg.parts.insert(
        "x".into(),
        Piece {
            size: 1,
            ..Default::default()
        },
    );
    msg
}

#[test]
fn renamed_types_round_trip() {
    let msg = sample();
    let decoded = round_trip(&msg);
    assert_eq!(decoded, msg);
    assert_eq!(decoded.part.size, 3);
    assert_eq!(decoded.children[0].label, "child");
}

#[test]
fn other_messages_refer_to_the_renamed_types() {
    let holder = Holder {
        item: buffa::MessageField::some(sample()),
        part: buffa::MessageField::some(Piece::default()),
        level: Severity::LEVEL_HIGH.into(),
        mode: Setting::MODE_ON.into(),
        ..Default::default()
    };
    assert_eq!(round_trip(&holder), holder);

    // A second package imports the types by their proto names.
    let user = User {
        item: buffa::MessageField::some(sample()),
        part: buffa::MessageField::some(Piece::default()),
        level: Severity::LEVEL_HIGH.into(),
        ..Default::default()
    };
    assert_eq!(round_trip(&user), user);
}

#[test]
fn option_separates_a_lower_case_message_from_its_module() {
    use crate::ext_type_name::{item, Item};
    let msg = Item {
        inner: buffa::MessageField::some(item::Inner {
            n: 4,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
    assert_eq!(Item::FULL_NAME, "test.exttypename.item");
}

#[test]
fn view_follows_the_option() {
    let bytes = sample().encode_to_vec();
    let view = KindView::decode_view(&bytes).unwrap();
    assert_eq!(view.label, "note");
    assert_eq!(view.part.size, 3);
}

#[test]
fn debug_prints_the_rust_name() {
    assert!(format!("{:?}", Kind::default()).starts_with("Kind {"));
    assert_eq!(format!("{:?}", Severity::LEVEL_HIGH), "LEVEL_HIGH");
}

#[test]
fn full_name_and_type_url_keep_the_proto_names() {
    assert_eq!(Kind::FULL_NAME, "test.exttypename.Type");
    assert_eq!(Kind::NAME, "Type");
    assert_eq!(Piece::FULL_NAME, "test.exttypename.Type.Part");
    assert_eq!(Kind::TYPE_URL, "type.googleapis.com/test.exttypename.Type");
}

#[test]
fn json_keeps_the_proto_names() {
    let msg = Kind {
        label: "note".into(),
        level: Severity::LEVEL_HIGH.into(),
        mode: Setting::MODE_ON.into(),
        ..Default::default()
    };
    let json = serde_json::to_value(&msg).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"label": "note", "level": "LEVEL_HIGH", "mode": "MODE_ON"})
    );
    let back: Kind = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn text_format_round_trips() {
    let msg = sample();
    let text = buffa::text::encode_to_string(&msg);
    assert!(text.contains("level: LEVEL_HIGH"), "{text}");
    let back: Kind = buffa::text::decode_from_str(&text).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn reflection_keeps_the_proto_names() {
    let msg = sample();
    let reflected: &dyn ReflectMessage = &msg;
    let descriptor = reflected.message_descriptor();
    assert_eq!(descriptor.full_name(), "test.exttypename.Type");
    // The option is in the embedded descriptor, so a tool that reads the
    // descriptor can find the Rust name.
    let option = descriptor
        .options()
        .unwrap()
        .extension(&buffa_proto_options::MESSAGE)
        .unwrap();
    assert_eq!(option.name.as_deref(), Some("Kind"));
}
