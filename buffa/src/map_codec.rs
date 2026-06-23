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
//! size += ::buffa::map_codec::field_len::<Str, Int32, _>(&self.stock, 1u32);
//! ::buffa::map_codec::write_field::<Str, Int32, _>(&self.stock, 5u32, buf);
//! ::buffa::map_codec::merge_entry::<Str, Int32, _>(&mut self.stock, buf, ctx)?;
//! ```
//!
//! The trailing `_` is the container type ([`MapStorage`]); it is inferred from
//! the map argument but must be written explicitly, because a partial turbofish
//! on a three-generic function is a hard error.
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

/// The default owned `HashMap` type generated `map<K, V>` fields use.
///
/// On `std` builds this resolves to `std::collections::HashMap<K, V,
/// foldhash::fast::RandomState>`; on `no_std` builds it is
/// `hashbrown::HashMap<K, V>` (which defaults to the same `foldhash` hasher).
/// This alias is the recommended way to name a generated map field's type in
/// downstream code, so that the concrete hasher and container stay an
/// implementation detail.
///
/// Re-exported at the crate root as `buffa::Map`, alongside [`MapStorage`]:
/// `use buffa::Map;`.
///
/// # Construction
///
/// Use [`Map::default`] to construct an empty map. The inherent
/// `HashMap::new()` and `HashMap::with_capacity()` are only defined for the
/// std default hasher, so they are unavailable on `Map` under `std` (and would
/// be non-portable across the `std`/`no_std` boundary). For literals, use
/// `[(k, v), ..].into_iter().collect()` rather than `Map::from([...])` —
/// `From<[_; N]>` is likewise hasher-pinned in std. To preallocate, use
/// `std::collections::HashMap::with_capacity_and_hasher(n, Default::default())`.
///
/// # Hasher and HashDoS
///
/// `foldhash::fast` is per-instance seeded — each `Map::default()` derives a
/// fresh seed from the stack pointer mixed with a thread-local counter,
/// layered on a process-wide seed derived from ASLR addresses and process
/// start time. That nondeterminism means a single static collision set will
/// not transfer across processes, but the seed is **not** drawn from a
/// CSPRNG (unlike `std::hash::RandomState`, which seeds SipHash from
/// `getrandom`), and `foldhash::fast` does not advertise HashDoS resistance.
/// Treat this default as **not hardened** against adversarial hash flooding.
///
/// This is the same trade-off Google's `protobuf-v4`/upb makes (Wyhash).
/// Consumers decoding `map` fields with attacker-controlled keys who need a
/// hardened bound should select `MapRepr::BTreeMap` (no hashing, O(log n)
/// worst case) or supply a SipHash-backed map via `MapRepr::Custom`. The
/// [`MapStorage`] impl is generic over the hasher, so
/// `std::collections::HashMap<K, V, std::hash::RandomState>` works without a
/// newtype — only a foreign *container* type (e.g. `IndexMap`) needs one.
pub type Map<K, V> = crate::__private::HashMap<K, V>;

