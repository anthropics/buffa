//! JSON policies for custom `bytes` and `string` representations.
//!
//! A representation listed with `buffa_build::Config::json_policy_type` (plugin
//! option `json_policy_type=<rust_type>`) implements [`ProtoBytesJson`] or
//! [`ProtoStringJson`]. Generated code then routes every JSON use of that type
//! through the trait: singular, optional, repeated and oneof fields, `map`
//! values, and the matching fields of generated views and owned views. The
//! type's own `serde` impls, if any, are not consulted.
//!
//! The functions are type-level: serialization receives the field's borrowed
//! `&[u8]` or `&str`, which is all a view holds, so one policy governs the
//! owned message and its views alike. A policy cannot depend on per-value
//! state that a view does not carry.

use core::marker::PhantomData;
use serde::de::{DeserializeSeed, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

/// Protobuf JSON encoding for a custom [`ProtoBytes`](crate::ProtoBytes)
/// representation, selected with `json_policy_type`.
///
/// There is no default implementation and no blanket impl: a listed type that
/// does not implement this trait fails to compile, naming the trait.
/// [`json_helpers::bytes`](super::bytes) provides buffa's standard base64
/// encoding for implementations that only need to control where decoded bytes
/// are stored, or that want to reject export by returning an error.
///
/// # Contract
///
/// - `serialize_json` receives the field's bytes, from the owned value or from
///   a view, and must produce protobuf JSON for `bytes` (a base64 string) or
///   fail.
/// - `deserialize_json` never receives JSON `null`. Generated code handles
///   `null` before calling it: an implicit-presence field becomes the default
///   value, an optional field or oneof variant becomes unset, and a `null`
///   element of a `repeated` field or `map` value is rejected.
/// - A deserialized value must equal one built with `From<Vec<u8>>` from the
///   same bytes.
pub trait ProtoBytesJson: crate::ProtoBytes {
    /// Serialize a field's bytes as protobuf JSON.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error, or a policy error built with
    /// [`serde::ser::Error::custom`].
    fn serialize_json<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>;

    /// Deserialize a non-null protobuf JSON `bytes` value.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's error for malformed input, or a policy error
    /// built with [`serde::de::Error::custom`].
    fn deserialize_json<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// Protobuf JSON encoding for a custom [`ProtoString`](crate::ProtoString)
/// representation, selected with `json_policy_type`.
///
/// There is no default implementation and no blanket impl: a listed type that
/// does not implement this trait fails to compile, naming the trait.
///
/// # Contract
///
/// - `serialize_json` receives the field's text, from the owned value or from
///   a view, and must produce a JSON string or fail.
/// - `deserialize_json` never receives JSON `null`. Generated code handles
///   `null` before calling it: an implicit-presence field becomes the default
///   value, an optional field or oneof variant becomes unset, and a `null`
///   element of a `repeated` field or `map` value is rejected.
/// - A deserialized value must equal one built with `From<String>` from the
///   same text.
/// - A policy used for a `map` key must serialize as a JSON string. Maps with
///   enum values do not support policy keys because their unknown-enum
///   filtering needs a separate decoding path; codegen rejects that shape.
pub trait ProtoStringJson: crate::ProtoString {
    /// Serialize a field's text as protobuf JSON.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error, or a policy error built with
    /// [`serde::ser::Error::custom`].
    fn serialize_json<S: Serializer>(text: &str, serializer: S) -> Result<S::Ok, S::Error>;

    /// Deserialize a non-null protobuf JSON `string` value.
    ///
    /// # Errors
    ///
    /// Returns the deserializer's error for malformed input, or a policy error
    /// built with [`serde::de::Error::custom`].
    fn deserialize_json<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error>;
}

/// Generates the generated-code plumbing for one policy trait.
macro_rules! policy_plumbing {
    (
        trait: $trait:ident,
        borrowed: $borrowed:ty,
        owned: $owned:ident,
        singular: $singular:ident,
        optional: $optional:ident,
        adapter: $adapter:ident,
        seq_adapter: $seq_adapter:ident,
        seed: $seed:ident,
        $(,)?
    ) => {
        /// Owned value deserialized through its policy, so that
        /// `Option<_>` handles JSON `null` before the policy runs.
        struct $owned<T>(T);

        impl<'de, T: $trait> Deserialize<'de> for $owned<T> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                T::deserialize_json(d).map($owned)
            }
        }

        /// Serde with-module for an implicit-presence field whose type implements
        /// this policy trait. JSON `null` deserializes to the default value.
        #[doc(hidden)]
        pub mod $singular {
            use super::$trait;
            use serde::{Deserializer, Serializer};

            /// Serialize through the type's policy.
            pub fn serialize<T: $trait, S: Serializer>(value: &T, s: S) -> Result<S::Ok, S::Error> {
                T::serialize_json(value.as_ref(), s)
            }

            /// Deserialize through the type's policy, mapping `null` to the
            /// default value.
            pub fn deserialize<'de, T: $trait, D: Deserializer<'de>>(d: D) -> Result<T, D::Error> {
                <Option<super::$owned<T>> as serde::Deserialize>::deserialize(d)
                    .map(|value| value.map(|owned| owned.0).unwrap_or_default())
            }
        }

        /// Serde with-module for an explicit-presence field whose type implements
        /// this policy trait. JSON `null` deserializes to `None`.
        #[doc(hidden)]
        pub mod $optional {
            use super::$trait;
            use serde::{Deserializer, Serializer};

            /// Serialize through the type's policy.
            pub fn serialize<T: $trait, S: Serializer>(
                value: &Option<T>,
                s: S,
            ) -> Result<S::Ok, S::Error> {
                match value {
                    Some(v) => T::serialize_json(v.as_ref(), s),
                    None => s.serialize_none(),
                }
            }

            /// Deserialize through the type's policy, mapping `null` to `None`.
            pub fn deserialize<'de, T: $trait, D: Deserializer<'de>>(
                d: D,
            ) -> Result<Option<T>, D::Error> {
                <Option<super::$owned<T>> as serde::Deserialize>::deserialize(d)
                    .map(|value| value.map(|owned| owned.0))
            }
        }

        /// Serializes a borrowed value through the policy of `T`, for generated
        /// views and oneof variants.
        #[doc(hidden)]
        pub struct $adapter<'a, T: ?Sized>(&'a $borrowed, PhantomData<fn() -> T>);

        impl<'a, T: $trait> $adapter<'a, T> {
            /// Wrap a borrowed value for serialization under `T`'s policy.
            pub fn new(value: &'a $borrowed) -> Self {
                Self(value, PhantomData)
            }
        }

        impl<T: $trait> Serialize for $adapter<'_, T> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                T::serialize_json(self.0, s)
            }
        }

        /// Serializes borrowed repeated elements through the policy of `T`, for
        /// generated views.
        #[doc(hidden)]
        pub struct $seq_adapter<'a, T: ?Sized>(&'a [&'a $borrowed], PhantomData<fn() -> T>);

        impl<'a, T: $trait> $seq_adapter<'a, T> {
            /// Wrap borrowed elements for serialization under `T`'s policy.
            pub fn new(elements: &'a [&'a $borrowed]) -> Self {
                Self(elements, PhantomData)
            }
        }

        impl<T: $trait> Serialize for $seq_adapter<'_, T> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq as _;
                let mut seq = s.serialize_seq(Some(self.0.len()))?;
                for element in self.0 {
                    seq.serialize_element(&$adapter::<T>::new(element))?;
                }
                seq.end()
            }
        }

        /// Deserializes a non-null value through the policy of `T`, for generated
        /// oneof variants.
        #[doc(hidden)]
        pub struct $seed<T>(PhantomData<fn() -> T>);

        impl<T> $seed<T> {
            /// Create the seed.
            pub fn new() -> Self {
                Self(PhantomData)
            }
        }

        impl<T> Default for $seed<T> {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<'de, T: $trait> DeserializeSeed<'de> for $seed<T> {
            type Value = T;
            fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<T, D::Error> {
                T::deserialize_json(d)
            }
        }
    };
}

