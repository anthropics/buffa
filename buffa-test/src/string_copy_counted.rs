//! A string representation that counts its `copy_from_str` calls.

use std::cell::Cell;

thread_local! {
    static COPIES: Cell<usize> = const { Cell::new(0) };
}

/// The number of `CountedStr::copy_from_str` calls made on this thread.
///
/// The count is per thread, so tests running in parallel do not disturb each
/// other; compare two readings rather than assuming a starting value.
pub fn copies() -> usize {
    COPIES.with(Cell::get)
}

/// Only `copy_from_str` bumps [`copies`]; `From<&str>` and `From<String>` do
/// not, so a test can tell which constructor generated code called.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct CountedStr(String);

impl From<&str> for CountedStr {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for CountedStr {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for CountedStr {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for CountedStr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl buffa::ProtoString for CountedStr {
    fn copy_from_str(value: &str) -> Self {
        COPIES.with(|copies| copies.set(copies.get() + 1));
        Self(value.to_owned())
    }

    fn from_wire(payload: buffa::WirePayload<'_>) -> Result<Self, buffa::DecodeError> {
        payload.to_str().map(Self::from)
    }
}

buffa::include_proto!("string_copy_counted");
