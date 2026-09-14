//! A string representation with distinct borrowing and owning constructors.

use std::borrow::Cow;

/// Models a string library whose `From<&str>` preserves the input lifetime.
/// Native serde is needed for optional, repeated, map, and oneof JSON fields.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BorrowingStr<'a>(pub Cow<'a, str>);

impl<'a> From<&'a str> for BorrowingStr<'a> {
    fn from(value: &'a str) -> Self {
        Self(Cow::Borrowed(value))
    }
}

impl From<String> for BorrowingStr<'_> {
    fn from(value: String) -> Self {
        Self(Cow::Owned(value))
    }
}

impl std::ops::Deref for BorrowingStr<'_> {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for BorrowingStr<'_> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The generated message stores owned strings, not borrows from its input.
pub type OwnedStr = BorrowingStr<'static>;

impl buffa::ProtoString for OwnedStr {
    fn from_wire(payload: buffa::WirePayload<'_>) -> Result<Self, buffa::DecodeError> {
        payload.to_str().map(|value| Self::from(value.to_owned()))
    }
}

buffa::include_proto!("string_copy");
