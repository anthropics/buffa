//! The core [`Message`] trait and [`DecodeOptions`] builder.
//!
//! Every generated message type implements [`Message`], which provides
//! encode/decode/merge methods and a two-pass serialization model
//! (`compute_size` → `write_to`) that avoids the exponential-time
//! problem affecting naïve length-delimited encoders.

use bytes::{Buf, BufMut};

use crate::error::DecodeError;
use crate::message_field::DefaultInstance;

/// Default recursion depth limit for decoding nested messages.
///
/// Protobuf implementations are required to enforce a recursion limit to
/// prevent stack overflow from deeply nested messages in untrusted input.
/// This value (100) matches the limit used by the official protobuf
/// implementations and the protobuf conformance suite.
///
/// Pass this constant as the `depth` argument when calling [`Message::merge`]
/// at a top-level decode site.  The provided convenience methods ([`Message::decode`],
/// [`Message::decode_from_slice`], [`Message::merge_from_slice`]) use this
/// limit automatically.
pub const RECURSION_LIMIT: u32 = 100;

/// The core trait implemented by all protobuf message types.
///
/// This trait is implemented by **generated code** — you write a `.proto` file,
/// codegen emits the Rust struct and its `Message` impl. You should almost
/// never implement this trait by hand.
///
/// # Manual implementation is discouraged
///
/// The only reason to implement `Message` yourself is when you need a
/// custom in-memory representation that codegen cannot produce — for
/// example, wrapping a `std::ops::Range<i64>` as a leaf message so the
/// rest of your code uses the natural Rust type. If you just want a message
/// type, **write a `.proto` file instead.**
///
/// Manual implementation is intentionally high-friction:
/// - You must correctly implement the two-pass serialization contract
///   (`compute_size` caches sizes before `write_to` uses them).
/// - You must implement wire-format decoding in `merge_field`.
/// - You must implement the [`DefaultInstance`] supertrait, which provides
///   the lazily-initialized static default that [`MessageField`](crate::MessageField)
///   dereferences to when unset.
///
/// If you still need to do this, see the [custom types section of the
/// user guide](https://github.com/anthropics/buffa/blob/main/docs/guide.md#custom-type-implementations)
/// for a complete worked example.
///
/// # Serialization model
///
/// Serialization is a two-pass process to avoid the exponential-time problem
/// that affects prost with deeply nested messages:
///
/// 1. **`compute_size()`** — walks the message tree and caches the encoded
///    size of every sub-message in its `CachedSize` field.
/// 2. **`write_to()`** — walks the tree again, writing bytes and using the
///    cached sizes for length-prefixed sub-messages.
///
/// The convenience method `encode()` performs both passes. If you need to
/// serialize the same message multiple times without mutation in between,
/// you can call `compute_size()` once and then `write_to()` repeatedly.
///
/// # Thread safety
///
/// `Message` requires `Send + Sync`. The `CachedSize` field uses `AtomicU32`
/// with `Relaxed` ordering, so messages can be placed in an `Arc` and shared
/// across threads. Serialization (`compute_size` → `write_to`) must still be
/// sequenced on a single thread per message — `merge` requires `&mut self`,
/// and `compute_size`/`write_to` must be called in order without interleaving
/// from another thread to produce a valid encoding.
pub trait Message: DefaultInstance + Clone + PartialEq + Send + Sync {
    /// Compute and cache the encoded byte size of this message.
    ///
    /// This recursively computes sizes for all sub-messages and stores them
    /// in each message's `CachedSize` field. Must be called before `write_to()`.
    ///
    /// # Size limit
    ///
    /// The protobuf specification limits messages to 2 GiB. The return type
    /// is `u32`, so messages whose encoded size exceeds `u32::MAX` (4 GiB)
    /// will produce a wrapped (undefined) size and a truncated encoding.
    /// Stay well within the 2 GiB spec limit.
    #[must_use = "compute_size has the side-effect of populating cached sizes; \
                  if you only need that, call encode() instead"]
    fn compute_size(&self) -> u32;

    /// Write this message's encoded bytes to a buffer.
    ///
    /// Assumes `compute_size()` has already been called. Uses cached sizes
    /// for length-delimited sub-message headers.
    fn write_to(&self, buf: &mut impl BufMut);

    /// Convenience: compute size, then write. This is the primary encoding API.
    fn encode(&self, buf: &mut impl BufMut) {
        let _ = self.compute_size();
        self.write_to(buf);
    }

    /// Encode this message as a length-delimited byte sequence.
    fn encode_length_delimited(&self, buf: &mut impl BufMut) {
        let len = self.compute_size();
        crate::encoding::encode_varint(len as u64, buf);
        self.write_to(buf);
    }

    /// Encode this message to a new `Vec<u8>`.
    #[must_use]
    fn encode_to_vec(&self) -> alloc::vec::Vec<u8> {
        let size = self.compute_size() as usize;
        let mut buf = alloc::vec::Vec::with_capacity(size);
        self.write_to(&mut buf);
        buf
    }

    /// Encode this message to a new [`bytes::Bytes`].
    ///
    /// Useful when handing off to networking code (hyper, tonic, axum)
    /// that expects `Bytes` frame or body payloads. Works in `no_std`.
    ///
    /// This is equivalent to `Bytes::from(self.encode_to_vec())` — both
    /// are zero-copy with respect to the encoded bytes — but saves readers
    /// from having to know that `From<Vec<u8>> for Bytes` is zero-copy.
    #[must_use]
    fn encode_to_bytes(&self) -> bytes::Bytes {
        let size = self.compute_size() as usize;
        let mut buf = bytes::BytesMut::with_capacity(size);
        self.write_to(&mut buf);
        buf.freeze()
    }

    /// Decode a message from a buffer.
    fn decode(buf: &mut impl Buf) -> Result<Self, DecodeError>
    where
        Self: Sized,
    {
        let mut msg = Self::default();
        msg.merge(buf, RECURSION_LIMIT)?;
        Ok(msg)
    }

    /// Decode a message from a byte slice.
    ///
    /// Convenience wrapper around [`decode`](Self::decode) that avoids the
    /// `&mut bytes.as_slice()` incantation.
    fn decode_from_slice(mut data: &[u8]) -> Result<Self, DecodeError>
    where
        Self: Sized,
    {
        // `mut data` creates a local mutable copy of the fat pointer so that
        // `Buf::advance` can move the read cursor without affecting the caller.
        Self::decode(&mut data)
    }

