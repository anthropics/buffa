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

/// Serialize a protobuf message to a YAML byte stream.
///
/// Follows the same encoding rules as [`to_string`].
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails or the writer returns an I/O
/// error.
pub fn to_writer<W, M>(w: W, msg: &M) -> Result<(), Error>
where
    W: io::Write,
    M: Message + serde::Serialize,
{
    serde_norway::to_writer(w, msg).map_err(Error::from_carrier)
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

/// Serialize a zero-copy message view to a YAML byte stream.
///
/// Follows the same encoding rules as [`to_string_view`].
///
/// # Errors
///
/// Returns an [`Error`] if serialization fails or the writer returns an I/O
/// error.
pub fn to_writer_view<'a, W, V>(w: W, view: &V) -> Result<(), Error>
where
    W: io::Write,
    V: MessageView<'a> + serde::Serialize,
{
    serde_norway::to_writer(w, view).map_err(Error::from_carrier)
}
