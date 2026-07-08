use buffa::{ProtoString, WirePayload};
use buffa_remote_derive::ProtoString as DeriveProtoString;

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString)]
struct MyEcoString(pub ecow::EcoString);

#[test]
fn from_wire_decodes_valid_utf8() {
    let s = MyEcoString::from_wire(WirePayload::borrowed(b"hello")).unwrap();
    assert_eq!(s.as_ref(), "hello");
}

#[test]
fn from_wire_rejects_invalid_utf8() {
    assert!(MyEcoString::from_wire(WirePayload::borrowed(&[0xff, 0xfe])).is_err());
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

// A remote type implementing both `AsRef<str>` and `AsRef<[u8]>` would make
// plain `.as_ref()` method-call syntax in the generated body ambiguous
// (return-type-directed resolution doesn't apply to method calls); the derive
// must use fully qualified `<Remote as AsRef<str>>::as_ref` instead. This
// type exists to catch a regression back to plain method-call syntax.
#[derive(Clone, PartialEq, Default, Debug)]
struct DualAsRef(String);
impl AsRef<str> for DualAsRef {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}
impl AsRef<[u8]> for DualAsRef {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}
impl From<String> for DualAsRef {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for DualAsRef {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = DualAsRef)]
struct MyDualAsRef(pub DualAsRef);

#[test]
fn remote_with_multiple_as_ref_impls_resolves_unambiguously() {
    let s = MyDualAsRef::from("disambiguated");
    assert_eq!(&*s, "disambiguated");
    assert_eq!(s.as_ref(), "disambiguated");
}
