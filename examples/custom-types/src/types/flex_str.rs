//! `string_type_custom("crate::types::FlexStr")`

use buffa::{DecodeError, ProtoString, WirePayload};

/// A `ProtoString` backed by [`flexstr::SharedStr`]: short strings inline
/// (no heap), long strings shared via `Arc<str>` so clones are `O(1)`.
#[derive(Clone, PartialEq, Eq, Hash, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FlexStr(pub flexstr::SharedStr);
super::assert_transparent!(FlexStr, flexstr::SharedStr);

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
