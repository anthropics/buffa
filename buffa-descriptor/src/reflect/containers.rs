//! Reflective container access for vtable-mode reflection.
//!
//! This module bridges the reflection value model ([`ValueRef`],
//! [`ReflectList`], [`ReflectMap`]) to the concrete containers generated code
//! holds, so a vtable-mode `impl ReflectMessage` can return
//! `ValueRef::List(&self.tags)` / `ValueRef::Map(&self.labels)` directly:
//!
//! - **View types** — [`RepeatedView`](buffa::RepeatedView) /
//!   [`MapView`](buffa::MapView), borrowing `&str` / `&[u8]` elements.
//! - **Owned types** — `Vec<T>` / `std::collections::HashMap<K, V>`, with owned
//!   `String` / `Vec<u8>` / [`Bytes`](buffa::bytes::Bytes) elements.
//!
//! The generic `ReflectList for Vec<T>` impl also subsumes the bridge
//! [`DynamicMessage`](super::DynamicMessage)'s `Vec<Value>` storage (via
//! `impl ReflectElement for Value`), so there is a single list impl rather than
//! a bespoke one per backing type. [`MapValue`](super::MapValue) keeps its own
//! [`ReflectMap`] impl in `value.rs` (it is a distinct sorted-vec type).
//!
//! ## Why a per-element helper trait, and why it is not a blanket
//!
//! The container impls are fully generic over the element type via
//! [`ReflectElement`] (and [`ReflectMapKey`] for map keys). The tempting shape
//! is a single blanket `impl<M: ReflectMessage> ReflectElement for M` that lets
//! repeated-of-message fall out for free. Rust rejects it (E0119): a
//! trait-bound blanket overlaps with every concrete impl — `impl ReflectElement
//! for i32` included — because the compiler cannot prove `i32: !ReflectMessage`.
//! So the closed set of element types (scalars, `&str`, `&[u8]`,
//! [`EnumValue`](buffa::EnumValue)) gets concrete impls here, and the
//! open-ended cases — message views and bare closed enums — get a one-line
//! `impl ReflectElement` emitted by codegen (a foreign-trait-for-local-type
//! impl, which the orphan rule permits in the consumer crate). That impl is per
//! *type*, not per *field*: a message used in ten repeated fields gets one impl.
//!
//! See `docs/investigations/reflection-vtable.md` §3 for the full rationale.

use alloc::string::String;
use alloc::vec::Vec;

use buffa::bytes::Bytes;
use buffa::{EnumValue, Enumeration, MapView, RepeatedView};

use super::value::{MapKey, MapKeyRef, ReflectList, ReflectMap, Value, ValueRef};

/// Conversion of a single repeated-field element (or map value) to a borrowed
/// [`ValueRef`].
///
/// Implemented here for the closed set of element types the runtime knows:
/// scalars, `&str` / `&[u8]` (view storage), `String` / `Vec<u8>` /
/// [`Bytes`](buffa::bytes::Bytes) (owned storage), [`EnumValue`], and
/// [`Value`](super::Value) (bridge storage). Codegen emits a one-line impl for
/// each generated message type (view and owned, yielding [`ValueRef::Message`])
/// and each bare closed enum (yielding [`ValueRef::EnumNumber`]) — these cannot
/// be covered by a blanket impl without colliding with the scalar impls under
/// Rust's coherence rules.
///
/// # Contract
///
/// `as_value_ref` must return the **same variant** on every call for a given
/// value — the generic [`ReflectList`] / [`ReflectMap`] impls and their callers
/// assume a `Vec<T>` / `HashMap<_, T>` is homogeneous. An implementer must also
/// derive [`Debug`] (the supertrait lets the generic container impls satisfy
/// *their* `Debug` supertrait through the container's derive).
///
/// For a custom `string_type` / `bytes_type` element used in a `repeated`
/// field under vtable reflection, codegen emits this impl for the element type
/// (there is no blanket impl — it would collide with the concrete scalar impls
/// under coherence). That emitted `impl ReflectElement for <your type>` only
/// compiles when the type is **local** to the generating crate, so a foreign
/// representation used as a repeated element must be wrapped in a crate-local
/// newtype. Singular / optional / oneof custom elements need no such impl
/// (they reflect through `Deref`).
#[rustversion::attr(
    since(1.78),
    diagnostic::on_unimplemented(
        message = "`{Self}` does not implement `ReflectElement`, which vtable-mode reflection requires on repeated-field and map-value element types",
        note = "if `{Self}` comes from another buffa-generated crate via an extern path (well-known types resolve to `buffa-types` by default), enable that crate's reflection feature, e.g. `buffa-types = {{ version = \"...\", features = [\"reflect\"] }}`",
        note = "if `{Self}` is a message generated in this crate, enable reflection in its `build.rs` config — either reflection mode emits this impl",
        note = "if `{Self}` is a custom `string_type`/`bytes_type` used as a `repeated` element, it must be a crate-local type (e.g. a newtype) so codegen can emit `impl ReflectElement` for it"
    )
)]
pub trait ReflectElement: core::fmt::Debug {
    /// Borrow this element as a [`ValueRef`].
    #[must_use]
    fn as_value_ref(&self) -> ValueRef<'_>;
}

