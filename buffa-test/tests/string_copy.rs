use buffa::{Message, MessageView, ProtoString};
use buffa_test::string_copy::{
    __buffa::{oneof::strings::Choice, view::StringsView},
    BorrowingStr, OwnedStr, Strings,
};
use std::borrow::Cow;

fn sample() -> Strings {
    Strings {
        name: OwnedStr::from(String::from("hello")),
        maybe: Some(OwnedStr::from(String::from("optional"))),
        items: vec![OwnedStr::from(String::from("item")), OwnedStr::default()],
        by_name: [(
            OwnedStr::from(String::from("key")),
            OwnedStr::from(String::from("value")),
        )]
        .into_iter()
        .collect(),
        choice: Some(Choice::Label(OwnedStr::from(String::from("chosen")))),
        ..Default::default()
    }
}

#[test]
fn copying_does_not_change_borrowing_conversion() {
    let owned = {
        let input = String::from("temporary");
        let borrowed = BorrowingStr::from(input.as_str());
        assert!(matches!(borrowed.0, Cow::Borrowed(_)));
        OwnedStr::copy_from_str(&input)
    };
    assert_eq!(owned.as_ref(), "temporary");
    assert!(matches!(owned.0, Cow::Owned(_)));
}

#[test]
fn all_string_shapes_outlive_the_view_input() {
    let expected = sample();
    let owned = {
        let wire = expected.encode_to_vec();
        let view = StringsView::decode_view(&wire).unwrap();
        view.to_owned_message().unwrap()
    };
    assert_eq!(owned, expected);
    assert_eq!(
        Strings::decode(&mut expected.encode_to_vec().as_slice()).unwrap(),
        expected
    );
}

#[test]
fn json_copies_temporary_strings_and_preserves_null_defaults() {
    let expected = sample();
    let decoded: Strings = {
        let json = serde_json::to_string(&expected).unwrap();
        serde_json::from_str(&json).unwrap()
    };
    assert_eq!(decoded, expected);
    let empty: Strings =
        serde_json::from_str(r#"{"name":null,"maybe":null,"label":null}"#).unwrap();
    assert!(empty.name.is_empty());
    assert!(empty.maybe.is_none());
    assert!(empty.choice.is_none());
}

#[test]
fn text_and_clear_keep_existing_behavior() {
    let expected = sample();
    let text = buffa::text::encode_to_string(&expected);
    let mut decoded: Strings = buffa::text::decode_from_str(&text).unwrap();
    assert_eq!(decoded, expected);
    decoded.clear();
    assert_eq!(decoded, Strings::default());
}
