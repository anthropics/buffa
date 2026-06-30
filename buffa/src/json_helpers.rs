//! Proto3 JSON encoding helpers for use with serde.
//!
//! The protobuf JSON mapping (proto3 JSON) has several types whose wire
//! representation differs from their JSON representation:
//!
//! | Proto type          | JSON representation                          |
//! |---------------------|----------------------------------------------|
//! | `int64` / `sint64`  | Decimal string (e.g. `"9007199254740993"`)   |
//! | `uint64` / `fixed64`| Decimal string (e.g. `"18446744073709551615"`)|
//! | `float`             | Number, or `"NaN"` / `"Infinity"` / `"-Infinity"` |
//! | `double`            | Number, or `"NaN"` / `"Infinity"` / `"-Infinity"` |
//! | `bytes`             | Base64-encoded string (RFC 4648 standard)    |
//!
//! Each submodule provides `serialize` / `deserialize` functions compatible
//! with serde's `#[serde(with = "...")]` attribute.
//!
//! A [`skip_if`] submodule provides `skip_serializing_if` predicates for
//! omitting default-valued fields from JSON output.
//!
//! The [`wkt`] submodule provides the shared formatting and parsing
//! primitives for the well-known types' JSON forms (`Timestamp` RFC 3339,
//! `Duration` decimal seconds, `FieldMask` camelCase). Both `buffa-types`'s
//! typed serde impls and `buffa-descriptor`'s reflective JSON codec call
//! into it, so the two paths can't drift on edge cases the conformance
//! suite exercises. It's `#[doc(hidden)]` because the supported entry
//! points are the typed serde impls and `DynamicMessage`'s JSON codec —
//! these helpers operate on raw scalars and have no semver contract.

#[doc(hidden)]
pub mod wkt;

use alloc::string::ToString;

// ── Lenient base64 engines ──────────────────────────────────────────────────
//
// The pre-built `general_purpose` engines reject non-zero trailing bits.
// The protobuf JSON spec requires accepting any valid standard or URL-safe
// base64, so we configure lenient engines that tolerate trailing bits and
// optional padding.

use base64::alphabet;
use base64::engine::{
    general_purpose::STANDARD, DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig,
};

const LENIENT_CFG: GeneralPurposeConfig = GeneralPurposeConfig::new()
    .with_decode_allow_trailing_bits(true)
    .with_decode_padding_mode(DecodePaddingMode::Indifferent);

const STANDARD_LENIENT: GeneralPurpose = GeneralPurpose::new(&alphabet::STANDARD, LENIENT_CFG);
const URL_SAFE_LENIENT: GeneralPurpose = GeneralPurpose::new(&alphabet::URL_SAFE, LENIENT_CFG);

/// Maximum elements to pre-allocate from a deserializer's `size_hint`.
///
/// `size_hint` comes from untrusted input in the general case (any
/// `Deserializer<'de>`, not just `serde_json`). A hostile implementation
/// could return `Some(usize::MAX)`, causing `Vec::with_capacity` to abort.
/// Capping the hint bounds worst-case preallocation while still avoiding
/// reallocs for small-to-medium collections.
const MAX_PREALLOC_HINT: usize = 4096;

/// Clamp a deserializer size hint to [`MAX_PREALLOC_HINT`].
#[inline]
fn clamp_size_hint(hint: Option<usize>) -> usize {
    hint.unwrap_or(0).min(MAX_PREALLOC_HINT)
}

// ── ProtoElemJson: per-element proto3-JSON dispatch trait ───────────────────
//
// Proto3 JSON has per-type encoding rules (int64 → quoted string, float NaN
// → "NaN", bytes → base64). For singular/optional fields these are handled
// by the int64/float/bytes/etc. modules below. For CONTAINERS (repeated, map
// values) we need to apply the same per-element encoding, but serde's `with =`
// attribute doesn't compose. This trait provides the dispatch point.
//
// proto_seq and proto_map are the ONLY two container modules needed — they
// are generic over T: ProtoElemJson. Codegen emits ProtoElemJson impls for
// each generated message and enum type.
//
// Named `Json` (not `Serde`) because these are JSON-specific encoding rules.
// A future YAML encoder would have different per-element rules (YAML has native
// int64, no quoting needed) and would get its own trait.

/// Per-element proto3-JSON encoding.
///
/// Implemented in this crate for primitives (i64 → quoted, f64 → NaN/Inf
/// strings, `Vec<u8>` → base64, etc.). Codegen generates impls for message
/// and enum types. Used by [`proto_seq`] (repeated fields) and [`proto_map`]
/// (map values) to apply proto-JSON encoding to each element.
pub trait ProtoElemJson: Sized {
    /// Serialize this value with proto3 JSON semantics.
    fn serialize_proto_json<S: serde::Serializer>(v: &Self, s: S) -> Result<S::Ok, S::Error>;
    /// Deserialize a value with proto3 JSON semantics.
    fn deserialize_proto_json<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error>;
}

/// Wraps `&T: ProtoElemJson` as `serde::Serialize`.
///
/// Generated code uses this (and the sibling `*Json` adapters below) where
/// a value needs proto3-JSON encoding inside a hand-rolled `Serialize` impl
/// — oneof variant entries, view map/sequence elements — instead of
/// emitting a local newtype + impl at every site.
///
/// The adapters are generated-code plumbing, not a stable consumer-facing
/// surface (the `#[serde(with = ...)]` modules in this file are the
/// supported API); they are hidden from rustdoc accordingly.
#[doc(hidden)]
pub struct ProtoJson<'a, T>(pub &'a T);
impl<T: ProtoElemJson> serde::Serialize for ProtoJson<'_, T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        T::serialize_proto_json(self.0, s)
    }
}

/// Serialize a borrowed `bytes` payload as base64 (the proto3 JSON
/// encoding), for view types whose bytes fields are `&[u8]` (no
/// [`ProtoElemJson`] impl exists for the unsized slice).
#[doc(hidden)]
pub struct BytesJson<'a>(pub &'a [u8]);
impl serde::Serialize for BytesJson<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        bytes::serialize(self.0, s)
    }
}

/// Serialize a closed enum as its proto name string.
#[doc(hidden)]
pub struct ClosedEnumJson<'a, E>(pub &'a E);
impl<E: crate::Enumeration> serde::Serialize for ClosedEnumJson<'_, E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        closed_enum::serialize(self.0, s)
    }
}

/// Serialize a proto map key by stringification (proto3 JSON maps always
/// use string keys), mirroring [`proto_map`]'s internal `DisplayKey`.
#[doc(hidden)]
pub struct MapKeyJson<'a, T: ?Sized>(pub &'a T);
impl<T: ?Sized + core::fmt::Display> serde::Serialize for MapKeyJson<'_, T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self.0)
    }
}

/// Serialize a slice of proto-JSON-encoded elements (see [`proto_seq`]).
#[doc(hidden)]
pub struct RepeatedJson<'a, T>(pub &'a [T]);
impl<T: ProtoElemJson> serde::Serialize for RepeatedJson<'_, T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        proto_seq::serialize(self.0, s)
    }
}

/// Serialize a slice of borrowed `bytes` payloads, each base64-encoded.
#[doc(hidden)]
pub struct BytesSeqJson<'a>(pub &'a [&'a [u8]]);
impl serde::Serialize for BytesSeqJson<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(self.0.len()))?;
        for v in self.0 {
            seq.serialize_element(&BytesJson(v))?;
        }
        seq.end()
    }
}

/// Serialize a slice of open-enum values (see [`repeated_enum`]).
#[doc(hidden)]
pub struct EnumSeqJson<'a, E: crate::Enumeration>(pub &'a [crate::EnumValue<E>]);
impl<E: crate::Enumeration> serde::Serialize for EnumSeqJson<'_, E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        repeated_enum::serialize(self.0, s)
    }
}

/// Serialize a slice of closed-enum values as proto name strings.
#[doc(hidden)]
pub struct ClosedEnumSeqJson<'a, E>(pub &'a [E]);
impl<E: crate::Enumeration> serde::Serialize for ClosedEnumSeqJson<'_, E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        repeated_closed_enum::serialize(self.0, s)
    }
}

