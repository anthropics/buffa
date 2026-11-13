//! Test-local representations: policy belongs to the consuming proto crate.
//! Clone is required by the runtime; every copy remains a separate wipe owner.

use buffa::bytes::Buf;
use buffa::json_helpers::{ProtoBytesJson, ProtoStringJson};
use buffa::FieldPayload;
use buffa::{DecodeError, ProtoBytes, ProtoList, ProtoString, WirePayload};
use core::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
};
use secrecy::{ExposeSecret, SecretBox};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub struct SecretBytes(SecretBox<Vec<u8>>);

impl Clone for SecretBytes {
    fn clone(&self) -> Self {
        self.0.expose_secret().clone().into()
    }
}

impl Default for SecretBytes {
    fn default() -> Self {
        Vec::new().into()
    }
}
impl From<Vec<u8>> for SecretBytes {
    fn from(value: Vec<u8>) -> Self {
        // Box the Vec header without shrinking its buffer: converting to a
        // boxed slice could release spare capacity containing earlier secrets.
        Self(SecretBox::new(Box::new(value)))
    }
}
impl Deref for SecretBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0.expose_secret()
    }
}
impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}
impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes([REDACTED])")
    }
}
impl PartialEq for SecretBytes {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.as_ref().ct_eq(other.as_ref()))
    }
}
impl Zeroize for SecretBytes {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}
impl ZeroizeOnDrop for SecretBytes {}

/// Copy one bounded field payload into a single wipe-owned allocation.
fn read_guarded(payload: &mut FieldPayload<impl Buf>) -> Result<Zeroizing<Vec<u8>>, DecodeError> {
    let mut bytes = Zeroizing::new(Vec::new());
    // The length is already checked against the input, and the reservation
    // happens before any secret byte is written, so no reallocation can leave
    // a copy behind.
    bytes
        .try_reserve_exact(payload.remaining())
        .map_err(|_| DecodeError::MessageTooLarge)?;
    while payload.has_remaining() {
        let chunk = payload.chunk();
        bytes.extend_from_slice(chunk);
        let count = chunk.len();
        payload.advance(count);
    }
    Ok(bytes)
}

impl ProtoBytes for SecretBytes {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        Ok(payload.as_slice().to_vec().into())
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        let mut bytes = read_guarded(payload)?;
        Ok(core::mem::take(&mut *bytes).into())
    }
}

impl ProtoBytesJson for SecretBytes {
    fn serialize_json<S: serde::Serializer>(
        bytes: &[u8],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use base64::Engine as _;
        let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(bytes));
        serializer.serialize_str(&encoded)
    }

    fn deserialize_json<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl serde::de::Visitor<'_> for BytesVisitor {
            type Value = SecretBytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a base64 string")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                let mut bytes = Zeroizing::new(Vec::new());
                // Base64 output never exceeds the input length. Reserve once,
                // before any secret is written, so reallocation cannot expose it.
                bytes
                    .try_reserve_exact(value.len())
                    .map_err(|_| E::custom(GuardedJsonErrorReason::AllocationFailed))?;
                bytes.resize(value.len(), 0);
                let length = decode_base64_into(value, &mut bytes).map_err(E::custom)?;
                bytes.truncate(length);
                Ok(core::mem::take(&mut *bytes).into())
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                let value = Zeroizing::new(value);
                self.visit_str(&value)
            }
        }
        deserializer.deserialize_any(BytesVisitor)
    }
}

fn decode_base64_into(value: &str, output: &mut [u8]) -> Result<usize, GuardedJsonErrorReason> {
    use base64::alphabet;
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    use base64::Engine as _;
    const CONFIG: GeneralPurposeConfig = GeneralPurposeConfig::new()
        .with_decode_allow_trailing_bits(true)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let standard = GeneralPurpose::new(&alphabet::STANDARD, CONFIG);
    let url_safe = GeneralPurpose::new(&alphabet::URL_SAFE, CONFIG);
    standard
        .decode_slice(value, &mut *output)
        .or_else(|_| url_safe.decode_slice(value, output))
        .map_err(|_| GuardedJsonErrorReason::InvalidBase64)
}

#[derive(Debug, thiserror::Error)]
enum GuardedJsonErrorReason {
    #[error("invalid base64 in guarded field")]
    InvalidBase64,
    #[error("guarded JSON buffer allocation failed")]
    AllocationFailed,
}

