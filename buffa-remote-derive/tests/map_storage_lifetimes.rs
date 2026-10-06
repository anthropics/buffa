use buffa::MapStorage;
use buffa_remote_derive::MapStorage as DeriveMapStorage;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, Hasher};
use std::marker::PhantomData;

struct BorrowedHasher<'first, 'second, 'third, const N: usize> {
    first: &'first [u8; N],
    second: &'second [u8; N],
    third: &'third [u8; N],
}

impl<const N: usize> BuildHasher for BorrowedHasher<'_, '_, '_, N> {
    type Hasher = DefaultHasher;

    fn build_hasher(&self) -> Self::Hasher {
        let mut hasher = DefaultHasher::new();
        hasher.write(self.first);
        hasher.write(self.second);
        hasher.write(self.third);
        hasher
    }
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, BorrowedHasher<'a, 'a, 'a, N>>)]
struct TupleMap<'a, K: Hash + Eq, V, const N: usize>(HashMap<K, V, BorrowedHasher<'a, 'a, 'a, N>>);

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, BorrowedHasher<'__buffa_storage_iter, '__buffa_storage_iter_, '__buffa_storage_iter__, N>>)]
struct NamedMap<
    '__buffa_storage_iter,
    '__buffa_storage_iter_,
    '__buffa_storage_iter__,
    K,
    V,
    const N: usize,
> where
    K: Hash + Eq,
{
    inner: HashMap<
        K,
        V,
        BorrowedHasher<'__buffa_storage_iter, '__buffa_storage_iter_, '__buffa_storage_iter__, N>,
    >,
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, BorrowedHasher<'r#a, 'r#__buffa_storage_iter, 'r#__buffa_storage_iter_, N>>)]
struct RawMap<
    'r#a,
    'r#__buffa_storage_iter,
    'r#__buffa_storage_iter_,
    K: Hash + Eq,
    V,
    const N: usize,
>(HashMap<K, V, BorrowedHasher<'r#a, 'r#__buffa_storage_iter, 'r#__buffa_storage_iter_, N>>);

struct MarkedHasher<'seed, F>(BorrowedHasher<'seed, 'seed, 'seed, 2>, PhantomData<F>);

impl<F> BuildHasher for MarkedHasher<'_, F> {
    type Hasher = DefaultHasher;

    fn build_hasher(&self) -> Self::Hasher {
        self.0.build_hasher()
    }
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, MarkedHasher<'a, for<'__buffa_storage_iter> fn(&'__buffa_storage_iter str)>>)]
#[allow(clippy::type_complexity)]
struct HigherRankedMap<'a, K: Hash + Eq, V>(
    HashMap<K, V, MarkedHasher<'a, for<'__buffa_storage_iter> fn(&'__buffa_storage_iter str)>>,
);

struct IterOverride<F>(PhantomData<F>);

impl<F> IterOverride<F> {
    fn entries<'map, K: 'map, V: 'map, S>(
        map: &'map HashMap<K, V, S>,
    ) -> impl Iterator<Item = (&'map K, &'map V)> {
        map.iter()
    }
}

#[derive(DeriveMapStorage)]
#[buffa(
    remote = HashMap<K, V, BorrowedHasher<'a, 'a, 'a, N>>,
    iter = IterOverride::<for<'__buffa_storage_iter> fn(&'__buffa_storage_iter str)>::entries
)]
struct OverrideMap<'a, K: Hash + Eq, V, const N: usize>(
    HashMap<K, V, BorrowedHasher<'a, 'a, 'a, N>>,
);

#[test]
fn tuple_map_with_a_lifetime_supports_storage_operations() {
    let seed = [1, 2, 3];
    let mut map = TupleMap(HashMap::with_hasher(BorrowedHasher {
        first: &seed,
        second: &seed,
        third: &seed,
    }));

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
fn named_map_skips_multiple_colliding_lifetimes() {
    let first = [1];
    let second = [2];
    let third = [3];
    let mut map = NamedMap {
        inner: HashMap::with_hasher(BorrowedHasher {
            first: &first,
            second: &second,
            third: &third,
        }),
    };

    map.storage_insert(1, 10);
    assert_eq!(map.storage_iter().next(), Some((&1, &10)));
    map.storage_insert(1, 20);
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&1, &20)));
    map.storage_clear();
    assert_eq!(map.storage_iter().next(), None);
}

#[test]
fn raw_lifetime_names_do_not_shadow_iterator_lifetime() {
    let first = [1, 2];
    let second = [3, 4];
    let mut map = RawMap(HashMap::with_hasher(BorrowedHasher {
        first: &first,
        second: &second,
        third: &first,
    }));

    map.storage_insert(1, "first");
    assert_eq!(map.storage_iter().next(), Some((&1, &"first")));
    map.storage_insert(1, "second");
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&1, &"second")));
    map.storage_clear();
    assert_eq!(map.storage_iter().next(), None);
}

#[test]
fn iterator_lifetime_does_not_shadow_a_field_types_higher_ranked_bound() {
    let seed = [1, 2];
    let mut map = HigherRankedMap(HashMap::with_hasher(MarkedHasher(
        BorrowedHasher {
            first: &seed,
            second: &seed,
            third: &seed,
        },
        PhantomData,
    )));

    map.storage_insert(1, 10);
    assert_eq!(map.storage_iter().next(), Some((&1, &10)));
    map.storage_clear();
    assert_eq!(map.storage_len(), 0);
}

#[test]
fn iterator_borrow_can_be_shorter_than_map_and_entry_lifetimes() {
    let seed = [4, 5];
    let key = String::from("key");
    let first_value = String::from("first");
    let second_value = String::from("second");
    let mut map = TupleMap(HashMap::with_hasher(BorrowedHasher {
        first: &seed,
        second: &seed,
        third: &seed,
    }));

    map.storage_insert(key.as_str(), first_value.as_str());
    let first_entry = map.storage_iter().map(|(&k, &v)| (k, v)).next();
    map.storage_insert(key.as_str(), second_value.as_str());

    assert_eq!(first_entry, Some(("key", "first")));
    assert_eq!(map.storage_len(), 1);
    assert_eq!(map.storage_iter().next(), Some((&"key", &"second")));
}

#[test]
fn iterator_lifetime_does_not_shadow_an_overrides_higher_ranked_bound() {
    let seed = [1, 2];
    let mut map = OverrideMap(HashMap::with_hasher(BorrowedHasher {
        first: &seed,
        second: &seed,
        third: &seed,
    }));

    map.storage_insert(1, 10);
    assert_eq!(map.storage_iter().next(), Some((&1, &10)));
    map.storage_insert(1, 20);
    assert_eq!(map.storage_iter().next(), Some((&1, &20)));
}

macro_rules! higher_ranked_marker {
    () => {
        for<'a_> fn(&'a_ str)
    };
}

#[derive(DeriveMapStorage)]
#[buffa(remote = HashMap<K, V, MarkedHasher<'a, higher_ranked_marker!()>>)]
#[allow(clippy::type_complexity)]
struct MacroMap<'a, K: Hash + Eq, V>(HashMap<K, V, MarkedHasher<'a, higher_ranked_marker!()>>);

#[test]
fn iterator_lifetime_does_not_shadow_a_macros_higher_ranked_bound() {
    let seed = [1, 2];
    let mut map = MacroMap(HashMap::with_hasher(MarkedHasher(
        BorrowedHasher {
            first: &seed,
            second: &seed,
            third: &seed,
        },
        PhantomData,
    )));
    map.storage_insert(1, 10);
    assert_eq!(map.storage_iter().next(), Some((&1, &10)));
}
