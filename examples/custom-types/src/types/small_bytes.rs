//! `bytes_type_custom("crate::types::SmallBytes")`

use buffa::{DecodeError, ProtoBytes, WirePayload};

/// A `ProtoBytes` backed by an inline-capable buffer: payloads up to 24 bytes
/// live on the stack, longer ones spill to the heap.
///
/// JSON never goes through this type's own serde — codegen routes singular
/// and repeated bytes through buffa's base64 with-module, which only needs
/// `AsRef<[u8]>` / `From<Vec<u8>>`.
#[derive(Clone, PartialEq, Eq, Default, Debug)]
#[repr(transparent)]
pub struct SmallBytes(pub smallvec::SmallVec<[u8; 24]>);
super::assert_transparent!(SmallBytes, smallvec::SmallVec<[u8; 24]>);

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
