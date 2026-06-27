use buffa::{ProtoString, WirePayload};
use buffa_remote_derive::ProtoString as DeriveProtoString;

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString)]
struct MyEcoString(pub ecow::EcoString);

#[test]
fn from_wire_decodes_valid_utf8() {
    let s = MyEcoString::from_wire(WirePayload::Borrowed(b"hello")).unwrap();
    assert_eq!(s.as_ref(), "hello");
}

#[test]
fn from_wire_rejects_invalid_utf8() {
    assert!(MyEcoString::from_wire(WirePayload::Borrowed(&[0xff, 0xfe])).is_err());
}

#[test]
fn deref_and_as_ref_agree() {
    let s = MyEcoString::from("hi there");
    assert_eq!(&*s, "hi there");
    assert_eq!(s.as_ref(), "hi there");
}

#[test]
fn from_string_and_from_str_round_trip() {
    let from_owned = MyEcoString::from(String::from("owned"));
    let from_borrowed = MyEcoString::from("owned");
    assert_eq!(from_owned, from_borrowed);
}

// Named-field struct shape (not just tuple structs) is also supported.
#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString)]
struct NamedEcoString {
    inner: ecow::EcoString,
}

#[test]
fn named_field_struct_works() {
    let s = NamedEcoString::from("named");
    assert_eq!(s.as_ref(), "named");
}