/// Bridge seed: deserializes a single T via `ProtoElemJson::deserialize_proto_json`,
/// but REJECTS `null`. Per proto3 JSON spec, `null` as an element of a repeated
/// field or as a map value is invalid (only the container itself may be `null`,
/// meaning empty). The singular helper modules accept null → default, which is
/// correct for singular fields but wrong for container elements.
struct ProtoElemSeed<T>(core::marker::PhantomData<T>);
impl<'de, T: ProtoElemJson> serde::de::DeserializeSeed<'de> for ProtoElemSeed<T> {
    type Value = T;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        // Peek-via-Option: serde_json calls visit_none for null. If the element
        // is null, that's an error; otherwise deserialize normally.
        struct NoNull<T>(core::marker::PhantomData<T>);
        impl<'de, T: ProtoElemJson> serde::de::Visitor<'de> for NoNull<T> {
            type Value = T;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a non-null value")
            }
            fn visit_none<E: serde::de::Error>(self) -> Result<T, E> {
                Err(E::custom(
                    "null is not a valid repeated-field element or map value",
                ))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<T, E> {
                Err(E::custom(
                    "null is not a valid repeated-field element or map value",
                ))
            }
            fn visit_some<D2: serde::Deserializer<'de>>(self, d: D2) -> Result<T, D2::Error> {
                T::deserialize_proto_json(d)
            }
        }
        d.deserialize_option(NoNull::<T>(core::marker::PhantomData))
    }
}

// ── Primitive impls ──────────────────────────────────────────────────────────

/// Delegate-to-existing-module macro: for types where a singular `with`
/// module already exists (int64, float, bytes, etc.), the ProtoElemJson
/// impl just forwards to that module's serialize/deserialize.
macro_rules! proto_elem_json_delegate {
    ($ty:ty, $mod:ident) => {
        impl ProtoElemJson for $ty {
            fn serialize_proto_json<S: serde::Serializer>(
                v: &Self,
                s: S,
            ) -> Result<S::Ok, S::Error> {
                $mod::serialize(v, s)
            }
            fn deserialize_proto_json<'de, D: serde::Deserializer<'de>>(
                d: D,
            ) -> Result<Self, D::Error> {
                $mod::deserialize(d)
            }
        }
    };
}

// ── proto_seq: generic repeated-field module ─────────────────────────────────

/// Serde with-module for `repeated T` fields where `T: ProtoElemJson`.
///
/// Each element is serialized/deserialized via [`ProtoElemJson`], applying
/// proto3 JSON semantics (quoted int64, NaN/Inf strings, base64 bytes, etc.).
/// JSON `null` deserializes to an empty vec.
///
/// Use with `#[serde(with = "::buffa::json_helpers::proto_seq")]`.
pub mod proto_seq {
    use super::{ProtoElemJson, ProtoElemSeed, ProtoJson};
    use alloc::vec::Vec;
    use serde::{Deserializer, Serializer};

    pub fn serialize<T: ProtoElemJson, S: Serializer>(v: &[T], s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for elem in v {
            seq.serialize_element(&ProtoJson(elem))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, T, C, D>(d: D) -> Result<C, D::Error>
    where
        T: ProtoElemJson,
        C: From<Vec<T>>,
        D: Deserializer<'de>,
    {
        struct V<T>(core::marker::PhantomData<T>);
        impl<'de, T: ProtoElemJson> serde::de::Visitor<'de> for V<T> {
            type Value = Vec<T>;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("an array or null")
            }
            fn visit_unit<E>(self) -> Result<Vec<T>, E> {
                Ok(Vec::new())
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Vec<T>, A::Error> {
                let mut out = Vec::with_capacity(super::clamp_size_hint(seq.size_hint()));
                while let Some(elem) =
                    seq.next_element_seed(ProtoElemSeed::<T>(core::marker::PhantomData))?
                {
                    out.push(elem);
                }
                Ok(out)
            }
        }
        d.deserialize_any(V::<T>(core::marker::PhantomData))
            .map(C::from)
    }
}

// ── proto_map: generic map module ────────────────────────────────────────────

/// Serde with-module for `map<K, V>` fields where `V: ProtoElemJson`.
///
/// Keys are stringified (via `Display`) on serialize and parsed (via `FromStr`)
/// on deserialize — proto3 JSON uses string keys for all maps. Values use
/// [`ProtoElemJson`] for proto-JSON encoding. JSON `null` deserializes to
/// an empty map.
///
/// Use with `#[serde(with = "::buffa::json_helpers::proto_map")]`.
pub mod proto_map {
    use super::{ProtoElemJson, ProtoElemSeed, ProtoJson};
    use crate::map_codec::MapStorage;
    use alloc::string::String;
    use core::fmt::Display;
    use core::str::FromStr;
    use serde::{Deserializer, Serializer};

    /// Wraps a `Display`-able key as a serde string without allocating.
    ///
    /// `collect_str` lets serde_json write the Display output directly to
    /// its internal buffer. For `K = String`, this is still one copy (into
    /// serde_json's buffer) but avoids the intermediate `String` allocation
    /// that `.to_string()` would create.
    struct DisplayKey<'a, K: ?Sized>(&'a K);
    impl<K: Display + ?Sized> serde::Serialize for DisplayKey<'_, K> {
        #[inline]
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.collect_str(self.0)
        }
    }

    pub fn serialize<C, S>(m: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: Display,
        C::Value: ProtoElemJson,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(m.storage_len()))?;
        for (k, v) in m.storage_iter() {
            map.serialize_entry(&DisplayKey(k), &ProtoJson(v))?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: FromStr,
        <C::Key as FromStr>::Err: Display,
        C::Value: ProtoElemJson,
        D: Deserializer<'de>,
    {
        struct Vis<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for Vis<C>
        where
            C: MapStorage + Default,
            C::Key: FromStr,
            <C::Key as FromStr>::Err: Display,
            C::Value: ProtoElemJson,
        {
            type Value = C;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a JSON object with string keys, or null")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(C::default())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(key) = map.next_key::<String>()? {
                    let k = <C::Key as FromStr>::from_str(&key).map_err(|e| {
                        serde::de::Error::custom(alloc::format!("invalid map key '{key}': {e}"))
                    })?;
                    let v =
                        map.next_value_seed(ProtoElemSeed::<C::Value>(core::marker::PhantomData))?;
                    out.storage_insert(k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_any(Vis::<C>(core::marker::PhantomData))
    }
}

/// Serde with-module for `map<string, V>` fields whose **value** needs
/// proto3-JSON encoding ([`ProtoElemJson`]: int64→quoted, float→NaN token,
/// bytes→base64) and whose **key** is a custom
/// [`ProtoString`](crate::types::ProtoString) type.
///
/// This is the string-key twin of [`proto_map`]. [`proto_map`] stringifies its
/// key via `Display` / `FromStr`, which a `ProtoString` newtype need not
/// implement; here the key is already a JSON string, so it is serialized and
/// deserialized through its own `serde` impls (a custom string type used in a
/// map is required to derive `Serialize` / `Deserialize`). The value path is
/// identical to [`proto_map`], and JSON `null` likewise deserializes to an empty
/// map.
pub mod proto_str_key_map {
    use super::{ProtoElemJson, ProtoElemSeed, ProtoJson};
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(m: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: serde::Serialize,
        C::Value: ProtoElemJson,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(m.storage_len()))?;
        for (k, v) in m.storage_iter() {
            map.serialize_entry(k, &ProtoJson(v))?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: serde::Deserialize<'de>,
        C::Value: ProtoElemJson,
        D: Deserializer<'de>,
    {
        struct Vis<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for Vis<C>
        where
            C: MapStorage + Default,
            C::Key: serde::Deserialize<'de>,
            C::Value: ProtoElemJson,
        {
            type Value = C;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a JSON object with string keys, or null")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(C::default())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(k) = map.next_key::<C::Key>()? {
                    let v =
                        map.next_value_seed(ProtoElemSeed::<C::Value>(core::marker::PhantomData))?;
                    out.storage_insert(k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_any(Vis::<C>(core::marker::PhantomData))
    }
}

// ── bool ─────────────────────────────────────────────────────────────────────

/// Serde with-module for `bool` fields that accepts JSON `null` as `false`.
///
/// Use with `#[serde(with = "::buffa::json_helpers::proto_bool")]`.
pub mod proto_bool {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &bool, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(*value)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = bool;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a boolean or null")
            }
            fn visit_unit<E>(self) -> Result<bool, E> {
                Ok(false)
            }
            fn visit_bool<E>(self, v: bool) -> Result<bool, E> {
                Ok(v)
            }
        }
        d.deserialize_any(V)
    }
}

// ── string ───────────────────────────────────────────────────────────────────

/// Serde with-module for `string` fields that accepts JSON `null` as `""`.
///
/// Use with `#[serde(with = "::buffa::json_helpers::proto_string")]`.
pub mod proto_string {
    use serde::{Deserializer, Serializer};

    /// Serialize a `string` field.
    ///
    /// Generic over `T: AsRef<str>` so configurable string types
    /// (`smol_str::SmolStr`, `ecow::EcoString`, ...) serialize without relying
    /// on `Deref<Target = str>` coercion at the `#[serde(with = ...)]` call
    /// site. `String` and `&str` both satisfy the bound.
    pub fn serialize<T: AsRef<str> + ?Sized, S: Serializer>(
        value: &T,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(value.as_ref())
    }

    /// Deserialize a `string` field (or JSON `null` → `""`).
    ///
    /// Generic over the return type so that codegen's `string_type` knob (which
    /// can map the field to `smol_str::SmolStr`, `ecow::EcoString`, etc.) works
    /// without a per-type shim. The visitor constructs the target type directly:
    /// `visit_str` goes through `From<&str>`, so a short string is inlined by an
    /// SSO type without first allocating an intermediate `String`. `String`
    /// itself satisfies both `From<&str>` and `From<String>`, keeping the
    /// default path zero-extra-cost. Type inference picks `T` from the field
    /// type at the serde call site.
    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: for<'a> From<&'a str> + From<alloc::string::String>,
        D: Deserializer<'de>,
    {
        struct V<T>(core::marker::PhantomData<T>);
        impl<'de, T> serde::de::Visitor<'de> for V<T>
        where
            T: for<'a> From<&'a str> + From<alloc::string::String>,
        {
            type Value = T;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a string or null")
            }
            fn visit_unit<E>(self) -> Result<T, E> {
                Ok(T::from(""))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<T, E> {
                Ok(T::from(v))
            }
            fn visit_string<E>(self, v: alloc::string::String) -> Result<T, E> {
                Ok(T::from(v))
            }
        }
        d.deserialize_any(V::<T>(core::marker::PhantomData))
    }
}

// ── enum ─────────────────────────────────────────────────────────────────────

/// Serde with-module for `EnumValue<E>` fields that accepts JSON `null` as the default.
///
/// Use with `#[serde(with = "::buffa::json_helpers::proto_enum")]`.
pub mod proto_enum {
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &crate::EnumValue<E>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(value, s)
    }

    pub fn deserialize<'de, E: crate::Enumeration, D: Deserializer<'de>>(
        d: D,
    ) -> Result<crate::EnumValue<E>, D::Error> {
        struct V<E>(core::marker::PhantomData<E>);
        impl<'de, E: crate::Enumeration> serde::de::Visitor<'de> for V<E> {
            type Value = crate::EnumValue<E>;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a protobuf enum name string, integer value, or null")
            }
            fn visit_unit<Err>(self) -> Result<crate::EnumValue<E>, Err> {
                Ok(crate::EnumValue::from(0))
            }
            fn visit_i64<Err: serde::de::Error>(self, v: i64) -> Result<crate::EnumValue<E>, Err> {
                let i = i32::try_from(v).map_err(|_| {
                    serde::de::Error::invalid_value(
                        serde::de::Unexpected::Signed(v),
                        &"an integer in i32 range",
                    )
                })?;
                Ok(crate::EnumValue::from(i))
            }
            fn visit_u64<Err: serde::de::Error>(self, v: u64) -> Result<crate::EnumValue<E>, Err> {
                let i = i32::try_from(v).map_err(|_| {
                    serde::de::Error::invalid_value(
                        serde::de::Unexpected::Unsigned(v),
                        &"an integer in i32 range",
                    )
                })?;
                Ok(crate::EnumValue::from(i))
            }
            fn visit_str<Err: serde::de::Error>(self, v: &str) -> Result<crate::EnumValue<E>, Err> {
                match E::from_proto_name(v) {
                    Some(e) => Ok(crate::EnumValue::from(e)),
                    None => {
                        if crate::json::ignore_unknown_enum_values() {
                            return Ok(crate::EnumValue::from(0));
                        }
                        Err(serde::de::Error::invalid_value(
                            serde::de::Unexpected::Str(v),
                            &"a known enum variant name",
                        ))
                    }
                }
            }
        }
        d.deserialize_any(V(core::marker::PhantomData))
    }
}

