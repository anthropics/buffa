/// An error from YAML serialization or deserialization.
///
/// Wraps the underlying `serde_norway` error (the carrier) and exposes the
/// [`Location`] (line and column) of the failure in the YAML input, so callers
/// can render diagnostics without depending on `serde_norway` directly.
///
/// The message is the carrier's message, and
/// [`source`](std::error::Error::source) is the carrier's source. The carrier
/// is never a link in the chain itself, so a reporter that walks the chain
/// prints each message once.
///
/// `source` is `None` for malformed YAML, for a value that does not fit its
/// field, and for an I/O error from the operating system. The carrier reports
/// a source when a reader or writer fails with an [`std::io::Error`] whose
/// payload has a source of its own, and that source is what is returned. The
/// `io::Error` is not in the chain, so its [`ErrorKind`](std::io::ErrorKind)
/// cannot be recovered from this type.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
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
