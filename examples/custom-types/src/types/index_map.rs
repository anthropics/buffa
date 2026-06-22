//! `map_type_custom("crate::types::IndexMap")`

use buffa::map_codec::MapStorage;

/// A [`MapStorage`] backed by [`indexmap::IndexMap`]: iteration follows
/// **insertion order**.
///
/// Encoded bytes and JSON output are deterministic in the order entries were
/// added — not key-sorted like `BTreeMap`, and not hash-random like the
/// default `HashMap`. `Default` is hand-written to avoid forcing
/// `K: Default` / `V: Default`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct IndexMap<K: core::hash::Hash + Eq, V>(pub indexmap::IndexMap<K, V>);
super::assert_transparent!(IndexMap<i64, u32>, indexmap::IndexMap<i64, u32>);

impl<K: core::hash::Hash + Eq, V> Default for IndexMap<K, V> {
    #[inline]
    fn default() -> Self {
        Self(indexmap::IndexMap::new())
    }
}
impl<K: core::hash::Hash + Eq, V> FromIterator<(K, V)> for IndexMap<K, V> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(indexmap::IndexMap::from_iter(iter))
    }
}
impl<K: core::hash::Hash + Eq, V> MapStorage for IndexMap<K, V> {
    type Key = K;
    type Value = V;
    #[inline]
    fn storage_len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn storage_insert(&mut self, key: K, value: V) {
        self.0.insert(key, value);
    }
    #[inline]
    fn storage_clear(&mut self) {
        self.0.clear();
    }
    #[inline]
    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a,
    {
        self.0.iter()
    }
}
