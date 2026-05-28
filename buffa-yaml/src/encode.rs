use crate::Error;
use buffa::Message;
use std::io;

/// Serialize a protobuf message to a YAML string.
///
/// Encoding follows the protobuf JSON mapping: field names are camelCase,
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
    serde_norway::to_string(msg).map_err(Error::from)
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
    serde_norway::to_writer(w, msg).map_err(Error::from)
}