    /// Decode a length-delimited message from a buffer.
    ///
    /// This is a **top-level** entry point.  It reads a varint length prefix,
    /// then decodes using arithmetic bounds checking, calling
    /// [`merge_to_limit`](Self::merge_to_limit) with a fresh
    /// [`RECURSION_LIMIT`] budget.  Any sub-messages inside are decoded via
    /// [`merge_length_delimited`](Self::merge_length_delimited), which tracks
    /// and decrements the budget.
    ///
    /// Do **not** call this method from within a
    /// [`merge_to_limit`](Self::merge_to_limit) implementation to decode a
    /// nested sub-message field; use
    /// [`merge_length_delimited`](Self::merge_length_delimited) instead so
    /// that the caller's depth budget is propagated correctly.
    fn decode_length_delimited(buf: &mut impl Buf) -> Result<Self, DecodeError>
    where
        Self: Sized,
    {
        // Refuse messages larger than 2 GiB to prevent allocating attacker-
        // controlled amounts of memory from a crafted length prefix.
        const MAX_MESSAGE_BYTES: u64 = 0x7FFF_FFFF;
        let len_u64 = crate::encoding::decode_varint(buf)?;
        if len_u64 > MAX_MESSAGE_BYTES {
            return Err(DecodeError::MessageTooLarge);
        }
        // Safe on 32-bit: len_u64 <= 2 GiB - 1 < u32::MAX, so the cast never truncates.
        let len = usize::try_from(len_u64).map_err(|_| DecodeError::MessageTooLarge)?;
        if buf.remaining() < len {
            return Err(DecodeError::UnexpectedEof);
        }
        // Arithmetic limit: decode `len` bytes from the buffer without
        // wrapping it in `Take`.  This keeps the buffer type `B` unchanged
        // through every recursion level, avoiding E0275 for recursive
        // message types like `google.protobuf.Struct ↔ Value`.
        let limit = buf.remaining() - len;
        let mut msg = Self::default();
        msg.merge_to_limit(buf, RECURSION_LIMIT, limit)?;
        if buf.remaining() != limit {
            let remaining = buf.remaining();
            if remaining > limit {
                buf.advance(remaining - limit);
            } else {
                return Err(DecodeError::UnexpectedEof);
            }
        }
        Ok(msg)
    }

    /// Processes a single already-decoded tag and its associated field data
    /// from `buf`.
    ///
    /// This is the per-field dispatch method generated for each message type.
    /// Both [`merge_to_limit`](Self::merge_to_limit) and
    /// [`merge_group`](Self::merge_group) call this in their respective loops.
    ///
    /// `depth` is the remaining nesting budget.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if:
    /// - the buffer is truncated or malformed,
    /// - a wire-type mismatch is detected for a known field, or
    /// - the recursion limit is exceeded.
    fn merge_field(
        &mut self,
        tag: crate::encoding::Tag,
        buf: &mut impl Buf,
        depth: u32,
    ) -> Result<(), DecodeError>;

    /// Merge fields from a buffer until `buf.remaining()` reaches `limit`.
    ///
    /// This is the core decode loop.  [`merge`](Self::merge) delegates to this
    /// with `limit = 0` (read until exhausted).
    /// [`merge_length_delimited`](Self::merge_length_delimited) computes
    /// `limit` from the declared sub-message length and calls this directly.
    ///
    /// The caller must ensure `limit <= buf.remaining()`.  The default
    /// implementations of [`merge`](Self::merge) and
    /// [`merge_length_delimited`](Self::merge_length_delimited) uphold this
    /// invariant.
    ///
    /// `depth` is the remaining nesting budget.  Each call to
    /// [`merge_length_delimited`](Self::merge_length_delimited) decrements it
    /// by one before recursing; when it reaches zero the call returns
    /// [`DecodeError::RecursionLimitExceeded`].
    fn merge_to_limit(
        &mut self,
        buf: &mut impl Buf,
        depth: u32,
        limit: usize,
    ) -> Result<(), DecodeError> {
        while buf.remaining() > limit {
            let tag = crate::encoding::Tag::decode(buf)?;
            self.merge_field(tag, buf, depth)?;
        }
        Ok(())
    }

