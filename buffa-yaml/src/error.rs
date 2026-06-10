/// An error from YAML serialization or deserialization.
///
/// Wraps the underlying carrier error and exposes its source [`Location`] (line
/// and column) so callers can render diagnostics without depending on
/// `serde_norway` directly.
#[derive(Debug, thiserror::Error)]
#[error("{inner}")]
pub struct Error {
    inner: serde_norway::Error,
}

impl Error {
    /// Returns the line and column in the YAML input where the error occurred,
    /// if the carrier was able to determine one.
    #[must_use]
    pub fn location(&self) -> Option<Location> {
        self.inner.location().map(|loc| Location {
            line: loc.line(),
            column: loc.column(),
        })
    }

    pub(crate) fn from_carrier(inner: serde_norway::Error) -> Self {
        Self { inner }
    }
}

/// Source location within a YAML document (1-based line and column).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub column: usize,
}
