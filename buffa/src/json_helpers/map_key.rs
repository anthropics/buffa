//! Redact key conversion failures without replacing reader or syntax errors.
//!
//! Parsing stays with the original deserializer. Replaying a decoded string
//! would change numeric-key grammar, escaping, borrowing and non-JSON keys.

use core::{cell::Cell, marker::PhantomData};
use serde::de::{
    Deserialize, DeserializeSeed, Deserializer, EnumAccess, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};

/// Errors raised by a key representation differ from errors raised while
/// reading its source. Never format consumer-provided conversion context.
#[derive(Debug, thiserror::Error)]
enum KeyErrorReason<E> {
    #[error(transparent)]
    Source(E),
    #[error("{0}")]
    Invalid(super::JsonValueError),
}
impl<E: serde::de::Error> serde::de::Error for KeyErrorReason<E> {
    fn custom<T: core::fmt::Display>(_: T) -> Self {
        Self::Invalid(super::JsonValueError::MapKey)
    }
}
impl<E: serde::de::Error> KeyErrorReason<E> {
    fn into_source(self) -> E {
        match self {
            Self::Source(error) => error,
            Self::Invalid(reason) => E::custom(reason),
        }
    }
}

pub(super) struct KeySeed<K>(PhantomData<fn() -> K>);
impl<K> KeySeed<K> {
    pub(super) fn new() -> Self {
        Self(PhantomData)
    }
}
impl<'de, K: Deserialize<'de>> DeserializeSeed<'de> for KeySeed<K> {
    type Value = K;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<K, D::Error> {
        let expected_error = Cell::new(false);
        K::deserialize(KeyDeserializer {
            inner: d,
            expected_error: &expected_error,
        })
        .map_err(KeyErrorReason::into_source)
    }
}

