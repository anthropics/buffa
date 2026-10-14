//! The parameters of the generated methods are not taken over by a constant
//! in scope at the derive (#652).
//!
//! An identifier pattern that names a constant in scope is a constant
//! pattern, not a new binding. So a constant named like a generated parameter
//! turned that parameter into a pattern of the constant's type, and a bare
//! override path with the parameter's name resolved to the parameter.

use std::collections::HashMap;
use std::hash::Hash;

use arbitrary::{Arbitrary, Unstructured};
use buffa::{
    MapStorage as _, ProtoBox as _, ProtoBytes as _, ProtoList as _, ProtoString as _, WirePayload,
};
use buffa_remote_derive::{MapStorage, ProtoBox, ProtoBytes, ProtoList, ProtoString};

const INPUT: &[u8] = b"acegikmoqsuwyACEGIKMOQSUWYacegikmoqsuwyACEGIKMOQSUWY";

// An `into_inner` override takes the pointer by value, so the `Box<T>`
// parameter is the required signature.
#[allow(clippy::boxed_local)]
fn unbox<T>(boxed: Box<T>) -> T {
    *boxed
}

/// Every parameter name the generated methods used before #652, as a
/// constant in scope at each derive. No constant has its parameter's type.
#[allow(dead_code, non_upper_case_globals)]
mod constants {
    use super::*;

    const iter: u8 = 0;
    const v: u8 = 0;
    const s: u8 = 0;
    const payload: u8 = 0;
    const value: u8 = 0;
    const key: u8 = 0;
    const u: u8 = 0;

    #[derive(Clone, PartialEq, Default, Debug, ProtoString)]
    #[buffa(remote = String, arbitrary, serde)]
    pub struct Text(pub String);

    #[derive(Clone, PartialEq, Default, Debug, ProtoBytes)]
    #[buffa(remote = Vec<u8>, arbitrary, serde)]
    pub struct Bytes(pub Vec<u8>);

    #[derive(Clone, PartialEq, Debug, ProtoList)]
    #[buffa(remote = Vec<T>, arbitrary, serde)]
    pub struct List<T>(pub Vec<T>);

    impl<T> Default for List<T> {
        fn default() -> Self {
            Self(Vec::new())
        }
    }

    #[derive(Clone, PartialEq, Debug, ProtoBox)]
    #[buffa(remote = Box<T>, into_inner = unbox, arbitrary, serde)]
    pub struct Pointer<T>(pub Box<T>);

    #[derive(Clone, PartialEq, Debug, MapStorage)]
    #[buffa(remote = HashMap<K, V>, arbitrary, serde)]
    pub struct Map<K: Hash + Eq, V>(pub HashMap<K, V>);

    impl<K: Hash + Eq, V> Default for Map<K, V> {
        fn default() -> Self {
            Self(HashMap::new())
        }
    }

    impl<K: Hash + Eq, V> FromIterator<(K, V)> for Map<K, V> {
        fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
            Self(HashMap::from_iter(entries))
        }
    }
}

/// Const generic parameters named like generated parameters, which are in
/// scope in every generated impl.
#[allow(non_upper_case_globals)]
mod const_generics {
    use super::*;

    #[derive(Clone, PartialEq, Debug, ProtoList)]
    #[buffa(remote = Vec<T>)]
    pub struct List<T, const iter: usize, const v: usize, const value: usize>(pub Vec<T>);

    impl<T, const iter: usize, const v: usize, const value: usize> Default for List<T, iter, v, value> {
        fn default() -> Self {
            Self(Vec::new())
        }
    }

    #[derive(ProtoBox)]
    #[buffa(remote = Box<T>, into_inner = unbox, arbitrary)]
    pub struct Pointer<T, const value: usize, const u: usize>(pub Box<T>);

    #[derive(MapStorage)]
    #[buffa(remote = HashMap<K, V>)]
    pub struct Map<K: Hash + Eq, V, const key: usize, const value: usize>(pub HashMap<K, V>);
}

/// A static, a unit struct and a tuple struct named like generated
/// parameters. A binding cannot shadow any of them either.
#[allow(dead_code, non_camel_case_types, non_upper_case_globals)]
mod other_items {
    use super::*;

    static v: u8 = 0;
    struct value;
    struct key(u8);

    #[derive(Clone, PartialEq, Debug, ProtoList)]
    #[buffa(remote = Vec<T>)]
    pub struct List<T>(pub Vec<T>);

    impl<T> Default for List<T> {
        fn default() -> Self {
            Self(Vec::new())
        }
    }

    #[derive(MapStorage)]
    #[buffa(remote = HashMap<K, V>)]
    pub struct Map<K: Hash + Eq, V>(pub HashMap<K, V>);
}

/// One-segment override paths named like the parameters the generated
/// methods bound before #652.
mod bare_overrides {
    use super::*;

    pub fn value<T>(inner: T) -> Box<T> {
        Box::new(inner)
    }

