//! Runtime value representation for reflection.
//!
//! [`Value`] is the owned form, [`ValueRef`] the borrowed form, [`MapKey`]
//! the restricted set of types usable as protobuf map keys, [`MapValue`] the
//! map container. Scalars are stored inline (no heap boxing) — this is the
//! same rationale protobuf-go gives for hand-rolling its 24-byte `Value`
//! union, except in Rust the tagged enum is the natural representation.
//!
//! `ValueRef` is sized at ~32 bytes: the largest variant is `String(&str)`
//! / `Bytes(&[u8])` at 16 bytes plus tag and padding, or `Message(ReflectCow)`
//! at 16 bytes (one `Box<dyn ...>` or `&dyn ...` fat pointer plus a 1-byte
//! tag). Scalar reads — the hot path for field-mask application and
//! interceptors — are stack-only.
//!
//! [`MapValue`] is a sorted `Vec<(MapKey, Value)>` rather than a `BTreeMap`.
//! That choice is load-bearing for two reasons:
//!
//! 1. **Allocation-free string lookup.** A `BTreeMap<MapKey, Value>::get`
//!    needs a `&MapKey`, which for a string key means constructing
//!    `MapKey::String(name.to_owned())` per access. CEL evaluating
//!    `m["key"]` does this in a hot loop. A sorted `Vec` lets
//!    [`MapValue::get_str`] `binary_search_by` against the borrowed `&str`
//!    directly.
//! 2. **`const` empty.** `Vec::new()` is `const fn`, so the absent-map
//!    [`ValueRef::Map`] can borrow a real `static MapValue` with no leak
//!    pattern, no `OnceLock`, and no `unsafe`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use super::message::{ReflectCow, ReflectMessage};
use super::DynamicMessage;

/// An owned reflective value.
///
/// Mirrors the wire-level scalar types (`I32` covers `int32`/`sint32`/
/// `sfixed32` — the wire form, not the proto type). Container variants
/// hold owned collections.
///
/// The variant set is closed: it covers the wire format's scalar/aggregate
/// types and will not gain new variants. Exhaustive matching is safe.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    /// An enum value as its raw `i32` number. Both open and closed enums
    /// store the number; whether out-of-range values are valid is a property
    /// of the [`EnumDescriptor`](crate::EnumDescriptor).
    EnumNumber(i32),
    /// A nested dynamic message.
    Message(DynamicMessage),
    /// A repeated field's elements, in wire order.
    List(Vec<Value>),
    /// A `map<K, V>` field's entries.
    Map(MapValue),
}