/// Conversion of a single map key to a borrowed [`MapKeyRef`].
///
/// Implemented for the spec-valid protobuf map-key types: the integral types,
/// `bool`, and `&str`. The `&[u8]` impl exists only for the editions
/// `utf8_validation = NONE` edge case, where a `string` map key is typed as
/// bytes in the view; it converts via UTF-8 and is documented as best-effort
/// (see the impl).
///
/// The [`PartialEq`] supertrait is required so the generic [`ReflectMap`] impl
/// can deduplicate wire entries via [`MapView::iter_unique`](buffa::MapView::iter_unique),
/// matching the bridge path's distinct-key semantics. The [`Debug`] supertrait
/// plays the same role as it does for [`ReflectElement`].
pub trait ReflectMapKey: core::fmt::Debug + PartialEq {
    /// Borrow this key as a [`MapKeyRef`].
    #[must_use]
    fn as_map_key_ref(&self) -> MapKeyRef<'_>;
}

// ── Element impls (closed set) ──────────────────────────────────────────────

macro_rules! impl_scalar_element {
    ($($t:ty => $variant:ident),* $(,)?) => {$(
        impl ReflectElement for $t {
            fn as_value_ref(&self) -> ValueRef<'_> {
                ValueRef::$variant(*self)
            }
        }
    )*};
}

impl_scalar_element! {
    i32 => I32,
    i64 => I64,
    u32 => U32,
    u64 => U64,
    bool => Bool,
    f32 => F32,
    f64 => F64,
}

impl ReflectElement for &str {
    fn as_value_ref(&self) -> ValueRef<'_> {
        // `self` (a `&&str`) auto-derefs to the inner `&str`; an explicit
        // `*self` would trip `clippy::explicit_auto_deref`.
        ValueRef::String(self)
    }
}

impl ReflectElement for &[u8] {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::Bytes(self)
    }
}

impl<E: Enumeration> ReflectElement for EnumValue<E> {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::EnumNumber(self.to_i32())
    }
}

// ── Owned element impls ─────────────────────────────────────────────────────
//
// Owned messages hold `String` / `Vec<u8>` / `Bytes` (rather than the view
// path's borrowed `&str` / `&[u8]`) and store repeated/map fields as `Vec` /
// `HashMap`. These impls let the generic container impls below cover owned
// collections too. `Value` is included so the bridge `DynamicMessage`'s
// `Vec<Value>` rides the same generic `ReflectList` impl.

impl ReflectElement for String {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::String(self)
    }
}

impl ReflectElement for Vec<u8> {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::Bytes(self)
    }
}

impl ReflectElement for Bytes {
    fn as_value_ref(&self) -> ValueRef<'_> {
        ValueRef::Bytes(self)
    }
}

impl ReflectElement for Value {
    fn as_value_ref(&self) -> ValueRef<'_> {
        self.as_ref()
    }
}

// A custom `string_type`/`bytes_type` element used in a `repeated` field or a
// `map` slot under vtable reflection gets its `ReflectElement` impl (and, for a
// custom `string` map key, its `ReflectMapKey` impl) emitted by codegen into the
// generating crate (where the type is local, so the orphan rule permits it).
// Singular fields need no such impl: they reflect via `&self.field` (any repr
// derefs to `str`/`[u8]`).

// ── Map key impls (spec-valid key set) ──────────────────────────────────────

macro_rules! impl_scalar_key {
    ($($t:ty => $variant:ident),* $(,)?) => {$(
        impl ReflectMapKey for $t {
            fn as_map_key_ref(&self) -> MapKeyRef<'_> {
                MapKeyRef::$variant(*self)
            }
        }
    )*};
}

impl_scalar_key! {
    i32 => I32,
    i64 => I64,
    u32 => U32,
    u64 => U64,
    bool => Bool,
}