policy_plumbing! {
    trait: ProtoBytesJson,
    borrowed: [u8],
    owned: OwnedBytes,
    singular: bytes_policy,
    optional: opt_bytes_policy,
    adapter: BytesPolicyJson,
    seq_adapter: BytesPolicySeqJson,
    seed: BytesPolicySeed,
}

policy_plumbing! {
    trait: ProtoStringJson,
    borrowed: str,
    owned: OwnedString,
    singular: string_policy,
    optional: opt_string_policy,
    adapter: StringPolicyJson,
    seq_adapter: StringPolicySeqJson,
    seed: StringPolicySeed,
}

/// Serde with-module for a `map` field whose `string` key type implements
/// [`ProtoStringJson`]. Keys go through the key type's policy and values
/// through [`ProtoElemJson`](super::ProtoElemJson); JSON `null` deserializes
/// to an empty map, and a `null` value is rejected as in
/// [`proto_map`](super::proto_map).
#[doc(hidden)]
pub mod policy_str_key_map {
    use super::{ProtoStringJson, StringPolicyJson, StringPolicySeed};
    use crate::json_helpers::{ProtoElemJson, ProtoElemSeed, ProtoJson};
    use crate::map_codec::MapStorage;
    use serde::{Deserializer, Serializer};

    /// Serialize the map, keys through their policy.
    pub fn serialize<C, S>(m: &C, s: S) -> Result<S::Ok, S::Error>
    where
        C: MapStorage,
        C::Key: ProtoStringJson,
        C::Value: ProtoElemJson,
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(m.storage_len()))?;
        for (k, v) in m.storage_iter() {
            map.serialize_entry(&StringPolicyJson::<C::Key>::new(k.as_ref()), &ProtoJson(v))?;
        }
        map.end()
    }

    /// Deserialize the map, keys through their policy.
    pub fn deserialize<'de, C, D>(d: D) -> Result<C, D::Error>
    where
        C: MapStorage + Default,
        C::Key: ProtoStringJson,
        C::Value: ProtoElemJson,
        D: Deserializer<'de>,
    {
        struct Vis<C>(core::marker::PhantomData<C>);
        impl<'de, C> serde::de::Visitor<'de> for Vis<C>
        where
            C: MapStorage + Default,
            C::Key: ProtoStringJson,
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
                while let Some(k) = map.next_key_seed(StringPolicySeed::<C::Key>::new())? {
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

#[cfg(test)]
mod tests;
