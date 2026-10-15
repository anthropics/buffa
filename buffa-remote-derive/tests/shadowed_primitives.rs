//! The generated impls keep their meaning beside an item that shadows a
//! primitive type or a `WirePayload` method (#707).
//!
//! A type or alias named like a primitive takes the primitive's place in a
//! bare type path. A trait in scope with a by-value method named like one of
//! `WirePayload`'s `&self` methods is found first by method-call syntax,
//! because method lookup tries the by-value receiver before it
//! auto-references.

use std::collections::HashMap;

use arbitrary::{Arbitrary, Unstructured};
use buffa::{MapStorage as _, ProtoBytes as _, ProtoString as _, WirePayload};
use buffa_remote_derive::{MapStorage, ProtoBytes, ProtoString};

const INPUT: &[u8] = b"acegikmoqsuwyACEGIKMOQSUWYacegikmoqsuwyACEGIKMOQSUWY";

/// Aliases named like the primitives the impls use, each for another type.
#[allow(dead_code, non_camel_case_types)]
mod aliases {
    use super::*;

    type str = String;
    type u8 = u32;
    type usize = u32;

    #[derive(Clone, PartialEq, Default, Debug, ProtoString)]
    #[buffa(remote = String, arbitrary, serde)]
    pub struct Text(pub String);

    #[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
    #[buffa(remote = Vec<::core::primitive::u8>, arbitrary, serde)]
    pub struct Bytes(pub Vec<::core::primitive::u8>);

    #[derive(Clone, PartialEq, Default, Debug, MapStorage)]
    #[buffa(remote = HashMap<K, V>, serde)]
    pub struct Map<K: std::hash::Hash + Eq, V>(pub HashMap<K, V>);
}

/// Unit structs named like the primitives the impls use.
#[allow(dead_code, non_camel_case_types)]
mod structs {
    use super::*;

    struct str;
    struct u8;
    struct usize;

    #[derive(Clone, PartialEq, Default, Debug, ProtoString)]
    #[buffa(remote = String, arbitrary, serde)]
    pub struct Text(pub String);

    #[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
    #[buffa(remote = Vec<::core::primitive::u8>, arbitrary, serde)]
    pub struct Bytes(pub Vec<::core::primitive::u8>);

    #[derive(Clone, PartialEq, Default, Debug, MapStorage)]
    #[buffa(remote = HashMap<K, V>, serde)]
    pub struct Map<K: std::hash::Hash + Eq, V>(pub HashMap<K, V>);
}

/// A trait with by-value `to_str` and `as_slice`, implemented for
/// `WirePayload`. Its methods return other data than the payload holds, so
/// an impl that calls them decodes the wrong value.
// The by-value `as_slice` is the point of the trait, and no correct impl
// calls it.
#[allow(dead_code, clippy::wrong_self_convention)]
mod payload_methods {
    use super::*;

    pub trait Shadow {
        fn to_str(self) -> Result<&'static str, buffa::DecodeError>;
        fn as_slice(self) -> &'static [u8];
    }

    impl Shadow for WirePayload<'_> {
        fn to_str(self) -> Result<&'static str, buffa::DecodeError> {
            Ok("shadowed")
        }
        fn as_slice(self) -> &'static [u8] {
            b"shadowed"
        }
    }

    #[derive(Clone, PartialEq, Default, Debug, ProtoString)]
    #[buffa(remote = String)]
    pub struct Text(pub String);

    #[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
    #[buffa(remote = Vec<u8>)]
    pub struct Bytes(pub Vec<u8>);
}

#[test]
fn string_beside_primitive_names() {
    fn check<T>(unwrap: impl Fn(&T) -> &String)
    where
        T: buffa::ProtoString
            + core::ops::Deref<Target = str>
            + for<'a> From<&'a str>
            + serde::Serialize
            + for<'de> serde::Deserialize<'de>
            + for<'a> Arbitrary<'a>,
    {
        assert_eq!(unwrap(&T::from("borrowed")), "borrowed");
        let decoded = T::from_wire(WirePayload::borrowed(b"wire")).unwrap();
        assert_eq!(unwrap(&decoded), "wire");
        assert_eq!(&*decoded, "wire");
        let json = serde_json::to_string(&decoded).unwrap();
        assert_eq!(json, r#""wire""#);
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(unwrap(&back), "wire");
        let arbitrary = T::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
        let want = String::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
        assert_eq!(unwrap(&arbitrary), &want);
    }
    check(|text: &aliases::Text| &text.0);
    check(|text: &structs::Text| &text.0);
}

#[test]
fn bytes_beside_primitive_names() {
    fn check<T>(unwrap: impl Fn(&T) -> &Vec<u8>)
    where
        T: buffa::ProtoBytes
            + core::ops::Deref<Target = [u8]>
            + serde::Serialize
            + for<'de> serde::Deserialize<'de>
            + for<'a> Arbitrary<'a>,
    {
        let decoded = T::from_wire(WirePayload::borrowed(&[3, 4])).unwrap();
        assert_eq!(unwrap(&decoded), &[3, 4]);
        assert_eq!(&*decoded, [3, 4]);
        let json = serde_json::to_string(&decoded).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(unwrap(&back), &[3, 4]);
        let arbitrary = T::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
        let want = Vec::<u8>::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
        assert_eq!(unwrap(&arbitrary), &want);
    }
    check(|bytes: &aliases::Bytes| &bytes.0);
    check(|bytes: &structs::Bytes| &bytes.0);
}

#[test]
fn map_beside_primitive_names() {
    let mut map = aliases::Map(HashMap::new());
    map.storage_insert(1, "one");
    assert_eq!(map.storage_len(), 1);

    let mut map = structs::Map(HashMap::new());
    map.storage_insert(1, "one");
    assert_eq!(map.storage_len(), 1);
}

#[test]
fn from_wire_reads_the_payload_beside_a_shadowing_trait() {
    let text = payload_methods::Text::from_wire(WirePayload::borrowed(b"wire")).unwrap();
    assert_eq!(text.0, "wire");
    let bytes = payload_methods::Bytes::from_wire(WirePayload::borrowed(b"wire")).unwrap();
    assert_eq!(bytes.0, b"wire");
}
