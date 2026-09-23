//! Behavior of the `#[buffa(arbitrary)]` forwarder.
//!
//! The emitted impls are `#[cfg(feature = "arbitrary")]`-gated on the
//! consuming crate's feature, and this test crate *is* the consuming crate —
//! hence the file-level `cfg` and `cargo test -p buffa-remote-derive
//! --features arbitrary`. That the impls are gated at all (and that an
//! unflagged newtype gets none) is asserted at the token level in
//! `src/forwarders.rs`, which runs on the default feature set.
//!
//! Every property here is a *parity* assertion against the canonical owned
//! type the forwarder materializes: same value, and the same number of bytes
//! taken out of the `Unstructured`. Parity is the point — a message field
//! generated with a custom representation must consume a fuzz corpus exactly
//! as the default representation would.
#![cfg(feature = "arbitrary")]

use std::collections::HashMap;

use arbitrary::{Arbitrary, Unstructured};
use buffa_remote_derive::{
    MapStorage as DeriveMapStorage, ProtoBox as DeriveProtoBox, ProtoBytes as DeriveProtoBytes,
    ProtoList as DeriveProtoList, ProtoString as DeriveProtoString,
};

/// Every byte is odd, which is what `Unstructured::arbitrary_iter` reads as
/// "keep going" — so the collection families produce non-empty values instead
/// of stopping at zero elements. Each test asserts non-emptiness so a parity
/// check cannot pass vacuously.
const SEED: &[u8] = b"acegikmoqsuwyACEGIKMOQSUWYacegikmoqsuwyACEGIKMOQSUWY";

/// Two readers over the same bytes: one for the newtype, one for the canonical
/// type it must agree with.
fn twin() -> (Unstructured<'static>, Unstructured<'static>) {
    (Unstructured::new(SEED), Unstructured::new(SEED))
}

// `ecow::EcoString` has no `Arbitrary` impl of its own, which is the whole
// point: the forwarder builds a `String` and converts, so the remote type is
// never asked for one.
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

#[test]
fn string_forwarder_matches_canonical_string() {
    let (mut mine, mut canonical) = twin();
    let got = MyEcoString::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = String::arbitrary(&mut canonical).expect("String builds a value");

    assert!(!want.is_empty(), "seed must produce a non-empty string");
    assert_eq!(got.as_ref(), want.as_str());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "forwarder must consume exactly as many bytes as `String`"
    );
}

#[test]
fn named_field_newtype_forwards_too() {
    let (mut mine, mut canonical) = twin();
    let got = NamedEcoString::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = String::arbitrary(&mut canonical).expect("String builds a value");

    assert_eq!(got.inner.as_str(), want.as_str());
    assert_eq!(mine.len(), canonical.len());
}

#[test]
fn bytes_forwarder_matches_canonical_vec_u8() {
    let (mut mine, mut canonical) = twin();
    let got = MyBytes::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = Vec::<u8>::arbitrary(&mut canonical).expect("Vec<u8> builds a value");

    assert!(!want.is_empty(), "seed must produce a non-empty payload");
    assert_eq!(got.as_ref(), want.as_slice());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "forwarder must consume exactly as many bytes as `Vec<u8>`"
    );
}

#[test]
fn list_forwarder_matches_canonical_vec() {
    let (mut mine, mut canonical) = twin();
    let got = MyList::<u32>::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = Vec::<u32>::arbitrary(&mut canonical).expect("Vec<u32> builds a value");

    assert!(!want.is_empty(), "seed must produce a non-empty list");
    assert_eq!(&*got, want.as_slice());
    assert_eq!(
        mine.len(),
        canonical.len(),
        "forwarder must consume exactly as many bytes as `Vec<T>`"
    );
}

#[test]
fn box_forwarder_matches_canonical_box() {
    let (mut mine, mut canonical) = twin();
    let got = MyBox::<u64>::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = Box::<u64>::arbitrary(&mut canonical).expect("Box<u64> builds a value");

    assert_eq!(*got, *want);
    assert_eq!(
        mine.len(),
        canonical.len(),
        "forwarder must consume exactly as many bytes as `Box<T>`"
    );
}

#[test]
fn map_forwarder_matches_canonical_hash_map() {
    let (mut mine, mut canonical) = twin();
    let got = MyIndexMap::<u32, u32>::arbitrary(&mut mine).expect("forwarder builds a value");
    let want = HashMap::<u32, u32>::arbitrary(&mut canonical).expect("HashMap builds a value");

    assert!(!want.is_empty(), "seed must produce a non-empty map");
    let mut got_entries: Vec<(u32, u32)> = got.0.iter().map(|(k, v)| (*k, *v)).collect();
    let mut want_entries: Vec<(u32, u32)> = want.iter().map(|(k, v)| (*k, *v)).collect();
    got_entries.sort_unstable();
    want_entries.sort_unstable();
    assert_eq!(got_entries, want_entries);
    assert_eq!(
        mine.len(),
        canonical.len(),
        "forwarder must consume exactly as many bytes as `HashMap<K, V>`"
    );
}

/// The map forwarder goes through the user's `FromIterator`, so a duplicate
/// key must resolve the way the canonical `HashMap` resolves it (last write
/// wins) rather than being dropped or appended twice.
#[test]
fn map_forwarder_applies_last_write_wins() {
    let built = MyIndexMap::<u8, u8>::from_iter([(1u8, 10u8), (1, 20)]);
    assert_eq!(built.0.len(), 1);
    assert_eq!(built.0.get(&1), Some(&20));
}

/// `arbitrary_take_rest` is forwarded, not left at the trait default (which
/// would call `arbitrary` and stop short of the buffer's end). `String`
/// overrides it, so the difference is observable.
#[test]
fn take_rest_forwards_to_the_seed() {
    let got = MyEcoString::arbitrary_take_rest(Unstructured::new(SEED)).expect("take_rest builds");
    let want = String::arbitrary_take_rest(Unstructured::new(SEED)).expect("take_rest builds");
    assert_eq!(got.as_ref(), want.as_str());

    // Guards against the trait default silently standing in: `arbitrary` and
    // `arbitrary_take_rest` must not agree here, or the assertion above is
    // testing nothing.
    let plain = String::arbitrary(&mut Unstructured::new(SEED)).expect("arbitrary builds");
    assert_ne!(want, plain, "seed must distinguish the two entry points");
}

#[test]
fn size_hint_forwards_to_the_seed() {
    assert_eq!(
        <MyEcoString as Arbitrary>::size_hint(0),
        <String as Arbitrary>::size_hint(0)
    );
    assert_eq!(
        <MyList<u32> as Arbitrary>::size_hint(0),
        <Vec<u32> as Arbitrary>::size_hint(0)
    );
    assert_eq!(
        <MyBox<u64> as Arbitrary>::size_hint(0),
        <u64 as Arbitrary>::size_hint(0)
    );
}

/// Exhausted input must surface as the canonical type's error rather than a
/// panic or a silently-default value.
#[test]
fn empty_input_behaves_like_the_canonical_type() {
    let mine = MyBox::<u64>::arbitrary(&mut Unstructured::new(&[]));
    let canonical = Box::<u64>::arbitrary(&mut Unstructured::new(&[]));
    assert_eq!(mine.is_ok(), canonical.is_ok());
    if let (Ok(mine), Ok(canonical)) = (mine, canonical) {
        assert_eq!(*mine, *canonical);
    }
}