impl ReflectMapKey for &str {
    fn as_map_key_ref(&self) -> MapKeyRef<'_> {
        MapKeyRef::String(self)
    }
}

impl ReflectMapKey for String {
    fn as_map_key_ref(&self) -> MapKeyRef<'_> {
        MapKeyRef::String(self)
    }
}

impl ReflectMapKey for &[u8] {
    fn as_map_key_ref(&self) -> MapKeyRef<'_> {
        // Reached only for a `string` map key with editions
        // `utf8_validation = NONE`, which the view types as `&[u8]`. The proto
        // type is `string`, so the bytes are normally valid UTF-8; this is a
        // best-effort conversion, not a hard error.
        //
        // Known limitation: the reflection key model ([`MapKey`] / [`MapKeyRef`])
        // represents string keys as UTF-8, with no bytes variant (bytes are not
        // a spec-valid map key). The bridge path shares this constraint — it
        // also stores string keys as `String`. Non-UTF-8 keys therefore collapse
        // to `""`, which can collide with a genuine empty-string key. This is
        // accepted because the case is exotic (a `string` field is normally
        // UTF-8) and faithful representation is not possible without widening
        // the spec-valid key set.
        MapKeyRef::String(core::str::from_utf8(self).unwrap_or(""))
    }
}

// ── Container impls ─────────────────────────────────────────────────────────

impl<T: ReflectElement> ReflectList for RepeatedView<'_, T> {
    fn len(&self) -> usize {
        let elements: &[T] = self;
        elements.len()
    }

    fn get(&self, idx: usize) -> Option<ValueRef<'_>> {
        let elements: &[T] = self;
        elements.get(idx).map(ReflectElement::as_value_ref)
    }

    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>)) {
        for elem in self.iter() {
            f(elem.as_value_ref());
        }
    }
}

impl<K: ReflectMapKey, V: ReflectElement> ReflectMap for MapView<'_, K, V> {
    fn len(&self) -> usize {
        // Distinct-key count, matching the bridge path (`MapValue` dedups at
        // construction). `MapView` preserves duplicate wire entries, so a raw
        // `iter().count()` would over-count and diverge from `MapValue::len`.
        self.len_unique()
    }

    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>> {
        // Reverse scan finds the last occurrence first — last-write-wins,
        // matching protobuf map decode semantics and the documented `O(n)`
        // `MapView` lookup. A consumer needing `O(1)` collects into a `HashMap`.
        let want = key.as_ref();
        self.iter()
            .rev()
            .find(|(k, _)| k.as_map_key_ref() == want)
            .map(|(_, v)| v.as_value_ref())
    }

    fn get_str(&self, key: &str) -> Option<ValueRef<'_>> {
        self.iter()
            .rev()
            .find(|(k, _)| matches!(k.as_map_key_ref(), MapKeyRef::String(s) if s == key))
            .map(|(_, v)| v.as_value_ref())
    }

    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>)) {
        // Dedup to distinct keys (last-write-wins), matching the bridge path so
        // both reflection modes visit each logical entry exactly once.
        for (k, v) in self.iter_unique() {
            f(k.as_map_key_ref(), v.as_value_ref());
        }
    }
}

// ── Owned containers ────────────────────────────────────────────────────────

/// Owned repeated storage (`Vec<T>`). Also subsumes the bridge
/// `DynamicMessage`'s `Vec<Value>` (via `impl ReflectElement for Value`), so
/// there is no separate `ReflectList for Vec<Value>`.
impl<T: ReflectElement> ReflectList for Vec<T> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn get(&self, idx: usize) -> Option<ValueRef<'_>> {
        self.as_slice().get(idx).map(ReflectElement::as_value_ref)
    }

    fn for_each(&self, f: &mut dyn FnMut(ValueRef<'_>)) {
        for elem in self {
            f(elem.as_value_ref());
        }
    }
}

/// Owned map storage. Keys are unique by construction (no dedup needed). Vtable
/// reflection requires `std` (the descriptor pool uses `OnceLock`), so the
/// owned-map impl is `std`-gated and targets `std::collections::HashMap` — the
/// concrete type generated code uses for `map` fields under `std`. The impl is
/// generic over the hasher `S` so it covers both the buffa default (`foldhash`)
/// and any user-selected hasher reached via `MapRepr::Custom`.
#[cfg(feature = "std")]
impl<K: ReflectMapKey, V: ReflectElement, S: core::hash::BuildHasher> ReflectMap
    for std::collections::HashMap<K, V, S>
{
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>> {
        let want = key.as_ref();
        self.iter()
            .find(|(k, _)| k.as_map_key_ref() == want)
            .map(|(_, v)| v.as_value_ref())
    }

    fn get_str(&self, key: &str) -> Option<ValueRef<'_>> {
        self.iter()
            .find(|(k, _)| matches!(k.as_map_key_ref(), MapKeyRef::String(s) if s == key))
            .map(|(_, v)| v.as_value_ref())
    }

    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>)) {
        for (k, v) in self {
            f(k.as_map_key_ref(), v.as_value_ref());
        }
    }
}

