use alloc::string::String;
use core::fmt;
use serde::de::{self, EnumAccess, VariantAccess, Visitor};
use serde::{Deserialize, Deserializer};

const JSON_NUMBER_MARKER: &str = "$serde_json::private::Number";
const NUMBER_TOKEN_WITH_EXTRA_FIELD_LENGTH: usize = 2;

/// Adapter for serde_json object-key and arbitrary-precision-number dispatch.
#[doc(hidden)]
#[derive(Debug, PartialEq, Eq)]
pub enum JsonObjectKey {
    /// Number-marker dispatch from the serde_json deserializer.
    NumberToken,
    /// Ordinary object member, including a literal numeric-marker name.
    Field(String),
}

impl JsonObjectKey {
    /// Parse the dispatched numeric text as a finite floating value.
    ///
    /// Requires the key and map from the same serde_json numeric dispatch.
    ///
    /// # Errors
    /// Rejects ordinary fields, invalid floating text and nonfinite values.
    #[doc(hidden)]
    pub fn into_number_f64<'de, A: de::MapAccess<'de>>(self, map: &mut A) -> Result<f64, A::Error> {
        let token = self.into_number_value(map)?;
        let value = token.parse::<f64>().map_err(de::Error::custom)?;
        if !value.is_finite() {
            return Err(de::Error::invalid_value(
                de::Unexpected::Str(&token),
                &"a finite JSON number",
            ));
        }
        Ok(value)
    }

    /// Read numeric text from the serde_json number-marker map.
    ///
    /// Requires the key and map from the same serde_json numeric dispatch.
    /// Validates the map shape; numeric consumers validate the text.
    ///
    /// # Errors
    /// Rejects ordinary fields, non-string payloads and extra fields.
    #[doc(hidden)]
    pub fn into_number_value<'de, A: de::MapAccess<'de>>(
        self,
        map: &mut A,
    ) -> Result<String, A::Error> {
        if self != Self::NumberToken {
            return Err(de::Error::invalid_type(
                de::Unexpected::Map,
                &"an original JSON number token",
            ));
        }
        let token = map.next_value::<String>()?;
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::invalid_length(
                NUMBER_TOKEN_WITH_EXTRA_FIELD_LENGTH,
                &"an isolated JSON number token",
            ));
        }
        Ok(token)
    }
}

pub(super) fn number_from_map<'de, A: de::MapAccess<'de>>(mut map: A) -> Result<String, A::Error> {
    let key = map
        .next_key::<JsonObjectKey>()?
        .ok_or_else(|| de::Error::invalid_length(0, &"a JSON number token"))?;
    key.into_number_value(&mut map)
}

pub(super) fn f64_from_map<'de, A: de::MapAccess<'de>>(mut map: A) -> Result<f64, A::Error> {
    let key = map
        .next_key::<JsonObjectKey>()?
        .ok_or_else(|| de::Error::invalid_length(0, &"a JSON number token"))?;
    key.into_number_f64(&mut map)
}

impl<'de> Deserialize<'de> for JsonObjectKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;

        impl<'de> Visitor<'de> for KeyVisitor {
            type Value = JsonObjectKey;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object field or original number token")
            }

            fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
                if value == JSON_NUMBER_MARKER {
                    Ok(JsonObjectKey::NumberToken)
                } else {
                    Err(E::invalid_value(de::Unexpected::Str(value), &self))
                }
            }

            fn visit_enum<A: EnumAccess<'de>>(self, value: A) -> Result<Self::Value, A::Error> {
                let (field, variant) = value.variant::<String>()?;
                variant.unit_variant()?;
                Ok(JsonObjectKey::Field(field))
            }
        }

        // Numeric markers bypass string enum dispatch in serde_json.
        deserializer.deserialize_enum("JsonObjectKey", &[], KeyVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec::Vec;
    use serde::de::{IgnoredAny, MapAccess};

    #[derive(Debug, PartialEq)]
    enum Origin {
        Number(String),
        Float(f64),
        Object(Vec<String>),
    }

    impl<'de> Deserialize<'de> for Origin {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct OriginVisitor;
            impl<'de> Visitor<'de> for OriginVisitor {
                type Value = Origin;
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str("a number or object")
                }
                fn visit_f64<E>(self, value: f64) -> Result<Origin, E> {
                    Ok(Origin::Float(value))
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Origin, A::Error> {
                    let mut fields = Vec::new();
                    while let Some(key) = map.next_key::<JsonObjectKey>()? {
                        match key {
                            JsonObjectKey::NumberToken => {
                                let token = map.next_value::<String>()?;
                                if map.next_key::<IgnoredAny>()?.is_some() {
                                    return Err(de::Error::custom("extra numeric marker field"));
                                }
                                return Ok(Origin::Number(token));
                            }
                            JsonObjectKey::Field(field) => {
                                fields.push(field);
                                map.next_value::<IgnoredAny>()?;
                            }
                        }
                    }
                    Ok(Origin::Object(fields))
                }
            }
            deserializer.deserialize_any(OriginVisitor)
        }
    }

    #[test]
    fn real_number_origin_agrees_for_slice_reader_and_owned_value() {
        let source = "1e1";
        let number = serde_json::from_str::<serde_json::Number>(source).unwrap();
        let lexical = number.to_string();
        let expected = match lexical.as_str() {
            "10.0" => Origin::Float(10.0),
            "1e1" | "1e+1" => Origin::Number(lexical),
            other => panic!("unexpected independent Number encoding: {other}"),
        };
        assert_eq!(serde_json::from_str::<Origin>(source).unwrap(), expected);
        assert_eq!(
            serde_json::from_slice::<Origin>(source.as_bytes()).unwrap(),
            expected
        );
        #[cfg(feature = "std")]
        assert_eq!(
            serde_json::from_reader::<_, Origin>(source.as_bytes()).unwrap(),
            expected
        );
        assert_eq!(
            serde_json::from_value::<Origin>(serde_json::Value::Number(number)).unwrap(),
            expected
        );
    }

    #[test]
    fn literal_marker_objects_remain_objects_through_direct_and_buffered_inputs() {
        for source in [
            r#"{"$serde_json::private::Number":"1e1"}"#,
            r#"{"$serde_json::private::Number":"0.999999999999999999999"}"#,
        ] {
            let expected = Origin::Object(alloc::vec![JSON_NUMBER_MARKER.into()]);
            assert_eq!(serde_json::from_str::<Origin>(source).unwrap(), expected);
            #[cfg(feature = "std")]
            assert_eq!(
                serde_json::from_reader::<_, Origin>(source.as_bytes()).unwrap(),
                expected
            );
            let mut object = serde_json::Map::new();
            object.insert(
                JSON_NUMBER_MARKER.into(),
                serde_json::Value::String("1e1".into()),
            );
            assert_eq!(
                serde_json::from_value::<Origin>(serde_json::Value::Object(object)).unwrap(),
                expected
            );
            #[derive(Deserialize)]
            #[serde(untagged)]
            enum Buffered {
                Value(Origin),
                Other(bool),
            }
            match serde_json::from_str::<Buffered>(source).unwrap() {
                Buffered::Value(value) => assert_eq!(value, expected),
                Buffered::Other(value) => {
                    panic!("a marker object cannot become a boolean: {value}")
                }
            }
        }
    }
}
