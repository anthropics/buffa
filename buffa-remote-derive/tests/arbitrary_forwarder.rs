//! The `Arbitrary` impl emitted for `#[buffa(arbitrary)]`.
//!
//! Each impl is compared with the canonical owned type it builds from: same
//! value, same number of bytes consumed. The crate docs give the reason.

use std::collections::HashMap;

use arbitrary::{Arbitrary, Unstructured};
use buffa_remote_derive::{
    MapStorage as DeriveMapStorage, ProtoBox as DeriveProtoBox, ProtoBytes as DeriveProtoBytes,
    ProtoList as DeriveProtoList, ProtoString as DeriveProtoString,
};

/// Every byte is odd, which is what `Unstructured::arbitrary_iter` reads as
/// "keep going", so the collection families produce non-empty values instead
/// of stopping at zero elements. The collection tests assert non-emptiness so
/// a comparison cannot pass vacuously.
const INPUT: &[u8] = b"acegikmoqsuwyACEGIKMOQSUWYacegikmoqsuwyACEGIKMOQSUWY";

/// Two readers over the same bytes: one for the newtype, one for the canonical
/// type it must agree with.
fn twin() -> (Unstructured<'static>, Unstructured<'static>) {
    (Unstructured::new(INPUT), Unstructured::new(INPUT))
}

// `ecow::EcoString` does not implement `Arbitrary`.
#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString, arbitrary)]
struct MyEcoString(pub ecow::EcoString);

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString, arbitrary)]
struct NamedEcoString {
    inner: ecow::EcoString,
}

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoBytes)]
#[buffa(remote = smallvec::SmallVec<[u8; 16]>, arbitrary)]
struct MyBytes(pub smallvec::SmallVec<[u8; 16]>);

#[derive(Clone, PartialEq, Debug, DeriveProtoList)]
#[buffa(remote = smallvec::SmallVec<[T; 4]>, arbitrary)]
struct MyList<T>(pub smallvec::SmallVec<[T; 4]>);

impl<T> Default for MyList<T> {
    fn default() -> Self {
        Self(smallvec::SmallVec::new())
    }
}

#[derive(Clone, PartialEq, Debug, DeriveProtoBox)]
#[buffa(remote = smallbox::SmallBox<T, smallbox::space::S4>, arbitrary)]
struct MyBox<T>(pub smallbox::SmallBox<T, smallbox::space::S4>);

#[derive(Clone, PartialEq, Debug, DeriveMapStorage)]
#[buffa(remote = indexmap::IndexMap<K, V>, arbitrary)]
struct MyIndexMap<K: core::hash::Hash + Eq, V>(pub indexmap::IndexMap<K, V>);

impl<K: core::hash::Hash + Eq, V> Default for MyIndexMap<K, V> {
    fn default() -> Self {
        Self(indexmap::IndexMap::new())
    }
}

impl<K: core::hash::Hash + Eq, V> FromIterator<(K, V)> for MyIndexMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(indexmap::IndexMap::from_iter(iter))
    }
}

// `cfg(test)` is set for an integration test, so the first newtype gets the
// key and the second does not. Only the types are inspected; no value is
// built, hence `dead_code`.
#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString)]
#[cfg_attr(test, buffa(arbitrary))]
#[allow(dead_code)]
struct KeyBehindTrueCfg(pub ecow::EcoString);

#[derive(Clone, PartialEq, Default, Debug, DeriveProtoString)]
#[buffa(remote = ecow::EcoString)]
#[cfg_attr(not(test), buffa(arbitrary))]
#[allow(dead_code)]
struct KeyBehindFalseCfg(pub ecow::EcoString);

/// `<Probe<T>>::IS_ARBITRARY` is `true` when `T: Arbitrary`: the inherent
/// constant applies only where its bound holds, and the trait's is the
/// fallback.
#[allow(dead_code)]
struct Probe<T>(core::marker::PhantomData<T>);

trait NotArbitrary {
    const IS_ARBITRARY: bool = false;
}

impl<T> NotArbitrary for Probe<T> {}

#[allow(dead_code)]
impl<T: for<'a> Arbitrary<'a>> Probe<T> {
    const IS_ARBITRARY: bool = true;
}

#[test]
fn string_impl_matches_canonical_string() {
    let (mut mine, mut canonical) = twin();
    let got = MyEcoString::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = String::arbitrary(&mut canonical).expect("String builds a value");

    assert!(!want.is_empty(), "input must produce a non-empty string");
    assert_eq!(got.as_ref(), want.as_str());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "the newtype must consume exactly as many bytes as `String`"
    );
}

#[test]
fn named_field_newtype_matches_canonical_string() {
    let (mut mine, mut canonical) = twin();
    let got = NamedEcoString::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = String::arbitrary(&mut canonical).expect("String builds a value");

    assert_eq!(got.inner.as_str(), want.as_str());
    assert_eq!(mine.len(), canonical.len());
}

#[test]
fn bytes_impl_matches_canonical_vec_u8() {
    let (mut mine, mut canonical) = twin();
    let got = MyBytes::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = Vec::<u8>::arbitrary(&mut canonical).expect("Vec<u8> builds a value");

    assert!(!want.is_empty(), "input must produce a non-empty payload");
    assert_eq!(got.as_ref(), want.as_slice());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "the newtype must consume exactly as many bytes as `Vec<u8>`"
    );
}