/// `BTreeMap` is an `alloc` collection, so buffa-descriptor (which owns
/// `ReflectMap`) can implement it directly — no orphan-rule problem and no new
/// dependency. This is the asymmetry versus a *foreign* map such as
/// `indexmap::IndexMap`, which would be a foreign type for both buffa-descriptor
/// and the consumer crate and so needs a crate-local newtype. `BTreeMap` is the
/// container `buffa_build`'s `map_type(MapRepr::BTreeMap)` selects, and like the
/// `HashMap` impl above it is `std`-gated (vtable reflection requires `std`).
#[cfg(feature = "std")]
impl<K: ReflectMapKey, V: ReflectElement> ReflectMap for alloc::collections::BTreeMap<K, V> {
    fn len(&self) -> usize {
        Self::len(self)
    }

    // `get`/`get_str` are deliberate O(n) linear scans (not BTreeMap's native
    // O(log n) lookup), mirroring the `HashMap` impl above: reflective lookup
    // compares through the `MapKey` abstraction rather than the concrete key
    // type, so an ordered/hashed probe isn't available without a key conversion.
    // Reflection is not a hot path; keep parity with `HashMap` here.
    fn get(&self, key: &MapKey) -> Option<ValueRef<'_>> {
        let want = key.as_ref();
        self.iter()
            .find(|(k, _)| k.as_map_key_ref() == want)
            .map(|(_, v)| v.as_value_ref())
    }

    fn get_str(&self, key: &str) -> Option<ValueRef<'_>> {
        self.iter()
            .find(|(k, _)| matches!(k.as_map_key_ref(), MapKeyRef::String(s) if s == key))
            .map(|(_, v)| v.as_value_ref())
    }

    fn for_each(&self, f: &mut dyn FnMut(MapKeyRef<'_>, ValueRef<'_>)) {
        for (k, v) in self {
            f(k.as_map_key_ref(), v.as_value_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn repeated_scalar_list() {
        let view: RepeatedView<'_, i32> = RepeatedView::new(vec![7, 11, 13]);
        let list: &dyn ReflectList = &view;
        assert_eq!(list.len(), 3);
        assert!(!list.is_empty());
        assert!(matches!(list.get(1), Some(ValueRef::I32(11))));
        assert!(list.get(3).is_none());

        let mut sum = 0;
        list.for_each(&mut |v| {
            if let ValueRef::I32(n) = v {
                sum += n;
            }
        });
        assert_eq!(sum, 31);
    }

    #[test]
    fn repeated_string_list_is_borrowed() {
        let view: RepeatedView<'_, &str> = RepeatedView::new(vec!["a", "b"]);
        let list: &dyn ReflectList = &view;
        assert_eq!(list.len(), 2);
        assert!(matches!(list.get(0), Some(ValueRef::String("a"))));
        assert!(matches!(list.get(1), Some(ValueRef::String("b"))));
    }

    #[test]
    fn repeated_bytes_list() {
        let a: &[u8] = &[0xDE, 0xAD];
        let b: &[u8] = &[0xBE, 0xEF];
        let view: RepeatedView<'_, &[u8]> = RepeatedView::new(vec![a, b]);
        let list: &dyn ReflectList = &view;
        match list.get(0) {
            Some(ValueRef::Bytes(bytes)) => assert_eq!(bytes, &[0xDE, 0xAD]),
            other => panic!("expected Bytes, got {other:?}"),
        }
    }

    #[test]
    fn empty_list() {
        let view: RepeatedView<'_, i32> = RepeatedView::default();
        let list: &dyn ReflectList = &view;
        assert_eq!(list.len(), 0);
        assert!(list.is_empty());
        assert!(list.get(0).is_none());
    }

    #[test]
    fn map_string_keyed() {
        let view: MapView<'_, &str, i32> = MapView::new(vec![("apples", 3), ("oranges", 7)]);
        let map: &dyn ReflectMap = &view;
        assert_eq!(map.len(), 2);
        assert!(!map.is_empty());

        // The no-alloc CEL hot path.
        assert!(matches!(map.get_str("apples"), Some(ValueRef::I32(3))));
        assert!(matches!(map.get_str("oranges"), Some(ValueRef::I32(7))));
        assert!(map.get_str("durian").is_none());

        // The descriptor-keyed path.
        assert!(matches!(
            map.get(&MapKey::String("apples".into())),
            Some(ValueRef::I32(3))
        ));
        assert!(map.get(&MapKey::String("missing".into())).is_none());

        let mut total = 0;
        map.for_each(&mut |_k, v| {
            if let ValueRef::I32(n) = v {
                total += n;
            }
        });
        assert_eq!(total, 10);
    }

    #[test]
    fn map_int_keyed_get_str_returns_none() {
        let view: MapView<'_, i32, &str> = MapView::new(vec![(1, "one"), (2, "two")]);
        let map: &dyn ReflectMap = &view;
        // get_str on a non-string-keyed map yields None rather than matching.
        assert!(map.get_str("1").is_none());
        assert!(matches!(
            map.get(&MapKey::I32(2)),
            Some(ValueRef::String("two"))
        ));
    }

    #[test]
    fn map_last_write_wins_on_duplicate_keys() {
        // MapView preserves wire order including duplicates; reflection
        // resolves duplicates last-write-wins and presents distinct keys,
        // matching the bridge path (`MapValue` dedups at construction).
        let view: MapView<'_, &str, i32> = MapView::new(vec![("k", 1), ("k", 2)]);
        let map: &dyn ReflectMap = &view;
        assert!(matches!(map.get_str("k"), Some(ValueRef::I32(2))));
        assert!(matches!(
            map.get(&MapKey::String("k".into())),
            Some(ValueRef::I32(2))
        ));
        // Distinct-key count and single visit, not the 2 raw wire entries.
        assert_eq!(map.len(), 1);
        let mut visits = Vec::new();
        map.for_each(&mut |_k, v| {
            if let ValueRef::I32(n) = v {
                visits.push(n);
            }
        });
        assert_eq!(visits, vec![2]);
    }

    #[test]
    fn map_bytes_keyed_utf8() {
        // The `utf8_validation = NONE` edge case: a `string` map key typed as
        // `&[u8]` in the view. Valid UTF-8 bytes reflect as a string key.
        let apples: &[u8] = b"apples";
        let view: MapView<'_, &[u8], i32> = MapView::new(vec![(apples, 5)]);
        let map: &dyn ReflectMap = &view;
        assert_eq!(map.len(), 1);
        assert!(matches!(map.get_str("apples"), Some(ValueRef::I32(5))));
        let mut keys = Vec::new();
        map.for_each(&mut |k, _v| {
            if let MapKeyRef::String(s) = k {
                keys.push(s.to_string());
            }
        });
        assert_eq!(keys, vec!["apples".to_string()]);
    }

    // ── Owned containers (owned-message vtable path) ─────────────────────────

    #[test]
    fn owned_vec_of_string() {
        let v: Vec<String> = vec!["a".to_string(), "b".to_string()];
        let list: &dyn ReflectList = &v;
        assert_eq!(list.len(), 2);
        assert!(matches!(list.get(0), Some(ValueRef::String("a"))));
        assert!(matches!(list.get(1), Some(ValueRef::String("b"))));
    }

    #[test]
    fn owned_vec_of_value_still_reflects() {
        // The bridge `DynamicMessage` repeated storage rides the generic impl
        // via `impl ReflectElement for Value`.
        let v: Vec<Value> = vec![Value::I32(7), Value::I32(11)];
        let list: &dyn ReflectList = &v;
        assert_eq!(list.len(), 2);
        assert!(matches!(list.get(1), Some(ValueRef::I32(11))));
    }

    #[cfg(feature = "std")]
    #[test]
    fn owned_hashmap() {
        let mut m: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
        m.insert("apples".to_string(), 3);
        m.insert("oranges".to_string(), 7);
        let map: &dyn ReflectMap = &m;
        assert_eq!(map.len(), 2);
        assert!(matches!(map.get_str("apples"), Some(ValueRef::I32(3))));
        assert!(matches!(
            map.get(&MapKey::String("oranges".into())),
            Some(ValueRef::I32(7))
        ));
        assert!(map.get_str("durian").is_none());
        let mut total = 0;
        map.for_each(&mut |_k, v| {
            if let ValueRef::I32(n) = v {
                total += n;
            }
        });
        assert_eq!(total, 10);
    }
}