/// A borrowed reflective value.
///
/// Returned by [`ReflectMessage::get`]. Scalar variants are inline copies
/// (`Copy` types); container variants borrow from the source message.
/// `Message` carries a [`ReflectCow`] so a nested generated message can be
/// either borrowed (vtable mode) or owned (bridge mode).
///
/// The variant set is closed (same as [`Value`]); exhaustive matching is safe.
#[derive(Debug)]
pub enum ValueRef<'a> {
    Bool(bool),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    F32(f32),
    F64(f64),
    String(&'a str),
    Bytes(&'a [u8]),
    EnumNumber(i32),
    Message(ReflectCow<'a>),
    /// A repeated field, accessed through [`ReflectList`].
    ///
    /// This is a trait object rather than `&[Value]` so a future vtable-mode
    /// `impl ReflectMessage for FooView<'a>` can hand out a borrow of its
    /// `RepeatedView<'a, T>` without materializing a `Vec<Value>`. Bridge
    /// mode (`DynamicMessage`) implements [`ReflectList`] for `[Value]`, so
    /// the bridge path is a slice with one extra vtable indirection per
    /// element access. Both are 16-byte fat pointers.
    List(&'a dyn ReflectList),
    /// A map field, accessed through [`ReflectMap`].
    ///
    /// Same rationale as `List` — a trait object so vtable mode can borrow
    /// `MapView<'a, K, V>` directly.
    Map(&'a dyn ReflectMap),
}

impl Value {
    /// Borrow this value as a [`ValueRef`].
    #[must_use]
    pub fn as_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Bool(v) => ValueRef::Bool(*v),
            Self::I32(v) => ValueRef::I32(*v),
            Self::I64(v) => ValueRef::I64(*v),
            Self::U32(v) => ValueRef::U32(*v),
            Self::U64(v) => ValueRef::U64(*v),
            Self::F32(v) => ValueRef::F32(*v),
            Self::F64(v) => ValueRef::F64(*v),
            Self::String(v) => ValueRef::String(v),
            Self::Bytes(v) => ValueRef::Bytes(v),
            Self::EnumNumber(v) => ValueRef::EnumNumber(*v),
            Self::Message(v) => ValueRef::Message(ReflectCow::Borrowed(v as &dyn ReflectMessage)),
            Self::List(v) => ValueRef::List(v),
            Self::Map(v) => ValueRef::Map(v),
        }
    }
}

impl<'a> ValueRef<'a> {
    /// Convert this borrowed value into an owned [`Value`], cloning as needed.
    ///
    /// For `Message`, the dynamic snapshot is taken via
    /// [`ReflectMessage::to_dynamic`], which is a clone for already-dynamic
    /// messages and an encode/decode round-trip for bridge-mode generated
    /// types.
    ///
    /// # Performance
    ///
    /// `List` and `Map` are deep-cloned (every element through the
    /// [`ReflectList`]/[`ReflectMap`] surface). `Message` pays a wire
    /// round-trip in bridge mode. For a CEL workload that needs an owned
    /// copy of a single scalar, prefer matching the scalar variant directly
    /// rather than calling `to_owned` on the whole `ValueRef`.
    #[must_use]
    pub fn to_owned(&self) -> Value {
        match self {
            Self::Bool(v) => Value::Bool(*v),
            Self::I32(v) => Value::I32(*v),
            Self::I64(v) => Value::I64(*v),
            Self::U32(v) => Value::U32(*v),
            Self::U64(v) => Value::U64(*v),
            Self::F32(v) => Value::F32(*v),
            Self::F64(v) => Value::F64(*v),
            Self::String(v) => Value::String((*v).into()),
            Self::Bytes(v) => Value::Bytes((*v).into()),
            Self::EnumNumber(v) => Value::EnumNumber(*v),
            Self::Message(cow) => Value::Message(cow.to_dynamic()),
            Self::List(v) => {
                let mut out = Vec::with_capacity(v.len());
                v.for_each(&mut |elem| out.push(elem.to_owned()));
                Value::List(out)
            }
            Self::Map(v) => {
                let mut out = Vec::with_capacity(v.len());
                v.for_each(&mut |k, val| out.push((k.to_owned(), val.to_owned())));
                Value::Map(MapValue::from_entries(out))
            }
        }
    }
}

// ── Reflective container traits ─────────────────────────────────────────────

/// A reflective view over a repeated field's elements.
///
/// `DynamicMessage` (bridge mode) implements this for `Vec<Value>`. Vtable mode
/// implements it generically for [`RepeatedView`](buffa::RepeatedView) over any
/// [`ReflectElement`](super::ReflectElement), so a repeated field can be read
/// without materializing a `Vec<Value>`.
///
/// `for_each` is the dyn-safe non-allocating iteration form, mirroring
/// [`ReflectMessage::for_each_set`]. There is no `iter()` because trait
/// objects can't return `impl Iterator` and `Box<dyn Iterator>` would put a
/// heap allocation on the hot path.
///
/// The `Debug` supertrait lets `ValueRef` derive `Debug` (a `Value::List`
/// debug-formats as a slice; a future `RepeatedView` debug-formats as the
/// view).
pub trait ReflectList: core::fmt::Debug {
    /// Number of elements.
    fn len(&self) -> usize;
    /// Whether the list is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Element at `idx`, or `None` if out of bounds.
    fn get(&self, idx: usize) -> Option<ValueRef<'_>>;
    /// Visit each element in wire order.
    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>));
}