// ── lenient enum deserialization helpers ──────────────────────────────────────

/// Try to decode a `serde_json::Value` as a closed enum via [`Enumeration`].
///
/// When `ignore_unknown_enum_values` is active, returns `Ok(None)` for any
/// value that fails to decode, so the caller can drop the entry from its
/// container (or leave the optional unset). Strict mode propagates the
/// error. The lenient catch-all (any error → `None`, not just
/// unknown-variant) matches [`try_deserialize_enum`]'s behaviour for open
/// enums — if you are tightening this to only swallow unknown-variant
/// errors, tighten the open-enum path in the same change so the two stay
/// consistent.
///
/// Why not [`try_deserialize_enum::<E>`]? It routes through
/// `serde_json::from_value::<E>()`, which requires `E: DeserializeOwned` —
/// i.e. the enum itself must `impl Deserialize`. That impl is only emitted
/// by codegen when `generate_json = true`, so closed-enum fields whose enum
/// type lives in an externally-generated crate built *without* json (e.g.
/// `google.protobuf.FieldDescriptorProto.Type` from `buffa-descriptor`)
/// would refuse to compile. Decoding directly via the [`Enumeration`] trait
/// removes the impl requirement, and is exactly the dispatch the
/// codegen-emitted `Deserialize` impl performs anyway — `from_proto_name`
/// for strings, `from_i32` after range-check for integers, default for
/// `null` — so behaviour is unchanged for enums that *do* have one.
///
/// Unlike [`try_deserialize_enum`], lenient filtering works in both `std`
/// and `no_std` builds: there's no inner deserialize whose own lenient
/// handling could mask the unknown-value case, so no scoped strict-mode
/// override is needed. (Open-enum containers via [`try_deserialize_enum`]
/// still need the `std` thread-local override and so don't filter under
/// `no_std`.)
///
/// [`Enumeration`]: crate::Enumeration
#[inline]
fn try_deserialize_closed_enum<E: crate::Enumeration + Default>(
    raw: &serde_json::Value,
) -> Result<Option<E>, serde_json::Error> {
    let result = decode_closed_enum_strict::<E>(raw);
    match result {
        Ok(e) => Ok(Some(e)),
        Err(_) if crate::json::ignore_unknown_enum_values() => Ok(None),
        Err(e) => Err(e),
    }
}

/// Strict closed-enum decode of a buffered `serde_json::Value`, bound only
/// on [`Enumeration`]. Any failure — unknown variant, out-of-range integer,
/// wrong JSON type — is an error. [`try_deserialize_closed_enum`] applies
/// lenient filtering on top.
///
/// Mirrors the codegen-emitted `impl Deserialize for SomeEnum` (see
/// `buffa-codegen/src/enumeration.rs`):
///
/// | JSON | Codegen Visitor | This fn |
/// |---|---|---|
/// | `null` | `visit_unit` → default | default |
/// | `"NAME"` | `visit_str` → `from_proto_name` | `from_proto_name` |
/// | integer | `visit_i64`/`visit_u64` → range-check → `from_i32` | range-check → `from_i32` |
/// | float, bool, object, array | no Visitor method → serde type error | type error |
///
/// [`Enumeration`]: crate::Enumeration
fn decode_closed_enum_strict<E: crate::Enumeration + Default>(
    raw: &serde_json::Value,
) -> Result<E, serde_json::Error> {
    use serde::de::Error as _;
    use serde_json::Value;

    match raw {
        // Mirror the codegen-emitted `Deserialize` impl's `visit_unit`:
        // a bare `null` (e.g. an array element) decodes to the default
        // (zero-numbered) variant, not to "unknown".
        Value::Null => Ok(E::default()),
        Value::String(s) => {
            E::from_proto_name(s).ok_or_else(|| serde_json::Error::unknown_variant(s, &[]))
        }
        Value::Number(n) if n.is_f64() => Err(serde_json::Error::custom(alloc::format!(
            "expected integer or string for enum value, got float {n}"
        ))),
        Value::Number(n) => {
            // `as_i64()` / `as_u64()` are exclusive (a `Number` is stored
            // as exactly one of i64 / u64 / f64; the float case is handled
            // above). Try both before declaring it un-coerceable.
            let v32 = n
                .as_i64()
                .and_then(|v| i32::try_from(v).ok())
                .or_else(|| n.as_u64().and_then(|v| i32::try_from(v).ok()))
                .ok_or_else(|| {
                    serde_json::Error::custom(alloc::format!("enum value {n} out of i32 range"))
                })?;
            E::from_i32(v32).ok_or_else(|| {
                serde_json::Error::custom(alloc::format!("unknown enum value {v32}"))
            })
        }
        other => Err(serde_json::Error::custom(alloc::format!(
            "expected a protobuf enum name string, integer value, or null, got {other}"
        ))),
    }
}

