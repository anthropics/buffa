//! Repeated enum fields stored in a custom collection, with JSON on.
//!
//! `repeated_enum_json.proto` is compiled with
//! `repeated_type_custom("crate::repeated_type::CustomList<*>")` and
//! `generate_json(true)`. Compiling `crate::repeated_enum_json` is most of the
//! test: the `repeated_enum` and `repeated_closed_enum` deserializers must
//! return the field's `CustomList`, not a `Vec`.

use crate::repeated_enum_json::{swatch, ClosedColor, OpenColor, Palette, Swatch};
use crate::repeated_type::CustomList;
use buffa::EnumValue;

fn sample() -> Palette {
    Palette {
        open: ::buffa::alloc::vec![EnumValue::Known(OpenColor::OPEN_RED), EnumValue::Unknown(7)]
            .into(),
        closed: ::buffa::alloc::vec![ClosedColor::CLOSED_RED, ClosedColor::CLOSED_UNSPECIFIED]
            .into(),
        ..Default::default()
    }
}

#[test]
fn field_types_are_custom_list() {
    let msg = sample();
    let _: &CustomList<EnumValue<OpenColor>> = &msg.open;
    let _: &CustomList<ClosedColor> = &msg.closed;
}

#[test]
fn custom_list_enum_fields_json_roundtrip() {
    let msg = sample();
    let json = serde_json::to_string(&msg).expect("serialize");
    assert!(json.contains(r#""open":["OPEN_RED",7]"#), "got: {json}");
    assert!(
        json.contains(r#""closed":["CLOSED_RED","CLOSED_UNSPECIFIED"]"#),
        "got: {json}"
    );

    let back: Palette = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.open, msg.open);
    assert_eq!(back.closed, msg.closed);
}

#[test]
fn custom_list_enum_fields_null_is_empty() {
    let msg: Palette = serde_json::from_str(r#"{"open":null,"closed":null}"#).unwrap();
    assert!(msg.open.is_empty());
    assert!(msg.closed.is_empty());
}

#[test]
fn custom_list_enum_fields_skip_unknown_names_when_lenient() {
    let opts = buffa::json::JsonParseOptions::new().ignore_unknown_enum_values(true);
    let msg: Palette = buffa::json::with_json_parse_options(&opts, || {
        serde_json::from_str(r#"{"open":["OPEN_RED","PURPLE"],"closed":["PURPLE","CLOSED_RED"]}"#)
            .unwrap()
    });
    assert_eq!(
        msg.open,
        CustomList(::buffa::alloc::vec![EnumValue::Known(OpenColor::OPEN_RED)])
    );
    assert_eq!(
        msg.closed,
        CustomList(::buffa::alloc::vec![ClosedColor::CLOSED_RED])
    );
}

#[test]
fn custom_list_enum_fields_roundtrip_through_hand_generated_deserialize() {
    let msg: Swatch =
        serde_json::from_str(r#"{"open":["OPEN_RED",7],"closed":["CLOSED_RED"],"name":"sky"}"#)
            .unwrap();
    assert_eq!(
        msg.open,
        CustomList(::buffa::alloc::vec![
            EnumValue::Known(OpenColor::OPEN_RED),
            EnumValue::Unknown(7)
        ])
    );
    assert_eq!(
        msg.closed,
        CustomList(::buffa::alloc::vec![ClosedColor::CLOSED_RED])
    );
    assert_eq!(msg.label, Some(swatch::Label::Name("sky".into())));

    let json = serde_json::to_string(&msg).unwrap();
    let back: Swatch = serde_json::from_str(&json).unwrap();
    assert_eq!(back, msg);
}
