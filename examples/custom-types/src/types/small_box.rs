//! `box_type_custom("crate::types::SmallBox<*>")`

use buffa::ProtoBox;

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
super::assert_transparent!(SmallBox<u64>, smallbox::SmallBox<u64, smallbox::space::S4>);

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
