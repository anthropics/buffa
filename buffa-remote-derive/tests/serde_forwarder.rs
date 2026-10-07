//! Optional serde forwarding for remote storage types, including remotes
//! without serde support and generic collections with conditional bounds.

use buffa_remote_derive::{MapStorage, ProtoBox, ProtoBytes, ProtoList, ProtoString};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone, PartialEq, Default, Debug, ProtoString)]
#[buffa(remote = ecow::EcoString, serde)]
struct Text(ecow::EcoString);

#[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
#[buffa(remote = smallvec::SmallVec<[u8; 16]>, serde)]
struct Bytes {
    inner: smallvec::SmallVec<[u8; 16]>,
}

#[derive(Clone, PartialEq, Debug, ProtoList)]
#[buffa(remote = Vec<T>)]
// `cfg(test)` holds in an integration test, so this is the `cfg_attr`
// spelling of the key.
#[cfg_attr(test, buffa(serde))]
struct List<T>(Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

// An `into_inner` override takes the pointer by value, so the `Box<T>`
// parameter is the required signature.
#[allow(clippy::boxed_local)]
fn unbox<T>(value: Box<T>) -> T {
    *value
}

#[derive(Clone, PartialEq, Debug, ProtoBox)]
#[buffa(remote = Box<T>, into_inner = unbox, serde)]
struct Pointer<T> {
    inner: Box<T>,
}

#[derive(Clone, PartialEq, Debug, MapStorage)]
#[buffa(remote = indexmap::IndexMap<K, V>, serde)]
struct Map<K: core::hash::Hash + Eq, V>(indexmap::IndexMap<K, V>);

impl<K: core::hash::Hash + Eq, V> Default for Map<K, V> {
    fn default() -> Self {
        Self(indexmap::IndexMap::new())
    }
}

impl<K: core::hash::Hash + Eq, V> FromIterator<(K, V)> for Map<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(indexmap::IndexMap::from_iter(iter))
    }
}

/// A pointer type with a lifetime parameter, as an arena handle has.
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
struct Scoped<'s, T>(Box<T>, #[serde(skip)] core::marker::PhantomData<&'s ()>);

impl<T> Scoped<'_, T> {
    fn new(value: T) -> Self {
        Self(Box::new(value), core::marker::PhantomData)
    }
    fn into_inner(self) -> T {
        *self.0
    }
}

impl<T> core::ops::Deref for Scoped<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> core::ops::DerefMut for Scoped<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[derive(Clone, PartialEq, Debug, ProtoBox)]
#[buffa(remote = Scoped<'s, T>, serde, arbitrary)]
struct ScopedPointer<'s, T>(Scoped<'s, T>);

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + core::fmt::Debug>(value: T, json: &str) {
    assert_eq!(serde_json::to_string(&value).unwrap(), json);
    assert_eq!(serde_json::from_str::<T>(json).unwrap(), value);
}

