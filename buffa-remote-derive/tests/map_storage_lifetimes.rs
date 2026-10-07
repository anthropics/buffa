use buffa::MapStorage;
use buffa_remote_derive::MapStorage as DeriveMapStorage;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};

struct BorrowedHasher<'seed, const N: usize>(&'seed [u8; N]);

impl<const N: usize> BuildHasher for BorrowedHasher<'_, N> {
    type Hasher = DefaultHasher;

    fn build_hasher(&self) -> Self::Hasher {
        let mut hasher = DefaultHasher::new();
        hasher.write(self.0);
        hasher
    }
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, BorrowedHasher<'a, N>>)]
struct TupleMap<'a, K: Hash + Eq, V, const N: usize>(HashMap<K, V, BorrowedHasher<'a, N>>);

fn entries<'map, K: 'map, V: 'map, S>(
    map: &'map HashMap<K, V, S>,
) -> impl Iterator<Item = (&'map K, &'map V)> {
    map.iter()
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, BorrowedHasher<'a, N>>, iter = entries)]
struct OverrideMap<'a, K: Hash + Eq, V, const N: usize>(HashMap<K, V, BorrowedHasher<'a, N>>);

#[test]
fn tuple_map_with_a_lifetime_supports_storage_operations() {
    let seed = [1, 2, 3];
    let mut map = TupleMap(HashMap::with_hasher(BorrowedHasher(&seed)));

    assert_eq!(map.storage_len(), 0);
    assert_eq!(map.storage_iter().next(), None);
    map.storage_insert(1, "first");
    map.storage_insert(1, "second");
    map.storage_insert(2, "other");
    assert_eq!(map.storage_len(), 2);

    let mut entries: Vec<_> = map.storage_iter().map(|(&k, &v)| (k, v)).collect();
    entries.sort_unstable();
    assert_eq!(entries, [(1, "second"), (2, "other")]);

    map.storage_clear();
    assert_eq!(map.storage_len(), 0);
    assert_eq!(map.storage_iter().next(), None);
    map.storage_insert(3, "after clear");
    assert_eq!(map.storage_iter().next(), Some((&3, &"after clear")));
}

#[test]
fn iterator_borrow_can_be_shorter_than_map_and_entry_lifetimes() {
    let seed = [4, 5];
    let key = String::from("key");
    let first_value = String::from("first");
    let second_value = String::from("second");
    let mut map = TupleMap(HashMap::with_hasher(BorrowedHasher(&seed)));

    map.storage_insert(key.as_str(), first_value.as_str());
    let first_entry = map.storage_iter().map(|(&k, &v)| (k, v)).next();
    map.storage_insert(key.as_str(), second_value.as_str());

    assert_eq!(first_entry, Some(("key", "first")));
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&"key", &"second")));
}

#[test]
fn iter_override_works_on_a_map_with_a_lifetime() {
    let seed = [1, 2];
    let mut map = OverrideMap(HashMap::with_hasher(BorrowedHasher(&seed)));

    map.storage_insert(1, 10);
    assert_eq!(map.storage_iter().next(), Some((&1, &10)));
    map.storage_insert(1, 20);
    assert_eq!(map.storage_iter().next(), Some((&1, &20)));
}
