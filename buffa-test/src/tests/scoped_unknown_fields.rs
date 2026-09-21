//! Path-scoped unknown-field preservation (`preserve_unknown_fields_in`).
//!
//! `protos/scoped_unknown_fields.proto` is compiled with the global default
//! off and `.test.scopedunknown.Keep` re-enabled, so `Keep` and its nested
//! messages preserve unknown fields while the structurally identical `Drop`
//! and the name-prefix sibling `Keeper` do not. Every codec that reads the
//! flag per message is generated for both kinds in one crate; these tests
//! check the observable difference through each.

use super::{length_delimited_field, repeated_varint_field, varint_field};
use crate::scopedunknown::{keep, Color, Keep, KeepLazyView, KeepView, Keeper};
use buffa::view::LazyMessageView;
use buffa::{Message, MessageView, ViewEncode};

/// Field number no message in the fixture declares.
const UNKNOWN: u32 = 90;

/// `a = 5` followed by an unknown varint record.
fn with_unknown() -> (Vec<u8>, Vec<u8>) {
    let known = varint_field(1, 5);
    let mut wire = known.clone();
    wire.extend(varint_field(UNKNOWN, 7));
    (known, wire)
}

#[test]
fn owned_keep_round_trips_unknown_fields_and_drop_discards_them() {
    let (known, wire) = with_unknown();

    let kept = Keep::decode_from_slice(&wire).unwrap();
    assert_eq!(kept.a, Some(5));
    assert_eq!(kept.__buffa_unknown_fields.iter().count(), 1);
    assert_eq!(kept.encode_to_vec(), wire);

    let dropped = super::super::scopedunknown::Drop::decode_from_slice(&wire).unwrap();
    assert_eq!(dropped.a, Some(5));
    assert_eq!(dropped.encode_to_vec(), known);
}

#[test]
fn name_prefix_sibling_is_not_covered_by_the_rule() {
    // `Keeper` shares the characters of `Keep` but not the proto segment.
    let (known, wire) = with_unknown();
    let keeper = Keeper::decode_from_slice(&wire).unwrap();
    assert_eq!(keeper.encode_to_vec(), known);
}

#[test]
fn nested_messages_follow_their_enclosing_rule() {
    use crate::scopedunknown::drop as drop_mod;

    // A rule naming `Keep` covers `Keep.Child`; `Drop.Child` is uncovered.
    let (known, wire) = with_unknown();
    let kept = keep::Child::decode_from_slice(&wire).unwrap();
    assert_eq!(kept.encode_to_vec(), wire);
    let dropped = drop_mod::Child::decode_from_slice(&wire).unwrap();
    assert_eq!(dropped.encode_to_vec(), known);

    // Unknown fields inside a nested message are stored on that message.
    let mut outer = varint_field(1, 1);
    outer.extend(length_delimited_field(7, &wire));
    let kept = Keep::decode_from_slice(&outer).unwrap();
    assert_eq!(kept.__buffa_unknown_fields.iter().count(), 0);
    assert_eq!(
        kept.child
            .as_option()
            .unwrap()
            .__buffa_unknown_fields
            .iter()
            .count(),
        1
    );
    assert_eq!(kept.encode_to_vec(), outer);
}

#[test]
fn closed_enum_unknown_values_route_per_message() {
    use crate::scopedunknown::Drop as DropMsg;

    // Field 2 (optional Color), field 3 (repeated Color) with values that are
    // not declared in `Color`.
    let mut wire = varint_field(2, 99);
    wire.extend(repeated_varint_field(3, &[1, 98]));

    let kept = Keep::decode_from_slice(&wire).unwrap();
    assert_eq!(kept.color, None);
    assert_eq!(kept.colors, [Color::GREEN]);
    assert_eq!(kept.__buffa_unknown_fields.iter().count(), 2);
    let re = Keep::decode_from_slice(&kept.encode_to_vec()).unwrap();
    assert_eq!(re.__buffa_unknown_fields.iter().count(), 2);

    let dropped = DropMsg::decode_from_slice(&wire).unwrap();
    assert_eq!(dropped.color, None);
    assert_eq!(dropped.colors, [Color::GREEN]);
    assert_eq!(dropped.encode_to_vec(), repeated_varint_field(3, &[1]));
}

