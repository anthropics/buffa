//! Ergonomic helpers for [`google::protobuf::Any`](crate::google::protobuf::Any).

use alloc::string::String;

use crate::google::protobuf::Any;

impl Any {
    /// Pack a message into an [`Any`] with the given type URL.
    ///
    /// The type URL is conventionally of the form
    /// `type.googleapis.com/fully.qualified.TypeName`, but this method does
    /// not enforce that convention — any string is accepted.
    pub fn pack(msg: &impl buffa::Message, type_url: impl Into<String>) -> Self {
        Self {
            type_url: type_url.into(),
            value: msg.encode_to_vec(),
            ..Default::default()
        }
    }

    /// Unpack the contained message, decoding its bytes as `T`, **without
    /// checking the `type_url`**.
    ///
    /// This method always attempts to decode the payload as `T` regardless
    /// of whether `type_url` actually identifies `T`. Use [`Any::unpack_if`]
    /// when you need to verify the stored type before decoding.
    ///
    /// # Errors
    ///
    /// Returns a [`buffa::DecodeError`] if the bytes cannot be decoded as `T`.
    pub fn unpack_unchecked<T: buffa::Message>(&self) -> Result<T, buffa::DecodeError> {
        T::decode(&mut self.value.as_slice())
    }

    /// Unpack the contained message as `T`, but only if the `type_url`
    /// matches `expected_type_url`.
    ///
    /// Returns `Ok(None)` when the type URL does not match.
    ///
    /// # Errors
    ///
    /// Returns a [`buffa::DecodeError`] if the type URL matches but the bytes
    /// cannot be decoded as `T`.
    pub fn unpack_if<T: buffa::Message>(
        &self,
        expected_type_url: &str,
    ) -> Result<Option<T>, buffa::DecodeError> {
        if self.type_url != expected_type_url {
            return Ok(None);
        }
        T::decode(&mut self.value.as_slice()).map(Some)
    }

    /// Returns `true` if this [`Any`]'s `type_url` matches the given string.
    pub fn is_type(&self, type_url: &str) -> bool {
        self.type_url == type_url
    }

    /// Returns the type URL stored in this [`Any`].
    pub fn type_url(&self) -> &str {
        &self.type_url
    }
}

// ── WKT type registry ───────────────────────────────────────────────────────

/// Registers all well-known types with the given [`AnyRegistry`](buffa::any_registry::AnyRegistry).
///
/// This registers Duration, Timestamp, FieldMask, Value, Struct, ListValue,
/// Empty, all wrapper types, and Any itself, enabling proto3-compliant JSON
/// serialization when these types appear inside `google.protobuf.Any` fields.
///
/// # Example
///
/// ```rust,no_run
/// use buffa::any_registry::AnyRegistry;
///
/// let mut registry = AnyRegistry::new();
/// buffa_types::register_wkt_types(&mut registry);
/// ```
#[cfg(feature = "json")]
pub fn register_wkt_types(registry: &mut buffa::any_registry::AnyRegistry) {
    use crate::google::protobuf::*;
    use alloc::string::ToString;
    use buffa::any_registry::AnyTypeEntry;

    macro_rules! register_type {
        ($type:ty, $wkt:expr) => {
            registry.register(AnyTypeEntry {
                type_url: <$type>::TYPE_URL,
                to_json: |bytes| {
                    let msg = <$type as buffa::Message>::decode(&mut &*bytes)
                        .map_err(|e| e.to_string())?;
                    serde_json::to_value(&msg).map_err(|e| e.to_string())
                },
                from_json: |value| {
                    let msg: $type = serde_json::from_value(value).map_err(|e| e.to_string())?;
                    Ok(buffa::Message::encode_to_vec(&msg))
                },
                is_wkt: $wkt,
            });
        };
    }

    // WKTs with special JSON mappings (use "value" wrapping in Any).
    register_type!(Duration, true);
    register_type!(Timestamp, true);
    register_type!(FieldMask, true);
    register_type!(Value, true);
    register_type!(Struct, true);
    register_type!(ListValue, true);
    register_type!(BoolValue, true);
    register_type!(Int32Value, true);
    register_type!(UInt32Value, true);
    register_type!(Int64Value, true);
    register_type!(UInt64Value, true);
    register_type!(FloatValue, true);
    register_type!(DoubleValue, true);
    register_type!(StringValue, true);
    register_type!(BytesValue, true);
    register_type!(Any, true);

    // Regular messages (fields inlined in Any JSON).
    register_type!(Empty, false);
}

