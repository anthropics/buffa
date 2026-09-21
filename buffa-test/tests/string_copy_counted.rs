//! Generated code must build custom strings with `ProtoString::copy_from_str`,
//! not `From<String>` or `From<&str>`, so an override that avoids the
//! intermediate allocation actually runs.

use buffa::{Message, MessageView};
use buffa_test::string_copy_counted::{
    __buffa::{oneof::strings::Choice, view::StringsView},
    copies, CountedStr, Strings,
};

fn s(value: &str) -> CountedStr {
    CountedStr::from(value.to_owned())
}

fn with(populate: impl FnOnce(&mut Strings)) -> Strings {
    let mut msg = Strings::default();
    populate(&mut msg);
    msg
}

/// The `copy_from_str` calls made while converting the view of `msg` back to
/// an owned message.
fn copies_in_view_to_owned(msg: &Strings) -> usize {
    let wire = msg.encode_to_vec();
    let view = StringsView::decode_view(&wire).unwrap();
    let before = copies();
    let owned = view.to_owned_message().unwrap();
    let count = copies() - before;
    assert_eq!(&owned, msg);
    count
}

#[test]
fn view_to_owned_copies_each_string_shape_with_copy_from_str() {
    // The implicit-presence `name` is converted even when empty, so it adds one
    // copy to every case; each other shape adds one per string it holds.
    let cases = [
        ("singular", with(|m| m.name = s("n")), 1),
        ("optional", with(|m| m.maybe = Some(s("m"))), 1 + 1),
        ("repeated", with(|m| m.items = vec![s("a"), s("b")]), 1 + 2),
        (
            "map key and value",
            with(|m| m.by_name = [(s("k"), s("v"))].into_iter().collect()),
            1 + 2,
        ),
        (
            "oneof",
            with(|m| m.choice = Some(Choice::Label(s("l")))),
            1 + 1,
        ),
    ];
    for (shape, msg, expected) in cases {
        assert_eq!(copies_in_view_to_owned(&msg), expected, "{shape}");
    }
}

#[test]
fn json_deserialize_copies_with_copy_from_str() {
    let before = copies();
    let msg: Strings = serde_json::from_str(r#"{"name":"n"}"#).unwrap();
    assert_eq!(&*msg.name, "n");
    assert_eq!(copies() - before, 1);

    // `null` builds the empty default the same way.
    let before = copies();
    let msg: Strings = serde_json::from_str(r#"{"name":null}"#).unwrap();
    assert!(msg.name.is_empty());
    assert_eq!(copies() - before, 1);
}
