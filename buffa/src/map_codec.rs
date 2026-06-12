//! Generic wire codec for `map<K, V>` fields.
//!
//! Generated code used to inline ~40 lines of entry encode/size/merge per
//! map field. The shape only varies by the key/value proto types, so the
//! variation is captured here as zero-sized **codec** types (one per proto
//! scalar type — proto types, not Rust types, because e.g. `int32` /
//! `sint32` / `sfixed32` all map to `i32` with different encodings) plus
//! generic per-field helpers. Generated call sites name the codecs by
//! turbofish and let the map's own key/value types drive inference:
//!
//! ```ignore
//! size += ::buffa::map_codec::field_len::<Str, Int32>(&self.stock, 1u32);
//! ::buffa::map_codec::write_field::<Str, Int32>(&self.stock, 5u32, buf);
//! ::buffa::map_codec::merge_entry::<Str, Int32>(&mut self.stock, buf, ctx)?;
//! ```
//!
//! Everything monomorphizes to the same code the previous inline expansion
//! produced; the fixed-width fast path (`len() * const`) is preserved via
//! [`MapCodec::FIXED_LEN`], which folds at compile time.
//!
//! Message-typed values are the one asymmetry: their encoded size is
//! two-pass (a [`SizeCache`] slot is reserved during `compute_size` and
//! consumed during `write_to`, in identical iteration order). They get
//! dedicated [`message_field_len`] / [`write_message_field`] helpers, and
//! implement only [`MapValueDecode`] (via [`Msg`]) for the merge path.

use crate::bytes::{Buf, BufMut};
use crate::encoding::{
    check_wire_type, decode_varint, encode_varint, skip_field_depth, varint_len, Tag, WireType,
};
use crate::error::DecodeError;
use crate::types;
use crate::{DecodeContext, EnumValue, Enumeration, Message, SizeCache};
use core::hash::Hash;

/// The `HashMap` type generated map fields use (`std` or `hashbrown`).
///
/// An alias for the deliberately unstable `__private::HashMap`; it appears
/// in these helpers' signatures only so generated call sites type-check.
/// Like the rest of this module it is not a stable consumer-facing surface.
pub type Map<K, V> = crate::__private::HashMap<K, V>;

mod sealed {
    /// Seals [`MapValueDecode`](super::MapValueDecode) / [`MapCodec`](super::MapCodec).
    ///
    /// The traits carry invariants the type system cannot enforce —
    /// `WIRE_TYPE` must match the payload `merge` reads and `encode`
    /// writes, and a wrong `FIXED_LEN` would make [`field_len`](super::field_len)
    /// disagree with [`write_field`](super::write_field), corrupting output.
    /// The proto scalar set is closed, so only buffa's own codecs implement
    /// them.
    pub trait Sealed {}
}

/// Decode side of a map key/value codec.
///
/// Implemented by every codec, including [`Msg`] for message-typed values
/// (whose *encode* side is two-pass and lives in [`message_field_len`] /
/// [`write_message_field`] instead of [`MapCodec`]).
pub trait MapValueDecode: sealed::Sealed {
    /// The Rust type this codec reads into.
    type Value: Default;
    /// The wire type every payload of this codec carries.
    const WIRE_TYPE: WireType;

    /// Merge one payload from `buf` into `value`.
    ///
    /// `ctx` carries the remaining recursion and unknown-field budgets
    /// (used by message values; scalar codecs ignore it).
    ///
    /// # Errors
    ///
    /// Returns a [`DecodeError`] on malformed payloads.
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError>;
}

/// Full (encode + size + decode) map key/value codec for non-message types.
pub trait MapCodec: MapValueDecode {
    /// `Some(n)` when every payload encodes to exactly `n` bytes
    /// (fixed-width scalars and `bool`). Lets [`field_len`] fold the whole
    /// field to `len() * entry_size` at compile time.
    ///
    /// Must equal the length [`encode`](Self::encode) actually writes for
    /// every value — [`field_len`] sizes the buffer with it. (One reason the
    /// traits are sealed.)
    const FIXED_LEN: Option<u32> = None;