/// A reflective view over a map field's entries.
///
/// `DynamicMessage` (bridge mode) implements this for [`MapValue`]. Vtable mode
/// implements it generically for [`MapView`](buffa::MapView), deduplicating
/// duplicate wire entries to distinct keys so both modes present the same
/// logical map.
///
/// Iteration order is unspecified — callers must not depend on it. The
/// bridge-mode `MapValue` happens to iterate in `MapKey`-sorted order; vtable
/// mode may iterate in declaration order. Both are spec-valid.
pub trait ReflectMap: core::fmt::Debug {
    /// Number of entries.
    fn len(&self) -> usize;
    /// Whether the map is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Look up by key.
    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>>;
    /// Look up by string key, with no `MapKey` allocation. Returns `None`
    /// if the map is not string-keyed. The CEL `m["key"]` hot path.
    fn get_str(&self, key: &str) -> Option<ValueRef<'_>>;
    /// Visit each `(key, value)` entry.
    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>));
}

/// A borrowed protobuf map key.
///
/// The borrowed counterpart of [`MapKey`] — same restricted variant set
/// (integral, `bool`, `string`; floats and bytes are not valid map keys).
/// Returned by [`ReflectMap::for_each`] so consumers can match exhaustively
/// over only the spec-valid key types.
///
/// The variant set is closed; exhaustive matching is safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapKeyRef<'a> {
    Bool(bool),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    String(&'a str),
}

impl MapKeyRef<'_> {
    /// Convert to an owned [`MapKey`], cloning the string variant.
    ///
    /// Takes `self` by value because `MapKeyRef` is `Copy` (the largest
    /// variant is a `&str` fat pointer).
    #[must_use]
    pub fn to_owned(self) -> MapKey {
        match self {
            Self::Bool(v) => MapKey::Bool(v),
            Self::I32(v) => MapKey::I32(v),
            Self::I64(v) => MapKey::I64(v),
            Self::U32(v) => MapKey::U32(v),
            Self::U64(v) => MapKey::U64(v),
            Self::String(v) => MapKey::String(v.into()),
        }
    }
}

impl MapKey {
    /// Borrow this key as a [`MapKeyRef`].
    #[must_use]
    pub fn as_ref(&self) -> MapKeyRef<'_> {
        match self {
            Self::Bool(v) => MapKeyRef::Bool(*v),
            Self::I32(v) => MapKeyRef::I32(*v),
            Self::I64(v) => MapKeyRef::I64(*v),
            Self::U32(v) => MapKeyRef::U32(*v),
            Self::U64(v) => MapKeyRef::U64(*v),
            Self::String(v) => MapKeyRef::String(v),
        }
    }
}

// `Vec<Value>` reflects through the generic `impl<T: ReflectElement>
// ReflectList for Vec<T>` in `view.rs` (with `impl ReflectElement for Value`),
// so there is no bespoke `ReflectList for Vec<Value>` here.

impl ReflectMap for MapValue {
    fn len(&self) -> usize {
        Self::len(self)
    }
    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>> {
        Self::get(self, key).map(Value::as_ref)
    }
    fn get_str(&self, key: &str) -> Option<ValueRef<'_>> {
        Self::get_str(self, key).map(Value::as_ref)
    }
    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>)) {
        for (k, v) in self.iter() {
            f(k.as_ref(), v.as_ref());
        }
    }
}

/// A protobuf map key.
///
/// Per the protobuf spec, map keys are restricted to integral types, `bool`,
/// and `string`. Floats and bytes are not allowed.
///
/// The variant set is closed; exhaustive matching is safe.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapKey {
    Bool(bool),
    I32(i32),
    I64(i64),
    U32(u32),
    U64(u64),
    String(String),
}

