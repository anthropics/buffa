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
/// Reader and writer failures also retain the reported
/// [`std::io::ErrorKind`], available through [`io_error_kind`](Self::io_error_kind).
/// A writer that returns zero bytes for a non-empty buffer reports
/// [`WriteZero`](std::io::ErrorKind::WriteZero).
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error {
    inner: InnerError,
}

#[derive(Debug)]
struct InnerError {
    carrier: serde_norway::Error,
    io_error_kind: Option<std::io::ErrorKind>,
}

impl std::fmt::Display for InnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.carrier, f)
    }
}

impl std::error::Error for InnerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.carrier.source()
    }
}

impl Error {
    /// Returns the line and column in the YAML input where the error occurred,
    /// if the carrier was able to determine one.
    #[must_use]
    pub fn location(&self) -> Option<Location> {
        self.inner.carrier.location().map(|loc| Location {
            line: loc.line(),
            column: loc.column(),
        })
    }

    /// Returns the I/O error kind reported while reading or writing, or `None`
    /// for parsing and conversion errors.
    #[must_use]
    pub fn io_error_kind(&self) -> Option<std::io::ErrorKind> {
        self.inner.io_error_kind
    }

    pub(crate) fn from_carrier(inner: serde_norway::Error) -> Self {
        Self {
            inner: InnerError {
                carrier: inner,
                io_error_kind: None,
            },
        }
    }

    pub(crate) fn from_carrier_with_io_error_kind(
        inner: serde_norway::Error,
        io_error_kind: Option<std::io::ErrorKind>,
    ) -> Self {
        Self {
            inner: InnerError {
                carrier: inner,
                io_error_kind,
            },
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct IoErrorCapture(std::rc::Rc<std::cell::Cell<Option<std::io::ErrorKind>>>);

impl IoErrorCapture {
    fn record(&self, error: std::io::Error) -> std::io::Error {
        if error.kind() != std::io::ErrorKind::Interrupted {
            self.record_kind(error.kind());
        }
        error
    }

    fn record_kind(&self, kind: std::io::ErrorKind) {
        self.0.set(Some(kind));
    }

    pub(crate) fn error_kind(&self) -> Option<std::io::ErrorKind> {
        self.0.get()
    }
}

pub(crate) struct CapturedIo<T> {
    inner: T,
    capture: IoErrorCapture,
}

impl<T> CapturedIo<T> {
    pub(crate) fn new(inner: T, capture: IoErrorCapture) -> Self {
        Self { inner, capture }
    }
}

impl<T: std::io::Read> std::io::Read for CapturedIo<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner
            .read(buf)
            .map_err(|error| self.capture.record(error))
    }
}

impl<T: std::io::Write> std::io::Write for CapturedIo<T> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.inner.write(buf) {
            Ok(0) if !buf.is_empty() => {
                self.capture.record_kind(std::io::ErrorKind::WriteZero);
                Ok(0)
            }
            Err(error) => Err(self.capture.record(error)),
            result => result,
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner
            .flush()
            .map_err(|error| self.capture.record(error))
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