    /// Encoded payload length in bytes (no tag).
    fn encoded_len(value: &Self::Value) -> u32;

    /// Write the payload (no tag) to `buf`.
    fn encode(value: &Self::Value, buf: &mut impl BufMut);
}

/// Stamp a varint/fixed scalar codec from the existing `types::` functions.
macro_rules! scalar_codec {
    ($(#[$doc:meta])* $name:ident, $value:ty, $wire:expr, $fixed:expr,
     len: $len:expr, encode: $encode:expr, decode: $decode:expr) => {
        $(#[$doc])*
        pub struct $name;

        impl sealed::Sealed for $name {}

        impl MapValueDecode for $name {
            type Value = $value;
            const WIRE_TYPE: WireType = $wire;

            #[inline]
            fn merge(
                value: &mut Self::Value,
                buf: &mut impl Buf,
                _ctx: DecodeContext<'_>,
            ) -> Result<(), DecodeError> {
                *value = $decode(buf)?;
                Ok(())
            }
        }

        impl MapCodec for $name {
            const FIXED_LEN: Option<u32> = $fixed;

            #[inline]
            #[allow(clippy::redundant_closure_call)]
            fn encoded_len(value: &Self::Value) -> u32 {
                ($len)(value) as u32
            }

            #[inline]
            #[allow(clippy::redundant_closure_call)]
            fn encode(value: &Self::Value, buf: &mut impl BufMut) {
                ($encode)(value, buf)
            }
        }
    };
}

scalar_codec!(
    /// `int32` codec.
    Int32, i32, WireType::Varint, None,
    len: |v: &i32| types::int32_encoded_len(*v),
    encode: |v: &i32, buf: &mut _| types::encode_int32(*v, buf),
    decode: types::decode_int32
);
scalar_codec!(
    /// `int64` codec.
    Int64, i64, WireType::Varint, None,
    len: |v: &i64| types::int64_encoded_len(*v),
    encode: |v: &i64, buf: &mut _| types::encode_int64(*v, buf),
    decode: types::decode_int64
);
scalar_codec!(
    /// `uint32` codec.
    Uint32, u32, WireType::Varint, None,
    len: |v: &u32| types::uint32_encoded_len(*v),
    encode: |v: &u32, buf: &mut _| types::encode_uint32(*v, buf),
    decode: types::decode_uint32
);
scalar_codec!(
    /// `uint64` codec.
    Uint64, u64, WireType::Varint, None,
    len: |v: &u64| types::uint64_encoded_len(*v),
    encode: |v: &u64, buf: &mut _| types::encode_uint64(*v, buf),
    decode: types::decode_uint64
);
scalar_codec!(
    /// `sint32` (zigzag) codec.
    Sint32, i32, WireType::Varint, None,
    len: |v: &i32| types::sint32_encoded_len(*v),
    encode: |v: &i32, buf: &mut _| types::encode_sint32(*v, buf),
    decode: types::decode_sint32
);
scalar_codec!(
    /// `sint64` (zigzag) codec.
    Sint64, i64, WireType::Varint, None,
    len: |v: &i64| types::sint64_encoded_len(*v),
    encode: |v: &i64, buf: &mut _| types::encode_sint64(*v, buf),
    decode: types::decode_sint64
);
scalar_codec!(
    /// `bool` codec.
    Bool, bool, WireType::Varint, Some(types::BOOL_ENCODED_LEN as u32),
    len: |_: &bool| types::BOOL_ENCODED_LEN,
    encode: |v: &bool, buf: &mut _| types::encode_bool(*v, buf),
    decode: types::decode_bool
);
scalar_codec!(
    /// `fixed32` codec.
    Fixed32, u32, WireType::Fixed32, Some(types::FIXED32_ENCODED_LEN as u32),
    len: |_: &u32| types::FIXED32_ENCODED_LEN,
    encode: |v: &u32, buf: &mut _| types::encode_fixed32(*v, buf),
    decode: types::decode_fixed32
);
scalar_codec!(
    /// `fixed64` codec.
    Fixed64, u64, WireType::Fixed64, Some(types::FIXED64_ENCODED_LEN as u32),
    len: |_: &u64| types::FIXED64_ENCODED_LEN,
    encode: |v: &u64, buf: &mut _| types::encode_fixed64(*v, buf),
    decode: types::decode_fixed64
);
scalar_codec!(
    /// `sfixed32` codec.
    Sfixed32, i32, WireType::Fixed32, Some(types::FIXED32_ENCODED_LEN as u32),
    len: |_: &i32| types::FIXED32_ENCODED_LEN,
    encode: |v: &i32, buf: &mut _| types::encode_sfixed32(*v, buf),
    decode: types::decode_sfixed32
);
scalar_codec!(
    /// `sfixed64` codec.
    Sfixed64, i64, WireType::Fixed64, Some(types::FIXED64_ENCODED_LEN as u32),
    len: |_: &i64| types::FIXED64_ENCODED_LEN,
    encode: |v: &i64, buf: &mut _| types::encode_sfixed64(*v, buf),
    decode: types::decode_sfixed64
);
scalar_codec!(
    /// `float` codec.
    Float, f32, WireType::Fixed32, Some(types::FIXED32_ENCODED_LEN as u32),
    len: |_: &f32| types::FIXED32_ENCODED_LEN,
    encode: |v: &f32, buf: &mut _| types::encode_float(*v, buf),
    decode: types::decode_float
);
scalar_codec!(
    /// `double` codec.
    Double, f64, WireType::Fixed64, Some(types::FIXED64_ENCODED_LEN as u32),
    len: |_: &f64| types::FIXED64_ENCODED_LEN,
    encode: |v: &f64, buf: &mut _| types::encode_double(*v, buf),
    decode: types::decode_double
);
scalar_codec!(
    /// `string` codec.
    Str, crate::alloc::string::String, WireType::LengthDelimited, None,
    len: |v: &crate::alloc::string::String| types::string_encoded_len(v),
    encode: |v: &crate::alloc::string::String, buf: &mut _| types::encode_string(v, buf),
    decode: types::decode_string
);
scalar_codec!(
    /// `bytes` codec (`Vec<u8>` representation).
    BytesVec, crate::alloc::vec::Vec<u8>, WireType::LengthDelimited, None,
    len: |v: &crate::alloc::vec::Vec<u8>| types::bytes_encoded_len(v),
    encode: |v: &crate::alloc::vec::Vec<u8>, buf: &mut _| types::encode_bytes(v, buf),
    decode: types::decode_bytes
);
scalar_codec!(
    /// `bytes` codec (`bytes::Bytes` representation, via the `bytes_fields`
    /// codegen option; zero-copy when the source buffer is `Bytes`-backed).
    BytesBuf, crate::bytes::Bytes, WireType::LengthDelimited, None,
    len: |v: &crate::bytes::Bytes| types::bytes_encoded_len(v),
    encode: |v: &crate::bytes::Bytes, buf: &mut _| types::encode_bytes(v, buf),
    decode: types::decode_bytes_to_bytes
);

/// Open-enum codec: values decode into [`EnumValue<E>`], preserving unknown
/// numbers.
pub struct OpenEnum<E>(core::marker::PhantomData<E>);

impl<E: Enumeration> sealed::Sealed for OpenEnum<E> {}

impl<E: Enumeration> MapValueDecode for OpenEnum<E> {
    type Value = EnumValue<E>;
    const WIRE_TYPE: WireType = WireType::Varint;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        _ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        *value = EnumValue::from(types::decode_int32(buf)?);
        Ok(())
    }
}

impl<E: Enumeration> MapCodec for OpenEnum<E> {
    #[inline]
    fn encoded_len(value: &Self::Value) -> u32 {
        types::int32_encoded_len(value.to_i32()) as u32
    }

    #[inline]
    fn encode(value: &Self::Value, buf: &mut impl BufMut) {
        types::encode_int32(value.to_i32(), buf);
    }
}

/// Closed-enum codec: values decode into the bare enum `E`.
///
/// Unknown numeric values are dropped, leaving the entry's value at its
/// previous (default) state — matching the long-standing generated-code
/// behaviour for closed-enum map values (the proto2 spec's
/// route-entire-entry-to-unknown-fields semantics are a known gap; see
/// DESIGN.md).
pub struct ClosedEnum<E>(core::marker::PhantomData<E>);

impl<E: Enumeration + Default> sealed::Sealed for ClosedEnum<E> {}

impl<E: Enumeration + Default> MapValueDecode for ClosedEnum<E> {
    type Value = E;
    const WIRE_TYPE: WireType = WireType::Varint;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        _ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        let raw = types::decode_int32(buf)?;
        if let Some(v) = E::from_i32(raw) {
            *value = v;
        }
        Ok(())
    }
}