pub struct SecretString(SecretBox<String>);
impl Clone for SecretString {
    fn clone(&self) -> Self {
        self.0.expose_secret().clone().into()
    }
}
impl Default for SecretString {
    fn default() -> Self {
        String::new().into()
    }
}
impl From<String> for SecretString {
    fn from(value: String) -> Self {
        // Preserve the String allocation and its spare capacity under the
        // wipe owner rather than shrinking it into a boxed str.
        Self(SecretBox::new(Box::new(value)))
    }
}
impl Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        self.0.expose_secret()
    }
}
impl AsRef<str> for SecretString {
    fn as_ref(&self) -> &str {
        self
    }
}
impl core::borrow::Borrow<str> for SecretString {
    fn borrow(&self) -> &str {
        self
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}
impl PartialEq for SecretString {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.as_bytes().ct_eq(other.as_bytes()))
    }
}
impl Eq for SecretString {}
impl PartialOrd for SecretString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for SecretString {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}
impl Hash for SecretString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}
impl Zeroize for SecretString {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}
impl ZeroizeOnDrop for SecretString {}
impl ProtoString for SecretString {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        payload.to_str().map(|value| value.to_owned().into())
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        let mut bytes = read_guarded(payload)?;
        if core::str::from_utf8(&bytes).is_err() {
            return Err(DecodeError::InvalidUtf8);
        }
        // Validated above, so the conversion keeps the guarded allocation. A
        // failure would hand the bytes back, and they are wiped before return.
        match String::from_utf8(core::mem::take(&mut *bytes)) {
            Ok(text) => Ok(text.into()),
            Err(error) => {
                let mut rejected = error.into_bytes();
                rejected.zeroize();
                Err(DecodeError::InvalidUtf8)
            }
        }
    }
}
impl ProtoStringJson for SecretString {
    fn serialize_json<S: serde::Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(text)
    }

    fn deserialize_json<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        struct TextVisitor;
        impl serde::de::Visitor<'_> for TextVisitor {
            type Value = SecretString;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(value.to_owned().into())
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(value.into())
            }
        }
        deserializer.deserialize_string(TextVisitor)
    }
}

/// Bytes that may be decoded from JSON but never exported to it, from the
/// owned message or from any view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SealedBytes(SecretBytes);

impl From<Vec<u8>> for SealedBytes {
    fn from(value: Vec<u8>) -> Self {
        Self(value.into())
    }
}
impl Zeroize for SealedBytes {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}
impl ZeroizeOnDrop for SealedBytes {}
impl Deref for SealedBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}
impl AsRef<[u8]> for SealedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl ProtoBytes for SealedBytes {
    fn from_wire(payload: WirePayload<'_>) -> Result<Self, DecodeError> {
        SecretBytes::from_wire(payload).map(Self)
    }
    fn decode_length_delimited(payload: &mut FieldPayload<impl Buf>) -> Result<Self, DecodeError> {
        SecretBytes::decode_length_delimited(payload).map(Self)
    }
}

/// Fixed policy failure for an export of [`SealedBytes`].
#[derive(Debug, thiserror::Error)]
#[error("sealed field cannot be exported to JSON")]
pub struct SealedExport;

impl ProtoBytesJson for SealedBytes {
    fn serialize_json<S: serde::Serializer>(_: &[u8], _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom(SealedExport))
    }

    fn deserialize_json<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        SecretBytes::deserialize_json(deserializer).map(Self)
    }
}

/// A `repeated` collection whose every allocation, including those it
/// outgrows, is wiped before release.
pub struct SecretList<T: Zeroize>(Vec<T>);

impl<T: Zeroize> SecretList<T> {
    /// Grow into a fresh allocation and wipe the one being replaced, instead of
    /// letting `Vec` reallocate and free the old buffer with its contents.
    fn grow(&mut self) {
        let capacity = self.0.capacity().saturating_mul(2).max(4);
        let mut next = Vec::with_capacity(capacity);
        next.append(&mut self.0);
        let mut previous = core::mem::replace(&mut self.0, next);
        previous.zeroize();
    }
}

impl<T: Zeroize> Drop for SecretList<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl<T: Zeroize> ZeroizeOnDrop for SecretList<T> {}
impl<T: Zeroize> Default for SecretList<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<T: Zeroize + Clone> Clone for SecretList<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: Zeroize + PartialEq> PartialEq for SecretList<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T: Zeroize> fmt::Debug for SecretList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretList([REDACTED])")
    }
}
impl<T: Zeroize> Deref for SecretList<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}
impl<T: Zeroize> From<Vec<T>> for SecretList<T> {
    fn from(value: Vec<T>) -> Self {
        Self(value)
    }
}
impl<T: Zeroize> FromIterator<T> for SecretList<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Self::default();
        for value in iter {
            list.push_guarded(value);
        }
        list
    }
}
impl<T: Zeroize> SecretList<T> {
    fn push_guarded(&mut self, value: T) {
        if self.0.len() == self.0.capacity() {
            self.grow();
        }
        self.0.push(value);
    }
}
impl<T> ProtoList<T> for SecretList<T>
where
    T: Zeroize + Clone + PartialEq + fmt::Debug + Send + Sync,
{
    fn push(&mut self, value: T) {
        self.push_guarded(value);
    }
    fn clear(&mut self) {
        // Wipes the elements and the spare capacity, then empties the list.
        self.0.zeroize();
    }
}