/// A protobuf `map<K, V>` field's entries.
///
/// Stored as a `Vec<(MapKey, Value)>` sorted by key. The invariant — sorted,
/// no duplicate keys — is maintained by every constructor and mutator on this
/// type. Reading the entries directly via [`Self::entries`] sees the sorted
/// form; constructing via [`Self::from_entries`] or [`Self::insert`] enforces
/// the invariant.
///
/// Lookup is `O(log n)` binary search. The string-keyed variants
/// ([`Self::get_str`]) compare borrowed `&str` directly with no `MapKey`
/// allocation — that's the CEL `m["key"]` hot path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapValue {
    /// Entries sorted by [`MapKey`]'s `Ord`, no duplicates.
    entries: Vec<(MapKey, Value)>,
}

impl MapValue {
    /// An empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Build a map from a list of entries. Sorts by key; on duplicate keys
    /// the **last** entry wins, matching the protobuf wire-decode semantics
    /// (later map-entry fields overwrite earlier ones).
    #[must_use]
    pub fn from_entries(mut entries: Vec<(MapKey, Value)>) -> Self {
        // Stable sort preserves source order within each key group, so a
        // forward dedup that keeps the last entry per key implements
        // last-write-wins.
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut deduped: Vec<(MapKey, Value)> = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(last) = deduped.last_mut() {
                if last.0 == entry.0 {
                    *last = entry;
                    continue;
                }
            }
            deduped.push(entry);
        }
        Self { entries: deduped }
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the map is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a value by `MapKey`. `O(log n)`.
    #[must_use]
    pub fn get(&self, key: &MapKey) -> Option<&Value> {
        self.entries
            .binary_search_by(|(k, _)| k.cmp(key))
            .ok()
            .map(|i| &self.entries[i].1)
    }

    /// Look up a value by string key, with no `MapKey` allocation. `O(log n)`.
    ///
    /// Returns `None` if the map is not string-keyed. This is the CEL
    /// `m["key"]` hot path — no `MapKey::String(name.to_owned())` per call.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(ValueRef::Map(m)) = msg.field_by_name("labels") {
    ///     let v = m.get_str("env"); // borrows the &str, no allocation
    /// }
    /// ```
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&Value> {
        // The protobuf spec restricts a `map<K, V>` field to a single key
        // type, so a well-formed map's entries are homogeneous. The
        // comparator below would be non-total over a mixed-key map; this
        // assert catches a corrupt insert in tests before it can confuse
        // a binary search.
        debug_assert!(
            self.entries
                .first()
                // MSRV: `Option::is_none_or` requires 1.82.
                .map_or(true, |(k, _)| matches!(k, MapKey::String(_))),
            "get_str called on a non-string-keyed MapValue"
        );
        self.entries
            .binary_search_by(|(k, _)| match k {
                MapKey::String(s) => s.as_str().cmp(key),
                // Unreachable for a well-formed map; the debug_assert above
                // catches the corrupt case in tests. Total-order fallback.
                _ => core::cmp::Ordering::Less,
            })
            .ok()
            .and_then(|i| match &self.entries[i].0 {
                MapKey::String(_) => Some(&self.entries[i].1),
                _ => None,
            })
    }

    /// Look up a value by `i64` key. Resolves to `I32`, `I64`, `U32`, or
    /// `U64` keys depending on what the map holds. `O(log n)`.
    #[must_use]
    pub fn get_i64(&self, key: i64) -> Option<&Value> {
        match self.entries.first().map(|(k, _)| k) {
            Some(MapKey::I32(_)) => self.get(&MapKey::I32(i32::try_from(key).ok()?)),
            Some(MapKey::I64(_)) => self.get(&MapKey::I64(key)),
            Some(MapKey::U32(_)) => self.get(&MapKey::U32(u32::try_from(key).ok()?)),
            Some(MapKey::U64(_)) => self.get(&MapKey::U64(u64::try_from(key).ok()?)),
            _ => None,
        }
    }