/// The owned collection backing a proto `map<K, V>` field.
///
/// The default is the `__private::HashMap` alias above; `buffa_build`'s
/// `map_type` knob can instead select the buffa-provided
/// [`BTreeMap`](alloc::collections::BTreeMap) (deterministic iteration order,
/// no extra dependency) or a custom map. The five runtime helpers in this
/// module need only three operations from the container — count entries,
/// borrow-iterate entries, and insert one entry — so this trait captures that
/// minimal surface and lets the helpers stay generic over *which* map a field
/// uses rather than hard-coding `__private::HashMap`.
///
/// The wire format is identical regardless of the container; only the in-memory
/// owned type changes, and view types are unaffected.
///
/// It is re-exported at the crate root, so downstream code can write
/// `use buffa::MapStorage;` (the longer `buffa::map_codec::MapStorage` path also
/// works).
///
/// # Sealing
///
/// Unlike [`MapCodec`] / [`MapValueDecode`], this trait is **not** sealed.
/// Those traits guard wire-format invariants the type system cannot express, so
/// they must stay closed. `MapStorage` carries no wire invariant: a buggy impl
/// can at worst iterate in an unusual order (already permitted for proto maps)
/// or drop the consumer's own entries. Leaving it open lets a downstream crate
/// wrap a foreign map (e.g. `indexmap::IndexMap`) in a crate-local newtype and
/// implement `MapStorage` on it, exactly as the orphan rule requires. The
/// wire-format seal is orthogonal and unaffected.
///
/// # Contract
///
/// - [`storage_insert`](Self::storage_insert) is **last-write-wins**, matching
///   proto map-merge semantics (a later entry for an existing key replaces the
///   earlier value).
/// - [`storage_iter`](Self::storage_iter) and [`storage_len`](Self::storage_len)
///   must agree: the iterator yields exactly `storage_len()` entries.
///
/// The key/value types are **associated types** ([`Key`](Self::Key) /
/// [`Value`](Self::Value)), not trait parameters, and the per-collection key
/// bound (`Eq + Hash` for `HashMap`, `Ord` for `BTreeMap`) lives on each impl.
/// The generic helpers bound only `C: MapStorage<Key = …, Value = …>` and never
/// name a key bound themselves. Because the key/value are associated, a type
/// implements `MapStorage` at most once, so the container — and its key and
/// value types — resolve unambiguously everywhere the trait is used as a bound.
///
/// # Requirements (for a *custom* map)
///
/// This is the canonical list of what a custom map must provide. A custom map is
/// always a **crate-local newtype** (the orphan rule blocks implementing the
/// buffa-owned reflection / serde traits on a foreign map), and must implement:
///
/// - `MapStorage` itself (this trait), naming its `Key` / `Value`.
/// - `Default` + `Clone` + `PartialEq` + `Debug` — the generated owned message
///   derives these, so its map field must support them.
/// - [`FromIterator<(Key, Value)>`] — the view→owned conversion `.collect()`s
///   entries into the owned map.
/// - `buffa_descriptor`'s `ReflectMap` under the reflection / vtable path — not
///   derivable, but a `BTreeMap`/`HashMap`-backed newtype can delegate to the
///   inner map's impl. This requirement is `std`-only (vtable reflection
///   requires `std`), so a `no_std` build never needs it.
/// - `arbitrary::Arbitrary` under the `arbitrary` feature (derivable on a
///   newtype).
/// - `serde::Serialize` / `Deserialize` under a JSON-enabled build. The proto3
///   JSON codec drives serialization through this trait, so **every** proto map
///   key/value type is supported regardless of the container — there is no
///   string-key or scalar-value restriction. (When a newtype wraps the inner map
///   in a single field, a derived `Serialize` / `Deserialize` already routes
///   transparently to the inner map; the with-modules call this trait's methods
///   directly rather than the newtype's serde, so even that is only needed when
///   the field type bypasses a with-module.)
///
/// The buffa-provided `HashMap` and `BTreeMap` already satisfy all of these, so
/// selecting `BTreeMap` needs no consumer code.
///
/// This trait is used only as a generic bound; it is **not object-safe** (the
/// [`storage_iter`](Self::storage_iter) RPITIT precludes `dyn MapStorage`).
pub trait MapStorage {
    /// The map key type.
    type Key;
    /// The map value type.
    type Value;
    /// Number of entries.
    fn storage_len(&self) -> usize;
    /// Insert one entry (last-write-wins, matching proto map-merge semantics).
    fn storage_insert(&mut self, key: Self::Key, value: Self::Value);
    /// Remove all entries (the field's cleared / default state), retaining
    /// capacity where the underlying type allows. Invoked by the generated
    /// message `clear()`.
    fn storage_clear(&mut self);
    /// Borrow-iterate entries in the container's native order.
    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a Self::Key, &'a Self::Value)>
    where
        Self::Key: 'a,
        Self::Value: 'a;
}

/// Implements [`MapStorage`] for both the `std` and `no_std` `HashMap` types,
/// generic over the hasher `S`. The buffa default `S` is `foldhash` on both
/// paths (see [`Map`]); the `S` parameter lets `MapRepr::Custom` users reach
/// any `BuildHasher` without a newtype. The `S: Default` bound is required so
/// generated owned messages can `Default`-construct the field — the read-only
/// `ReflectMap` impl in `buffa-descriptor` relaxes to `S: BuildHasher`.
macro_rules! map_storage_hashmap {
    ($($ty:tt)*) => {
        impl<K: Eq + Hash, V, S: core::hash::BuildHasher + Default> MapStorage for $($ty)*<K, V, S> {
            type Key = K;
            type Value = V;
            #[inline]
            fn storage_len(&self) -> usize {
                self.len()
            }
            #[inline]
            fn storage_insert(&mut self, key: K, value: V) {
                self.insert(key, value);
            }
            #[inline]
            fn storage_clear(&mut self) {
                self.clear();
            }
            #[inline]
            fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
            where
                K: 'a,
                V: 'a,
            {
                self.iter()
            }
        }
    };
}
#[cfg(feature = "std")]
map_storage_hashmap!(std::collections::HashMap);
#[cfg(not(feature = "std"))]
map_storage_hashmap!(hashbrown::HashMap);