impl<E: Enumeration + Default> MapCodec for ClosedEnum<E> {
    #[inline]
    fn encoded_len(value: &Self::Value) -> u32 {
        types::int32_encoded_len(value.to_i32()) as u32
    }

    #[inline]
    fn encode(value: &Self::Value, buf: &mut impl BufMut) {
        types::encode_int32(value.to_i32(), buf);
    }
}

/// Message-value codec (decode side only — encode is two-pass via
/// [`message_field_len`] / [`write_message_field`]).
pub struct Msg<M>(core::marker::PhantomData<M>);

impl<M: Message + Default> sealed::Sealed for Msg<M> {}

impl<M: Message + Default> MapValueDecode for Msg<M> {
    type Value = M;
    const WIRE_TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        ctx: DecodeContext<'_>,
    ) -> Result<(), DecodeError> {
        Message::merge_length_delimited(value, buf, ctx)
    }
}

/// Key tag (field 1) and value tag (field 2) are both single-byte for every
/// wire type, so each entry carries exactly two tag bytes.
const ENTRY_TAG_LEN: u32 = 2;

#[inline]
fn entry_len<KC: MapCodec, VC: MapCodec>(k: &KC::Value, v: &VC::Value) -> u32 {
    ENTRY_TAG_LEN + KC::encoded_len(k) + VC::encoded_len(v)
}