    /// Merges a group-encoded message from `buf`, reading fields until an
    /// EndGroup tag with the given `field_number` is encountered.
    ///
    /// Proto2 groups use StartGroup/EndGroup wire types instead of
    /// length-delimited encoding. The opening StartGroup tag has already been
    /// consumed by the caller; this method reads the group body and the
    /// closing EndGroup tag.
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] if:
    /// - the buffer is truncated before the EndGroup tag,
    /// - an EndGroup tag is encountered with a mismatched field number,
    /// - a wire-type mismatch is detected for a known field, or
    /// - the recursion limit is exceeded.
    fn merge_group(
        &mut self,
        buf: &mut impl Buf,
        depth: u32,
        field_number: u32,
    ) -> Result<(), DecodeError> {
        let depth = depth
            .checked_sub(1)
            .ok_or(DecodeError::RecursionLimitExceeded)?;
        loop {
            if !buf.has_remaining() {
                return Err(DecodeError::UnexpectedEof);
            }
            let tag = crate::encoding::Tag::decode(buf)?;
            if tag.wire_type() == crate::encoding::WireType::EndGroup {
                return if tag.field_number() == field_number {
                    Ok(())
                } else {
                    Err(DecodeError::InvalidEndGroup(tag.field_number()))
                };
            }
            self.merge_field(tag, buf, depth)?;
        }
    }

    /// Merge fields from a buffer into this message.
    ///
    /// Fields that are already set will be overwritten for singular fields,
    /// or appended for repeated fields, following standard protobuf merge
    /// semantics.
    ///
    /// `depth` is the remaining nesting budget.  Each call to
    /// [`merge_length_delimited`](Self::merge_length_delimited) decrements it
    /// by one before recursing; when it reaches zero the call returns
    /// [`DecodeError::RecursionLimitExceeded`].  Pass [`RECURSION_LIMIT`] at
    /// the outermost call site, or use the convenience methods
    /// ([`decode`](Self::decode), [`merge_from_slice`](Self::merge_from_slice))
    /// which do this automatically.
    fn merge(&mut self, buf: &mut impl Buf, depth: u32) -> Result<(), DecodeError> {
        self.merge_to_limit(buf, depth, 0)
    }

    /// Merge fields from a byte slice into this message.
    ///
    /// Convenience wrapper around [`merge`](Self::merge) that avoids the
    /// `&mut bytes.as_slice()` incantation.
    fn merge_from_slice(&mut self, mut data: &[u8]) -> Result<(), DecodeError> {
        self.merge(&mut data, RECURSION_LIMIT)
    }

    /// Merge fields from a length-delimited sub-message payload into this message.
    ///
    /// Reads a varint length prefix, then calls [`merge_to_limit`](Self::merge_to_limit)
    /// with an arithmetic bound derived from the declared sub-message length.
    /// The buffer type `B` passes through unchanged at every recursion level,
    /// avoiding the `E0275` trait-solver recursion limit that occurs with
    /// `Take<&mut Take<&mut T>>` type growth.
    ///
    /// Used by generated code when decoding singular `MessageField<T>` fields
    /// — the sub-message is merged into the existing value rather than
    /// replaced, per protobuf merge semantics.
    ///
    /// `depth` is the remaining nesting budget passed down from the enclosing
    /// [`merge_to_limit`](Self::merge_to_limit) call.  This method decrements
    /// it by one before calling the inner `merge_to_limit`; when it reaches
    /// zero it returns [`DecodeError::RecursionLimitExceeded`].
    ///
    /// Enforces the same 2 GiB safety limit as [`decode_length_delimited`](Self::decode_length_delimited).
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer is too short, if the declared length
    /// exceeds 2 GiB, if the recursion limit is reached, or if the inner
    /// `merge_to_limit` call fails.
    fn merge_length_delimited(
        &mut self,
        buf: &mut impl Buf,
        depth: u32,
    ) -> Result<(), DecodeError> {
        let depth = depth
            .checked_sub(1)
            .ok_or(DecodeError::RecursionLimitExceeded)?;
        const MAX_SUB_MESSAGE_BYTES: u64 = 0x7FFF_FFFF;
        let len_u64 = crate::encoding::decode_varint(buf)?;
        if len_u64 > MAX_SUB_MESSAGE_BYTES {
            return Err(DecodeError::MessageTooLarge);
        }
        let len = usize::try_from(len_u64).map_err(|_| DecodeError::MessageTooLarge)?;
        if buf.remaining() < len {
            return Err(DecodeError::UnexpectedEof);
        }
        // Arithmetic limit: the sub-message occupies `len` bytes, so the
        // decode loop should stop when `buf.remaining()` drops to
        // `remaining - len`.  This avoids wrapping the buffer in `Take`,
        // which would grow the type at each recursion level and trigger
        // E0275 for recursive message types like `Struct ↔ Value`.
        let limit = buf.remaining() - len;
        self.merge_to_limit(buf, depth, limit)?;
        if buf.remaining() != limit {
            let remaining = buf.remaining();
            if remaining > limit {
                // Sub-message consumed fewer bytes than declared; skip the rest.
                buf.advance(remaining - limit);
            } else {
                return Err(DecodeError::UnexpectedEof);
            }
        }
        Ok(())
    }

    /// The cached encoded size from the last `compute_size()` call.
    ///
    /// Returns 0 if `compute_size()` has never been called.
    #[must_use]
    fn cached_size(&self) -> u32;

    /// Clear all fields to their default values.
    fn clear(&mut self);
}

/// Options for configuring message decoding behavior.
///
/// Use this to set custom recursion depth limits or maximum message sizes
/// when decoding from untrusted input.
///
/// # Examples
///
/// ```no_run
/// # use buffa::__doctest_fixtures::Person;
/// use buffa::DecodeOptions;
///
/// # fn example(bytes: &[u8]) -> Result<(), buffa::DecodeError> {
/// // Restrict recursion depth to 50 and message size to 1 MiB:
/// let msg: Person = DecodeOptions::new()
///     .with_recursion_limit(50)
///     .with_max_message_size(1024 * 1024)
///     .decode_from_slice(bytes)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct DecodeOptions {
    recursion_limit: u32,
    max_message_size: usize,
}

/// Default maximum message size: 2 GiB - 1 (matches the internal sub-message
/// limit in `merge_length_delimited`).
const DEFAULT_MAX_MESSAGE_SIZE: usize = 0x7FFF_FFFF;

impl Default for DecodeOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeOptions {
    /// Create new decode options with defaults.
    ///
    /// Defaults:
    /// - `recursion_limit`: 100 (same as [`RECURSION_LIMIT`])
    /// - `max_message_size`: 2 GiB - 1
    pub fn new() -> Self {
        Self {
            recursion_limit: RECURSION_LIMIT,
            max_message_size: DEFAULT_MAX_MESSAGE_SIZE,
        }
    }

    /// Set the maximum recursion depth for nested messages.
    ///
    /// Each nested sub-message consumes one level of depth budget. When
    /// the budget reaches zero, decoding returns
    /// [`DecodeError::RecursionLimitExceeded`].
    ///
    /// Default: 100.
    #[must_use]
    pub fn with_recursion_limit(mut self, limit: u32) -> Self {
        self.recursion_limit = limit;
        self
    }

    /// Set the maximum total message size in bytes.
    ///
    /// If the input buffer or length-delimited payload exceeds this size,
    /// decoding returns [`DecodeError::MessageTooLarge`].
    ///
    /// This is checked at the top-level decode entry point. Individual
    /// sub-messages are still bounded by the internal 2 GiB limit
    /// regardless of this setting.
    ///
    /// Default: 2 GiB - 1 (0x7FFF_FFFF).
    #[must_use]
    pub fn with_max_message_size(mut self, max_bytes: usize) -> Self {
        self.max_message_size = max_bytes;
        self
    }

    /// Returns the configured recursion depth limit.
    pub fn recursion_limit(&self) -> u32 {
        self.recursion_limit
    }

    /// Returns the configured maximum message size in bytes.
    pub fn max_message_size(&self) -> usize {
        self.max_message_size
    }