#[test]
fn strings_use_proto_json_without_remote_serde() {
    roundtrip(Text::from("hello"), r#""hello""#);
    type_error::<Text>("123", "integer `123`", "a string");
    type_error::<Text>("{}", "map", "a string");
    // An owned string reaches `visit_string`, a borrowed one `visit_str`.
    assert_eq!(
        serde_json::from_value::<Text>(serde_json::json!("owned")).unwrap(),
        Text::from("owned")
    );
}

/// Asserts that `json` fails with serde's `invalid type` error: `found` is
/// the token it names, and `expected` the visitor's `expecting` text.
fn type_error<T: DeserializeOwned + core::fmt::Debug>(json: &str, found: &str, expected: &str) {
    let err = serde_json::from_str::<T>(json).expect_err("the token is rejected");
    let want = format!("invalid type: {found}, expected {expected}");
    assert!(err.to_string().contains(&want), "{json}: {err}");
}

fn null_error<T: DeserializeOwned + core::fmt::Debug>(json: &str, expected: &str) {
    type_error::<T>(json, "null", expected);
}

#[test]
fn string_rejects_null() {
    null_error::<Text>("null", "a string");
    null_error::<Vec<Text>>(r#"["a",null]"#, "a string");
    null_error::<List<Text>>("[null]", "a string");
    null_error::<std::collections::HashMap<String, Text>>(r#"{"k":null}"#, "a string");
    null_error::<Map<String, Text>>(r#"{"k":null}"#, "a string");
    assert!(serde_json::from_value::<Text>(serde_json::Value::Null).is_err());
    // `Option` consumes the `null` of an `optional` field itself.
    assert_eq!(serde_json::from_str::<Option<Text>>("null").unwrap(), None);
}

#[test]
fn bytes_use_base64_without_remote_serde() {
    roundtrip(Bytes::from(vec![0, 255, 128]), r#""AP+A""#);
    type_error::<Bytes>("[0,255,128]", "sequence", "a base64-encoded string");
    let err = serde_json::from_str::<Bytes>(r#""!!!""#).expect_err("invalid base64");
    assert!(err.to_string().contains("Invalid symbol 33"), "{err}");
    // The URL-safe alphabet and unpadded input are accepted.
    assert_eq!(
        serde_json::from_str::<Bytes>(r#""AP-A""#).unwrap(),
        Bytes::from(vec![0, 255, 128])
    );
    assert_eq!(
        serde_json::from_str::<Bytes>(r#""AP8""#).unwrap(),
        Bytes::from(vec![0, 255])
    );
}

#[test]
fn bytes_reject_null() {
    null_error::<Bytes>("null", "a base64-encoded string");
    null_error::<Vec<Bytes>>(r#"["AP+A",null]"#, "a base64-encoded string");
    null_error::<std::collections::HashMap<String, Bytes>>(
        r#"{"k":null}"#,
        "a base64-encoded string",
    );
    assert!(serde_json::from_value::<Bytes>(serde_json::Value::Null).is_err());
    assert_eq!(serde_json::from_str::<Option<Bytes>>("null").unwrap(), None);
}

#[test]
fn generic_containers_forward_the_remote_serde_shape() {
    roundtrip(List(vec![Text::from("a"), Text::from("b")]), r#"["a","b"]"#);
    roundtrip(
        Pointer {
            inner: Box::new(Text::from("boxed")),
        },
        r#""boxed""#,
    );
    roundtrip(
        Map::from_iter([(String::from("key"), Text::from("value"))]),
        r#"{"key":"value"}"#,
    );
}

#[test]
fn container_serde_bounds_do_not_restrict_binary_storage() {
    #[derive(Clone, PartialEq, Debug)]
    struct BinaryOnly;
    // These compile despite the missing serde impl on the element/pointee.
    let _: List<BinaryOnly> = List::default();
    let _ = Pointer {
        inner: Box::new(BinaryOnly),
    };
    let _: Map<String, BinaryOnly> = Map::default();
}

/// The std prelude is out of scope in this module, as it is in a `#![no_std]`
/// crate: the expansions compile only if they name `String`, `Vec`, `Result`
/// and the rest by absolute path.
#[no_implicit_prelude]
mod no_prelude {
    use ::core::clone::Clone;
    use ::core::cmp::PartialEq;
    use ::core::default::Default;
    use ::core::fmt::Debug;

    #[derive(Clone, PartialEq, Default, Debug, ::buffa_remote_derive::ProtoString)]
    #[buffa(remote = ::std::string::String, serde)]
    pub struct Text(pub ::std::string::String);

    #[derive(Clone, PartialEq, Default, Debug, ::buffa_remote_derive::ProtoBytes)]
    #[buffa(remote = ::std::vec::Vec<u8>, serde)]
    pub struct Bytes(pub ::std::vec::Vec<u8>);

    #[derive(Clone, PartialEq, Default, Debug, ::buffa_remote_derive::ProtoList)]
    #[buffa(remote = ::std::vec::Vec<T>, serde)]
    pub struct List<T>(pub ::std::vec::Vec<T>);

    #[derive(Clone, PartialEq, Debug, ::buffa_remote_derive::ProtoBox)]
    #[buffa(remote = ::std::boxed::Box<T>, into_inner = super::unbox, serde)]
    pub struct Pointer<T>(pub ::std::boxed::Box<T>);

    #[derive(Clone, PartialEq, Default, Debug, ::buffa_remote_derive::MapStorage)]
    #[buffa(remote = ::std::collections::BTreeMap<K, V>, serde)]
    pub struct Map<K: ::core::cmp::Ord, V>(pub ::std::collections::BTreeMap<K, V>);
}

#[test]
fn expansions_do_not_rely_on_the_std_prelude() {
    roundtrip(no_prelude::Text(String::from("hello")), r#""hello""#);
    roundtrip(no_prelude::Bytes(vec![0, 255, 128]), r#""AP+A""#);
    roundtrip(no_prelude::List(vec![1, 2]), "[1,2]");
    roundtrip(no_prelude::Pointer(Box::new(7)), "7");
    roundtrip(
        no_prelude::Map([(String::from("k"), 1)].into_iter().collect()),
        r#"{"k":1}"#,
    );
}

/// A newtype with its own lifetime parameter gets both impls, and its
/// `Deserialize` impl accepts a pointee that borrows from the input.
#[test]
fn lifetime_parameterised_newtype_borrows_from_the_input() {
    let json = String::from(r#""borrowed""#);
    let pointer: ScopedPointer<'_, &str> = serde_json::from_str(&json).unwrap();
    assert_eq!(*pointer, "borrowed");
    assert_eq!(serde_json::to_string(&pointer).unwrap(), json);
}

/// `serde` and `arbitrary` in one attribute each emit their impl.
#[test]
fn serde_and_arbitrary_keys_compose() {
    use arbitrary::{Arbitrary, Unstructured};

    let pointer = ScopedPointer::<'static, u8>::arbitrary(&mut Unstructured::new(&[7])).unwrap();
    roundtrip(pointer, "7");
}
