//! Eager views whose wire allocation is wiped when the owner is dropped.
use alloc::vec::Vec;
use bytes::Bytes;

use crate::view::{ViewLifetimeParametric, ViewReborrow};
use crate::{DecodeError, DecodeOptions, EncodeSink, Message, MessageView, OwnedView};

use super::{ZeroizeOnDrop, Zeroizing};

/// An error constructing a [`GuardedOwnedView`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GuardedViewError {
    /// The input or encoded message exceeds a size limit.
    #[error("guarded view size limit exceeded")]
    SizeLimit,
    /// Reserving the wire allocation failed.
    #[error("could not reserve guarded view storage")]
    Allocation,
    /// The encoder wrote a different length than it reported.
    #[error("guarded view encoding length mismatch")]
    EncodingLength,
    /// The wire bytes could not be decoded as the selected message.
    #[error("guarded view decode failed: {0}")]
    Decode(#[from] DecodeError),
}

/// An eager view with a private wire allocation that is wiped on drop.
///
/// This type exposes neither a clonable handle nor the underlying `Bytes`.
/// The input passed to [`Self::decode`] remains the caller's responsibility.
/// For a message value, [`Self::from_owned`] encodes directly into the wiping
/// allocation. It does not stage a second wire buffer. The view may own
/// decoded values separately; those follow the view type's own cleanup
/// rules. The owner implements `ZeroizeOnDrop` only when the view does too.
///
/// ```compile_fail
/// use buffa::sensitive::GuardedOwnedView;
/// fn duplicate<V: buffa::view::ViewLifetimeParametric + Clone>(
///     view: &GuardedOwnedView<V>,
/// ) -> GuardedOwnedView<V> {
///     (*view).clone()
/// }
/// ```
pub struct GuardedOwnedView<V> {
    inner: OwnedView<V>,
}

struct WireOwner(Zeroizing<Vec<u8>>);
impl ZeroizeOnDrop for WireOwner {}

impl AsRef<[u8]> for WireOwner {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

// Restrict writes to the pre-reserved length. A malformed manual Message
// implementation must not make the buffer reallocate and release unwiped data,
// or panic when a write exceeds its declared size.
struct WireSink {
    bytes: Zeroizing<Vec<u8>>,
    expected_len: usize,
    overrun: bool,
}

impl ZeroizeOnDrop for WireSink {}

impl WireSink {
    fn new(expected_len: usize) -> Result<Self, GuardedViewError> {
        let mut bytes = Zeroizing::new(Vec::new());
        bytes
            .try_reserve_exact(expected_len)
            .map_err(|_| GuardedViewError::Allocation)?;
        Ok(Self {
            bytes,
            expected_len,
            overrun: false,
        })
    }

    fn finish(self) -> Result<WireOwner, GuardedViewError> {
        if self.overrun || self.bytes.len() != self.expected_len {
            return Err(GuardedViewError::EncodingLength);
        }
        Ok(WireOwner(self.bytes))
    }
}

impl EncodeSink for WireSink {
    fn put_u8(&mut self, byte: u8) {
        self.put_slice(core::slice::from_ref(&byte));
    }
    fn put_slice(&mut self, source: &[u8]) {
        if self.overrun {
            return;
        }
        if self
            .bytes
            .len()
            .checked_add(source.len())
            .is_some_and(|len| len <= self.expected_len)
        {
            self.bytes.extend_from_slice(source);
        } else {
            self.overrun = true;
        }
    }
    fn put_u32_le(&mut self, value: u32) {
        self.put_slice(&value.to_le_bytes());
    }
    fn put_u64_le(&mut self, value: u64) {
        self.put_slice(&value.to_le_bytes());
    }
}

impl<V: ViewLifetimeParametric> GuardedOwnedView<V> {
    /// Copy a borrowed wire buffer into private storage and decode eagerly.
    ///
    /// # Errors
    ///
    /// Rejects over-limit input before allocating and returns an error on
    /// wire buffer reservation failure or malformed wire data.
    pub fn decode(source: &[u8], options: &DecodeOptions) -> Result<Self, GuardedViewError> {
        Self::check_size(source.len(), options)?;
        let mut sink = WireSink::new(source.len())?;
        sink.put_slice(source);
        Self::admit(sink.finish()?, options)
    }

    /// Encode directly into private storage, then decode eagerly.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded length exceeds a limit, reserving the
    /// wire buffer fails, the encoder's actual length disagrees, or the view rejects the
    /// resulting bytes under the supplied options.
    pub fn from_owned(
        message: &V::Owned,
        options: &DecodeOptions,
    ) -> Result<Self, GuardedViewError> {
        let mut cache = crate::SizeCache::new();
        let size = usize::try_from(message.compute_size(&mut cache))
            .map_err(|_| GuardedViewError::SizeLimit)?;
        Self::check_size(size, options)?;
        let mut sink = WireSink::new(size)?;
        message.write_to(&mut cache, &mut sink);
        Self::admit(sink.finish()?, options)
    }

    fn check_size(size: usize, options: &DecodeOptions) -> Result<(), GuardedViewError> {
        if size > options.max_message_size() {
            Err(GuardedViewError::SizeLimit)
        } else {
            Ok(())
        }
    }

    fn admit(owner: WireOwner, options: &DecodeOptions) -> Result<Self, GuardedViewError> {
        let inner = OwnedView::decode_with_options(Bytes::from_owner(owner), options)?;
        Ok(Self { inner })
    }
}

impl<V: ViewReborrow> GuardedOwnedView<V> {
    /// Borrow the decoded view for as long as this owner is borrowed.
    #[must_use]
    pub fn view(&self) -> &V::Reborrowed<'_> {
        self.inner.reborrow()
    }

    /// Borrow the private encoded bytes without transferring ownership.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.inner.bytes()
    }

    /// Convert fields to their owned representations without sharing the wire
    /// allocation. The message's field types determine their own cleanup.
    ///
    /// # Errors
    ///
    /// Returns the view's conversion error if its owned representation
    /// cannot be built.
    pub fn to_owned_message(&self) -> Result<V::Owned, DecodeError> {
        self.view().to_owned_message()
    }
}

impl<V> core::fmt::Debug for GuardedOwnedView<V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("GuardedOwnedView([REDACTED])")
    }
}

impl<V: ZeroizeOnDrop> ZeroizeOnDrop for GuardedOwnedView<V> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_sink_rejects_length_mismatches() {
        let mut overrun = WireSink::new(2).unwrap();
        overrun.put_u8(1);
        overrun.put_slice(&[2, 3]);
        overrun.put_u8(4);
        assert_eq!(overrun.bytes.as_slice(), [1]);
        assert!(matches!(
            overrun.finish(),
            Err(GuardedViewError::EncodingLength)
        ));

        let mut underrun = WireSink::new(2).unwrap();
        underrun.put_u8(1);
        assert!(matches!(
            underrun.finish(),
            Err(GuardedViewError::EncodingLength)
        ));
    }
}
