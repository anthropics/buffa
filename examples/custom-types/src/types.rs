//! Crate-local newtypes that satisfy buffa's pluggable owned-type traits.
//!
//! Each newtype wraps a *foreign* storage type (`flexstr`, `smallvec`,
//! `smallbox`). The orphan rule forbids implementing buffa's traits on those
//! types directly, so a thin `#[repr(transparent)]` wrapper in this crate is
//! the bridge — the same pattern as `buffa-smolstr`, reproduced here so the
//! example is self-contained.

use buffa::map_codec::MapStorage;
use buffa::{DecodeError, ProtoBox, ProtoBytes, ProtoList, ProtoString, WirePayload};

/// `#[repr(transparent)]` guarantees a newtype has the same layout and ABI as
/// its inner type, so the wrapper is zero-cost at every boundary. Freeze that
/// against accidental regression (e.g. a second field sneaking in).
macro_rules! assert_transparent {
    ($outer:ty, $inner:ty) => {
        const _: () = {
            assert!(core::mem::size_of::<$outer>() == core::mem::size_of::<$inner>());
            assert!(core::mem::align_of::<$outer>() == core::mem::align_of::<$inner>());
        };
    };
}

// ---------------------------------------------------------------------------
// FlexStr — `string_type_custom`
// ---------------------------------------------------------------------------

/// A `ProtoString` backed by [`flexstr::SharedStr`]: short strings inline
/// (no heap), long strings shared via `Arc<str>` so clones are `O(1)`.
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FlexStr(pub flexstr::SharedStr);
assert_transparent!(FlexStr, flexstr::SharedStr);

impl core::ops::Deref for FlexStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}
impl AsRef<str> for FlexStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}
impl From<String> for FlexStr {
    #[inline]
    fn from(s: String) -> Self {
        Self(flexstr::SharedStr::from(s))
    }
}
impl From<&str> for FlexStr {
    #[inline]
    fn from(s: &str) -> Self {
        Self(flexstr::SharedStr::from_ref(s))
    }
}
impl ProtoString for FlexStr {
    /// Validate UTF-8 and build directly from the borrowed slice — short
    /// strings inline with zero heap allocation, long ones go to `Arc<str>`.
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        core::str::from_utf8(payload.as_slice())
            .map(|s| Self(flexstr::SharedStr::from_ref(s)))
            .map_err(|_| DecodeError::InvalidUtf8)
    }
}

// ---------------------------------------------------------------------------
// SmallBytes — `bytes_type_custom`
// ---------------------------------------------------------------------------

/// A `ProtoBytes` backed by an inline-capable buffer: payloads up to 24 bytes
/// live on the stack, longer ones spill to the heap.
///
/// JSON never goes through this type's own serde — codegen routes singular
/// and repeated bytes through buffa's base64 with-module, which only needs
/// `AsRef<[u8]>` / `From<Vec<u8>>`.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct SmallBytes(pub smallvec::SmallVec<[u8; 24]>);
assert_transparent!(SmallBytes, smallvec::SmallVec<[u8; 24]>);

impl core::ops::Deref for SmallBytes {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for SmallBytes {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl From<Vec<u8>> for SmallBytes {
    #[inline]
    fn from(v: Vec<u8>) -> Self {
        Self(smallvec::SmallVec::from_vec(v))
    }
}
impl ProtoBytes for SmallBytes {
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(smallvec::SmallVec::from_slice(payload.as_slice())))
    }
}

// ---------------------------------------------------------------------------
// SmallVec<T> — `repeated_type_custom` (template `crate::types::SmallVec<*>`)
// ---------------------------------------------------------------------------

/// A `ProtoList<T>` backed by [`smallvec::SmallVec`] with four inline slots.
///
/// `Default` is hand-written so `T` is **not** forced to be `Default` — a
/// derived impl would add that bound and break message types. `Eq` is omitted
/// for the same reason: deriving it would force `T: Eq`, which excludes
/// `repeated float`/`double`.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct SmallVec<T>(pub smallvec::SmallVec<[T; 4]>);
assert_transparent!(SmallVec<u32>, smallvec::SmallVec<[u32; 4]>);

impl<T> Default for SmallVec<T> {
    #[inline]
    fn default() -> Self {
        Self(smallvec::SmallVec::new())
    }
}
impl<T> core::ops::Deref for SmallVec<T> {
    type Target = [T];
    #[inline]
    fn deref(&self) -> &[T] {
        &self.0
    }
}
impl<T> FromIterator<T> for SmallVec<T> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self(smallvec::SmallVec::from_iter(iter))
    }
}
impl<T> From<Vec<T>> for SmallVec<T> {
    #[inline]
    fn from(v: Vec<T>) -> Self {
        Self(smallvec::SmallVec::from_vec(v))
    }
}
impl<T> ProtoList<T> for SmallVec<T>
where
    T: Clone + PartialEq + core::fmt::Debug + Send + Sync,
{
    #[inline]
    fn push(&mut self, value: T) {
        self.0.push(value);
    }
    #[inline]
    fn clear(&mut self) {
        self.0.clear();
    }
    // `reserve` deliberately stays the trait's no-op default: the decoder's
    // hint is advisory and may be a loose upper bound, and forwarding it to
    // `smallvec::SmallVec::reserve` would spill the inline storage on the
    // first packed read — exactly what an inline collection is meant to avoid.
}

// ---------------------------------------------------------------------------
// IndexMap<K, V> — `map_type_custom` (path applied as `path<K, V>`)
// ---------------------------------------------------------------------------

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
assert_transparent!(IndexMap<i64, u32>, indexmap::IndexMap<i64, u32>);

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

// ---------------------------------------------------------------------------
// SmallBox<T> — `box_type_custom` (template `crate::types::SmallBox<*>`)
// ---------------------------------------------------------------------------

/// A `ProtoBox<T>` backed by [`smallbox::SmallBox`]: the pointee lives inline
/// if it fits in four machine words, otherwise on the heap.
///
/// `Metadata` here is `FlexStr` (~24 bytes) + `i64`, so `S4` is the smallest
/// space that keeps it inline on 64-bit.
///
/// `Serialize` is required **only for oneof message variants**: the generated
/// oneof `Serialize` passes the stored pointer straight to serde, so the
/// pointer must serialize transparently as `T` (the default `Box<T>` gets that
/// from serde's blanket impl). Everywhere else — optional-field serialize, and
/// *all* deserialize paths — codegen routes through `ProtoBox::new` /
/// `MessageField`'s blanket serde, so no `Deserialize` impl is needed.
#[repr(transparent)]
pub struct SmallBox<T>(pub smallbox::SmallBox<T, smallbox::space::S4>);
assert_transparent!(SmallBox<u64>, smallbox::SmallBox<u64, smallbox::space::S4>);

impl<T> core::ops::Deref for SmallBox<T> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T> core::ops::DerefMut for SmallBox<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
impl<T: Clone> Clone for SmallBox<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: PartialEq> PartialEq for SmallBox<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}
impl<T: core::fmt::Debug> core::fmt::Debug for SmallBox<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(&**self, f)
    }
}
impl<T: serde::Serialize> serde::Serialize for SmallBox<T> {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        T::serialize(self, s)
    }
}
impl<T> ProtoBox<T> for SmallBox<T> {
    #[inline]
    fn new(value: T) -> Self {
        Self(smallbox::SmallBox::new(value))
    }
    #[inline]
    fn into_inner(self) -> T {
        self.0.into_inner()
    }
}