    /// Decode a message from a buffer.
    pub fn decode<M: Message>(&self, buf: &mut impl Buf) -> Result<M, DecodeError> {
        if buf.remaining() > self.max_message_size {
            return Err(DecodeError::MessageTooLarge);
        }
        let mut msg = M::default();
        msg.merge(buf, self.recursion_limit)?;
        Ok(msg)
    }

    /// Decode a message from a byte slice.
    pub fn decode_from_slice<M: Message>(&self, data: &[u8]) -> Result<M, DecodeError> {
        if data.len() > self.max_message_size {
            return Err(DecodeError::MessageTooLarge);
        }
        let mut msg = M::default();
        msg.merge(&mut &*data, self.recursion_limit)?;
        Ok(msg)
    }

    /// Decode a length-delimited message from a buffer.
    pub fn decode_length_delimited<M: Message>(
        &self,
        buf: &mut impl Buf,
    ) -> Result<M, DecodeError> {
        // Enforce the 2 GiB internal safety cap even if the user sets a
        // larger max_message_size, to prevent allocating attacker-controlled
        // amounts of memory from a crafted length prefix.
        let max = core::cmp::min(
            self.max_message_size as u64,
            DEFAULT_MAX_MESSAGE_SIZE as u64,
        );
        let len_u64 = crate::encoding::decode_varint(buf)?;
        if len_u64 > max {
            return Err(DecodeError::MessageTooLarge);
        }
        let len = usize::try_from(len_u64).map_err(|_| DecodeError::MessageTooLarge)?;
        if buf.remaining() < len {
            return Err(DecodeError::UnexpectedEof);
        }
        let limit = buf.remaining() - len;
        let mut msg = M::default();
        msg.merge_to_limit(buf, self.recursion_limit, limit)?;
        if buf.remaining() != limit {
            let remaining = buf.remaining();
            if remaining > limit {
                buf.advance(remaining - limit);
            } else {
                return Err(DecodeError::UnexpectedEof);
            }
        }
        Ok(msg)
    }

    /// Merge fields from a buffer into an existing message.
    pub fn merge<M: Message>(&self, msg: &mut M, buf: &mut impl Buf) -> Result<(), DecodeError> {
        if buf.remaining() > self.max_message_size {
            return Err(DecodeError::MessageTooLarge);
        }
        msg.merge(buf, self.recursion_limit)
    }

    /// Merge fields from a byte slice into an existing message.
    pub fn merge_from_slice<M: Message>(
        &self,
        msg: &mut M,
        data: &[u8],
    ) -> Result<(), DecodeError> {
        if data.len() > self.max_message_size {
            return Err(DecodeError::MessageTooLarge);
        }
        msg.merge(&mut &*data, self.recursion_limit)
    }

    /// Decode a zero-copy view from a byte slice.
    pub fn decode_view<'a, V: crate::view::MessageView<'a>>(
        &self,
        buf: &'a [u8],
    ) -> Result<V, DecodeError> {
        if buf.len() > self.max_message_size {
            return Err(DecodeError::MessageTooLarge);
        }
        V::decode_view_with_limit(buf, self.recursion_limit)
    }

    /// Decode a message by reading all bytes from a [`std::io::Read`] source.
    ///
    /// Reads until EOF, enforces `max_message_size`, then decodes the
    /// buffered bytes. Returns `std::io::Error` to be compatible with
    /// `Read`-based error handling.
    #[cfg(feature = "std")]
    pub fn decode_reader<M: Message>(
        &self,
        reader: &mut impl std::io::Read,
    ) -> Result<M, std::io::Error> {
        let bytes = self.read_limited(reader)?;
        self.decode_from_slice::<M>(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Decode a length-delimited message from a [`std::io::Read`] source.
    ///
    /// Reads a varint length prefix, enforces `max_message_size`, reads
    /// exactly that many bytes, then decodes. Useful for reading sequential
    /// length-delimited messages from a file or stream.
    #[cfg(feature = "std")]
    pub fn decode_length_delimited_reader<M: Message>(
        &self,
        reader: &mut impl std::io::Read,
    ) -> Result<M, std::io::Error> {
        let len = read_varint(reader)?;
        let max = core::cmp::min(
            self.max_message_size as u64,
            DEFAULT_MAX_MESSAGE_SIZE as u64,
        );
        if len > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                DecodeError::MessageTooLarge,
            ));
        }
        let len = len as usize;
        let mut buf = alloc::vec![0u8; len];
        reader.read_exact(&mut buf)?;
        self.decode_from_slice::<M>(&buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Read all bytes from a reader up to `max_message_size`.
    #[cfg(feature = "std")]
    fn read_limited(
        &self,
        reader: &mut impl std::io::Read,
    ) -> Result<alloc::vec::Vec<u8>, std::io::Error> {
        use std::io::Read as _;
        let mut buf = alloc::vec::Vec::new();
        reader
            .take(self.max_message_size as u64 + 1)
            .read_to_end(&mut buf)?;
        if buf.len() > self.max_message_size {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                DecodeError::MessageTooLarge,
            ));
        }
        Ok(buf)
    }
}

