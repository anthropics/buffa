//! Ergonomic helpers for [`google::protobuf::Empty`](crate::google::protobuf::Empty).

#[cfg(feature = "json")]
use crate::google::protobuf::Empty;

#[cfg(feature = "json")]
impl serde::Serialize for Empty {
    /// Serializes as an empty JSON object `{}`.
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        s.serialize_map(Some(0))?.end()
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Empty {
    /// Deserializes from a JSON object (any fields are ignored).
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::{MapAccess, Visitor};
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Empty;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("an empty JSON object")
            }
            buffa::json_helpers::reject_other_json_kinds!(
                bool, i64, u64, i128, u128, f64, str, bytes
            );
            // An empty YAML value reaches `deserialize_any` as `visit_none`;
            // `deserialize_map` read it as an empty map. JSON never sends it.
            fn visit_enum<A: serde::de::EnumAccess<'de>>(
                self,
                tagged: A,
            ) -> Result<Self::Value, A::Error> {
                buffa::json_helpers::deserialize_tagged(self, tagged)
            }
            fn visit_none<E>(self) -> Result<Empty, E> {
                Ok(Empty::default())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Empty, A::Error> {
                while let Some(key) = map.next_key::<buffa::json_helpers::IgnoredObjectKey>()? {
                    if matches!(key, buffa::json_helpers::IgnoredObjectKey::NumberToken) {
                        return Err(serde::de::Error::invalid_type(
                            serde::de::Unexpected::Other("number"),
                            &self,
                        ));
                    }
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(Empty {
                    ..Default::default()
                })
            }
        }
        // `deserialize_any` so a non-object reaches `V`, whose rejections
        // name the JSON kind but not the value.
        if d.is_human_readable() {
            d.deserialize_any(V)
        } else {
            d.deserialize_map(V)
        }
    }
}

#[cfg(all(test, feature = "json"))]
mod tests {
    use super::*;

    #[test]
    fn empty_serializes_as_empty_object() {
        let e = Empty::default();
        assert_eq!(serde_json::to_string(&e).unwrap(), "{}");
    }

    #[test]
    fn empty_deserializes_from_empty_object() {
        let _: Empty = serde_json::from_str("{}").unwrap();
    }
}