/// Try to deserialize a `serde_json::Value` as `T` under strict enum parsing.
///
/// When `ignore_unknown_enum_values` is active, returns `Ok(None)` for
/// unknown values instead of propagating the error. This supports the
/// repeated-enum and map-enum filtering behaviour (skip unknown entries).
///
/// **`std` only**: filtering requires temporarily forcing strict mode to get
/// a distinguishable error for unknown values, which needs the scoped
/// thread-local. In `no_std` builds with global lenient enabled, singular
/// enum fields still get accept-with-default behaviour (via the unconditional
/// check in `open_enum_value::deserialize`), but repeated/map filtering
/// (removing unknown entries from the container) is unavailable — errors
/// propagate as in strict mode. Closed-enum containers are unaffected: they
/// use [`try_deserialize_closed_enum`], which doesn't have this limitation.
#[inline]
fn try_deserialize_enum<T: serde::de::DeserializeOwned>(
    raw: serde_json::Value,
) -> Result<Option<T>, serde_json::Error> {
    #[cfg(feature = "std")]
    {
        let ignore = crate::json::ignore_unknown_enum_values();
        let strict = crate::json::JsonParseOptions {
            ignore_unknown_enum_values: false,
            ..Default::default()
        };
        // Run the inner deserialize in STRICT mode so unknown enum values
        // produce a distinguishable error, then swallow that error if the
        // outer context wants lenient filtering.
        let result =
            crate::json::with_json_parse_options(&strict, || serde_json::from_value::<T>(raw));
        match result {
            Ok(v) => Ok(Some(v)),
            Err(_) if ignore => Ok(None),
            Err(e) => Err(e),
        }
    }
    #[cfg(not(feature = "std"))]
    {
        // no_std: no scoped override available. Errors propagate as-is.
        // (Global lenient mode only affects singular enum accept-with-default,
        //  not container filtering.)
        serde_json::from_value::<T>(raw).map(Some)
    }
}

// ── repeated_enum: Vec<EnumValue<E>> with unknown-value filtering ────────────

/// Serde with-module for `Vec<EnumValue<E>>` repeated enum fields.
///
/// When `ignore_unknown_enum_values` is active (std only), unknown enum
/// string values are silently skipped instead of producing an error.  In
/// default mode (or no_std builds) this behaves identically to the standard
/// `Vec<EnumValue<E>>` deserialization with null→empty-vec handling.
pub mod repeated_enum {
    use alloc::vec::Vec;
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &[crate::EnumValue<E>],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(value, s)
    }

    pub fn deserialize<'de, E: crate::Enumeration, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<crate::EnumValue<E>>, D::Error> {
        struct V<E>(core::marker::PhantomData<E>);
        impl<'de, E: crate::Enumeration> serde::de::Visitor<'de> for V<E> {
            type Value = Vec<crate::EnumValue<E>>;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a list of enum values or null")
            }

            fn visit_unit<Err>(self) -> Result<Vec<crate::EnumValue<E>>, Err> {
                Ok(Vec::new())
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Vec<crate::EnumValue<E>>, A::Error> {
                let mut out = Vec::with_capacity(super::clamp_size_hint(seq.size_hint()));
                while let Some(raw) = seq.next_element::<serde_json::Value>()? {
                    match super::try_deserialize_enum::<crate::EnumValue<E>>(raw) {
                        Ok(Some(v)) => out.push(v),
                        Ok(None) => continue,
                        Err(e) => return Err(serde::de::Error::custom(e)),
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_any(V(core::marker::PhantomData))
    }
}

// ── map_enum: HashMap<K, EnumValue<E>> with unknown-value filtering ─────────

/// Serde with-module for `HashMap<K, EnumValue<E>>` map fields where the
/// value is an enum type.
///
/// When `ignore_unknown_enum_values` is active (std only), map entries whose
/// value is an unknown enum string are silently dropped.  In default mode
/// (or no_std builds) this behaves identically to standard deserialization
/// with null→empty-map handling.
pub mod map_enum {
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(value: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: serde::Serialize,
        C::Value: serde::Serialize,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(value.storage_len()))?;
        for (k, v) in value.storage_iter() {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: serde::Deserialize<'de>,
        C::Value: serde::de::DeserializeOwned,
        D: Deserializer<'de>,
    {
        struct V<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for V<C>
        where
            C: MapStorage + Default,
            C::Key: serde::Deserialize<'de>,
            C::Value: serde::de::DeserializeOwned,
        {
            type Value = C;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a map with enum values or null")
            }

            fn visit_unit<Err>(self) -> Result<Self::Value, Err> {
                Ok(C::default())
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(key) = map.next_key::<C::Key>()? {
                    let raw = map.next_value::<serde_json::Value>()?;
                    match super::try_deserialize_enum::<C::Value>(raw) {
                        Ok(Some(v)) => {
                            out.storage_insert(key, v);
                        }
                        Ok(None) => continue,
                        Err(e) => return Err(serde::de::Error::custom(e)),
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_any(V::<C>(core::marker::PhantomData))
    }
}

// ── null_as_default: generic null→Default handler ────────────────────────────

/// Generic deserialize function that treats JSON `null` as `T::default()`.
///
/// Use with `#[serde(deserialize_with = "::buffa::json_helpers::null_as_default")]`
/// for repeated fields, map fields, and any other type where null should
/// silently produce the default value.
pub fn null_as_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(d).map(|opt| opt.unwrap_or_default())
}

// ── shared helper: parse string as integer, accepting floats/exponentials ────

/// Returns `true` if `f` has no fractional part (i.e. it is an exact integer).
///
/// Casts to `i128` and back — if the round-trip preserves the value, `f` has
/// no fractional part.  Deliberately not `f64::trunc()`-based: values outside
/// i128 range saturate, producing a mismatch that correctly returns `false`
/// (such values cannot fit any protobuf integer type anyway).
fn is_exact_integer(f: f64) -> bool {
    f.is_finite() && f == (f as i128 as f64)
}

/// Try to parse a string as an integer, handling float notation like `"1.0"`,
/// `"1e5"`, `"1.0e2"`.  Returns `None` if the value is not an exact integer.
fn parse_int_from_str<I: TryFrom<i128>>(v: &str) -> Option<I> {
    // First try direct integer parse.
    if let Ok(n) = v.parse::<i128>() {
        return I::try_from(n).ok();
    }
    let n = parse_exact_decimal_int(v)?;
    I::try_from(n).ok()
}

/// Parse a decimal/exponential string as an exact integer.
///
/// Accepts the numeric string forms the JSON integer helpers intentionally
/// support beyond plain integers: zero-fraction decimals like `"1.0"` and
/// decimal scientific notation like `"1e5"` / `"1.0e2"`. Returns `None` if
/// the value is not mathematically integral or would overflow `i128`.
fn parse_exact_decimal_int(v: &str) -> Option<i128> {
    let (mantissa, exp) = match v.split_once(['e', 'E']) {
        // `i64::from_str` handles the exponent's sign and rejects empty,
        // non-digit, and i64-overflowing exponents.
        Some((m, e)) => (m, e.parse::<i64>().ok()?),
        None => (v, 0),
    };
    let (negative, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
    };
    let (int_part, frac_part) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |(i, f)| (i, f));
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    // Trailing fractional zeros do not affect integrality and would only
    // inflate the significand before we re-scale it.
    let frac_part = frac_part.trim_end_matches('0');

    let mut significand = 0i128;
    for digit in int_part.bytes().chain(frac_part.bytes()) {
        if !digit.is_ascii_digit() {
            return None;
        }
        significand = significand
            .checked_mul(10)?
            .checked_add(i128::from(digit - b'0'))?;
    }
    if significand == 0 {
        // Zero is integral under any exponent; short-circuit before the
        // power-of-ten scaling below can overflow on e.g. "0e100".
        return Some(0);
    }

    // value = significand × 10^(exp − frac_len)
    let scale = exp.checked_sub(frac_part.len() as i64)?;
    if scale >= 0 {
        let pow10 = 10i128.checked_pow(u32::try_from(scale).ok()?)?;
        significand = significand.checked_mul(pow10)?;
    } else {
        let divisor = 10i128.checked_pow(u32::try_from(scale.unsigned_abs()).ok()?)?;
        if significand % divisor != 0 {
            return None;
        }
        significand /= divisor;
    }

    if negative {
        significand.checked_neg()
    } else {
        Some(significand)
    }
}

/// Try to interpret an f64 as an exact integer.
fn f64_to_int<I: TryFrom<i128>>(v: f64) -> Option<I> {
    if !is_exact_integer(v) {
        return None;
    }
    I::try_from(v as i128).ok()
}

// ── int32 / uint32 / int64 / uint64 ──────────────────────────────────────────
//
// These four serde with-modules share the same structural shape: a visitor
// that accepts `unit` (-> 0), `i64`, `u64`, `f64`, and `str` inputs.  Only
// the target type and serialization format differ (32-bit as JSON numbers,
// 64-bit as quoted decimal strings).

/// Generates a serde with-module for an integer type.
///
/// Parameters:
/// - `$mod_name` / `$int_type`: module name and Rust integer type.
/// - `serialize_body`: expression for the `serialize` function body, given
///   value identifier and serializer identifier.
/// - `expecting`: expecting string for the visitor.
/// - `doc`: doc string for the generated module.
/// - `visit_i64_body` / `visit_u64_body`: function pointers
///   `fn(value, &Expected) -> Result` for converting incoming visitor values.
/// - `visit_str_body`: function pointer for `visit_str`.
macro_rules! int_serde_module {
    (
        $mod_name:ident, $int_type:ty,
        doc = $doc:expr,
        serialize_body = |$val:ident, $ser:ident| $ser_body:expr,
        expecting = $expecting:expr,
        visit_i64_body = $visit_i64:expr,
        visit_u64_body = $visit_u64:expr,
        visit_str_body = $visit_str:expr $(,)?
    ) => {
        #[doc = $doc]
        pub mod $mod_name {
            use super::*;
            use serde::{Deserializer, Serializer};

            pub fn serialize<S: Serializer>(value: &$int_type, s: S) -> Result<S::Ok, S::Error> {
                let $val = value;
                let $ser = s;
                $ser_body
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<$int_type, D::Error> {
                struct Vis;
                impl<'de> serde::de::Visitor<'de> for Vis {
                    type Value = $int_type;
                    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                        f.write_str($expecting)
                    }
                    fn visit_unit<E>(self) -> Result<$int_type, E> {
                        Ok(0)
                    }
                    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<$int_type, E> {
                        let convert: fn(i64, &dyn serde::de::Expected) -> Result<$int_type, E> =
                            $visit_i64;
                        convert(v, &self)
                    }
                    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<$int_type, E> {
                        let convert: fn(u64, &dyn serde::de::Expected) -> Result<$int_type, E> =
                            $visit_u64;
                        convert(v, &self)
                    }
                    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<$int_type, E> {
                        f64_to_int::<$int_type>(v)
                            .ok_or_else(|| E::invalid_value(serde::de::Unexpected::Float(v), &self))
                    }
                    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<$int_type, E> {
                        let convert: fn(&str, &dyn serde::de::Expected) -> Result<$int_type, E> =
                            $visit_str;
                        convert(v, &self)
                    }
                }
                d.deserialize_any(Vis)
            }
        }
    };
}

/// Default `visit_str` implementation: delegates to `parse_int_from_str`.
fn default_visit_str<I: TryFrom<i128>, E: serde::de::Error>(
    v: &str,
    exp: &dyn serde::de::Expected,
) -> Result<I, E> {
    parse_int_from_str::<I>(v).ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(v), exp))
}

int_serde_module!(
    int32,
    i32,
    doc = "Serde with-module for `i32` fields.\n\n\
           Proto JSON accepts integers as numbers, quoted decimal strings, or\n\
           float notation (e.g. `1.0`, `1e5`).  Serializes as a JSON number.\n\n\
           Use with `#[serde(with = \"::buffa::json_helpers::int32\")]`.",
    serialize_body = |v, s| s.serialize_i32(*v),
    expecting = "an i32 as integer, string, float, or null",
    visit_i64_body = |v, exp| {
        i32::try_from(v)
            .map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), exp))
    },
    visit_u64_body = |v, exp| {
        i32::try_from(v)
            .map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), exp))
    },
    visit_str_body = default_visit_str,
);

