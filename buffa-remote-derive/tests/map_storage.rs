use buffa::MapStorage;
use buffa_remote_derive::MapStorage as DeriveMapStorage;
use indexmap::IndexMap;

#[derive(Clone, PartialEq, Debug, DeriveMapStorage)]
#[buffa(remote = indexmap::IndexMap<K, V>)]
struct MyIndexMap<K: core::hash::Hash + Eq, V>(pub IndexMap<K, V>);

impl<K: core::hash::Hash + Eq, V> Default for MyIndexMap<K, V> {
    fn default() -> Self {
        Self(IndexMap::new())
    }
}

impl<K: core::hash::Hash + Eq, V> FromIterator<(K, V)> for MyIndexMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(IndexMap::from_iter(iter))
    }
}

#[test]
fn insert_len_and_clear() {
    let mut map = MyIndexMap::<i64, &str>::default();
    assert_eq!(map.storage_len(), 0);
    map.storage_insert(1, "one");
    map.storage_insert(2, "two");
    assert_eq!(map.storage_len(), 2);
    map.storage_clear();
    assert_eq!(map.storage_len(), 0);
}

#[test]
fn iter_preserves_insertion_order() {
    let map: MyIndexMap<i64, &str> = [(2, "two"), (1, "one")].into_iter().collect();
    let keys: Vec<i64> = map.storage_iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, vec![2, 1]);
}

#[test]
fn insert_overwrites_last_write_wins() {
    let mut map = MyIndexMap::<i64, &str>::default();
    map.storage_insert(1, "first");
    map.storage_insert(1, "second");
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&1, &"second")));
}

// A map with non-conventional method names, exercising the `len`/`insert`/
// `clear`/`iter` attribute overrides instead of the defaults.
#[derive(Clone, PartialEq, Debug)]
struct OddMap<K, V>(std::collections::BTreeMap<K, V>);
impl<K: Ord, V> OddMap<K, V> {
    fn count(&self) -> usize {
        self.0.len()
    }
    fn put(&mut self, key: K, value: V) {
        self.0.insert(key, value);
    }
    fn wipe(&mut self) {
        self.0.clear();
    }
    fn entries(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter()
    }
}

#[derive(Clone, PartialEq, Debug, DeriveMapStorage)]
#[buffa(
    remote = OddMap<K, V>,
    len = OddMap::count,
    insert = OddMap::put,
    clear = OddMap::wipe,
    iter = OddMap::entries
)]
struct MyOddMap<K: Ord, V>(pub OddMap<K, V>);

impl<K: Ord, V> Default for MyOddMap<K, V> {
    fn default() -> Self {
        Self(OddMap(std::collections::BTreeMap::new()))
    }
}

impl<K: Ord, V> FromIterator<(K, V)> for MyOddMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(OddMap(std::collections::BTreeMap::from_iter(iter)))
    }
}

#[test]
fn overridden_method_names_are_used() {
    let mut map = MyOddMap::<i64, &str>::default();
    map.storage_insert(1, "one");
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&1, &"one")));
    map.storage_clear();
    assert_eq!(map.storage_len(), 0);
}