#[test]
fn list_impl_matches_canonical_vec() {
    let (mut mine, mut canonical) = twin();
    let got = MyList::<u32>::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = Vec::<u32>::arbitrary(&mut canonical).expect("Vec<u32> builds a value");

    assert!(!want.is_empty(), "input must produce a non-empty list");
    assert_eq!(&*got, want.as_slice());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "the newtype must consume exactly as many bytes as `Vec<T>`"
    );
}

#[test]
fn box_impl_matches_canonical_box() {
    let (mut mine, mut canonical) = twin();
    let got = MyBox::<u64>::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = Box::<u64>::arbitrary(&mut canonical).expect("Box<u64> builds a value");

    assert_eq!(*got, *want);
    assert_eq!(
        mine.len(),
        canonical.len(),
        "the newtype must consume exactly as many bytes as `Box<T>`"
    );
}

#[test]
fn map_impl_matches_canonical_hash_map() {
    let (mut mine, mut canonical) = twin();
    let got = MyIndexMap::<u32, u32>::arbitrary(&mut mine).expect("the newtype builds a value");
    let want = HashMap::<u32, u32>::arbitrary(&mut canonical).expect("HashMap builds a value");

    assert!(!want.is_empty(), "input must produce a non-empty map");
    let mut got_entries: Vec<(u32, u32)> = got.0.iter().map(|(k, v)| (*k, *v)).collect();
    let mut want_entries: Vec<(u32, u32)> = want.iter().map(|(k, v)| (*k, *v)).collect();
    got_entries.sort_unstable();
    want_entries.sort_unstable();
    assert_eq!(got_entries, want_entries);
    assert_eq!(
        mine.len(),
        canonical.len(),
        "the newtype must consume exactly as many bytes as `HashMap<K, V>`"
    );
}

/// `arbitrary_take_rest` goes to `String`'s, which runs to the end of the
/// input where the trait default stops short.
#[test]
fn string_take_rest_matches_canonical_string() {
    let got = MyEcoString::arbitrary_take_rest(Unstructured::new(INPUT)).expect("take_rest builds");
    let want = String::arbitrary_take_rest(Unstructured::new(INPUT)).expect("take_rest builds");
    assert_eq!(got.as_ref(), want.as_str());

    // Guards against the trait default silently standing in: `arbitrary` and
    // `arbitrary_take_rest` must not agree here, or the assertion above is
    // testing nothing.
    let plain = String::arbitrary(&mut Unstructured::new(INPUT)).expect("arbitrary builds");
    assert_ne!(want, plain, "input must distinguish the two entry points");
}

/// `MyBox<T>` consumes what `Box<T>` consumes under `arbitrary_take_rest`,
/// for a pointee that overrides it.
#[test]
fn box_take_rest_matches_canonical_box() {
    let got =
        MyBox::<String>::arbitrary_take_rest(Unstructured::new(INPUT)).expect("take_rest builds");
    let want =
        Box::<String>::arbitrary_take_rest(Unstructured::new(INPUT)).expect("take_rest builds");
    assert_eq!(&*got, &*want);

    // The pointee has to be one that overrides `arbitrary_take_rest`, or the
    // assertion above holds vacuously: `String`'s runs to the end of the
    // buffer, while `Box<String>` stops where `String::arbitrary` stops.
    let unboxed = String::arbitrary_take_rest(Unstructured::new(INPUT)).expect("take_rest builds");
    assert_ne!(
        *want, unboxed,
        "pointee must distinguish the two entry points"
    );
}

/// No family overrides `size_hint`. `String` and `Vec<T>` report the trait
/// default too; `Box<T>` reports the pointee's hint, which the box family
/// leaves out so that a self-referential message is not walked again at
/// every level.
#[test]
fn size_hint_is_the_trait_default() {
    assert_eq!(<MyEcoString as Arbitrary>::size_hint(0), (0, None));
    assert_eq!(<MyList<u32> as Arbitrary>::size_hint(0), (0, None));
    assert_eq!(<MyBox<u64> as Arbitrary>::size_hint(0), (0, None));
    assert_eq!(<String as Arbitrary>::size_hint(0), (0, None));
    assert_eq!(<Vec<u32> as Arbitrary>::size_hint(0), (0, None));
}

/// Exhausted input lands where the canonical type lands. `arbitrary` fills
/// integers from an empty buffer rather than failing, and the newtype does
/// the same.
#[test]
fn box_impl_on_empty_input_matches_canonical_box() {
    let canonical = Box::<u64>::arbitrary(&mut Unstructured::new(&[]))
        .expect("`Box<u64>` fills from an exhausted buffer rather than failing");
    let mine = MyBox::<u64>::arbitrary(&mut Unstructured::new(&[]))
        .expect("the newtype adds no length check the canonical type does not have");
    assert_eq!(*mine, *canonical);
    assert_eq!(*canonical, 0, "`Box<u64>` from an empty buffer is 0");
}

#[test]
fn a_cfg_attr_on_the_key_decides_whether_the_impl_exists() {
    let implemented = [
        <Probe<MyEcoString>>::IS_ARBITRARY,
        <Probe<KeyBehindTrueCfg>>::IS_ARBITRARY,
        <Probe<KeyBehindFalseCfg>>::IS_ARBITRARY,
        <Probe<ecow::EcoString>>::IS_ARBITRARY,
    ];
    assert_eq!(implemented, [true, true, false, false]);
}
