//! [`buffa::ProtoString`] support for [`smol_str::SmolStr`].
//!
//! `SmolStr` is foreign to both `buffa` and this crate's consumers, so the
//! orphan rule forbids implementing `ProtoString` for it directly. This crate
//! provides a thin newtype, [`SmolStr`], that implements the full trait —
//! including an `O(1)`, allocation-free [`from_wire`](buffa::ProtoString::from_wire)
//! for short strings (inlined up to `smol_str`'s 23-byte capacity).
//!
//! Point `buffa_build`'s `string_type_custom` at the newtype path:
//!
//! ```rust,ignore
//! buffa_build::Config::new()
//!     .string_type_custom("::buffa_smolstr::SmolStr")
//!     .compile()?;
//! ```
//!
//! It is also the template for the other preset crates and for downstream
//! custom-type fixtures.

use buffa::{DecodeError, ProtoString, WirePayload};

/// A newtype around [`smol_str::SmolStr`] implementing [`buffa::ProtoString`].
///
/// Inlines strings up to 23 bytes (no heap allocation) and clones long strings
/// in `O(1)` via the inner `Arc<str>`. Immutable: assign a new value to mutate.
///
/// Under the `serde` feature it serializes transparently as a JSON string, so it
/// also works in `optional` / `repeated` string fields (which serialize through
/// the element's native serde rather than buffa's `proto_string` with-module).
#[derive(Clone, PartialEq, Eq, Default, Debug, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct SmolStr(pub smol_str::SmolStr);

// `#[repr(transparent)]` guarantees this newtype has the same layout and ABI as
// the inner `smol_str::SmolStr` — so storing it in a field or passing it by
// value/reference is free, with no wrapper word and no conversion at the
// boundary. Freeze that guarantee against accidental regression.
const _: () = {
    assert!(core::mem::size_of::<SmolStr>() == core::mem::size_of::<smol_str::SmolStr>());
    assert!(core::mem::align_of::<SmolStr>() == core::mem::align_of::<smol_str::SmolStr>());
};

impl SmolStr {
    /// Borrow as `&str`.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl core::ops::Deref for SmolStr {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for SmolStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<String> for SmolStr {
    #[inline]
    fn from(s: String) -> Self {
        Self(smol_str::SmolStr::from(s))
    }
}

impl From<&str> for SmolStr {
    #[inline]
    fn from(s: &str) -> Self {
        Self(smol_str::SmolStr::from(s))
    }
}

impl ProtoString for SmolStr {
    /// Validate the payload as UTF-8 and build a `SmolStr` directly from the
    /// borrowed `&str` — short strings inline with no heap allocation, avoiding
    /// the transient `String` a `From<String>` decode path would allocate.
    #[inline]
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(Self(smol_str::SmolStr::from(payload.to_str()?)))
    }
}

#[cfg(test)]
mod tests {
    use super::SmolStr;
    use buffa::{ProtoString, WirePayload};

    #[test]
    fn short_string_decodes_inline_without_heap() {
        // 5 bytes is well within smol_str's 23-byte inline capacity, so
        // `from_wire` must produce an inline value with no heap allocation.
        let s = SmolStr::from_wire(WirePayload::Borrowed(b"hello")).unwrap();
        assert_eq!(s.as_ref(), "hello");
        assert!(
            !s.0.is_heap_allocated(),
            "a short string must decode inline (zero heap allocation)"
        );
    }

    #[test]
    fn long_string_roundtrips() {
        let long = "x".repeat(64);
        let s = SmolStr::from_wire(WirePayload::Borrowed(long.as_bytes())).unwrap();
        assert_eq!(s.as_ref(), long.as_str());
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        assert!(SmolStr::from_wire(WirePayload::Borrowed(&[0xff, 0xfe])).is_err());
    }

    #[test]
    fn owned_payload_decodes() {
        let payload = WirePayload::Owned(::buffa::bytes::Bytes::from_static(b"hi"));
        let s = SmolStr::from_wire(payload).unwrap();
        assert_eq!(s.as_ref(), "hi");
        assert!(!s.0.is_heap_allocated());
    }
}