// ── serde impls ──────────────────────────────────────────────────────────────
//
// Proto3 JSON for `Any` uses the global `AnyRegistry` to serialize the
// embedded message with its fields inline (regular messages) or wrapped in a
// `"value"` key (WKTs). Falls back to base64-encoded `value` when the
// registry is absent or the type URL is not registered.

#[cfg(feature = "json")]
struct Base64Bytes<'a>(&'a [u8]);

#[cfg(feature = "json")]
impl serde::Serialize for Base64Bytes<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        buffa::json_helpers::bytes::serialize(self.0, s)
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for Any {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        if self.type_url.is_empty() {
            return s.serialize_map(Some(0))?.end();
        }

        let lookup = buffa::any_registry::with_any_registry(|reg| {
            reg.and_then(|r| r.lookup(&self.type_url))
                .map(|e| (e.to_json, e.is_wkt))
        });

        match lookup {
            Some((to_json, is_wkt)) => {
                let json_val = to_json(&self.value).map_err(serde::ser::Error::custom)?;
                if is_wkt {
                    let mut map = s.serialize_map(Some(2))?;
                    map.serialize_entry("@type", &self.type_url)?;
                    map.serialize_entry("value", &json_val)?;
                    map.end()
                } else {
                    let fields = match &json_val {
                        serde_json::Value::Object(m) => m,
                        _ => {
                            return Err(serde::ser::Error::custom(
                                "Any: to_json for non-WKT must return a JSON object",
                            ))
                        }
                    };
                    let mut map = s.serialize_map(Some(1 + fields.len()))?;
                    map.serialize_entry("@type", &self.type_url)?;
                    for (k, v) in fields {
                        map.serialize_entry(k, v)?;
                    }
                    map.end()
                }
            }
            None => {
                let mut map = s.serialize_map(Some(2))?;
                map.serialize_entry("@type", &self.type_url)?;
                map.serialize_entry("value", &Base64Bytes(&self.value))?;
                map.end()
            }
        }
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for Any {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Buffer the entire object so @type can appear at any position.
        let mut obj: serde_json::Map<String, serde_json::Value> =
            serde::Deserialize::deserialize(d)?;

        let type_url = match obj.remove("@type") {
            Some(serde_json::Value::String(s)) => s,
            Some(_) => {
                return Err(serde::de::Error::custom("@type must be a string"));
            }
            None => return Ok(Self::default()),
        };

        // The type URL must be non-empty and contain a '/' separating the
        // host/authority from the fully-qualified type name (e.g.
        // "type.googleapis.com/google.protobuf.Duration").
        if type_url.is_empty() || !type_url.contains('/') {
            return Err(serde::de::Error::custom(
                "@type must be a valid type URL containing a '/' (e.g. type.googleapis.com/pkg.Type)",
            ));
        }

        let lookup = buffa::any_registry::with_any_registry(|reg| {
            reg.and_then(|r| r.lookup(&type_url))
                .map(|e| (e.from_json, e.is_wkt))
        });

        let value = match lookup {
            Some((from_json, true)) => {
                let json_val = obj.remove("value").unwrap_or(serde_json::Value::Null);
                from_json(json_val).map_err(serde::de::Error::custom)?
            }
            Some((from_json, false)) => {
                let json_obj = serde_json::Value::Object(obj);
                from_json(json_obj).map_err(serde::de::Error::custom)?
            }
            None => {
                // Fallback: base64 decode the "value" field.
                match obj.remove("value") {
                    Some(serde_json::Value::String(s)) => buffa::json_helpers::bytes::deserialize(
                        serde::de::value::StringDeserializer::<D::Error>::new(s),
                    )?,
                    _ => alloc::vec::Vec::new(),
                }
            }
        };

        Ok(Self {
            type_url,
            value,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::protobuf::Timestamp;
    use buffa::Message as _;

    #[test]
    fn pack_and_unpack() {
        let ts = Timestamp {
            seconds: 1_000_000_000,
            nanos: 0,
            ..Default::default()
        };
        let any = Any::pack(&ts, "type.googleapis.com/google.protobuf.Timestamp");
        assert_eq!(
            any.type_url(),
            "type.googleapis.com/google.protobuf.Timestamp"
        );

        let decoded: Timestamp = any.unpack_unchecked().unwrap();
        assert_eq!(decoded, ts);
    }

    #[test]
    fn unpack_if_matching() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let any = Any::pack(&ts, "type.googleapis.com/google.protobuf.Timestamp");

        let result: Option<Timestamp> = any
            .unpack_if("type.googleapis.com/google.protobuf.Timestamp")
            .unwrap();
        assert_eq!(result, Some(ts));
    }

    #[test]
    fn unpack_if_wrong_type_returns_none() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let any = Any::pack(&ts, "type.googleapis.com/google.protobuf.Timestamp");

        let result: Option<Timestamp> = any
            .unpack_if("type.googleapis.com/google.protobuf.Duration")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn is_type() {
        let ts = Timestamp::default();
        let any = Any::pack(&ts, "type.googleapis.com/google.protobuf.Timestamp");
        assert!(any.is_type("type.googleapis.com/google.protobuf.Timestamp"));
        assert!(!any.is_type("type.googleapis.com/google.protobuf.Duration"));
    }

    #[test]
    fn round_trip_encoding() {
        let ts = Timestamp {
            seconds: 99,
            nanos: 1,
            ..Default::default()
        };
        let any = Any::pack(&ts, "test");

        let bytes = any.encode_to_vec();
        let decoded_any = Any::decode(&mut bytes.as_slice()).unwrap();
        let decoded_ts: Timestamp = decoded_any.unpack_unchecked().unwrap();
        assert_eq!(decoded_ts, ts);
    }

    #[cfg(feature = "json")]
    mod serde_tests {
        use super::*;
        use crate::google::protobuf::Duration;
        use buffa::any_registry::{clear_any_registry, set_any_registry, AnyRegistry};

        /// Mutex to serialize tests that manipulate the global AnyRegistry.
        /// Each test binary needs its own lock since #[cfg(test)] modules
        /// cannot be shared across crates.
        static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn with_registry<R>(f: impl FnOnce() -> R) -> R {
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut registry = AnyRegistry::new();
            register_wkt_types(&mut registry);
            set_any_registry(Box::new(registry));
            let result = f();
            clear_any_registry();
            result
        }

        fn without_registry<R>(f: impl FnOnce() -> R) -> R {
            let _guard = REGISTRY_LOCK.lock().unwrap();
            clear_any_registry();
            f()
        }

        #[test]
        fn serialize_wkt_uses_value_wrapping() {
            with_registry(|| {
                let ts = Timestamp {
                    seconds: 1_000_000_000,
                    nanos: 0,
                    ..Default::default()
                };
                let any = Any::pack(&ts, Timestamp::TYPE_URL);
                let json = serde_json::to_value(&any).unwrap();
                assert_eq!(json["@type"], Timestamp::TYPE_URL);
                assert_eq!(json["value"], "2001-09-09T01:46:40Z");
            });
        }

        #[test]
        fn serialize_duration_wkt() {
            with_registry(|| {
                let dur = Duration::from_secs_nanos(1, 500_000_000);
                let any = Any::pack(&dur, Duration::TYPE_URL);
                let json = serde_json::to_value(&any).unwrap();
                assert_eq!(json["@type"], Duration::TYPE_URL);
                assert_eq!(json["value"], "1.500s");
            });
        }

        #[test]
        fn serialize_empty_any_is_empty_object() {
            with_registry(|| {
                let any = Any::default();
                let json = serde_json::to_string(&any).unwrap();
                assert_eq!(json, "{}");
            });
        }

        #[test]
        fn deserialize_wkt_from_json() {
            with_registry(|| {
                let json = r#"{
                    "@type": "type.googleapis.com/google.protobuf.Duration",
                    "value": "1.5s"
                }"#;
                let any: Any = serde_json::from_str(json).unwrap();
                assert_eq!(any.type_url, Duration::TYPE_URL);

                let dur: Duration = any.unpack_unchecked().unwrap();
                assert_eq!(dur.seconds, 1);
                assert_eq!(dur.nanos, 500_000_000);
            });
        }

        #[test]
        fn deserialize_unordered_type_tag() {
            with_registry(|| {
                // @type appears after the value field.
                let json = r#"{
                    "value": "1.5s",
                    "@type": "type.googleapis.com/google.protobuf.Duration"
                }"#;
                let any: Any = serde_json::from_str(json).unwrap();
                assert_eq!(any.type_url, Duration::TYPE_URL);

                let dur: Duration = any.unpack_unchecked().unwrap();
                assert_eq!(dur.seconds, 1);
                assert_eq!(dur.nanos, 500_000_000);
            });
        }

        #[test]
        fn roundtrip_wkt_json() {
            with_registry(|| {
                let ts = Timestamp {
                    seconds: 1_000_000_000,
                    nanos: 0,
                    ..Default::default()
                };
                let any = Any::pack(&ts, Timestamp::TYPE_URL);
                let json = serde_json::to_string(&any).unwrap();
                let decoded: Any = serde_json::from_str(&json).unwrap();
                let decoded_ts: Timestamp = decoded.unpack_unchecked().unwrap();
                assert_eq!(decoded_ts, ts);
            });
        }

        #[test]
        fn nested_any_roundtrip() {
            with_registry(|| {
                let dur = Duration::from_secs(42);
                let inner_any = Any::pack(&dur, Duration::TYPE_URL);
                let outer_any = Any::pack(&inner_any, Any::TYPE_URL);

                let json = serde_json::to_string(&outer_any).unwrap();
                let decoded_outer: Any = serde_json::from_str(&json).unwrap();
                let decoded_inner: Any = decoded_outer.unpack_unchecked().unwrap();
                let decoded_dur: Duration = decoded_inner.unpack_unchecked().unwrap();
                assert_eq!(decoded_dur.seconds, 42);
            });
        }

        #[test]
        fn fallback_base64_without_registry() {
            without_registry(|| {
                let any = Any {
                    type_url: "type.googleapis.com/unknown.Type".into(),
                    value: vec![0x08, 0x96, 0x01],
                    ..Default::default()
                };
                let json = serde_json::to_string(&any).unwrap();
                assert!(json.contains("@type"));
                assert!(json.contains("value"));

                let decoded: Any = serde_json::from_str(&json).unwrap();
                assert_eq!(decoded.type_url, any.type_url);
                assert_eq!(decoded.value, any.value);
            });
        }

        #[test]
        fn deserialize_missing_type_returns_default() {
            let json = r#"{}"#;
            let any: Any = serde_json::from_str(json).unwrap();
            assert_eq!(any, Any::default());
        }

        #[test]
        fn fallback_base64_with_registry_but_unknown_type() {
            with_registry(|| {
                let any = Any {
                    type_url: "type.googleapis.com/unknown.Type".into(),
                    value: vec![0x08, 0x96, 0x01],
                    ..Default::default()
                };
                let json = serde_json::to_string(&any).unwrap();
                let decoded: Any = serde_json::from_str(&json).unwrap();
                assert_eq!(decoded.type_url, any.type_url);
                assert_eq!(decoded.value, any.value);
            });
        }

        #[test]
        fn deserialize_rejects_empty_type_url() {
            let json = r#"{"@type": "", "value": ""}"#;
            let err = serde_json::from_str::<Any>(json).unwrap_err();
            assert!(err.to_string().contains("valid type URL"), "{err}");
        }

        #[test]
        fn deserialize_rejects_type_url_without_slash() {
            let json = r#"{"@type": "not_a_url", "value": ""}"#;
            let err = serde_json::from_str::<Any>(json).unwrap_err();
            assert!(err.to_string().contains("valid type URL"), "{err}");
        }

        // ── Non-WKT registered type (fields inlined at top level) ─────
        // WKTs use {"@type": ..., "value": <json>} wrapping.
        // Regular messages use {"@type": ..., "field1": ..., "field2": ...}.
        // Previously only the WKT path was tested.

        /// Hand-written to_json: decode the Any bytes as a single varint
        /// field (number=1), return it as a JSON object {"id": N}.
        fn user_type_to_json(bytes: &[u8]) -> Result<serde_json::Value, String> {
            use buffa::encoding::Tag;
            let mut cur = bytes;
            let mut id = 0i64;
            while !cur.is_empty() {
                let tag = Tag::decode(&mut cur).map_err(|e| e.to_string())?;
                if tag.field_number() == 1 {
                    id =
                        buffa::encoding::decode_varint(&mut cur).map_err(|e| e.to_string())? as i64;
                } else {
                    buffa::encoding::skip_field(tag, &mut cur).map_err(|e| e.to_string())?;
                }
            }
            Ok(serde_json::json!({ "id": id }))
        }

        /// Hand-written from_json: extract {"id": N}, encode as varint field 1.
        fn user_type_from_json(value: serde_json::Value) -> Result<alloc::vec::Vec<u8>, String> {
            use buffa::encoding::{encode_varint, Tag, WireType};
            let id = value
                .get("id")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| "missing or invalid 'id' field".to_string())?;
            let mut buf = alloc::vec::Vec::new();
            Tag::new(1, WireType::Varint).encode(&mut buf);
            encode_varint(id as u64, &mut buf);
            Ok(buf)
        }

        fn with_user_type_registry<R>(f: impl FnOnce() -> R) -> R {
            use buffa::any_registry::AnyTypeEntry;
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut registry = AnyRegistry::new();
            // Register as NON-WKT (is_wkt=false) — fields inline at top level.
            registry.register(AnyTypeEntry {
                type_url: "type.example.com/user.Thing",
                to_json: user_type_to_json,
                from_json: user_type_from_json,
                is_wkt: false,
            });
            set_any_registry(Box::new(registry));
            let result = f();
            clear_any_registry();
            result
        }

        #[test]
        fn serialize_non_wkt_inlines_fields() {
            with_user_type_registry(|| {
                // Encode {id: 42} as proto wire bytes.
                let any = Any {
                    type_url: "type.example.com/user.Thing".into(),
                    // field 1, varint 42: tag=0x08, value=0x2A
                    value: vec![0x08, 0x2A],
                    ..Default::default()
                };

                let json = serde_json::to_value(&any).unwrap();
                // Non-WKT format: fields at top level alongside @type.
                assert_eq!(json["@type"], "type.example.com/user.Thing");
                assert_eq!(json["id"], 42);
                // Should NOT have a "value" wrapper key.
                assert!(
                    json.get("value").is_none(),
                    "non-WKT should not use 'value' wrapping: {json}"
                );
            });
        }

        #[test]
        fn deserialize_non_wkt_from_inlined_fields() {
            with_user_type_registry(|| {
                let json = r#"{
                    "@type": "type.example.com/user.Thing",
                    "id": 99
                }"#;
                let any: Any = serde_json::from_str(json).unwrap();
                assert_eq!(any.type_url, "type.example.com/user.Thing");
                // Verify the from_json encoded it back to wire bytes.
                assert_eq!(any.value, vec![0x08, 99]);
            });
        }

        #[test]
        fn non_wkt_round_trip() {
            with_user_type_registry(|| {
                let original = Any {
                    type_url: "type.example.com/user.Thing".into(),
                    value: vec![0x08, 0x07], // id=7
                    ..Default::default()
                };
                let json = serde_json::to_string(&original).unwrap();
                let decoded: Any = serde_json::from_str(&json).unwrap();
                assert_eq!(decoded.type_url, original.type_url);
                assert_eq!(decoded.value, original.value);
            });
        }

        #[test]
        fn serialize_non_wkt_rejects_non_object_json() {
            // If to_json for a non-WKT type returns something other than a
            // JSON object, serialization must fail (can't inline non-object
            // fields alongside @type).
            use buffa::any_registry::AnyTypeEntry;
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut registry = AnyRegistry::new();
            registry.register(AnyTypeEntry {
                type_url: "type.example.com/user.BadType",
                to_json: |_bytes| Ok(serde_json::Value::Number(42.into())),
                from_json: |_v| Ok(alloc::vec::Vec::new()),
                is_wkt: false,
            });
            set_any_registry(Box::new(registry));

            let any = Any {
                type_url: "type.example.com/user.BadType".into(),
                value: vec![],
                ..Default::default()
            };
            let result = serde_json::to_string(&any);
            clear_any_registry();
            assert!(result.is_err(), "expected error for non-object to_json");
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("must return a JSON object"),
                "wrong error message"
            );
        }

        #[test]
        fn deserialize_rejects_non_string_type() {
            // @type as a non-string value → error.
            let json = r#"{"@type": 123}"#;
            let err = serde_json::from_str::<Any>(json).unwrap_err();
            assert!(err.to_string().contains("@type must be a string"), "{err}");
        }
    }
}