int_serde_module!(
    uint32,
    u32,
    doc = "Serde with-module for `u32` fields.\n\n\
           Proto JSON accepts integers as numbers, quoted decimal strings, or\n\
           float notation (e.g. `1.0`, `1e5`).  Serializes as a JSON number.\n\n\
           Use with `#[serde(with = \"::buffa::json_helpers::uint32\")]`.",
    serialize_body = |v, s| s.serialize_u32(*v),
    expecting = "a u32 as integer, string, float, or null",
    visit_i64_body = |v, exp| {
        u32::try_from(v)
            .map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), exp))
    },
    visit_u64_body = |v, exp| {
        u32::try_from(v)
            .map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), exp))
    },
    visit_str_body = default_visit_str,
);

int_serde_module!(
    int64,
    i64,
    doc = "Serde with-module for `i64` fields encoded as a quoted decimal string.\n\n\
           Proto JSON also accepts unquoted integers and float notation.\n\n\
           Use with `#[serde(with = \"::buffa::json_helpers::int64\")]`.",
    serialize_body = |v, s| s.serialize_str(&v.to_string()),
    expecting = "an i64 as a quoted decimal string, integer, float, or null",
    visit_i64_body = |v: i64, _exp| Ok(v),
    visit_u64_body = |v, _exp| {
        i64::try_from(v).map_err(|_| {
            serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), &"an i64 value")
        })
    },
    visit_str_body = default_visit_str,
);

/// `visit_str` for `u64`: tries direct `parse::<u64>` first to handle large
/// values exactly, then falls back to `parse_int_from_str` for float notation.
fn u64_visit_str<E: serde::de::Error>(v: &str, exp: &dyn serde::de::Expected) -> Result<u64, E> {
    if let Ok(n) = v.parse::<u64>() {
        return Ok(n);
    }
    parse_int_from_str::<u64>(v).ok_or_else(|| E::invalid_value(serde::de::Unexpected::Str(v), exp))
}

int_serde_module!(
    uint64,
    u64,
    doc = "Serde with-module for `u64` fields encoded as a quoted decimal string.\n\n\
           Proto JSON also accepts unquoted integers and float notation.\n\n\
           Use with `#[serde(with = \"::buffa::json_helpers::uint64\")]`.",
    serialize_body = |v, s| s.serialize_str(&v.to_string()),
    expecting = "a u64 as a quoted decimal string, integer, float, or null",
    visit_i64_body = |v, _exp| {
        u64::try_from(v).map_err(|_| {
            serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), &"a u64 value")
        })
    },
    visit_u64_body = |v: u64, _exp| Ok(v),
    visit_str_body = u64_visit_str,
);

// ── float ────────────────────────────────────────────────────────────────────