/// Read a varint from a `std::io::Read` source, one byte at a time.
///
/// Mirrors the validation in [`decode_varint_slow`](crate::encoding): a
/// 10th byte > `0x01` (overflow bits set, or continuation bit implying an
/// 11th byte) is rejected.
#[cfg(feature = "std")]
fn read_varint(reader: &mut impl std::io::Read) -> Result<u64, std::io::Error> {
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    loop {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        let b = byte[0];
        if shift < 63 {
            value |= ((b & 0x7F) as u64) << shift;
            if b < 0x80 {
                return Ok(value);
            }
            shift += 7;
        } else {
            // 10th byte: only bit 0 maps to bit 63 of the result. A byte
            // > 0x01 means either data overflow (bits 1-6 set) or an 11th
            // byte (continuation bit 0x80 set).
            if b > 0x01 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    DecodeError::VarintTooLong,
                ));
            }
            value |= (b as u64) << 63;
            return Ok(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cached_size::CachedSize;
    use crate::encoding::encode_varint;
    use crate::error::DecodeError;
    use crate::message_field::DefaultInstance;

    // Minimal hand-written Message for testing merge_length_delimited.
    // Includes `CachedSize` to mirror the shape of generated message structs.
    #[derive(Clone, Debug, Default, PartialEq)]
    struct FlatMsg {
        value: i32,
        __buffa_cached_size: CachedSize,
    }

    impl DefaultInstance for FlatMsg {
        fn default_instance() -> &'static Self {
            static INST: crate::__private::OnceBox<FlatMsg> = crate::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(FlatMsg::default()))
        }
    }

    impl Message for FlatMsg {
        fn compute_size(&self) -> u32 {
            let size = if self.value != 0 {
                1 + crate::types::int32_encoded_len(self.value) as u32
            } else {
                0
            };
            self.__buffa_cached_size.set(size);
            size
        }

        fn write_to(&self, buf: &mut impl BufMut) {
            if self.value != 0 {
                crate::encoding::Tag::new(1, crate::encoding::WireType::Varint).encode(buf);
                crate::types::encode_int32(self.value, buf);
            }
        }

        fn merge_field(
            &mut self,
            tag: crate::encoding::Tag,
            buf: &mut impl Buf,
            _depth: u32,
        ) -> Result<(), DecodeError> {
            match tag.field_number() {
                1 => {
                    self.value = crate::types::decode_int32(buf)?;
                }
                _ => {
                    crate::encoding::skip_field(tag, buf)?;
                }
            }
            Ok(())
        }

        fn cached_size(&self) -> u32 {
            self.__buffa_cached_size.get()
        }

        fn clear(&mut self) {
            *self = Self::default();
        }
    }

    fn wire_bytes(msg: &FlatMsg) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec::Vec::new();
        msg.encode_length_delimited(&mut buf);
        buf
    }

    #[test]
    fn test_merge_length_delimited_basic() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let mut dst = FlatMsg::default();
        dst.merge_length_delimited(&mut wire_bytes(&src).as_slice(), RECURSION_LIMIT)
            .unwrap();
        assert_eq!(dst.value, 42);
    }

    #[test]
    fn test_merge_length_delimited_merges_into_existing() {
        // Second merge overwrites (proto3 last-wins for scalar fields).
        let mut dst = FlatMsg::default();
        dst.merge_length_delimited(
            &mut wire_bytes(&FlatMsg {
                value: 1,
                __buffa_cached_size: CachedSize::default(),
            })
            .as_slice(),
            RECURSION_LIMIT,
        )
        .unwrap();
        assert_eq!(dst.value, 1);
        dst.merge_length_delimited(
            &mut wire_bytes(&FlatMsg {
                value: 2,
                __buffa_cached_size: CachedSize::default(),
            })
            .as_slice(),
            RECURSION_LIMIT,
        )
        .unwrap();
        assert_eq!(dst.value, 2);
    }

    #[test]
    fn test_merge_length_delimited_truncated() {
        // Length prefix says 10 bytes but buffer contains only 2.
        let mut buf = alloc::vec::Vec::new();
        encode_varint(10, &mut buf);
        buf.extend_from_slice(&[0x01, 0x01]);
        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_length_delimited(&mut buf.as_slice(), RECURSION_LIMIT),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn test_merge_length_delimited_oversized() {
        // Length prefix exceeds the 2 GiB safety limit.
        let mut buf = alloc::vec::Vec::new();
        encode_varint(0x8000_0000u64, &mut buf); // 2 GiB + 1
        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_length_delimited(&mut buf.as_slice(), RECURSION_LIMIT),
            Err(DecodeError::MessageTooLarge)
        );
    }

    #[test]
    fn test_merge_length_delimited_recursion_limit() {
        // depth=1 means merge_length_delimited will decrement to 0, then any
        // nested call would return RecursionLimitExceeded.  Passing depth=1
        // to merge_length_delimited itself is the boundary: it decrements to
        // 0 and calls merge with depth=0, which for FlatMsg (a leaf) succeeds.
        // Passing depth=0 directly must return RecursionLimitExceeded.
        let src = FlatMsg {
            value: 7,
            __buffa_cached_size: CachedSize::default(),
        };
        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_length_delimited(&mut wire_bytes(&src).as_slice(), 0),
            Err(DecodeError::RecursionLimitExceeded)
        );
        // depth=1 succeeds: exactly one level is consumed.
        dst.merge_length_delimited(&mut wire_bytes(&src).as_slice(), 1)
            .unwrap();
        assert_eq!(dst.value, 7);
    }

    #[test]
    fn test_decode_from_slice_basic() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let dst = FlatMsg::decode_from_slice(&bytes).unwrap();
        assert_eq!(dst.value, 42);
    }

    #[test]
    fn test_encode_to_bytes_matches_encode_to_vec() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let vec = src.encode_to_vec();
        let bytes = src.encode_to_bytes();
        assert_eq!(vec.as_slice(), bytes.as_ref());
        // Round-trip through the Bytes variant.
        let dst = FlatMsg::decode_from_slice(&bytes).unwrap();
        assert_eq!(dst.value, 42);
        // Empty-message case: zero-length buffer is well-defined.
        assert!(FlatMsg::default().encode_to_bytes().is_empty());
    }

    #[test]
    fn test_decode_from_slice_empty() {
        let dst = FlatMsg::decode_from_slice(&[]).unwrap();
        assert_eq!(dst.value, 0);
    }

    #[test]
    fn test_decode_from_slice_invalid_returns_error() {
        // A lone 0xFF byte is not a valid varint tag.
        let result = FlatMsg::decode_from_slice(&[0xFF]);
        assert!(result.is_err());
    }

    #[test]
    fn test_merge_from_slice_basic() {
        let src = FlatMsg {
            value: 7,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let mut dst = FlatMsg::default();
        dst.merge_from_slice(&bytes).unwrap();
        assert_eq!(dst.value, 7);
    }

    #[test]
    fn test_merge_from_slice_last_wins() {
        let src1 = FlatMsg {
            value: 1,
            __buffa_cached_size: CachedSize::default(),
        };
        let src2 = FlatMsg {
            value: 2,
            __buffa_cached_size: CachedSize::default(),
        };
        let mut dst = FlatMsg::default();
        dst.merge_from_slice(&src1.encode_to_vec()).unwrap();
        dst.merge_from_slice(&src2.encode_to_vec()).unwrap();
        // Proto3 last-wins semantics for scalar fields.
        assert_eq!(dst.value, 2);
    }

    // ── DecodeOptions tests ──────────────────────────────────────────

    #[test]
    fn test_decode_options_default_works() {
        let src = FlatMsg {
            value: 99,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let msg: FlatMsg = DecodeOptions::new().decode_from_slice(&bytes).unwrap();
        assert_eq!(msg.value, 99);
    }

    #[test]
    fn test_decode_options_max_message_size_rejects() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        // Set max size to 1 byte — smaller than the encoded message.
        let result: Result<FlatMsg, _> = DecodeOptions::new()
            .with_max_message_size(1)
            .decode_from_slice(&bytes);
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn test_decode_options_max_message_size_exact_boundary() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        // Exact size should succeed.
        let msg: FlatMsg = DecodeOptions::new()
            .with_max_message_size(bytes.len())
            .decode_from_slice(&bytes)
            .unwrap();
        assert_eq!(msg.value, 42);
        // One byte less should fail.
        let result: Result<FlatMsg, _> = DecodeOptions::new()
            .with_max_message_size(bytes.len() - 1)
            .decode_from_slice(&bytes);
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn test_decode_options_custom_recursion_limit() {
        // FlatMsg has no nested messages, so any recursion limit >= 0 works.
        // Just verify the API compiles and runs.
        let src = FlatMsg {
            value: 7,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let msg: FlatMsg = DecodeOptions::new()
            .with_recursion_limit(1)
            .decode_from_slice(&bytes)
            .unwrap();
        assert_eq!(msg.value, 7);
    }

    #[test]
    fn test_decode_options_merge() {
        let src = FlatMsg {
            value: 55,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let mut msg = FlatMsg::default();
        DecodeOptions::new()
            .merge_from_slice(&mut msg, &bytes)
            .unwrap();
        assert_eq!(msg.value, 55);
    }

    #[test]
    fn test_decode_options_merge_rejects_oversize() {
        let src = FlatMsg {
            value: 55,
            __buffa_cached_size: CachedSize::default(),
        };
        let bytes = src.encode_to_vec();
        let mut msg = FlatMsg::default();
        let result = DecodeOptions::new()
            .with_max_message_size(1)
            .merge_from_slice(&mut msg, &bytes);
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn test_decode_options_length_delimited() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let mut ld_bytes = alloc::vec::Vec::new();
        src.encode_length_delimited(&mut ld_bytes);
        let msg: FlatMsg = DecodeOptions::new()
            .decode_length_delimited(&mut ld_bytes.as_slice())
            .unwrap();
        assert_eq!(msg.value, 42);
    }

    #[test]
    fn test_decode_options_length_delimited_rejects_oversize() {
        let src = FlatMsg {
            value: 42,
            __buffa_cached_size: CachedSize::default(),
        };
        let mut ld_bytes = alloc::vec::Vec::new();
        src.encode_length_delimited(&mut ld_bytes);
        let result: Result<FlatMsg, _> = DecodeOptions::new()
            .with_max_message_size(1)
            .decode_length_delimited(&mut ld_bytes.as_slice());
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn decode_options_getters_return_defaults() {
        let opts = DecodeOptions::new();
        assert_eq!(opts.recursion_limit(), RECURSION_LIMIT);
        assert_eq!(opts.max_message_size(), 0x7FFF_FFFF);
    }

    #[test]
    fn decode_options_getters_return_custom_values() {
        let opts = DecodeOptions::new()
            .with_recursion_limit(42)
            .with_max_message_size(1024);
        assert_eq!(opts.recursion_limit(), 42);
        assert_eq!(opts.max_message_size(), 1024);
    }

    #[test]
    fn test_decode_options_default_impl() {
        // DecodeOptions::default() ≡ DecodeOptions::new().
        let opts = DecodeOptions::default();
        assert_eq!(opts.recursion_limit(), RECURSION_LIMIT);
        assert_eq!(opts.max_message_size(), 0x7FFF_FFFF);
    }

    #[test]
    fn test_decode_options_decode_buf() {
        // The Buf-taking decode() variant (vs decode_from_slice).
        let src = FlatMsg {
            value: 123,
            ..Default::default()
        };
        let bytes = src.encode_to_vec();
        let msg: FlatMsg = DecodeOptions::new().decode(&mut bytes.as_slice()).unwrap();
        assert_eq!(msg.value, 123);
        // Oversize check on the Buf variant.
        let result: Result<FlatMsg, _> = DecodeOptions::new()
            .with_max_message_size(1)
            .decode(&mut bytes.as_slice());
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn test_decode_options_merge_buf() {
        // The Buf-taking merge() variant (vs merge_from_slice).
        let src = FlatMsg {
            value: 77,
            ..Default::default()
        };
        let bytes = src.encode_to_vec();
        let mut msg = FlatMsg::default();
        DecodeOptions::new()
            .merge(&mut msg, &mut bytes.as_slice())
            .unwrap();
        assert_eq!(msg.value, 77);
        // Oversize check.
        let mut msg = FlatMsg::default();
        let result = DecodeOptions::new()
            .with_max_message_size(1)
            .merge(&mut msg, &mut bytes.as_slice());
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    // ── Message trait default methods ─────────────────────────────────

    #[test]
    fn test_message_encode_trait_default() {
        // Message::encode(buf) ≡ compute_size() then write_to(buf).
        let src = FlatMsg {
            value: 42,
            ..Default::default()
        };
        let mut buf = alloc::vec::Vec::new();
        src.encode(&mut buf);
        assert_eq!(buf, src.encode_to_vec());
    }

    #[test]
    fn test_message_decode_length_delimited_trait_default() {
        // The trait-level decode_length_delimited (distinct from
        // DecodeOptions::decode_length_delimited).
        let src = FlatMsg {
            value: 42,
            ..Default::default()
        };
        let mut ld = alloc::vec::Vec::new();
        src.encode_length_delimited(&mut ld);
        let got = FlatMsg::decode_length_delimited(&mut ld.as_slice()).unwrap();
        assert_eq!(got.value, 42);
    }

    #[test]
    fn test_message_decode_length_delimited_oversize() {
        // Length prefix > 2 GiB → MessageTooLarge.
        let mut buf = alloc::vec::Vec::new();
        encode_varint(0x8000_0000u64, &mut buf);
        let result = FlatMsg::decode_length_delimited(&mut buf.as_slice());
        assert_eq!(result, Err(DecodeError::MessageTooLarge));
    }

    #[test]
    fn test_message_decode_length_delimited_truncated() {
        // Length prefix says 10 bytes, buffer has 2.
        let mut buf = alloc::vec::Vec::new();
        encode_varint(10, &mut buf);
        buf.push(0x08);
        buf.push(0x01);
        let result = FlatMsg::decode_length_delimited(&mut buf.as_slice());
        assert_eq!(result, Err(DecodeError::UnexpectedEof));
    }

    #[test]
    fn test_message_decode_length_delimited_with_trailing() {
        // Buffer has two back-to-back length-delimited messages.
        // decode_length_delimited should consume exactly the first one
        // and leave the buffer positioned at the second.
        let a = FlatMsg {
            value: 1,
            ..Default::default()
        };
        let b = FlatMsg {
            value: 2,
            ..Default::default()
        };
        let mut buf = alloc::vec::Vec::new();
        a.encode_length_delimited(&mut buf);
        b.encode_length_delimited(&mut buf);

        let mut cur = buf.as_slice();
        let first = FlatMsg::decode_length_delimited(&mut cur).unwrap();
        assert_eq!(first.value, 1);
        let second = FlatMsg::decode_length_delimited(&mut cur).unwrap();
        assert_eq!(second.value, 2);
        assert!(cur.is_empty());
    }

    // ── merge_group tests ─────────────────────────────────────────────

    /// Build a group body for FlatMsg (field 1 = value) terminated by
    /// EndGroup with the given field number.
    fn group_bytes(value: i32, group_field_number: u32) -> alloc::vec::Vec<u8> {
        use crate::encoding::{Tag, WireType};
        let mut buf = alloc::vec::Vec::new();
        if value != 0 {
            Tag::new(1, WireType::Varint).encode(&mut buf);
            crate::types::encode_int32(value, &mut buf);
        }
        Tag::new(group_field_number, WireType::EndGroup).encode(&mut buf);
        buf
    }

    #[test]
    fn test_merge_group_basic() {
        let data = group_bytes(42, 5);
        let mut dst = FlatMsg::default();
        dst.merge_group(&mut data.as_slice(), RECURSION_LIMIT, 5)
            .unwrap();
        assert_eq!(dst.value, 42);
    }

    #[test]
    fn test_merge_group_empty() {
        // Group with no fields — just EndGroup.
        let data = group_bytes(0, 3);
        let mut dst = FlatMsg::default();
        dst.merge_group(&mut data.as_slice(), RECURSION_LIMIT, 3)
            .unwrap();
        assert_eq!(dst.value, 0);
    }

    #[test]
    fn test_merge_group_merges_into_existing() {
        let data1 = group_bytes(1, 5);
        let data2 = group_bytes(2, 5);
        let mut dst = FlatMsg::default();
        dst.merge_group(&mut data1.as_slice(), RECURSION_LIMIT, 5)
            .unwrap();
        assert_eq!(dst.value, 1);
        dst.merge_group(&mut data2.as_slice(), RECURSION_LIMIT, 5)
            .unwrap();
        assert_eq!(dst.value, 2);
    }

    #[test]
    fn test_merge_group_recursion_limit_zero() {
        // depth=0 should immediately fail with RecursionLimitExceeded
        // because merge_group decrements before entering the loop.
        let data = group_bytes(42, 5);
        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_group(&mut data.as_slice(), 0, 5),
            Err(DecodeError::RecursionLimitExceeded)
        );
    }

    #[test]
    fn test_merge_group_recursion_limit_one_succeeds() {
        // depth=1 succeeds: merge_group decrements to 0, but FlatMsg's
        // merge_field doesn't recurse further.
        let data = group_bytes(7, 5);
        let mut dst = FlatMsg::default();
        dst.merge_group(&mut data.as_slice(), 1, 5).unwrap();
        assert_eq!(dst.value, 7);
    }

    #[test]
    fn test_merge_group_mismatched_end() {
        // EndGroup with wrong field number.
        use crate::encoding::{Tag, WireType};
        let mut data = alloc::vec::Vec::new();
        Tag::new(99, WireType::EndGroup).encode(&mut data);

        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_group(&mut data.as_slice(), RECURSION_LIMIT, 5),
            Err(DecodeError::InvalidEndGroup(99))
        );
    }

    #[test]
    fn test_merge_group_truncated() {
        // Buffer ends without EndGroup tag.
        use crate::encoding::{Tag, WireType};
        let mut data = alloc::vec::Vec::new();
        Tag::new(1, WireType::Varint).encode(&mut data);
        crate::types::encode_int32(42, &mut data);
        // No EndGroup.

        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_group(&mut data.as_slice(), RECURSION_LIMIT, 5),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn test_merge_group_empty_buffer() {
        let mut dst = FlatMsg::default();
        assert_eq!(
            dst.merge_group(&mut [].as_slice(), RECURSION_LIMIT, 5),
            Err(DecodeError::UnexpectedEof)
        );
    }

    #[test]
    fn test_merge_group_unknown_fields_skipped() {
        // Group body contains an unknown field (field 99) which FlatMsg
        // routes to skip_field; the known field (field 1) should still
        // be decoded.
        use crate::encoding::{Tag, WireType};
        let mut data = alloc::vec::Vec::new();
        // Unknown varint field 99 = 0
        Tag::new(99, WireType::Varint).encode(&mut data);
        crate::encoding::encode_varint(0, &mut data);
        // Known field 1 = 99
        Tag::new(1, WireType::Varint).encode(&mut data);
        crate::types::encode_int32(99, &mut data);
        // EndGroup(5)
        Tag::new(5, WireType::EndGroup).encode(&mut data);

        let mut dst = FlatMsg::default();
        dst.merge_group(&mut data.as_slice(), RECURSION_LIMIT, 5)
            .unwrap();
        assert_eq!(dst.value, 99);
    }

    #[test]
    fn test_merge_group_trailing_data_preserved() {
        // After the EndGroup tag, trailing data should remain in the buffer.
        let mut data = group_bytes(42, 5);
        data.extend_from_slice(&[0xDE, 0xAD]);

        let mut cur = data.as_slice();
        let mut dst = FlatMsg::default();
        dst.merge_group(&mut cur, RECURSION_LIMIT, 5).unwrap();
        assert_eq!(dst.value, 42);
        assert_eq!(cur, &[0xDE, 0xAD]);
    }

    // ── read_varint (std::io::Read) tests ──────────────────────────────

    #[cfg(feature = "std")]
    mod read_varint_tests {
        use super::super::read_varint;
        use crate::encoding::encode_varint;

        #[test]
        fn roundtrip_values() {
            let cases: &[u64] = &[0, 1, 127, 128, 300, 1 << 14, 1 << 35, 1 << 63, u64::MAX];
            for &v in cases {
                let mut buf = Vec::new();
                encode_varint(v, &mut buf);
                let got = read_varint(&mut buf.as_slice()).unwrap();
                assert_eq!(got, v, "roundtrip failed for {v}");
            }
        }

        #[test]
        fn rejects_10th_byte_overflow() {
            // 9 continuation bytes + 10th byte with overflow bit (0x02).
            // Mirrors encoding::tests::test_varint_10th_byte_overflow_rejected.
            let mut bad: Vec<u8> = vec![0xFF; 9];
            bad.push(0x02);
            let err = read_varint(&mut bad.as_slice()).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        }

        #[test]
        fn rejects_11th_byte() {
            // 10 continuation bytes — implies an 11th byte is needed.
            let bad: &[u8] = &[0xFF; 10];
            let err = read_varint(&mut &bad[..]).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        }

        #[test]
        fn u64_max_roundtrips() {
            // u64::MAX requires 10 bytes with the 10th byte == 0x01.
            let mut buf = Vec::new();
            encode_varint(u64::MAX, &mut buf);
            assert_eq!(buf.len(), 10);
            assert_eq!(buf[9], 0x01);
            let got = read_varint(&mut buf.as_slice()).unwrap();
            assert_eq!(got, u64::MAX);
        }

        #[test]
        fn eof_before_terminator_is_error() {
            // Single continuation byte with no follow-up.
            let bad: &[u8] = &[0x80];
            let err = read_varint(&mut &bad[..]).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        }

        #[test]
        fn empty_input_is_error() {
            let err = read_varint(&mut &[][..]).unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        }
    }

    // ── DecodeOptions std::io::Read tests ─────────────────────────────

    #[cfg(feature = "std")]
    mod reader_tests {
        use super::*;

        #[test]
        fn decode_reader_basic() {
            let src = FlatMsg {
                value: 42,
                ..Default::default()
            };
            let bytes = src.encode_to_vec();
            let msg: FlatMsg = DecodeOptions::new()
                .decode_reader(&mut bytes.as_slice())
                .unwrap();
            assert_eq!(msg.value, 42);
        }

        #[test]
        fn decode_reader_rejects_oversize() {
            let src = FlatMsg {
                value: 42,
                ..Default::default()
            };
            let bytes = src.encode_to_vec();
            let err = DecodeOptions::new()
                .with_max_message_size(1)
                .decode_reader::<FlatMsg>(&mut bytes.as_slice())
                .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        }

        #[test]
        fn decode_reader_exact_boundary() {
            // max_message_size == encoded length → success.
            let src = FlatMsg {
                value: 42,
                ..Default::default()
            };
            let bytes = src.encode_to_vec();
            let msg: FlatMsg = DecodeOptions::new()
                .with_max_message_size(bytes.len())
                .decode_reader(&mut bytes.as_slice())
                .unwrap();
            assert_eq!(msg.value, 42);
        }

        #[test]
        fn decode_reader_propagates_read_error() {
            // A reader that errors immediately.
            struct ErrReader;
            impl std::io::Read for ErrReader {
                fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "gone"))
                }
            }
            let err = DecodeOptions::new()
                .decode_reader::<FlatMsg>(&mut ErrReader)
                .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
        }

        #[test]
        fn decode_length_delimited_reader_basic() {
            let src = FlatMsg {
                value: 99,
                ..Default::default()
            };
            let mut ld = Vec::new();
            src.encode_length_delimited(&mut ld);
            let msg: FlatMsg = DecodeOptions::new()
                .decode_length_delimited_reader(&mut ld.as_slice())
                .unwrap();
            assert_eq!(msg.value, 99);
        }

        #[test]
        fn decode_length_delimited_reader_rejects_oversize_prefix() {
            // Length prefix claims more bytes than max_message_size allows.
            let src = FlatMsg {
                value: 99,
                ..Default::default()
            };
            let mut ld = Vec::new();
            src.encode_length_delimited(&mut ld);
            let err = DecodeOptions::new()
                .with_max_message_size(1)
                .decode_length_delimited_reader::<FlatMsg>(&mut ld.as_slice())
                .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        }

        #[test]
        fn decode_length_delimited_reader_sequential() {
            // Two messages in a stream — typical log-file use case.
            let a = FlatMsg {
                value: 10,
                ..Default::default()
            };
            let b = FlatMsg {
                value: 20,
                ..Default::default()
            };
            let mut stream = Vec::new();
            a.encode_length_delimited(&mut stream);
            b.encode_length_delimited(&mut stream);

            let mut cursor = std::io::Cursor::new(stream);
            let first: FlatMsg = DecodeOptions::new()
                .decode_length_delimited_reader(&mut cursor)
                .unwrap();
            assert_eq!(first.value, 10);
            let second: FlatMsg = DecodeOptions::new()
                .decode_length_delimited_reader(&mut cursor)
                .unwrap();
            assert_eq!(second.value, 20);
        }

        #[test]
        fn decode_length_delimited_reader_truncated_body() {
            // Length prefix says N bytes but reader EOFs early.
            let mut buf = Vec::new();
            crate::encoding::encode_varint(100, &mut buf);
            buf.push(0x08);
            let err = DecodeOptions::new()
                .decode_length_delimited_reader::<FlatMsg>(&mut buf.as_slice())
                .unwrap_err();
            assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
        }
    }
}
