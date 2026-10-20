use crate::error::{CapturedIo, IoErrorCapture};
use crate::Error;
use buffa::{Message, MessageView};
use std::io;

/// Serialize a protobuf message to a YAML string.
///
/// Encoding follows the protobuf JSON mapping: field names are `camelCase`,
/// `int64`/`uint64` values are quoted strings, bytes are base64, enums are
/// string names, and well-known types use their canonical JSON encodings.
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails (e.g. the message contains a
/// value that cannot be represented in YAML).
pub fn to_string<M>(msg: &M) -> Result<String, Error>
where
    M: Message + serde::Serialize,
{
    serde_norway::to_string(msg).map_err(Error::from_carrier)
}

/// Serialize a protobuf message directly to a UTF-8 YAML byte vector.
///
/// This is the byte-vector counterpart to [`to_string`], using the same
/// protobuf JSON mapping without requiring the caller to create a writer.
///
/// # Errors
///
/// Returns an [`Error`] if the message cannot be serialized.
pub fn to_vec<M>(msg: &M) -> Result<Vec<u8>, Error>
where
    M: Message + serde::Serialize,
{
    let mut bytes = Vec::new();
    serde_norway::to_writer(&mut bytes, msg).map_err(Error::from_carrier)?;
    Ok(bytes)
}

/// Serialize a protobuf message to a YAML byte stream.
///
/// Follows the same encoding rules as [`to_string`].
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails or the writer returns an I/O
/// error. For a writer failure, [`Error::io_error_kind`] returns its
/// [`std::io::ErrorKind`].
pub fn to_writer<W, M>(w: W, msg: &M) -> Result<(), Error>
where
    W: io::Write,
    M: Message + serde::Serialize,
{
    let capture = IoErrorCapture::default();
    serde_norway::to_writer(CapturedIo::new(w, capture.clone()), msg)
        .map_err(|inner| Error::from_carrier_with_io_error_kind(inner, capture.error_kind()))
}

/// Serialize a zero-copy message view to a YAML string.
///
/// Produces the same YAML as [`to_string`] does for the corresponding owned
/// message: the generated view `Serialize` impls follow the protobuf JSON
/// mapping, so field names, `int64`/`uint64` quoting, base64 bytes, enum
/// names, and well-known-type encodings are identical. Views are encode-only —
/// YAML input cannot be borrowed from, so deserialization always targets the
/// owned message type via [`from_str`](crate::from_str) and friends.
///
/// To serialize an [`OwnedView`](buffa::OwnedView) handle, pass its reborrowed
/// view: `to_string_view(handle.reborrow())`.
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails (e.g. the view contains a
/// value that cannot be represented in YAML).
pub fn to_string_view<'a, V>(view: &V) -> Result<String, Error>
where
    V: MessageView<'a> + serde::Serialize,
{
    serde_norway::to_string(view).map_err(Error::from_carrier)
}

/// Serialize a zero-copy message view directly to a UTF-8 YAML byte vector.
///
/// Produces the same bytes as [`to_string_view`] does after UTF-8 encoding,
/// using the protobuf JSON mapping for generated view types.
///
/// # Errors
///
/// Returns an [`Error`] if the view cannot be serialized.
pub fn to_vec_view<'a, V>(view: &V) -> Result<Vec<u8>, Error>
where
    V: MessageView<'a> + serde::Serialize,
{
    let mut bytes = Vec::new();
    serde_norway::to_writer(&mut bytes, view).map_err(Error::from_carrier)?;
    Ok(bytes)
}

/// Serialize a zero-copy message view to a YAML byte stream.
///
/// Follows the same encoding rules as [`to_string_view`].
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails or the writer returns an I/O
/// error. For a writer failure, [`Error::io_error_kind`] returns its
/// [`std::io::ErrorKind`].
pub fn to_writer_view<'a, W, V>(w: W, view: &V) -> Result<(), Error>
where
    W: io::Write,
    V: MessageView<'a> + serde::Serialize,
{
    let capture = IoErrorCapture::default();
    serde_norway::to_writer(CapturedIo::new(w, capture.clone()), view)
        .map_err(|inner| Error::from_carrier_with_io_error_kind(inner, capture.error_kind()))
}
