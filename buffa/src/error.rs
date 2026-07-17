//! Error types for buffa encoding and decoding operations.

/// An error that occurred while decoding a protobuf message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// The buffer ended before a complete value could be read.
    #[error("unexpected end of buffer")]
    UnexpectedEof,

    /// A varint exceeded the maximum encoded length of 10 bytes.
    #[error("varint exceeded maximum length of 10 bytes")]
    VarintTooLong,

    /// The wire type in a tag was not a recognised protobuf wire type.
    ///
    /// Carries the raw 3-bit value from the tag for diagnostic purposes.
    #[error("invalid wire type: {0}")]
    InvalidWireType(u32),

    /// The field number decoded from a tag was zero, or the tag varint
    /// overflowed a `u32` — both indicate a malformed message.
    #[error("invalid field number")]
    InvalidFieldNumber,

    /// The message or sub-message length exceeded the size limit.
    ///
    /// By default, the limit is the 2 GiB protobuf maximum. Use
    /// [`DecodeOptions::with_max_message_size`](crate::DecodeOptions::with_max_message_size)
    /// to set a lower limit for untrusted input. Fallible re-encode paths
    /// ([`OwnedView::from_owned`](crate::view::OwnedView::from_owned)) also
    /// surface an over-limit *encode* through this variant, mirroring
    /// [`EncodeError::MessageTooLarge`].
    #[error("message length exceeds the size limit (2 GiB protobuf maximum, or a configured DecodeOptions limit)")]
    MessageTooLarge,

    /// The wire type of an incoming field did not match the type expected for
    /// that field number.
    ///
    /// Carries the field number and the raw wire type values (as `u8` to keep
    /// this type independent of the encoding module).
    #[error("wire type mismatch on field {field_number}: expected {expected}, got {actual}")]
    WireTypeMismatch {
        field_number: u32,
        expected: u8,
        actual: u8,
    },

    /// A `string` field contained bytes that are not valid UTF-8.
    #[error("invalid UTF-8 in string field")]
    InvalidUtf8,

    /// The message nesting depth exceeded the recursion limit.
    #[error("recursion limit exceeded")]
    RecursionLimitExceeded,

    /// An EndGroup tag was encountered with a field number that does not match
    /// the opening StartGroup tag, or an EndGroup was seen outside of a group.
    #[error("invalid end-group tag: field number {0}")]
    InvalidEndGroup(u32),

    /// A MessageSet `Item` group was malformed (missing or out-of-range
    /// `type_id`). Only occurs for messages declared with
    /// `option message_set_wire_format = true`.
    #[error("invalid MessageSet item: {0}")]
    InvalidMessageSet(&'static str),

    /// Decoding encountered more unknown fields than the configured limit.
    ///
    /// Unknown fields can be far smaller on the wire than in memory (a
    /// 2-byte varint field occupies ~40 bytes as an
    /// [`UnknownField`](crate::UnknownField)), so the decoder bounds how
    /// many it will materialize rather than trusting the input size. By
    /// default the limit is
    /// [`DEFAULT_UNKNOWN_FIELD_LIMIT`](crate::DEFAULT_UNKNOWN_FIELD_LIMIT)
    /// (1,000,000 fields per decode); use
    /// [`DecodeOptions::with_unknown_field_limit`](crate::DecodeOptions::with_unknown_field_limit)
    /// to raise it for trusted inputs that legitimately carry very many
    /// unknown fields.
    #[error("unknown field limit exceeded")]
    UnknownFieldLimitExceeded,

    /// A decode would materialize more memory in the elements of
    /// length-delimited containers — repeated message, string and bytes fields,
    /// and map entries — than its budget allows.
    ///
    /// These elements cost far more decoded than encoded: an empty message
    /// element is two bytes on the wire and `size_of::<T>()` in the `Vec` it
    /// lands in, so a payload well inside
    /// [`DecodeOptions::with_max_message_size`](crate::DecodeOptions::with_max_message_size)
    /// can still expand by two orders of magnitude. By default the budget is
    /// [`DEFAULT_ELEMENT_MEMORY_LIMIT`](crate::DEFAULT_ELEMENT_MEMORY_LIMIT)
    /// (32 MiB per decode); use
    /// [`DecodeOptions::with_element_memory_limit`](crate::DecodeOptions::with_element_memory_limit)
    /// to raise it for trusted inputs that legitimately decode into more.
    #[error("element memory limit exceeded")]
    ElementMemoryLimitExceeded,

    /// A custom `string`/`bytes` representation rejected the decoded payload in
    /// its [`from_wire`](crate::ProtoString::from_wire) constructor — for
    /// example a length or domain check beyond UTF-8 validation. Carries a
    /// static reason for diagnostics (mirroring [`InvalidMessageSet`]).
    ///
    /// Only produced by user-supplied [`ProtoString`](crate::ProtoString) /
    /// [`ProtoBytes`](crate::ProtoBytes) impls; the built-in representations
    /// never return it.
    ///
    /// [`InvalidMessageSet`]: DecodeError::InvalidMessageSet
    #[error("custom representation rejected the field value: {0}")]
    Custom(&'static str),
}

/// An error that occurred while encoding a protobuf message.
///
/// Returned by the `try_encode*` family
/// ([`Message::try_encode`](crate::Message::try_encode) and friends). The
/// panicking entry points ([`Message::encode`](crate::Message::encode) and
/// friends) raise the same conditions as panics instead.
///
/// The enum is `#[non_exhaustive]`: further variants (e.g. for a
/// fixed-capacity buffer encode path) may be added without a breaking
/// change to the type name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The message's encoded size exceeds the 2 GiB protobuf limit
    /// ([`MAX_MESSAGE_BYTES`](crate::MAX_MESSAGE_BYTES)).
    ///
    /// Encoding such a message would produce bytes that no conforming
    /// protobuf decoder — including buffa's own, which returns the mirror
    /// error [`DecodeError::MessageTooLarge`] — will accept. Shrink or
    /// split the message instead.
    #[error("message encoded size exceeds the 2 GiB protobuf limit")]
    MessageTooLarge,
}