impl<K: Ord, V> MapStorage for crate::alloc::collections::BTreeMap<K, V> {
    type Key = K;
    type Value = V;
    #[inline]
    fn storage_len(&self) -> usize {
        self.len()
    }
    #[inline]
    fn storage_insert(&mut self, key: K, value: V) {
        self.insert(key, value);
    }
    #[inline]
    fn storage_clear(&mut self) {
        self.clear();
    }
    #[inline]
    fn storage_iter<'a>(&'a self) -> impl Iterator<Item = (&'a K, &'a V)>
    where
        K: 'a,
        V: 'a,
    {
        self.iter()
    }
}

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
    /// Whether [`merge`](Self::merge) can report
    /// [`MapValueDecodeStatus::Unknown`].
    ///
    /// This is false for ordinary scalar/message codecs and true for closed
    /// enum codecs. [`merge_entry_with_unknowns`] uses it to avoid buffering
    /// map-entry payloads unless a value can actually force whole-entry
    /// unknown-field preservation.
    const MAY_RETURN_UNKNOWN: bool = false;

    /// Merge one payload from `buf` into `value`.
    ///
    /// Returns [`MapValueDecodeStatus::Known`] in the normal case, or
    /// [`MapValueDecodeStatus::Unknown`] when this codec is a closed enum and
    /// the payload's numeric value is not a declared variant — letting the
    /// caller route the whole map entry to unknown fields instead of inserting
    /// it.
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
    ) -> Result<MapValueDecodeStatus, DecodeError>;
}

/// Result of decoding one map-entry key/value payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MapValueDecodeStatus {
    /// The payload decoded to a normal map key/value.
    Known,
    /// The payload was an unknown value for a closed enum.
    ///
    /// Proto2 map semantics require the whole outer map-entry record to be
    /// treated as unknown in this case, not inserted with a default value.
    Unknown,
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
            ) -> Result<MapValueDecodeStatus, DecodeError> {
                *value = $decode(buf)?;
                Ok(MapValueDecodeStatus::Known)
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

/// `bytes` codec for a custom [`ProtoBytes`](crate::types::ProtoBytes) map-value
/// representation (via `bytes_type_custom`). Decodes through
/// [`from_wire`](crate::types::ProtoBytes::from_wire); encodes the borrowed
/// `&[u8]`. Generic over the value type, so the codec itself stays sealed in
/// buffa while the concrete representation is a downstream (crate-local) type.
pub struct ProtoBytesMap<B>(core::marker::PhantomData<B>);

impl<B: crate::types::ProtoBytes> sealed::Sealed for ProtoBytesMap<B> {}

impl<B: crate::types::ProtoBytes> MapValueDecode for ProtoBytesMap<B> {
    type Value = B;
    const WIRE_TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        _ctx: DecodeContext<'_>,
    ) -> Result<MapValueDecodeStatus, DecodeError> {
        *value = crate::types::decode_bytes_to::<B>(buf)?;
        Ok(MapValueDecodeStatus::Known)
    }
}

impl<B: crate::types::ProtoBytes> MapCodec for ProtoBytesMap<B> {
    #[inline]
    fn encoded_len(value: &Self::Value) -> u32 {
        types::bytes_encoded_len(value.as_ref()) as u32
    }

    #[inline]
    fn encode(value: &Self::Value, buf: &mut impl BufMut) {
        types::encode_bytes(value.as_ref(), buf);
    }
}