    pub fn u<T>(inner: T) -> Box<T> {
        Box::new(inner)
    }

    pub fn key<K: Hash + Eq, V>(map: &mut HashMap<K, V>, k: K, val: V) {
        map.insert(k, val);
    }

    #[derive(ProtoBox)]
    #[buffa(remote = Box<T>, new = value, into_inner = unbox)]
    pub struct ValuePointer<T>(pub Box<T>);

    // The `Arbitrary` impl calls `new` where `u` was its parameter.
    #[derive(ProtoBox)]
    #[buffa(remote = Box<T>, new = u, into_inner = unbox, arbitrary)]
    pub struct UPointer<T>(pub Box<T>);

    #[derive(MapStorage)]
    #[buffa(remote = HashMap<K, V>, insert = key)]
    pub struct KeyMap<K: Hash + Eq, V>(pub HashMap<K, V>);
}

#[test]
fn string_beside_constants() {
    use constants::Text;
    assert_eq!(Text::from("borrowed").0, "borrowed");
    assert_eq!(Text::from(String::from("owned")).0, "owned");
    assert_eq!(Text::copy_from_str("copied").0, "copied");
    let decoded = Text::from_wire(WirePayload::borrowed(b"wire")).unwrap();
    assert_eq!(decoded.0, "wire");
}

#[test]
fn bytes_beside_constants() {
    use constants::Bytes;
    assert_eq!(Bytes::from(vec![1, 2]).0, [1, 2]);
    let decoded = Bytes::from_wire(WirePayload::borrowed(&[3, 4])).unwrap();
    assert_eq!(decoded.0, [3, 4]);
}

#[test]
fn list_beside_constants() {
    let mut list: constants::List<u32> = (1..=2).collect();
    list.push(3);
    assert_eq!(list, constants::List::from(vec![1, 2, 3]));

    let mut list: const_generics::List<u32, 1, 2, 3> = (1..=2).collect();
    list.push(3);
    assert_eq!(list, const_generics::List::from(vec![1, 2, 3]));

    let mut list = other_items::List::from(vec![1, 2]);
    list.push(3);
    assert_eq!(&*list, [1, 2, 3]);
}

#[test]
fn pointer_beside_constants() {
    assert_eq!(constants::Pointer::new(7).into_inner(), 7);
    assert_eq!(const_generics::Pointer::<_, 1, 2>::new(7).into_inner(), 7);
}

#[test]
fn map_beside_constants() {
    let mut map = constants::Map::default();
    map.storage_insert(1, "one");
    assert_eq!(map.storage_iter().collect::<Vec<_>>(), [(&1, &"one")]);

    let mut map = const_generics::Map::<_, _, 1, 2>(HashMap::new());
    map.storage_insert(1, "one");
    assert_eq!(map.storage_iter().collect::<Vec<_>>(), [(&1, &"one")]);

    let mut map = other_items::Map(HashMap::new());
    map.storage_insert(1, "one");
    assert_eq!(map.storage_iter().collect::<Vec<_>>(), [(&1, &"one")]);
}

/// The `u` parameter belongs to the `Arbitrary` impls. Each newtype builds
/// the value its canonical type builds from the same input.
#[test]
fn arbitrary_beside_constants() {
    fn check<N, C>(unwrap: impl Fn(N) -> C)
    where
        N: for<'a> Arbitrary<'a>,
        C: for<'a> Arbitrary<'a> + PartialEq + core::fmt::Debug,
    {
        let got = unwrap(N::arbitrary(&mut Unstructured::new(INPUT)).unwrap());
        let want = C::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
        assert_eq!(got, want);
        let got = unwrap(N::arbitrary_take_rest(Unstructured::new(INPUT)).unwrap());
        let want = C::arbitrary_take_rest(Unstructured::new(INPUT)).unwrap();
        assert_eq!(got, want);
    }
    check(|text: constants::Text| text.0);
    check(|bytes: constants::Bytes| bytes.0);
    check(|list: constants::List<u32>| list.0);
    check(|pointer: constants::Pointer<u64>| pointer.0);
    check(|map: constants::Map<u32, u32>| map.0);
    check(|pointer: const_generics::Pointer<u64, 1, 2>| pointer.0);
}

#[test]
fn bare_override_paths_resolve_to_the_named_functions() {
    assert_eq!(bare_overrides::ValuePointer::new(5).into_inner(), 5);

    let mut map = bare_overrides::KeyMap(HashMap::new());
    map.storage_insert(1, "one");
    assert_eq!(map.storage_iter().collect::<Vec<_>>(), [(&1, &"one")]);

    let mut input = Unstructured::new(INPUT);
    let pointer = bare_overrides::UPointer::<u64>::arbitrary(&mut input).unwrap();
    let want = u64::arbitrary(&mut Unstructured::new(INPUT)).unwrap();
    assert_eq!(pointer.into_inner(), want);
}