/// Total encoded length of a scalar-valued map field, including each entry's
/// outer tag and length prefix.
///
/// `outer_tag_len` is the encoded length of the field's outer tag (a codegen
/// constant). When both codecs are fixed-width the per-entry size is a
/// compile-time constant and the loop folds to `len() * entry`.
pub fn field_len<KC: MapCodec, VC: MapCodec>(
    map: &Map<KC::Value, VC::Value>,
    outer_tag_len: u32,
) -> u32 {
    if let (Some(kf), Some(vf)) = (KC::FIXED_LEN, VC::FIXED_LEN) {
        let entry = ENTRY_TAG_LEN + kf + vf;
        return map.len() as u32 * (outer_tag_len + varint_len(entry as u64) as u32 + entry);
    }
    let mut size = 0u32;
    for (k, v) in map {
        let entry = entry_len::<KC, VC>(k, v);
        size += outer_tag_len + varint_len(entry as u64) as u32 + entry;
    }
    size
}

/// Write a scalar-valued map field: one `field_number`-tagged,
/// length-prefixed entry per element.
pub fn write_field<KC: MapCodec, VC: MapCodec>(
    map: &Map<KC::Value, VC::Value>,
    field_number: u32,
    buf: &mut impl BufMut,
) {
    for (k, v) in map {
        let entry = entry_len::<KC, VC>(k, v);
        Tag::new(field_number, WireType::LengthDelimited).encode(buf);
        encode_varint(entry as u64, buf);
        Tag::new(1, KC::WIRE_TYPE).encode(buf);
        KC::encode(k, buf);
        Tag::new(2, VC::WIRE_TYPE).encode(buf);
        VC::encode(v, buf);
    }
}