/// `string` codec for a custom [`ProtoString`](crate::types::ProtoString)
/// map key or value representation (via `string_type_custom`). Decodes through
/// [`from_wire`](crate::types::ProtoString::from_wire) (UTF-8 validation
/// included); encodes the borrowed `&str`. Generic over the type, so the codec
/// itself stays sealed in buffa while the concrete representation is a
/// downstream (crate-local) type.
///
/// Unlike [`ProtoBytesMap`], which is value-only (proto forbids `bytes` map
/// keys), this codec serves **both** map slots — `string` is a legal key and
/// value type. Used as a *key*, the type additionally needs the container's
/// `Eq + Hash` (`HashMap`) or `Ord` (`BTreeMap`) bound; that is enforced at the
/// generated map field type via the [`MapStorage`] impls, not here.
pub struct ProtoStringMap<S>(core::marker::PhantomData<S>);

impl<S: crate::types::ProtoString> sealed::Sealed for ProtoStringMap<S> {}

impl<S: crate::types::ProtoString> MapValueDecode for ProtoStringMap<S> {
    type Value = S;
    const WIRE_TYPE: WireType = WireType::LengthDelimited;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        _ctx: DecodeContext<'_>,
    ) -> Result<MapValueDecodeStatus, DecodeError> {
        *value = crate::types::decode_string_to::<S>(buf)?;
        Ok(MapValueDecodeStatus::Known)
    }
}

impl<S: crate::types::ProtoString> MapCodec for ProtoStringMap<S> {
    #[inline]
    fn encoded_len(value: &Self::Value) -> u32 {
        types::string_encoded_len(value.as_ref()) as u32
    }

