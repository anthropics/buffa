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
    assert_eq!(MyEcoString::copy_from_str("owned"), from_borrowed);
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
    assert_eq!(NamedEcoString::copy_from_str("named"), s);
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

fn own_text(value: &str) -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Owned(value.to_owned())
}

// Cow<'static, str> has From<&'static str>, but cannot copy an arbitrary &str
// through From. The override must cover all three borrowed-text entry points.
#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = std::borrow::Cow<'static, str>, copy_from_str = own_text, serde, arbitrary)]
struct OwnedText(std::borrow::Cow<'static, str>);

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = std::borrow::Cow<'static, str>, copy_from_str = own_text)]
struct NamedOwnedText {
    inner: std::borrow::Cow<'static, str>,
}

#[test]
fn owning_constructor_accepts_non_static_text() {
    let text = String::from("temporary");
    let from_str = OwnedText::from(text.as_str());
    let copied = OwnedText::copy_from_str(&text);
    let decoded = OwnedText::from_wire(WirePayload::borrowed(text.as_bytes())).unwrap();
    let named = NamedOwnedText::copy_from_str(&text);
    drop(text);
    assert_eq!(from_str.as_ref(), "temporary");
    assert_eq!(copied, from_str);
    assert_eq!(decoded, from_str);
    assert_eq!(named.as_ref(), from_str.as_ref());
    assert_eq!(NamedOwnedText::from("named").as_ref(), "named");
    assert_eq!(
        NamedOwnedText::from_wire(WirePayload::borrowed(b"wire"))
            .unwrap()
            .as_ref(),
        "wire"
    );
}

#[test]
fn owning_constructor_preserves_utf8_validation_and_forwarders() {
    assert!(OwnedText::from_wire(WirePayload::borrowed(&[0xff])).is_err());
    let decoded: OwnedText = serde_json::from_str("\"json\"").unwrap();
    assert_eq!(decoded.as_ref(), "json");
    let mut input = arbitrary::Unstructured::new(b"fuzz");
    let generated: OwnedText = arbitrary::Arbitrary::arbitrary(&mut input).unwrap();
    assert!(std::str::from_utf8(generated.as_bytes()).is_ok());
}
