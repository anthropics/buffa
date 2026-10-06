//! Optional serde forwarding for remote storage types, including remotes
//! without serde support and generic collections with conditional bounds.

use buffa_remote_derive::{MapStorage, ProtoBox, ProtoBytes, ProtoList, ProtoString};
use serde::{de::DeserializeOwned, Serialize};

#[derive(Clone, PartialEq, Default, Debug, ProtoString)]
#[buffa(remote = ecow::EcoString, serde)]
struct Text(ecow::EcoString);

#[derive(Clone, PartialEq, Default, Debug, ProtoString)]
#[buffa(remote = String, serde)]
struct WithHrtb(String)
where
    for<'__buffa_de> &'__buffa_de str: Into<String>;

#[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
#[buffa(remote = smallvec::SmallVec<[u8; 16]>, serde)]
struct Bytes {
    inner: smallvec::SmallVec<[u8; 16]>,
}

#[derive(Clone, PartialEq, Debug, ProtoList)]
#[buffa(remote = Vec<T>)]
#[cfg_attr(test, buffa(serde))]
struct List<T>(Vec<T>);

impl<T> Default for List<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

// Matches the ProtoBox ownership contract used by the override.
#[allow(clippy::boxed_local)]
fn unbox<T>(value: Box<T>) -> T {
    *value
}

#[derive(Clone, PartialEq, Debug, ProtoBox)]
#[buffa(remote = Box<T>, into_inner = unbox, serde)]
struct Pointer<T> {
    inner: Box<T>,
}

// The method type parameters in the serde impl must avoid these names.
#[derive(Clone, PartialEq, Debug, MapStorage)]
#[buffa(remote = indexmap::IndexMap<__BuffaSerializer, __BuffaDeserializer>, serde)]
struct Map<__BuffaSerializer: core::hash::Hash + Eq, __BuffaDeserializer>(
    indexmap::IndexMap<__BuffaSerializer, __BuffaDeserializer>,
);

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

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + core::fmt::Debug>(value: T, json: &str) {
    assert_eq!(serde_json::to_string(&value).unwrap(), json);
    assert_eq!(serde_json::from_str::<T>(json).unwrap(), value);
}

#[test]
fn strings_use_proto_json_without_remote_serde() {
    roundtrip(Text::from("hello"), r#""hello""#);
    roundtrip(WithHrtb::from("bound"), r#""bound""#);
    assert_eq!(
        serde_json::from_str::<Text>("null").unwrap(),
        Text::default()
    );
    assert!(serde_json::from_str::<Text>("123").is_err());
    assert!(serde_json::from_str::<Text>("{}").is_err());
}

#[test]
fn bytes_use_base64_without_remote_serde() {
    roundtrip(Bytes::from(vec![0, 255, 128]), r#""AP+A""#);
    assert_eq!(
        serde_json::from_str::<Bytes>("null").unwrap(),
        Bytes::default()
    );
    assert!(serde_json::from_str::<Bytes>("[0,255,128]").is_err());
    assert!(serde_json::from_str::<Bytes>(r#""!!!""#).is_err());
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
fn optional_repeated_and_map_string_positions_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Message {
        optional: Option<Text>,
        repeated: List<Text>,
        map: Map<String, Text>,
    }
    roundtrip(
        Message {
            optional: Some(Text::from("present")),
            repeated: List(vec![Text::from("element")]),
            map: Map::from_iter([(String::from("k"), Text::from("v"))]),
        },
        r#"{"optional":"present","repeated":["element"],"map":{"k":"v"}}"#,
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