    #[inline]
    fn encode(value: &Self::Value, buf: &mut impl BufMut) {
        types::encode_string(value.as_ref(), buf);
    }
}

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
    ) -> Result<MapValueDecodeStatus, DecodeError> {
        *value = EnumValue::from(types::decode_int32(buf)?);
        Ok(MapValueDecodeStatus::Known)
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
/// Unknown numeric values report [`MapValueDecodeStatus::Unknown`], letting
/// [`merge_entry_with_unknowns`] route the whole map-entry record to unknown
/// fields instead of inserting a default-valued entry.
pub struct ClosedEnum<E>(core::marker::PhantomData<E>);

impl<E: Enumeration + Default> sealed::Sealed for ClosedEnum<E> {}

impl<E: Enumeration + Default> MapValueDecode for ClosedEnum<E> {
    type Value = E;
    const WIRE_TYPE: WireType = WireType::Varint;
    const MAY_RETURN_UNKNOWN: bool = true;

    #[inline]
    fn merge(
        value: &mut Self::Value,
        buf: &mut impl Buf,
        _ctx: DecodeContext<'_>,
    ) -> Result<MapValueDecodeStatus, DecodeError> {
        let raw = types::decode_int32(buf)?;
        if let Some(v) = E::from_i32(raw) {
            *value = v;
            Ok(MapValueDecodeStatus::Known)
        } else {
            Ok(MapValueDecodeStatus::Unknown)
        }
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
    ) -> Result<MapValueDecodeStatus, DecodeError> {
        Message::merge_length_delimited(value, buf, ctx)?;
        Ok(MapValueDecodeStatus::Known)
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
pub fn field_len<KC: MapCodec, VC: MapCodec, C>(map: &C, outer_tag_len: u32) -> u32
where
    C: MapStorage<Key = KC::Value, Value = VC::Value>,
{
    if let (Some(kf), Some(vf)) = (KC::FIXED_LEN, VC::FIXED_LEN) {
        let entry = ENTRY_TAG_LEN + kf + vf;
        return map.storage_len() as u32
            * (outer_tag_len + varint_len(entry as u64) as u32 + entry);
    }
    let mut size = 0u32;
    for (k, v) in map.storage_iter() {
        let entry = entry_len::<KC, VC>(k, v);
        size += outer_tag_len + varint_len(entry as u64) as u32 + entry;
    }
    size
}

/// Write a scalar-valued map field: one `field_number`-tagged,
/// length-prefixed entry per element.
pub fn write_field<KC: MapCodec, VC: MapCodec, C>(map: &C, field_number: u32, buf: &mut impl BufMut)
where
    C: MapStorage<Key = KC::Value, Value = VC::Value>,
{
    for (k, v) in map.storage_iter() {
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
pub fn message_field_len<KC: MapCodec, M: Message, C>(
    map: &C,
    outer_tag_len: u32,
    cache: &mut SizeCache,
) -> u32
where
    C: MapStorage<Key = KC::Value, Value = M>,
{
    let mut size = 0u32;
    for (k, v) in map.storage_iter() {
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
pub fn write_message_field<KC: MapCodec, M: Message, C>(
    map: &C,
    field_number: u32,
    cache: &mut SizeCache,
    buf: &mut impl BufMut,
) where
    C: MapStorage<Key = KC::Value, Value = M>,
{
    for (k, v) in map.storage_iter() {
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

fn merge_entry_contents<KC, VC>(
    key: &mut KC::Value,
    val: &mut VC::Value,
    buf: &mut impl Buf,
    ctx: DecodeContext<'_>,
) -> Result<MapValueDecodeStatus, DecodeError>
where
    KC: MapValueDecode,
    VC: MapValueDecode,
{
    // Track the *value* field's status with last-wins semantics: only the
    // final field-2 occurrence decides whether the entry is unknown, matching
    // the protobuf reference implementation. Map keys are never closed enums,
    // so the key codec always returns `Known` and is ignored here.
    let mut val_status = MapValueDecodeStatus::Known;
    while buf.has_remaining() {
        let entry_tag = Tag::decode(buf)?;
        match entry_tag.field_number() {
            1 => {
                check_wire_type(entry_tag, KC::WIRE_TYPE)?;
                KC::merge(key, buf, ctx)?;
            }
            2 => {
                check_wire_type(entry_tag, VC::WIRE_TYPE)?;
                val_status = VC::merge(val, buf, ctx)?;
            }
            _ => {
                skip_field_depth(entry_tag, buf, ctx.depth())?;
            }
        }
    }
    Ok(val_status)
}

/// Decode one length-prefixed map entry from `buf`.
///
/// Closed-enum entries with unknown values are skipped. Use
/// [`merge_entry_with_unknowns`] when generated code has a parent
/// `UnknownFields` set available and needs to preserve those skipped entries.
pub fn merge_entry<KC, VC, C>(
    map: &mut C,
    buf: &mut impl Buf,
    ctx: DecodeContext<'_>,
) -> Result<(), DecodeError>
where
    KC: MapValueDecode,
    VC: MapValueDecode,
    C: MapStorage<Key = KC::Value, Value = VC::Value>,
{
    merge_entry_with_unknowns::<KC, VC, C>(map, buf, ctx, None)
}

/// Decode one length-prefixed map entry from `buf`, inserting it unless the
/// value is an unknown closed-enum number — in which case the whole entry is
/// optionally preserved as an unknown-field record.
///
/// Implements proto map-entry semantics: missing key/value fields take
/// their type defaults, repeated occurrences within one entry last-win,
/// and unknown entry fields are skipped.
///
/// If the *final* value-field occurrence in the entry is an unknown
/// closed-enum number, the entry is not inserted. When `unknown_fields` is
/// `Some((field_number, fields))`, the whole original map-entry payload is
/// preserved verbatim as a length-delimited unknown field with the outer map
/// field number.
///
/// # Errors
///
/// Returns a [`DecodeError`] on malformed lengths, payloads, or wire-type
/// mismatches inside the entry.
pub fn merge_entry_with_unknowns<KC, VC, C>(
    map: &mut C,
    buf: &mut impl Buf,
    ctx: DecodeContext<'_>,
    unknown_fields: Option<(u32, &mut crate::UnknownFields)>,
) -> Result<(), DecodeError>
where
    KC: MapValueDecode,
    VC: MapValueDecode,
    C: MapStorage<Key = KC::Value, Value = VC::Value>,
{
    let entry_len = decode_varint(buf)?;
    let entry_len = usize::try_from(entry_len).map_err(|_| DecodeError::MessageTooLarge)?;
    if buf.remaining() < entry_len {
        return Err(DecodeError::UnexpectedEof);
    }
    let mut key: KC::Value = Default::default();
    let mut val: VC::Value = Default::default();

    if unknown_fields.is_some() && (KC::MAY_RETURN_UNKNOWN || VC::MAY_RETURN_UNKNOWN) {
        // Fast path for contiguous bufs (the common `&[u8]` / `Bytes` case):
        // decode from a borrowed slice and only allocate when the entry
        // actually turns out to be unknown.
        if buf.chunk().len() >= entry_len {
            let preserved = {
                let entry_slice = &buf.chunk()[..entry_len];
                let mut entry_cur = entry_slice;
                let status =
                    merge_entry_contents::<KC, VC>(&mut key, &mut val, &mut entry_cur, ctx)?;
                matches!(status, MapValueDecodeStatus::Unknown).then(|| entry_slice.to_vec())
            };
            buf.advance(entry_len);
            match preserved {
                None => map.storage_insert(key, val),
                Some(payload) => {
                    if let Some((field_number, unknown_fields)) = unknown_fields {
                        ctx.register_unknown_field()?;
                        unknown_fields.push(crate::UnknownField {
                            number: field_number,
                            data: crate::UnknownFieldData::LengthDelimited(payload),
                        });
                    }
                }
            }
            return Ok(());
        }
        // Non-contiguous fallback: buffer the payload up front so it can be
        // preserved verbatim if decoding reports `Unknown`.
        let entry_payload = buf.copy_to_bytes(entry_len);
        let mut entry_cur = entry_payload.clone();
        let status = merge_entry_contents::<KC, VC>(&mut key, &mut val, &mut entry_cur, ctx)?;
        if matches!(status, MapValueDecodeStatus::Known) {
            map.storage_insert(key, val);
        } else if let Some((field_number, unknown_fields)) = unknown_fields {
            ctx.register_unknown_field()?;
            unknown_fields.push(crate::UnknownField {
                number: field_number,
                data: crate::UnknownFieldData::LengthDelimited(entry_payload.to_vec()),
            });
        }
        return Ok(());
    }

    let entry_limit = buf.remaining() - entry_len;
    let mut val_status = MapValueDecodeStatus::Known;
    while buf.remaining() > entry_limit {
        let entry_tag = Tag::decode(buf)?;
        match entry_tag.field_number() {
            1 => {
                check_wire_type(entry_tag, KC::WIRE_TYPE)?;
                KC::merge(&mut key, buf, ctx)?;
            }
            2 => {
                check_wire_type(entry_tag, VC::WIRE_TYPE)?;
                val_status = VC::merge(&mut val, buf, ctx)?;
            }
            _ => {
                skip_field_depth(entry_tag, buf, ctx.depth())?;
            }
        }
    }
    if buf.remaining() != entry_limit {
        let remaining = buf.remaining();
        if remaining > entry_limit {
            buf.advance(remaining - entry_limit);
        } else {
            return Err(DecodeError::UnexpectedEof);
        }
    }
    if matches!(val_status, MapValueDecodeStatus::Known) {
        map.storage_insert(key, val);
    }
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
    ) -> Vec<u8>
    where
        KC::Value: Eq + Hash,
    {
        let len = field_len::<KC, VC, _>(map, outer_tag_len);
        let mut buf = Vec::new();
        write_field::<KC, VC, _>(map, field_number, &mut buf);
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
            merge_entry::<KC, VC, _>(&mut map, &mut wire, ctx).unwrap();
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
    fn proto_string_map_codec_matches_str() {
        // `ProtoStringMap<String>` (the custom-string codec, monomorphized on the
        // built-in `String`) must produce byte-identical output to the canonical
        // `Str` codec and round-trip through it — proving the custom-string map
        // key/value path is wire-compatible with the default.
        let mut map: Map<String, i32> = Map::default();
        map.insert("a".into(), 1);
        map.insert("bee".into(), -7);

        let str_wire = encode_field::<Str, Int32>(&map, 5, 1);
        let custom_wire = encode_field::<ProtoStringMap<String>, Int32>(&map, 5, 1);
        // Map iteration order is unspecified, so compare via decode, not bytes.
        assert_eq!(
            decode_field::<ProtoStringMap<String>, Int32>(&str_wire),
            map
        );
        assert_eq!(decode_field::<Str, Int32>(&custom_wire), map);

        // As a value codec too: `map<int32, string>`.
        let mut vmap: Map<i32, String> = Map::default();
        vmap.insert(1, "x".into());
        let vwire = encode_field::<Int32, ProtoStringMap<String>>(&vmap, 3, 1);
        assert_eq!(decode_field::<Int32, ProtoStringMap<String>>(&vwire), vmap);
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
        merge_entry::<Str, Int32, _>(&mut map, &mut &wire[..], DecodeContext::new(10, &limit))
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
        merge_entry::<Str, Int32, _>(
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
        let err = merge_entry::<Str, Int32, _>(
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
            merge_entry::<Str, Int32, _>(&mut map, &mut &wire[..], DecodeContext::new(10, &limit))
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
        let len = message_field_len::<Int32, FlatMsg, _>(&map, 1, &mut cache);
        let mut wire = Vec::new();
        write_message_field::<Int32, FlatMsg, _>(&map, 4, &mut cache, &mut wire);
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

        // Closed codec drops the whole map entry instead of inserting the
        // default enum value.
        let back = decode_field::<Int32, ClosedEnum<E>>(&wire);
        assert!(!back.contains_key(&1));
    }

    #[test]
    fn closed_enum_unknown_preserves_whole_entry_when_requested() {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
        #[repr(i32)]
        enum E {
            #[default]
            A = 0,
            B = 1,
        }
        impl Enumeration for E {
            fn from_i32(value: i32) -> Option<Self> {
                match value {
                    0 => Some(E::A),
                    1 => Some(E::B),
                    _ => None,
                }
            }
            fn to_i32(&self) -> i32 {
                *self as i32
            }
            fn proto_name(&self) -> &'static str {
                match self {
                    E::A => "A",
                    E::B => "B",
                }
            }
        }

        let mut entry = Vec::new();
        Tag::new(1, WireType::Varint).encode(&mut entry);
        types::encode_int32(7, &mut entry);
        Tag::new(2, WireType::Varint).encode(&mut entry);
        types::encode_int32(99, &mut entry);

        let mut wire = Vec::new();
        encode_varint(entry.len() as u64, &mut wire);
        wire.extend_from_slice(&entry);

        let mut map: Map<i32, E> = Map::default();
        let mut unknown_fields = crate::UnknownFields::new();
        let limit = core::cell::Cell::new(crate::DEFAULT_UNKNOWN_FIELD_LIMIT);
        merge_entry_with_unknowns::<Int32, ClosedEnum<E>, _>(
            &mut map,
            &mut wire.as_slice(),
            DecodeContext::new(10, &limit),
            Some((5, &mut unknown_fields)),
        )
        .unwrap();

        assert!(map.is_empty());
        let unknowns: Vec<_> = unknown_fields.iter().collect();
        assert_eq!(unknowns.len(), 1);
        assert_eq!(unknowns[0].number, 5);
        assert!(matches!(
            &unknowns[0].data,
            crate::UnknownFieldData::LengthDelimited(payload) if payload == &entry
        ));

        // Non-contiguous Buf takes the buffering fallback and produces the
        // same outcome. Split *inside* the entry payload so that after the
        // length prefix is consumed, the first chunk is shorter than
        // `entry_len` and the contiguous fast path is bypassed.
        let mut map: Map<i32, E> = Map::default();
        let mut unknown_fields = crate::UnknownFields::new();
        let (a, b) = wire.split_at(3);
        let mut chained = bytes::Buf::chain(a, b);
        merge_entry_with_unknowns::<Int32, ClosedEnum<E>, _>(
            &mut map,
            &mut chained,
            DecodeContext::new(10, &limit),
            Some((5, &mut unknown_fields)),
        )
        .unwrap();
        assert!(map.is_empty());
        let unknowns: Vec<_> = unknown_fields.iter().collect();
        assert_eq!(unknowns.len(), 1);
        assert!(matches!(
            &unknowns[0].data,
            crate::UnknownFieldData::LengthDelimited(payload) if payload == &entry
        ));

        // Last-wins: an unknown value followed by a known one inserts the
        // known value and records nothing in unknown fields.
        let mut entry = Vec::new();
        Tag::new(1, WireType::Varint).encode(&mut entry);
        types::encode_int32(7, &mut entry);
        Tag::new(2, WireType::Varint).encode(&mut entry);
        types::encode_int32(99, &mut entry);
        Tag::new(2, WireType::Varint).encode(&mut entry);
        types::encode_int32(1, &mut entry);

        let mut wire = Vec::new();
        encode_varint(entry.len() as u64, &mut wire);
        wire.extend_from_slice(&entry);

        let mut map: Map<i32, E> = Map::default();
        let mut unknown_fields = crate::UnknownFields::new();
        merge_entry_with_unknowns::<Int32, ClosedEnum<E>, _>(
            &mut map,
            &mut wire.as_slice(),
            DecodeContext::new(10, &limit),
            Some((5, &mut unknown_fields)),
        )
        .unwrap();
        assert_eq!(map.get(&7), Some(&E::B));
        assert_eq!(unknown_fields.iter().count(), 0);
    }
}
