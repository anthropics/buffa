use crate::map_codec::MapStorage;
use serde::{Deserializer, Serializer};

pub(super) fn serialize<'a, C, S, V>(
    value: &'a C,
    serializer: S,
    element: impl Fn(&'a C::Value) -> V,
) -> Result<S::Ok, S::Error>
where
    C: MapStorage<Key = alloc::vec::Vec<u8>>,
    C::Value: 'a,
    V: serde::Serialize,
    S: Serializer,
{
    use serde::ser::SerializeMap;
    let mut map = serializer.serialize_map(Some(value.storage_len()))?;
    for (key, value) in value.storage_iter() {
        map.serialize_entry(&super::Base64Wrapper(key), &element(value))?;
    }
    map.end()
}

pub(super) fn deserialize<'de, C, D, Seed, Factory>(
    deserializer: D,
    element_seed: Factory,
) -> Result<C, D::Error>
where
    C: MapStorage<Key = alloc::vec::Vec<u8>> + Default,
    D: Deserializer<'de>,
    Seed: serde::de::DeserializeSeed<'de, Value = C::Value>,
    Factory: Fn() -> Seed,
{
    struct MapVisitor<C, Factory>(Factory, core::marker::PhantomData<C>);
    impl<'de, C, Seed, Factory> serde::de::Visitor<'de> for MapVisitor<C, Factory>
    where
        C: MapStorage<Key = alloc::vec::Vec<u8>> + Default,
        Seed: serde::de::DeserializeSeed<'de, Value = C::Value>,
        Factory: Fn() -> Seed,
    {
        type Value = C;

        fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            formatter.write_str("a map with base64-encoded keys, or null")
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(C::default())
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = C::default();
            while let Some(key) = map.next_key::<alloc::string::String>()? {
                let key = super::decode_base64(&key).map_err(serde::de::Error::custom)?;
                let value = map.next_value_seed((self.0)())?;
                values.storage_insert(key, value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_any(MapVisitor::<C, Factory>(
        element_seed,
        core::marker::PhantomData,
    ))
}