/// Serde with-module for `float` fields.
///
/// Serializes finite values as JSON numbers. Serializes `NaN` as `"NaN"`,
/// positive infinity as `"Infinity"`, and negative infinity as `"-Infinity"`.
/// Accepts numbers and those three string literals on deserialization.
///
/// Use with `#[serde(with = "::buffa::json_helpers::float")]`.
pub mod float {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f32, s: S) -> Result<S::Ok, S::Error> {
        if value.is_nan() {
            s.serialize_str("NaN")
        } else if *value == f32::INFINITY {
            s.serialize_str("Infinity")
        } else if *value == f32::NEG_INFINITY {
            s.serialize_str("-Infinity")
        } else {
            s.serialize_f32(*value)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = f32;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(r#"a float, "NaN", "Infinity", "-Infinity", or null"#)
            }

            fn visit_unit<E>(self) -> Result<f32, E> {
                Ok(0.0)
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<f32, E> {
                match v {
                    "NaN" => Ok(f32::NAN),
                    "Infinity" => Ok(f32::INFINITY),
                    "-Infinity" => Ok(f32::NEG_INFINITY),
                    _ => v.parse::<f32>().map_err(|_| {
                        E::invalid_value(
                            serde::de::Unexpected::Str(v),
                            &r#"a float, or "NaN", "Infinity", "-Infinity""#,
                        )
                    }),
                }
            }

            fn visit_f32<E: serde::de::Error>(self, v: f32) -> Result<f32, E> {
                Ok(v)
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<f32, E> {
                // Reject finite values that overflow f32 range.
                let f = v as f32;
                if v.is_finite() && !f.is_finite() {
                    return Err(E::invalid_value(
                        serde::de::Unexpected::Float(v),
                        &"a finite f32 value",
                    ));
                }
                Ok(f)
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<f32, E> {
                Ok(v as f32)
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<f32, E> {
                Ok(v as f32)
            }
        }
        d.deserialize_any(V)
    }
}

// ── double ───────────────────────────────────────────────────────────────────

/// Serde with-module for `double` fields.
///
/// Serializes finite values as JSON numbers. Serializes `NaN` as `"NaN"`,
/// positive infinity as `"Infinity"`, and negative infinity as `"-Infinity"`.
/// Accepts numbers and those three string literals on deserialization.
///
/// Use with `#[serde(with = "::buffa::json_helpers::double")]`.
pub mod double {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f64, s: S) -> Result<S::Ok, S::Error> {
        if value.is_nan() {
            s.serialize_str("NaN")
        } else if *value == f64::INFINITY {
            s.serialize_str("Infinity")
        } else if *value == f64::NEG_INFINITY {
            s.serialize_str("-Infinity")
        } else {
            s.serialize_f64(*value)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = f64;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(r#"a double, "NaN", "Infinity", "-Infinity", or null"#)
            }

            fn visit_unit<E>(self) -> Result<f64, E> {
                Ok(0.0)
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<f64, E> {
                match v {
                    "NaN" => Ok(f64::NAN),
                    "Infinity" => Ok(f64::INFINITY),
                    "-Infinity" => Ok(f64::NEG_INFINITY),
                    _ => v.parse::<f64>().map_err(|_| {
                        E::invalid_value(
                            serde::de::Unexpected::Str(v),
                            &r#"a double, or "NaN", "Infinity", "-Infinity""#,
                        )
                    }),
                }
            }

            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<f64, E> {
                Ok(v)
            }

            fn visit_f32<E: serde::de::Error>(self, v: f32) -> Result<f64, E> {
                Ok(v as f64)
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<f64, E> {
                Ok(v as f64)
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<f64, E> {
                Ok(v as f64)
            }
        }
        d.deserialize_any(V)
    }
}

// ── bytes ─────────────────────────────────────────────────────────────────────

/// Serde with-module for `bytes` fields encoded as standard base64 (RFC 4648).
///
/// Serializes as standard padded base64. Accepts standard base64, URL-safe
/// base64, and both padded and unpadded variants on deserialization, as
/// required by the [proto3 JSON spec].
///
/// [proto3 JSON spec]: https://protobuf.dev/programming-guides/proto3/#json
///
/// Use with `#[serde(with = "::buffa::json_helpers::bytes")]`.
pub mod bytes {
    use super::STANDARD;
    use base64::Engine as _;
    use serde::{Deserializer, Serializer};

    /// Serialize as a base64-encoded string.
    ///
    /// Takes `&[u8]` so both `Vec<u8>` and `bytes::Bytes` fields are
    /// accepted via deref coercion at the `#[serde(with = ...)]` call site.
    pub fn serialize<S: Serializer>(value: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD.encode(value))
    }

    /// Deserialize from a base64-encoded string (or JSON `null` → empty).
    ///
    /// Generic over the return type so that codegen's `use_bytes_type()`
    /// (which types the field as `bytes::Bytes`) works without a shim:
    /// the visitor produces `Vec<u8>`, the final `.into()` converts.
    /// `Vec<u8>: From<Vec<u8>>` (identity) keeps the default path zero-cost.
    /// Type inference picks `T` from the field type at the serde call site.
    pub fn deserialize<'de, T, D>(d: D) -> Result<T, D::Error>
    where
        T: From<alloc::vec::Vec<u8>>,
        D: Deserializer<'de>,
    {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = alloc::vec::Vec<u8>;

            fn visit_unit<E>(self) -> Result<alloc::vec::Vec<u8>, E> {
                Ok(alloc::vec::Vec::new())
            }

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a base64-encoded string, or null")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<alloc::vec::Vec<u8>, E> {
                super::decode_base64(v).map_err(serde::de::Error::custom)
            }
        }
        // Use deserialize_any (not deserialize_str) so null reaches visit_unit.
        d.deserialize_any(V).map(T::from)
    }
}

// ── ProtoElemJson primitive impls ────────────────────────────────────────────
//
// Delegates to the singular `with` modules above. This is the complete set
// of proto scalar types that can appear as repeated elements or map values.

proto_elem_json_delegate!(i32, int32);
proto_elem_json_delegate!(u32, uint32);
proto_elem_json_delegate!(i64, int64);
proto_elem_json_delegate!(u64, uint64);
proto_elem_json_delegate!(f32, float);
proto_elem_json_delegate!(f64, double);
proto_elem_json_delegate!(bool, proto_bool);
proto_elem_json_delegate!(alloc::string::String, proto_string);
proto_elem_json_delegate!(alloc::vec::Vec<u8>, bytes);

// Only a custom `bytes` element used in a `repeated` / `map` field gets a
// codegen-emitted `ProtoElemJson` impl (forwarding to the `bytes` base64
// with-module) into the generating crate, where the type is local. A custom
// `string` element does NOT: `repeated_serde_module` returns `None` for
// `TYPE_STRING`, so a repeated string serializes through the element's own
// native `Serialize`/`Deserialize` rather than `proto_seq`/`ProtoElemJson`.

// bytes::Bytes — for codegen's `use_bytes_type()` with `repeated bytes`.
// Serialize: `Bytes: Deref<Target=[u8]>` → `bytes::serialize(&[u8], s)`.
// Deserialize: `bytes::deserialize` is generic over `T: From<Vec<u8>>`;
// `Bytes::from(Vec<u8>)` takes ownership of the buffer (zero-copy).
//
// Not using the macro because `bytes::serialize(v, s)` needs `v: &[u8]`
// but `ProtoElemJson::serialize_proto_json` gets `v: &Self = &Bytes`.
// Deref coercion handles it, but being explicit avoids confusion.
impl ProtoElemJson for ::bytes::Bytes {
    fn serialize_proto_json<S: serde::Serializer>(v: &Self, s: S) -> Result<S::Ok, S::Error> {
        bytes::serialize(v, s)
    }
    fn deserialize_proto_json<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        bytes::deserialize(d)
    }
}

// EnumValue<E> (open enums) — uses its own Serialize/Deserialize (which
// handle proto JSON enum semantics: name string on serialize, accept name
// or integer on deserialize, preserve unknown values as the int).
impl<E: crate::Enumeration> ProtoElemJson for crate::EnumValue<E>
where
    Self: serde::Serialize + serde::de::DeserializeOwned,
{
    fn serialize_proto_json<S: serde::Serializer>(v: &Self, s: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(v, s)
    }
    fn deserialize_proto_json<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        <Self as serde::Deserialize>::deserialize(d)
    }
}

