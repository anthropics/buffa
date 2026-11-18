//! Classify an ignored object key without retaining or copying its text.

use serde::de::{
    Deserialize, Deserializer, EnumAccess, IgnoredAny, MapAccess, SeqAccess, VariantAccess, Visitor,
};

/// An ignored key, retaining only whether it is Serde JSON's number token.
/// Shared by typed and reflective Empty codecs to avoid copying unknown keys.
#[doc(hidden)]
pub enum IgnoredObjectKey {
    /// Serde JSON's synthetic number-token spelling.
    NumberToken,
    /// Any other key, discarded without retaining its payload.
    Other,
}

impl<'de> Deserialize<'de> for IgnoredObjectKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Key;
        impl<'de> Visitor<'de> for Key {
            type Value = IgnoredObjectKey;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("an ignored object key")
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                Ok(if value == super::SERDE_JSON_NUMBER_KEY {
                    IgnoredObjectKey::NumberToken
                } else {
                    IgnoredObjectKey::Other
                })
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_i128<E>(self, _: i128) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_u128<E>(self, _: u128) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_bytes<E>(self, _: &[u8]) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
                d.deserialize_ignored_any(self)
            }
            fn visit_newtype_struct<D2: Deserializer<'de>>(
                self,
                d: D2,
            ) -> Result<Self::Value, D2::Error> {
                d.deserialize_ignored_any(self)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_enum<A: EnumAccess<'de>>(self, data: A) -> Result<Self::Value, A::Error> {
                let (_, variant) = data.variant::<IgnoredAny>()?;
                variant.newtype_variant::<IgnoredAny>()?;
                Ok(IgnoredObjectKey::Other)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(IgnoredObjectKey::Other)
            }
        }
        d.deserialize_ignored_any(Key)
    }
}