struct KeyDeserializer<'a, D> {
    inner: D,
    expected_error: &'a Cell<bool>,
}
macro_rules! forward_key {
    ($method:ident $(, $arg:ident: $ty:ty)*) => {
        fn $method<V: Visitor<'de>>(self, $($arg:$ty,)* visitor:V) -> Result<V::Value, Self::Error> {
            self.inner.$method($($arg,)* KeyVisitor { inner:visitor, expected_error:self.expected_error })
                .map_err(|error| {
                    if self.expected_error.get() {
                        KeyErrorReason::Invalid(super::JsonValueError::MapKey)
                    } else {
                        KeyErrorReason::Source(error)
                    }
                })
        }
    };
}
impl<'de, D: Deserializer<'de>> Deserializer<'de> for KeyDeserializer<'_, D> {
    type Error = KeyErrorReason<D::Error>;
    forward_key!(deserialize_any);
    forward_key!(deserialize_bool);
    forward_key!(deserialize_i8);
    forward_key!(deserialize_i16);
    forward_key!(deserialize_i32);
    forward_key!(deserialize_i64);
    forward_key!(deserialize_i128);
    forward_key!(deserialize_u8);
    forward_key!(deserialize_u16);
    forward_key!(deserialize_u32);
    forward_key!(deserialize_u64);
    forward_key!(deserialize_u128);
    forward_key!(deserialize_f32);
    forward_key!(deserialize_f64);
    forward_key!(deserialize_char);
    forward_key!(deserialize_str);
    forward_key!(deserialize_string);
    forward_key!(deserialize_bytes);
    forward_key!(deserialize_byte_buf);
    forward_key!(deserialize_option);
    forward_key!(deserialize_unit);
    forward_key!(deserialize_seq);
    forward_key!(deserialize_map);
    forward_key!(deserialize_identifier);
    forward_key!(deserialize_ignored_any);
    forward_key!(deserialize_unit_struct, name: &'static str);
    forward_key!(deserialize_newtype_struct, name: &'static str);
    forward_key!(deserialize_tuple, len: usize);
    forward_key!(deserialize_tuple_struct, name: &'static str, len: usize);
    forward_key!(deserialize_struct, name: &'static str, fields: &'static [&'static str]);
    forward_key!(deserialize_enum, name: &'static str, variants: &'static [&'static str]);
    fn is_human_readable(&self) -> bool {
        self.inner.is_human_readable()
    }
}

struct KeyVisitor<'a, V> {
    inner: V,
    expected_error: &'a Cell<bool>,
}
macro_rules! scalar_key {
    ($method:ident, $ty:ty) => {
        fn $method<E: serde::de::Error>(self, value: $ty) -> Result<V::Value, E> {
            self.inner
                .$method::<KeyErrorReason<E>>(value)
                .map_err(KeyErrorReason::into_source)
        }
    };
}
impl<'de, V: Visitor<'de>> Visitor<'de> for KeyVisitor<'_, V> {
    type Value = V::Value;
    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // serde_json formats data errors eagerly, including a bool-key type
        // mismatch it constructs before calling a visitor method. Observe
        // that path without parsing error text. Syntax and I/O failures do
        // not request this expectation. This is a serde_json behavior, not
        // a guarantee made for arbitrary third-party error implementations.
        self.expected_error.set(true);
        // The original expectation is discarded. Do not format consumer
        // context merely to replace it with the fixed conversion reason.
        f.write_str("a protobuf map key")
    }
    scalar_key!(visit_bool, bool);
    scalar_key!(visit_i8, i8);
    scalar_key!(visit_i16, i16);
    scalar_key!(visit_i32, i32);
    scalar_key!(visit_i64, i64);
    scalar_key!(visit_i128, i128);
    scalar_key!(visit_u8, u8);
    scalar_key!(visit_u16, u16);
    scalar_key!(visit_u32, u32);
    scalar_key!(visit_u64, u64);
    scalar_key!(visit_u128, u128);
    scalar_key!(visit_f32, f32);
    scalar_key!(visit_f64, f64);
    scalar_key!(visit_char, char);
    scalar_key!(visit_str, &str);
    scalar_key!(visit_borrowed_str, &'de str);
    scalar_key!(visit_string, alloc::string::String);
    scalar_key!(visit_bytes, &[u8]);
    scalar_key!(visit_borrowed_bytes, &'de [u8]);
    scalar_key!(visit_byte_buf, alloc::vec::Vec<u8>);
    fn visit_unit<E: serde::de::Error>(self) -> Result<V::Value, E> {
        self.inner
            .visit_unit::<KeyErrorReason<E>>()
            .map_err(KeyErrorReason::into_source)
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<V::Value, E> {
        self.inner
            .visit_none::<KeyErrorReason<E>>()
            .map_err(KeyErrorReason::into_source)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<V::Value, D::Error> {
        self.inner
            .visit_some(KeyDeserializer {
                inner: d,
                expected_error: self.expected_error,
            })
            .map_err(KeyErrorReason::into_source)
    }
    fn visit_newtype_struct<D: Deserializer<'de>>(self, d: D) -> Result<V::Value, D::Error> {
        self.inner
            .visit_newtype_struct(KeyDeserializer {
                inner: d,
                expected_error: self.expected_error,
            })
            .map_err(KeyErrorReason::into_source)
    }
    // Sequence and map keys are not ProtoJSON keys. Leave their format-specific
    // traversal errors untouched instead of mistaking nested I/O for a
    // primitive key conversion failure.
    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<V::Value, A::Error> {
        self.inner.visit_seq(seq)
    }
    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<V::Value, A::Error> {
        self.inner.visit_map(map)
    }
    fn visit_enum<A: EnumAccess<'de>>(self, value: A) -> Result<V::Value, A::Error> {
        self.inner
            .visit_enum(KeyEnumAccess {
                inner: value,
                expected_error: self.expected_error,
            })
            .map_err(KeyErrorReason::into_source)
    }
}

// Unit enums are valid JSON string keys. Their variant identifier seed must
// cross the same conversion boundary as primitive and newtype keys.
struct KeyEnumAccess<'a, A> {
    inner: A,
    expected_error: &'a Cell<bool>,
}
struct KeyVariantAccess<'a, A> {
    inner: A,
    expected_error: &'a Cell<bool>,
}
struct KeySeedBridge<'a, S> {
    inner: S,
    expected_error: &'a Cell<bool>,
}
impl<'de, S: DeserializeSeed<'de>> DeserializeSeed<'de> for KeySeedBridge<'_, S> {
    type Value = S::Value;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<S::Value, D::Error> {
        self.inner
            .deserialize(KeyDeserializer {
                inner: d,
                expected_error: self.expected_error,
            })
            .map_err(KeyErrorReason::into_source)
    }
}
impl<'de, 'a, A: EnumAccess<'de>> EnumAccess<'de> for KeyEnumAccess<'a, A> {
    type Error = KeyErrorReason<A::Error>;
    type Variant = KeyVariantAccess<'a, A::Variant>;
    fn variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<(S::Value, Self::Variant), Self::Error> {
        self.inner
            .variant_seed(KeySeedBridge {
                inner: seed,
                expected_error: self.expected_error,
            })
            .map(|(value, inner)| {
                (
                    value,
                    KeyVariantAccess {
                        inner,
                        expected_error: self.expected_error,
                    },
                )
            })
            .map_err(KeyErrorReason::Source)
    }
}
impl<'de, A: VariantAccess<'de>> VariantAccess<'de> for KeyVariantAccess<'_, A> {
    type Error = KeyErrorReason<A::Error>;
    fn unit_variant(self) -> Result<(), Self::Error> {
        self.inner.unit_variant().map_err(KeyErrorReason::Source)
    }
    fn newtype_variant_seed<S: DeserializeSeed<'de>>(
        self,
        seed: S,
    ) -> Result<S::Value, Self::Error> {
        self.inner
            .newtype_variant_seed(KeySeedBridge {
                inner: seed,
                expected_error: self.expected_error,
            })
            .map_err(KeyErrorReason::Source)
    }
    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner
            .tuple_variant(
                len,
                KeyVisitor {
                    inner: visitor,
                    expected_error: self.expected_error,
                },
            )
            .map_err(KeyErrorReason::Source)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.inner
            .struct_variant(
                fields,
                KeyVisitor {
                    inner: visitor,
                    expected_error: self.expected_error,
                },
            )
            .map_err(KeyErrorReason::Source)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    struct SourceFailureAfterValue;
    impl<'de> Deserializer<'de> for SourceFailureAfterValue {
        type Error = serde_json::Error;
        fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let _ = visitor.visit_i64::<Self::Error>(1)?;
            struct Fail;
            impl std::io::Read for Fail {
                fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::other("synthetic read failure"))
                }
            }
            Err(serde_json::from_reader::<_, serde_json::Value>(Fail).unwrap_err())
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
            bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
            map struct enum identifier ignored_any
        }
    }
    #[test]
    fn io_after_a_successful_callback_remains_a_source_error() {
        let error = KeySeed::<i32>::new()
            .deserialize(SourceFailureAfterValue)
            .unwrap_err();
        assert!(error.is_io());
    }
}
