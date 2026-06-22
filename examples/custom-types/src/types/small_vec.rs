//! `repeated_type_custom("crate::types::SmallVec<*>")`

use buffa::ProtoList;

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
super::assert_transparent!(SmallVec<u32>, smallvec::SmallVec<[u32; 4]>);

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