/// Serde with-module for `map<string, ...>` fields where the key's
/// `utf8_validation = NONE` produced `Vec<u8>` keys under strict mapping.
///
/// Keys are serialized/deserialized as base64 strings (same as bytes values).
/// Values use their own serde impl — this is generic over `V`.
///
/// Use with `#[serde(with = "::buffa::json_helpers::bytes_key_map")]`.
pub mod bytes_key_map {
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(value: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage<Key = alloc::vec::Vec<u8>>,
        C::Value: serde::Serialize,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(value.storage_len()))?;
        for (k, v) in value.storage_iter() {
            map.serialize_entry(&super::Base64Wrapper(k), v)?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage<Key = alloc::vec::Vec<u8>> + Default,
        C::Value: serde::Deserialize<'de>,
        D: Deserializer<'de>,
    {
        struct Vis<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for Vis<C>
        where
            C: MapStorage<Key = alloc::vec::Vec<u8>> + Default,
            C::Value: serde::Deserialize<'de>,
        {
            type Value = C;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a map with base64-encoded keys, or null")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(C::default())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(key_str) = map.next_key::<alloc::string::String>()? {
                    let k = super::decode_base64(&key_str).map_err(serde::de::Error::custom)?;
                    let v: C::Value = map.next_value()?;
                    out.storage_insert(k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_any(Vis::<C>(core::marker::PhantomData))
    }
}

/// Serde with-module for `HashMap<Vec<u8>, Vec<u8>>` — both key and value
/// base64-encoded. This covers `map<string, string>` with `utf8_validation
/// = NONE` under strict mapping (both key and value normalized to bytes),
/// and `map<string, bytes>` with NONE on the key.
///
/// Use with `#[serde(with = "::buffa::json_helpers::bytes_key_bytes_val_map")]`.
pub mod bytes_key_bytes_val_map {
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(value: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage<Key = alloc::vec::Vec<u8>, Value = alloc::vec::Vec<u8>>,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(value.storage_len()))?;
        for (k, v) in value.storage_iter() {
            map.serialize_entry(&super::Base64Wrapper(k), &super::Base64Wrapper(v))?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage<Key = alloc::vec::Vec<u8>, Value = alloc::vec::Vec<u8>> + Default,
        D: Deserializer<'de>,
    {
        struct V<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for V<C>
        where
            C: MapStorage<Key = alloc::vec::Vec<u8>, Value = alloc::vec::Vec<u8>> + Default,
        {
            type Value = C;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a map with base64-encoded keys and values, or null")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(C::default())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some((ks, vs)) =
                    map.next_entry::<alloc::string::String, alloc::string::String>()?
                {
                    let k = super::decode_base64(&ks).map_err(serde::de::Error::custom)?;
                    let v = super::decode_base64(&vs).map_err(serde::de::Error::custom)?;
                    out.storage_insert(k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_any(V::<C>(core::marker::PhantomData))
    }
}

// ── string_key_map: non-string keys stringified, any MapStorage container ────

/// Serde with-module for map fields with non-string key types (int32, int64,
/// uint32, uint64, bool, etc.).
///
/// In proto3 JSON, all map keys are strings.  For non-string key types, the
/// key string is parsed using `FromStr` during deserialization and converted
/// to string using `ToString` during serialization.
///
/// Use with `#[serde(with = "::buffa::json_helpers::string_key_map")]`.
pub mod string_key_map {
    use crate::map_codec::MapStorage;
    use alloc::string::ToString;
    use core::fmt::Display;
    use core::str::FromStr;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(value: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: Display,
        C::Value: serde::Serialize,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(value.storage_len()))?;
        for (k, v) in value.storage_iter() {
            map.serialize_entry(&k.to_string(), v)?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: FromStr,
        <C::Key as FromStr>::Err: Display,
        C::Value: serde::Deserialize<'de>,
        D: Deserializer<'de>,
    {
        struct MapVisitor<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for MapVisitor<C>
        where
            C: MapStorage + Default,
            C::Key: FromStr,
            <C::Key as FromStr>::Err: Display,
            C::Value: serde::Deserialize<'de>,
        {
            type Value = C;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a JSON object with string keys, or null")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(C::default())
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(key_str) = map.next_key::<alloc::string::String>()? {
                    let key = key_str.parse::<C::Key>().map_err(|e| {
                        serde::de::Error::custom(alloc::format!(
                            "invalid map key '{}': {}",
                            key_str,
                            e
                        ))
                    })?;
                    let value = map.next_value()?;
                    out.storage_insert(key, value);
                }
                Ok(out)
            }
        }
        d.deserialize_any(MapVisitor::<C>(core::marker::PhantomData))
    }
}

/// Newtype for serializing `&[u8]` as base64 without a separate `with` module.
struct Base64Wrapper<'a>(&'a [u8]);

impl serde::Serialize for Base64Wrapper<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        bytes::serialize(self.0, s)
    }
}

/// Decode a base64 string, accepting standard and URL-safe alphabets with
/// lenient trailing-bit and padding handling.
fn decode_base64(v: &str) -> Result<alloc::vec::Vec<u8>, base64::DecodeError> {
    use base64::Engine as _;
    STANDARD_LENIENT
        .decode(v)
        .or_else(|std_err| URL_SAFE_LENIENT.decode(v).map_err(|_| std_err))
}

// ── Option<T> wrappers for proto2 optional scalar fields ─────────────────────
//
// serde's `#[serde(with = "...")]` on an `Option<T>` field needs the module
// to handle `&Option<T>` / `Option<T>`.  These thin wrappers delegate to the
// inner module for `Some` and pass through `None` transparently.

macro_rules! opt_serde_module {
    ($mod_name:ident, $inner:ident, $ty:ty) => {
        pub mod $mod_name {
            use serde::{Deserializer, Serializer};

            pub fn serialize<S: Serializer>(value: &Option<$ty>, s: S) -> Result<S::Ok, S::Error> {
                match value {
                    Some(v) => super::$inner::serialize(v, s),
                    None => s.serialize_none(),
                }
            }

            pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<$ty>, D::Error> {
                // Use Option<T> deserialization so null → None (unset),
                // not Some(default).  For non-null values, delegate to the
                // inner module's deserializer.
                <Option<super::$mod_name::Helper> as serde::Deserialize>::deserialize(d)
                    .map(|opt| opt.map(|h| h.0))
            }

            /// Newtype that deserializes via the inner module.
            struct Helper($ty);

            impl<'de> serde::Deserialize<'de> for Helper {
                fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                    super::$inner::deserialize(d).map(Helper)
                }
            }
        }
    };
}

opt_serde_module!(opt_int32, int32, i32);
opt_serde_module!(opt_uint32, uint32, u32);
opt_serde_module!(opt_int64, int64, i64);
opt_serde_module!(opt_uint64, uint64, u64);
opt_serde_module!(opt_float, float, f32);
opt_serde_module!(opt_double, double, f64);

/// Option wrapper for bytes fields (base64 encoding).
///
/// Generic over the inner type so both `Option<Vec<u8>>` (default) and
/// `Option<bytes::Bytes>` (codegen's `use_bytes_type()`) work. The
/// `AsRef<[u8]>` bound on serialize and `From<Vec<u8>>` on deserialize are
/// satisfied by both. `bytes::serialize` takes `&[u8]`, so we can't use
/// the `opt_*` macro — `&Option<T>` → `&[u8]` needs explicit unwrap.
pub mod opt_bytes {
    use serde::{Deserializer, Serializer};

    pub fn serialize<T, S>(value: &Option<T>, s: S) -> Result<S::Ok, S::Error>
    where
        T: AsRef<[u8]>,
        S: Serializer,
    {
        match value {
            Some(v) => super::bytes::serialize(v.as_ref(), s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
    where
        T: From<alloc::vec::Vec<u8>>,
        D: Deserializer<'de>,
    {
        // null → None (unset), non-null → delegate to bytes module.
        // The visitor stays Vec<u8>-typed; convert on the way out.
        struct Helper(alloc::vec::Vec<u8>);
        impl<'de> serde::Deserialize<'de> for Helper {
            fn deserialize<D2: Deserializer<'de>>(d: D2) -> Result<Self, D2::Error> {
                super::bytes::deserialize::<alloc::vec::Vec<u8>, _>(d).map(Helper)
            }
        }
        <Option<Helper> as serde::Deserialize>::deserialize(d).map(|opt| opt.map(|h| h.0.into()))
    }
}

// ── opt_enum: Option<EnumValue<E>> with unknown-value → None ─────────────────

/// Serde with-module for `Option<EnumValue<E>>` optional enum fields (proto2).
///
/// When `ignore_unknown_enum_values` is active (std only), unknown enum
/// string values produce `None` (field not set) instead of `Some(default)`.
/// In default mode (or no_std builds) unknown strings produce an error.
pub mod opt_enum {
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &Option<crate::EnumValue<E>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(v) => serde::Serialize::serialize(v, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, E: crate::Enumeration, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<crate::EnumValue<E>>, D::Error> {
        // First, deserialize the raw value. null → None immediately.
        let raw: Option<serde_json::Value> = serde::Deserialize::deserialize(d)?;
        let raw = match raw {
            Some(v) => v,
            None => return Ok(None),
        };

        super::try_deserialize_enum::<crate::EnumValue<E>>(raw).map_err(serde::de::Error::custom)
    }
}

// ── NullableDeserializeSeed ───────────────────────────────────────────────────

/// `DeserializeSeed` that detects JSON null and returns `None` instead of
/// delegating to the inner seed. Used by generated oneof deserializers so
/// that `null` means "this variant is not set".
pub struct NullableDeserializeSeed<S>(pub S);

impl<'de, S> serde::de::DeserializeSeed<'de> for NullableDeserializeSeed<S>
where
    S: serde::de::DeserializeSeed<'de>,
{
    type Value = Option<S::Value>;

    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_option(NullableVisitor(self.0))
    }
}

struct NullableVisitor<S>(S);

impl<'de, S> serde::de::Visitor<'de> for NullableVisitor<S>
where
    S: serde::de::DeserializeSeed<'de>,
{
    type Value = Option<S::Value>;

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a value or null")
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        self.0.deserialize(d).map(Some)
    }
}

// ── DefaultDeserializeSeed ───────────────────────────────────────────────────

/// `DeserializeSeed` that delegates to the type's standard `Deserialize` impl.
/// Pairs with `NullableDeserializeSeed` for oneof variants that don't
/// need a custom serde helper.
pub struct DefaultDeserializeSeed<T>(core::marker::PhantomData<T>);

impl<T> DefaultDeserializeSeed<T> {
    pub fn new() -> Self {
        Self(core::marker::PhantomData)
    }
}

impl<T> Default for DefaultDeserializeSeed<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::de::DeserializeSeed<'de>
    for DefaultDeserializeSeed<T>
{
    type Value = T;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
        T::deserialize(d)
    }
}

// ── message_field_always_present ──────────────────────────────────────────────

/// Deserialize a `MessageField<T>` by always forwarding to `T::deserialize`,
/// including for JSON `null`.
///
/// Normally, `MessageField<T>::deserialize` delegates to `Option<T>`, which
/// maps `null` → `None` (field absent). For types like `google.protobuf.Value`
/// where `null` is a valid value (`NullValue`), this function ensures `null`
/// reaches `T::deserialize` and the field is set rather than absent.
pub fn message_field_always_present<'de, T, P, D>(
    d: D,
) -> Result<crate::MessageField<T, P>, D::Error>
where
    T: Default + serde::Deserialize<'de>,
    P: crate::ProtoBox<T>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(d).map(crate::MessageField::some)
}

// ── EnumProtoNameRef ──────────────────────────────────────────────────────────

/// Helper to serialize a `&E` by its proto name string.
///
/// Used by `repeated_closed_enum` and `map_closed_enum` serialize functions
/// where elements need proto JSON encoding (name string, not integer).
pub(crate) struct EnumProtoNameRef<'a, E: crate::Enumeration>(pub &'a E);