/// Total encoded length of a message-valued map field.
///
/// Reserves one [`SizeCache`] slot per entry (in map iteration order);
/// [`write_message_field`] consumes the slots in the same order — both
/// helpers iterate the same map, so the orders match by construction.
pub fn message_field_len<KC: MapCodec, M: Message>(
    map: &Map<KC::Value, M>,
    outer_tag_len: u32,
    cache: &mut SizeCache,
) -> u32 {
    let mut size = 0u32;
    for (k, v) in map {
        let slot = cache.reserve();
        let inner = v.compute_size(cache);
        cache.set(slot, inner);
        let entry = ENTRY_TAG_LEN + KC::encoded_len(k) + varint_len(inner as u64) as u32 + inner;
        size += outer_tag_len + varint_len(entry as u64) as u32 + entry;
    }
    size
}

/// Write a message-valued map field, consuming the [`SizeCache`] slots
/// reserved by [`message_field_len`].
pub fn write_message_field<KC: MapCodec, M: Message>(
    map: &Map<KC::Value, M>,
    field_number: u32,
    cache: &mut SizeCache,
    buf: &mut impl BufMut,
) {
    for (k, v) in map {
        let inner = cache.consume_next();
        let entry = ENTRY_TAG_LEN + KC::encoded_len(k) + varint_len(inner as u64) as u32 + inner;
        Tag::new(field_number, WireType::LengthDelimited).encode(buf);
        encode_varint(entry as u64, buf);
        Tag::new(1, KC::WIRE_TYPE).encode(buf);
        KC::encode(k, buf);
        Tag::new(2, WireType::LengthDelimited).encode(buf);
        encode_varint(inner as u64, buf);
        v.write_to(cache, buf);
    }
}

