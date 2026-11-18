//! Destination cleanup runs only when an entry slot is discarded.

use buffa::bytes::Buf;
use buffa::map_codec::{self, ClosedEnum, MapStorage, Uint32, Uint64};
use buffa::{DecodeContext, DecodeError, Enumeration, UnknownFields};
use std::cell::Cell;
use std::collections::BTreeMap;

thread_local! {
    static ERASED: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
}

#[derive(Default)]
struct ObservedMap<V>(BTreeMap<u32, V>);

impl<V: Default> MapStorage for ObservedMap<V> {
    type Key = u32;
    type Value = V;

    fn storage_len(&self) -> usize {
        self.0.len()
    }

    fn storage_insert(&mut self, key: u32, value: V) {
        self.0.insert(key, value);
    }

    fn storage_clear(&mut self) {
        self.0.clear();
    }

    fn storage_erase_key(key: &mut u32) {
        *key = 0;
        ERASED.with(|count| {
            let (keys, values) = count.get();
            count.set((keys.checked_add(1).unwrap(), values));
        });
    }

    fn storage_erase_value(value: &mut V) {
        *value = V::default();
        ERASED.with(|count| {
            let (keys, values) = count.get();
            count.set((keys, values.checked_add(1).unwrap()));
        });
    }

    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a u32, &'a V)>
    where
        V: 'a,
    {
        self.0.iter()
    }
}

fn reset() {
    ERASED.with(|count| count.set((0, 0)));
}

fn assert_discard_policy() {
    let expected = if cfg!(feature = "zeroize") {
        (1, 1)
    } else {
        (0, 0)
    };
    ERASED.with(|count| assert_eq!(count.get(), expected));
}

#[test]
fn insertion_transfers_both_slots_without_erasing_them() {
    reset();
    let limit = Cell::new(buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
    let ctx = DecodeContext::new(buffa::RECURSION_LIMIT, &limit);
    let mut map = ObservedMap::<u64>::default();
    let mut wire = &[4, 0x08, 7, 0x10, 37][..];
    map_codec::merge_entry::<Uint32, Uint64, _>(&mut map, &mut wire, ctx).unwrap();
    assert_eq!(map.0.get(&7), Some(&37));
    assert!(wire.is_empty());
    ERASED.with(|count| assert_eq!(count.get(), (0, 0)));
}

#[test]
fn malformed_value_discards_the_partial_key_and_value() {
    reset();
    let limit = Cell::new(buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
    let ctx = DecodeContext::new(buffa::RECURSION_LIMIT, &limit);
    let mut map = ObservedMap::<u64>::default();
    let mut wire = &[4, 0x08, 7, 0x10, 0x80][..];
    let result = map_codec::merge_entry::<Uint32, Uint64, _>(&mut map, &mut wire, ctx);
    assert!(matches!(result, Err(DecodeError::UnexpectedEof)));
    assert!(map.0.is_empty());
    assert_discard_policy();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
enum Value {
    #[default]
    Known,
}

impl Enumeration for Value {
    fn from_i32(value: i32) -> Option<Self> {
        (value == 37).then_some(Self::Known)
    }

    fn to_i32(&self) -> i32 {
        37
    }

    fn proto_name(&self) -> &'static str {
        "KNOWN"
    }
}

fn decode_unknown(buf: &mut impl Buf, preserve: bool) -> UnknownFields {
    let limit = Cell::new(buffa::DEFAULT_UNKNOWN_FIELD_LIMIT);
    let ctx = DecodeContext::new(buffa::RECURSION_LIMIT, &limit);
    let mut map = ObservedMap::<Value>::default();
    let mut unknown = UnknownFields::default();
    let destination = preserve.then_some((5, &mut unknown));
    map_codec::merge_entry_with_unknowns::<Uint32, ClosedEnum<Value>, _>(
        &mut map,
        buf,
        ctx,
        destination,
    )
    .unwrap();
    assert!(map.0.is_empty());
    assert!(!buf.has_remaining());
    assert_discard_policy();
    unknown
}

#[test]
fn discarded_unknown_enum_entries_follow_the_destination_policy() {
    reset();
    assert!(decode_unknown(&mut &[4, 0x08, 7, 0x10, 77][..], false).is_empty());
}

#[test]
fn preserved_contiguous_unknown_entries_erase_their_temporary_slots() {
    reset();
    let unknown = decode_unknown(&mut &[4, 0x08, 7, 0x10, 77][..], true);
    assert_eq!(unknown.len(), 1);
}

#[test]
fn preserved_fragmented_unknown_entries_erase_their_temporary_slots() {
    reset();
    let wire = [4, 0x08, 7, 0x10, 77];
    let mut fragmented = wire[..2].chain(&wire[2..]);
    let unknown = decode_unknown(&mut fragmented, true);
    assert_eq!(unknown.len(), 1);
}
