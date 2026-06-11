use crate::Error;
use buffa::Message;
use std::io;

/// Deserialize a protobuf message from a YAML string.
///
/// Parsing follows the protobuf JSON mapping: `camelCase` and `snake_case` field
/// names are both accepted (via `#[serde(alias)]` on generated types), quoted
/// and unquoted integers are handled, bytes are base64-decoded, and well-known
/// types use their canonical JSON decodings.
///
/// # Errors
///
/// Returns an [`Error`] if the YAML is malformed or a field value cannot be
/// converted to its target type. Use [`Error::location`] to retrieve the line
/// and column of the failure.
pub fn from_str<M>(s: &str) -> Result<M, Error>
where
    M: Message + serde::de::DeserializeOwned,
{
    serde_norway::from_str(s).map_err(Error::from_carrier)
}

/// Deserialize a protobuf message from a UTF-8 YAML byte slice.
///
/// # Errors
///
/// Returns an [`Error`] if the bytes are not valid UTF-8 or the YAML cannot be
/// decoded into the target message type.
pub fn from_slice<M>(b: &[u8]) -> Result<M, Error>
where
    M: Message + serde::de::DeserializeOwned,
{
    serde_norway::from_slice(b).map_err(Error::from_carrier)
}

/// Deserialize a protobuf message from a YAML byte stream.
///
/// # Errors
///
/// Returns an [`Error`] if the stream cannot be read, the YAML is malformed,
/// or a field value cannot be converted to its target type.
pub fn from_reader<R, M>(r: R) -> Result<M, Error>
where
    R: io::Read,
    M: Message + serde::de::DeserializeOwned,
{
    serde_norway::from_reader(r).map_err(Error::from_carrier)
}