    /// Insert or replace an entry. `O(log n)` lookup plus `O(n)` insert if
    /// the key is new (a `Vec` shift). Decode paths should batch entries via
    /// [`Self::from_entries`] instead.
    pub fn insert(&mut self, key: MapKey, value: Value) {
        match self.entries.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(i) => self.entries[i].1 = value,
            Err(i) => self.entries.insert(i, (key, value)),
        }
    }

    /// Iterate the entries in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&MapKey, &Value)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    /// The sorted entries slice. Read-only — the invariant (sorted, no
    /// duplicates) is maintained by the constructors and mutators.
    #[must_use]
    pub fn entries(&self) -> &[(MapKey, Value)] {
        &self.entries
    }
}

impl FromIterator<(MapKey, Value)> for MapValue {
    fn from_iter<T: IntoIterator<Item = (MapKey, Value)>>(iter: T) -> Self {
        Self::from_entries(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a MapValue {
    type Item = (&'a MapKey, &'a Value);
    type IntoIter = core::iter::Map<
        core::slice::Iter<'a, (MapKey, Value)>,
        fn(&'a (MapKey, Value)) -> (&'a MapKey, &'a Value),
    >;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

const _: () = {
    // Lock in the ValueRef size budget. The design's allocation analysis
    // depends on Message(ReflectCow) staying at one fat-pointer width plus
    // tag; if a refactor inadvertently inlines DynamicMessage into ReflectCow
    // (e.g. by changing Box<dyn> to a struct), this assertion catches it.
    assert!(core::mem::size_of::<ValueRef<'_>>() <= 32);
    assert!(core::mem::size_of::<ReflectCow<'_>>() <= 24);
    let _ = Box::<()>::new; // suppress unused-import if alloc reorganizes
};

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    #[test]
    fn map_value_from_entries_dedup_keeps_last() {
        let m = MapValue::from_entries(vec![
            (MapKey::String("a".into()), Value::I32(1)),
            (MapKey::String("b".into()), Value::I32(2)),
            (MapKey::String("a".into()), Value::I32(3)),
        ]);
        assert_eq!(m.len(), 2);
        assert_eq!(m.get_str("a"), Some(&Value::I32(3)));
        assert_eq!(m.get_str("b"), Some(&Value::I32(2)));
    }

    #[test]
    fn map_value_get_str_no_alloc() {
        let m = MapValue::from_entries(vec![
            (MapKey::String("apple".into()), Value::I32(1)),
            (MapKey::String("banana".into()), Value::I32(2)),
            (MapKey::String("cherry".into()), Value::I32(3)),
        ]);
        // Lookup by borrowed &str — no MapKey constructed.
        assert_eq!(m.get_str("banana"), Some(&Value::I32(2)));
        assert_eq!(m.get_str("durian"), None);
    }

    #[test]
    fn map_value_insert_maintains_sort() {
        let mut m = MapValue::new();
        m.insert(MapKey::String("z".into()), Value::I32(26));
        m.insert(MapKey::String("a".into()), Value::I32(1));
        m.insert(MapKey::String("m".into()), Value::I32(13));
        m.insert(MapKey::String("a".into()), Value::I32(100)); // replace
        let keys: Vec<_> = m.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys,
            vec![
                MapKey::String("a".into()),
                MapKey::String("m".into()),
                MapKey::String("z".into())
            ]
        );
        assert_eq!(m.get_str("a"), Some(&Value::I32(100)));
    }

    #[test]
    fn map_value_get_i64() {
        let m = MapValue::from_entries(vec![
            (MapKey::I32(1), Value::String("one".to_string())),
            (MapKey::I32(2), Value::String("two".to_string())),
        ]);
        assert_eq!(m.get_i64(1), Some(&Value::String("one".to_string())));
        assert_eq!(m.get_i64(3), None);
        // Out of i32 range → None.
        assert_eq!(m.get_i64(i64::MAX), None);
    }

    #[test]
    fn map_value_const_empty() {
        // The const constructor proves the empty case needs no heap
        // allocation and no leak pattern.
        const EMPTY: MapValue = MapValue::new();
        assert!(EMPTY.is_empty());
        assert_eq!(EMPTY.get_str("anything"), None);
    }
}