impl<E: crate::Enumeration> serde::Serialize for EnumProtoNameRef<'_, E> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0.proto_name())
    }
}

// ── skip_if ───────────────────────────────────────────────────────────────────

/// Predicates for `#[serde(skip_serializing_if = "...")]` on proto3 fields.
///
/// In proto3 JSON, fields whose value equals the type default are omitted.
/// Attach these to the appropriate field types in generated `#[derive(Serialize)]`
/// structs to match that behaviour.
pub mod skip_if {
    pub fn is_zero_i32(v: &i32) -> bool {
        *v == 0
    }
    pub fn is_zero_i64(v: &i64) -> bool {
        *v == 0
    }
    pub fn is_zero_u32(v: &u32) -> bool {
        *v == 0
    }
    pub fn is_zero_u64(v: &u64) -> bool {
        *v == 0
    }
    pub fn is_false(v: &bool) -> bool {
        !*v
    }
    /// Treats `-0.0` as zero (IEEE 754: `-0.0 == 0.0`), so negative zero
    /// is omitted from JSON output. Correct for proto3 JSON but a
    /// round-trip through JSON will not preserve `-0.0`.
    pub fn is_zero_f32(v: &f32) -> bool {
        *v == 0.0
    }
    /// See [`is_zero_f32`] — same `-0.0` behavior applies.
    pub fn is_zero_f64(v: &f64) -> bool {
        *v == 0.0
    }
    pub fn is_empty_str(v: &str) -> bool {
        v.is_empty()
    }
    pub fn is_empty_bytes(v: &[u8]) -> bool {
        v.is_empty()
    }
    pub fn is_empty_vec<T>(v: &[T]) -> bool {
        v.is_empty()
    }
    /// Empty-check for a non-default map collection (`BTreeMap` or a custom
    /// map) via the [`MapStorage`](crate::map_codec::MapStorage) surface, so the
    /// `skip_serializing_if` predicate works regardless of the concrete map
    /// type. The default `HashMap` map fields keep `HashMap::is_empty`, so their
    /// generated output is unchanged.
    pub fn is_empty_map<C>(m: &C) -> bool
    where
        C: crate::map_codec::MapStorage,
    {
        m.storage_len() == 0
    }
    pub fn is_unset_message_field<T: Default, P: crate::ProtoBox<T>>(
        v: &crate::MessageField<T, P>,
    ) -> bool {
        v.is_unset()
    }
    pub fn is_default_enum_value<E: crate::Enumeration>(v: &crate::EnumValue<E>) -> bool {
        v.to_i32() == 0
    }
    pub fn is_default_closed_enum<E: crate::Enumeration>(v: &E) -> bool {
        v.to_i32() == 0
    }
}

// ── closed_enum: bare E with proto JSON handling ─────────────────────────────

/// Serde with-module for singular closed enum fields (bare `E`).
///
/// Accepts JSON `null` as `E::default()`, string via `from_proto_name`,
/// and integer via `from_i32`. When `ignore_unknown_enum_values` is active,
/// unknown string values produce `E::default()` instead of an error.
pub mod closed_enum {
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &E,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        s.serialize_str(value.proto_name())
    }

    pub fn deserialize<'de, E: crate::Enumeration + Default, D: Deserializer<'de>>(
        d: D,
    ) -> Result<E, D::Error> {
        struct V<E>(core::marker::PhantomData<E>);
        impl<'de, E: crate::Enumeration + Default> serde::de::Visitor<'de> for V<E> {
            type Value = E;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a protobuf enum name string, integer value, or null")
            }
            fn visit_unit<Err>(self) -> Result<E, Err> {
                Ok(E::default())
            }
            fn visit_i64<Err: serde::de::Error>(self, v: i64) -> Result<E, Err> {
                let v32 = i32::try_from(v).map_err(|_| {
                    serde::de::Error::custom(alloc::format!("enum value {} out of i32 range", v))
                })?;
                E::from_i32(v32).ok_or_else(|| {
                    serde::de::Error::custom(alloc::format!("unknown enum value {}", v32))
                })
            }
            fn visit_u64<Err: serde::de::Error>(self, v: u64) -> Result<E, Err> {
                let v32 = i32::try_from(v).map_err(|_| {
                    serde::de::Error::custom(alloc::format!("enum value {} out of i32 range", v))
                })?;
                E::from_i32(v32).ok_or_else(|| {
                    serde::de::Error::custom(alloc::format!("unknown enum value {}", v32))
                })
            }
            fn visit_str<Err: serde::de::Error>(self, v: &str) -> Result<E, Err> {
                match E::from_proto_name(v) {
                    Some(e) => Ok(e),
                    None => {
                        if crate::json::ignore_unknown_enum_values() {
                            return Ok(E::default());
                        }
                        Err(serde::de::Error::unknown_variant(v, &[]))
                    }
                }
            }
        }
        d.deserialize_any(V(core::marker::PhantomData))
    }
}

// ── opt_closed_enum: Option<E> with unknown-value handling ───────────────────

/// Serde with-module for `Option<E>` optional closed enum fields (proto2).
///
/// When `ignore_unknown_enum_values` is active, unknown enum
/// string values produce `None` (field not set) instead of an error.
pub mod opt_closed_enum {
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &Option<E>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(v) => s.serialize_str(v.proto_name()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, E: crate::Enumeration + Default, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<E>, D::Error> {
        let raw: Option<serde_json::Value> = serde::Deserialize::deserialize(d)?;
        let raw = match raw {
            Some(v) => v,
            None => return Ok(None),
        };

        super::try_deserialize_closed_enum::<E>(&raw).map_err(serde::de::Error::custom)
    }
}

// ── repeated_closed_enum: Vec<E> with unknown-value filtering ────────────────

/// Serde with-module for `Vec<E>` repeated closed enum fields.
///
/// When `ignore_unknown_enum_values` is active, unknown enum
/// string values are silently skipped.
pub mod repeated_closed_enum {
    use alloc::vec::Vec;
    use serde::{Deserializer, Serializer};

    pub fn serialize<E: crate::Enumeration, S: Serializer>(
        value: &[E],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = s.serialize_seq(Some(value.len()))?;
        for v in value {
            seq.serialize_element(&crate::json_helpers::EnumProtoNameRef(v))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, E: crate::Enumeration + Default, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<E>, D::Error> {
        struct V<E>(core::marker::PhantomData<E>);
        impl<'de, E: crate::Enumeration + Default> serde::de::Visitor<'de> for V<E> {
            type Value = Vec<E>;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a list of enum values or null")
            }

            fn visit_unit<Err>(self) -> Result<Vec<E>, Err> {
                Ok(Vec::new())
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Vec<E>, A::Error> {
                let mut out = Vec::with_capacity(super::clamp_size_hint(seq.size_hint()));
                while let Some(raw) = seq.next_element::<serde_json::Value>()? {
                    match super::try_deserialize_closed_enum::<E>(&raw) {
                        Ok(Some(v)) => out.push(v),
                        Ok(None) => continue,
                        Err(e) => return Err(serde::de::Error::custom(e)),
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_any(V(core::marker::PhantomData))
    }
}

// ── map_closed_enum: HashMap<K, E> with unknown-value filtering ──────────────

/// Serde with-module for `HashMap<K, E>` map fields where the value is a
/// closed enum type.
///
/// When `ignore_unknown_enum_values` is active, map entries whose
/// value is an unknown enum string are silently dropped.
pub mod map_closed_enum {
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    pub fn serialize<C, S>(value: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: serde::Serialize,
        C::Value: crate::Enumeration,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(value.storage_len()))?;
        for (k, v) in value.storage_iter() {
            map.serialize_entry(k, &crate::json_helpers::EnumProtoNameRef(v))?;
        }
        map.end()
    }

    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: serde::Deserialize<'de>,
        C::Value: crate::Enumeration + Default,
        D: Deserializer<'de>,
    {
        struct V<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for V<C>
        where
            C: MapStorage + Default,
            C::Key: serde::Deserialize<'de>,
            C::Value: crate::Enumeration + Default,
        {
            type Value = C;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a map with enum values or null")
            }

            fn visit_unit<Err>(self) -> Result<Self::Value, Err> {
                Ok(C::default())
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = C::default();
                while let Some(key) = map.next_key::<C::Key>()? {
                    let raw = map.next_value::<serde_json::Value>()?;
                    match super::try_deserialize_closed_enum::<C::Value>(&raw) {
                        Ok(Some(v)) => {
                            out.storage_insert(key, v);
                        }
                        Ok(None) => continue,
                        Err(e) => return Err(serde::de::Error::custom(e)),
                    }
                }
                Ok(out)
            }
        }
        d.deserialize_any(V::<C>(core::marker::PhantomData))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