#[test]
fn closed_enum_map_value_routes_per_message() {
    use crate::scopedunknown::Drop as DropMsg;
    use buffa::encoding::{encode_varint, Tag, WireType};

    // One `by_name` entry (field 4) whose value is an undeclared Color.
    let mut entry = Vec::new();
    Tag::new(1, WireType::LengthDelimited).encode(&mut entry);
    buffa::types::encode_string("k", &mut entry);
    Tag::new(2, WireType::Varint).encode(&mut entry);
    encode_varint(99, &mut entry);
    let wire = length_delimited_field(4, &entry);

    let kept = Keep::decode_from_slice(&wire).unwrap();
    assert!(kept.by_name.is_empty());
    assert_eq!(kept.__buffa_unknown_fields.iter().count(), 1);
    assert_eq!(kept.encode_to_vec(), wire);

    let dropped = DropMsg::decode_from_slice(&wire).unwrap();
    assert!(dropped.by_name.is_empty());
    assert!(dropped.encode_to_vec().is_empty());
}

#[test]
fn views_follow_the_per_message_setting() {
    use crate::scopedunknown::DropView;

    let (known, wire) = with_unknown();

    let kept = KeepView::decode_view(&wire).unwrap();
    assert_eq!(kept.a, Some(5));
    assert_eq!(kept.encode_to_vec(), wire);
    assert_eq!(kept.to_owned_message().unwrap().encode_to_vec(), wire);

    let dropped = DropView::decode_view(&wire).unwrap();
    assert_eq!(dropped.a, Some(5));
    assert_eq!(dropped.encode_to_vec(), known);
    assert_eq!(dropped.to_owned_message().unwrap().encode_to_vec(), known);
}

#[test]
fn lazy_views_follow_the_per_message_setting() {
    use crate::scopedunknown::DropLazyView;

    let (known, wire) = with_unknown();

    let kept = KeepLazyView::decode_lazy(&wire).unwrap();
    assert_eq!(kept.to_owned_message().unwrap().encode_to_vec(), wire);
    assert_eq!(kept.encode_to_vec(), wire);

    let dropped = DropLazyView::decode_lazy(&wire).unwrap();
    assert_eq!(dropped.to_owned_message().unwrap().encode_to_vec(), known);
    assert_eq!(dropped.encode_to_vec(), known);
}

#[test]
fn text_format_prints_unknown_fields_only_where_preserved() {
    use crate::scopedunknown::Drop as DropMsg;
    use buffa::text::{TextEncoder, TextFormat};

    // Unknown fields print only when the encoder opts in via `emit_unknown`.
    fn text_with_unknown<M: TextFormat>(msg: &M) -> String {
        let mut out = String::new();
        let mut enc = TextEncoder::new(&mut out).emit_unknown(true);
        msg.encode_text(&mut enc).unwrap();
        out
    }

    let (_, wire) = with_unknown();
    let kept = Keep::decode_from_slice(&wire).unwrap();
    assert_eq!(text_with_unknown(&kept), "a: 5 90: 7");
    let dropped = DropMsg::decode_from_slice(&wire).unwrap();
    assert_eq!(text_with_unknown(&dropped), "a: 5");
}

#[test]
fn json_and_extension_set_compile_for_both_kinds() {
    use crate::scopedunknown::Drop as DropMsg;

    // Only a preserving message has an `ExtensionSet` impl.
    fn takes_extension_set<T: buffa::ExtensionSet>(_: &T) {}
    takes_extension_set(&Keep::default());

    let kept: Keep = serde_json::from_str(r#"{"a": 5}"#).unwrap();
    assert_eq!(serde_json::to_string(&kept).unwrap(), r#"{"a":5}"#);
    let dropped: DropMsg = serde_json::from_str(r#"{"a": 5}"#).unwrap();
    assert_eq!(serde_json::to_string(&dropped).unwrap(), r#"{"a":5}"#);
}

#[test]
fn reflection_exposes_unknown_fields_only_where_preserved() {
    use crate::scopedunknown::Drop as DropMsg;
    use buffa_descriptor::reflect::Reflectable;

    let (_, wire) = with_unknown();
    let kept = Keep::decode_from_slice(&wire).unwrap();
    assert_eq!(kept.reflect().unknown_fields().iter().count(), 1);
    let dropped = DropMsg::decode_from_slice(&wire).unwrap();
    assert_eq!(dropped.reflect().unknown_fields().iter().count(), 0);
}