/// Decode one length-prefixed map entry from `buf` and insert it.
///
/// Implements proto map-entry semantics: missing key/value fields take
/// their type defaults, repeated occurrences within one entry last-win,
/// unknown entry fields are skipped, and a short or over-long entry payload
/// is corrected against the length prefix.
///
/// # Errors
///
/// Returns a [`DecodeError`] on malformed lengths, payloads, or wire-type
/// mismatches inside the entry.
pub fn merge_entry<KC, VC>(
    map: &mut Map<KC::Value, VC::Value>,
    buf: &mut impl Buf,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError>
where
    KC: MapValueDecode,
    KC::Value: Eq + Hash,
    VC: MapValueDecode,
{
    let entry_len = decode_varint(buf)?;
    let entry_len = usize::try_from(entry_len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < entry_len {
        return Err(DecodeError::UnexpectedEof);
    }
    let entry_limit = buf.remaining() - entry_len;
    let mut key: KC::Value = Default::default();
    let mut val: VC::Value = Default::default();
    while buf.remaining() > entry_limit {
        let entry_tag = Tag::decode(buf)?;
        match entry_tag.field_number() {
            1 => {
                check_wire_type(entry_tag, KC::WIRE_TYPE)?;
                KC::merge(&mut key, buf, ctx)?;
            }
            2 => {
                check_wire_type(entry_tag, VC::WIRE_TYPE)?;
                VC::merge(&mut val, buf, ctx)?;
            }
            _ => {
                skip_field_depth(entry_tag, buf, ctx.depth())?;
            }
        }
    }
    // Correct the buffer position if the entry was not fully consumed.
    if buf.remaining() != entry_limit {
        let remaining = buf.remaining();
        if remaining > entry_limit {
            buf.advance(remaining - entry_limit);
        } else {
            return Err(DecodeError::UnexpectedEof);
        }
    }
    map.insert(key, val);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc::string::String;
    use crate::alloc::vec::Vec;

    fn encode_field<KC: MapCodec, VC: MapCodec>(
        map: &Map<KC::Value, VC::Value>,
        field_number: u32,
        outer_tag_len: u32,
    ) -> Vec<u8> {
        let len = field_len::<KC, VC>(map, outer_tag_len);
        let mut buf = Vec::new();
        write_field::<KC, VC>(map, field_number, &mut buf);
        assert_eq!(buf.len() as u32, len, "field_len must match written bytes");
        buf
    }

    fn decode_field<KC, VC>(mut wire: &[u8]) -> Map<KC::Value, VC::Value>
    where
        KC: MapValueDecode,
        KC::Value: Eq + Hash,
        VC: MapValueDecode,
    {
        let mut map = Map::default();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        while !wire.is_empty() {
            let tag = Tag::decode(&mut wire).unwrap();
            assert_eq!(tag.wire_type(), WireType::LengthDelimited);
            let ctx = DecodeContext::new(crate::RECURSION_LIMIT, &limit);
            merge_entry::<KC, VC>(&mut map, &mut wire, ctx).unwrap();
        }
        map
    }

    #[test]
    fn string_int32_round_trip() {
        let mut map: Map<String, i32> = Map::default();
        map.insert("a".into(), 1);
        map.insert("bee".into(), -7);
        let wire = encode_field::<Str, Int32>(&map, 5, 1);
        let back = decode_field::<Str, Int32>(&wire);
        assert_eq!(back, map);
    }

    #[test]
    fn fixed_fixed_len_fold_matches_written_bytes() {
        let mut map: Map<u32, f64> = Map::default();
        map.insert(1, 0.5);
        map.insert(9, -2.25);
        map.insert(1000, 0.0);
        // Both codecs fixed-width → field_len takes the folded path; the
        // assert inside encode_field proves it equals the written bytes.
        let wire = encode_field::<Fixed32, Double>(&map, 3, 1);
        let back = decode_field::<Fixed32, Double>(&wire);
        assert_eq!(back, map);
    }

    #[test]
    fn missing_key_and_value_take_defaults() {
        // Entry with no fields at all: length prefix 0.
        let wire = [0x00u8];
        let mut map: Map<String, i32> = Map::default();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        merge_entry::<Str, Int32>(&mut map, &mut &wire[..], DecodeContext::new(10, &limit))
            .unwrap();
        assert_eq!(map.get(""), Some(&0));
    }

    #[test]
    fn unknown_entry_field_is_skipped() {
        // key "a" (field 1), unknown varint field 3, value 7 (field 2).
        let mut entry = Vec::new();
        Tag::new(1, WireType::LengthDelimited).encode(&mut entry);
        types::encode_string("a", &mut entry);
        Tag::new(3, WireType::Varint).encode(&mut entry);
        encode_varint(99, &mut entry);
        Tag::new(2, WireType::Varint).encode(&mut entry);
        types::encode_int32(7, &mut entry);
        let mut wire = Vec::new();
        encode_varint(entry.len() as u64, &mut wire);
        wire.extend_from_slice(&entry);

        let mut map: Map<String, i32> = Map::default();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        merge_entry::<Str, Int32>(
            &mut map,
            &mut wire.as_slice(),
            DecodeContext::new(10, &limit),
        )
        .unwrap();
        assert_eq!(map.get("a"), Some(&7));
    }

    #[test]
    fn entry_wire_type_mismatch_errors() {
        // Field 1 claims Fixed64 for a string key.
        let mut entry = Vec::new();
        Tag::new(1, WireType::Fixed64).encode(&mut entry);
        entry.extend_from_slice(&[0u8; 8]);
        let mut wire = Vec::new();
        encode_varint(entry.len() as u64, &mut wire);
        wire.extend_from_slice(&entry);

        let mut map: Map<String, i32> = Map::default();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let err = merge_entry::<Str, Int32>(
            &mut map,
            &mut wire.as_slice(),
            DecodeContext::new(10, &limit),
        )
        .unwrap_err();
        assert!(matches!(err, DecodeError::WireTypeMismatch { .. }));
    }

    #[test]
    fn truncated_entry_errors() {
        // Length prefix promises 5 bytes; only 1 available.
        let wire = [0x05u8, 0x08];
        let mut map: Map<String, i32> = Map::default();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        let err =
            merge_entry::<Str, Int32>(&mut map, &mut &wire[..], DecodeContext::new(10, &limit))
                .unwrap_err();
        assert!(matches!(err, DecodeError::UnexpectedEof));
    }

    #[test]
    fn message_map_two_pass_round_trip() {
        use crate::{DefaultInstance, SizeCache};

        #[derive(Clone, PartialEq, Eq, Debug, Default)]
        struct FlatMsg {
            value: i32,
        }

        impl DefaultInstance for FlatMsg {
            fn default_instance() -> &'static Self {
                static INST: crate::__private::OnceBox<FlatMsg> = crate::__private::OnceBox::new();
                INST.get_or_init(|| crate::alloc::boxed::Box::new(FlatMsg::default()))
            }
        }

        impl Message for FlatMsg {
            fn compute_size(&self, _cache: &mut SizeCache) -> u32 {
                if self.value != 0 {
                    1 + types::int32_encoded_len(self.value) as u32
                } else {
                    0
                }
            }
            fn write_to(&self, _cache: &mut SizeCache, buf: &mut impl BufMut) {
                if self.value != 0 {
                    Tag::new(1, WireType::Varint).encode(buf);
                    types::encode_int32(self.value, buf);
                }
            }
            fn merge_field(
                &mut self,
                tag: Tag,
                buf: &mut impl Buf,
                ctx: DecodeContext<'_>,
            ) -> Result<(), DecodeError> {
                match tag.field_number() {
                    1 => self.value = types::decode_int32(buf)?,
                    _ => skip_field_depth(tag, buf, ctx.depth())?,
                }
                Ok(())
            }
            fn clear(&mut self) {
                *self = Self::default();
            }
        }

        let mut map: Map<i32, FlatMsg> = Map::default();
        map.insert(1, FlatMsg { value: 0 }); // empty payload entry
        map.insert(2, FlatMsg { value: -3 }); // multi-byte varint payload
        map.insert(9, FlatMsg { value: 7 });

        // Two-pass: message_field_len reserves SizeCache slots in map
        // iteration order; write_message_field consumes them in the same
        // order. The size must equal the written bytes exactly.
        let mut cache = SizeCache::default();
        let len = message_field_len::<Int32, FlatMsg>(&map, 1, &mut cache);
        let mut wire = Vec::new();
        write_message_field::<Int32, FlatMsg>(&map, 4, &mut cache, &mut wire);
        assert_eq!(wire.len() as u32, len, "size pass must match write pass");

        let back = decode_field::<Int32, Msg<FlatMsg>>(&wire);
        assert_eq!(back, map);
    }

    #[test]
    fn open_enum_value_preserves_unknown() {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
        #[repr(i32)]
        enum E {
            #[default]
            A = 0,
        }
        impl Enumeration for E {
            fn from_i32(value: i32) -> Option<Self> {
                (value == 0).then_some(E::A)
            }
            fn to_i32(&self) -> i32 {
                *self as i32
            }
            fn proto_name(&self) -> &'static str {
                "A"
            }
            fn from_proto_name(name: &str) -> Option<Self> {
                (name == "A").then_some(E::A)
            }
        }

        let mut map: Map<i32, EnumValue<E>> = Map::default();
        map.insert(1, EnumValue::Unknown(42));
        let wire = encode_field::<Int32, OpenEnum<E>>(&map, 2, 1);
        let back = decode_field::<Int32, OpenEnum<E>>(&wire);
        assert_eq!(back.get(&1), Some(&EnumValue::Unknown(42)));

        // Closed codec drops the unknown value, keeping the default.
        let back = decode_field::<Int32, ClosedEnum<E>>(&wire);
        assert_eq!(back.get(&1), Some(&E::A));
    }
}
