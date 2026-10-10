//! Ergonomic helpers for [`google::protobuf::Any`](crate::google::protobuf::Any).

use alloc::string::String;

use crate::google::protobuf::Any;

impl Any {
    /// Pack a message into an [`Any`] with the given type URL.
    ///
    /// The type URL is conventionally of the form
    /// `type.googleapis.com/fully.qualified.TypeName`, but this method does
    /// not enforce that convention — any string is accepted.
    ///
    /// # Panics
    ///
    /// Panics if `msg`'s encoded size exceeds the 2 GiB protobuf limit
    /// ([`buffa::MAX_MESSAGE_BYTES`]) — see [`try_pack`](Self::try_pack)
    /// for the error-returning variant.
    pub fn pack(msg: &impl buffa::Message, type_url: impl Into<String>) -> Self {
        Self {
            type_url: type_url.into(),
            value: msg.encode_to_bytes(),
            ..Default::default()
        }
    }

    /// Pack a message into an [`Any`], returning an error instead of
    /// panicking if the message's encoded size exceeds the 2 GiB protobuf
    /// limit ([`buffa::MAX_MESSAGE_BYTES`]).
    ///
    /// # Errors
    ///
    /// Returns [`buffa::EncodeError::MessageTooLarge`] if the encoded size
    /// exceeds the limit.
    pub fn try_pack(
        msg: &impl buffa::Message,
        type_url: impl Into<String>,
    ) -> Result<Self, buffa::EncodeError> {
        Ok(Self {
            type_url: type_url.into(),
            value: msg.try_encode_to_bytes()?,
            ..Default::default()
        })
    }

    /// Pack a message into an [`Any`] under the message's own type URL,
    /// [`MessageName::TYPE_URL`](buffa::MessageName::TYPE_URL).
    ///
    /// [`unpack_message`](Self::unpack_message) finds the result only if
    /// `TYPE_URL` ends in `/` followed by
    /// [`FULL_NAME`](buffa::MessageName::FULL_NAME), as it does in every
    /// generated impl. Use [`pack`](Self::pack) to store a URL with a
    /// different prefix, or to pack a hand-written
    /// [`Message`](buffa::Message) that does not implement
    /// [`MessageName`](buffa::MessageName).
    ///
    /// # Panics
    ///
    /// Panics if the encoded size of `msg` exceeds the 2 GiB protobuf limit
    /// ([`buffa::MAX_MESSAGE_BYTES`]) — see
    /// [`try_pack_message`](Self::try_pack_message) for the error-returning
    /// variant.
    pub fn pack_message<T>(msg: &T) -> Self
    where
        T: buffa::Message + buffa::MessageName,
    {
        Self::pack(msg, <T as buffa::MessageName>::TYPE_URL)
    }

    /// Pack a message into an [`Any`] under the message's own type URL, as
    /// [`pack_message`](Self::pack_message) does.
    ///
    /// Returns an error for a message over the 2 GiB protobuf limit, where
    /// [`pack_message`](Self::pack_message) panics.
    ///
    /// # Errors
    ///
    /// Returns [`buffa::EncodeError::MessageTooLarge`] if the encoded size
    /// exceeds the limit.
    pub fn try_pack_message<T>(msg: &T) -> Result<Self, buffa::EncodeError>
    where
        T: buffa::Message + buffa::MessageName,
    {
        Self::try_pack(msg, <T as buffa::MessageName>::TYPE_URL)
    }

    /// Unpack the contained message, decoding its bytes as `T`, **without
    /// checking the `type_url`**.
    ///
    /// This method always attempts to decode the payload as `T` regardless
    /// of whether `type_url` actually identifies `T`. Use
    /// [`unpack_message`](Self::unpack_message) or
    /// [`unpack_if`](Self::unpack_if) to check the stored type before
    /// decoding.
    ///
    /// # Errors
    ///
    /// Returns a [`buffa::DecodeError`] if the bytes cannot be decoded as `T`.
    pub fn unpack_unchecked<T: buffa::Message>(&self) -> Result<T, buffa::DecodeError> {
        T::decode(&mut self.value.as_ref())
    }

    /// Unpack the contained message as `T`, but only if `type_url` is
    /// exactly `expected_type_url`, prefix included.
    ///
    /// Most callers want [`unpack_message`](Self::unpack_message), which
    /// accepts any prefix; use this method when the prefix itself carries
    /// meaning. Returns `Ok(None)` when the type URL does not match.
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
        T::decode(&mut self.value.as_ref()).map(Some)
    }

    /// Unpack the contained message as `T`, but only if the type URL
    /// identifies `T` under any prefix ([`is_message`](Self::is_message)).
    ///
    /// Returns `Ok(None)` when the message name does not match.
    ///
    /// `T` is an owned message. For a view, check `is_message::<FooView>()`
    /// and decode `any.value` with
    /// [`MessageView::decode_view`](buffa::MessageView::decode_view).
    ///
    /// # Examples
    ///
    /// ```
    /// use buffa_types::google::protobuf::{Any, Duration, Timestamp};
    ///
    /// let any = Any::pack_message(&Duration::from_secs(5));
    /// assert_eq!(any.unpack_message::<Duration>()?, Some(Duration::from_secs(5)));
    /// assert_eq!(any.unpack_message::<Timestamp>()?, None);
    /// # Ok::<(), buffa::DecodeError>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a [`buffa::DecodeError`] if the message name matches but the
    /// bytes cannot be decoded as `T`.
    pub fn unpack_message<T>(&self) -> Result<Option<T>, buffa::DecodeError>
    where
        T: buffa::Message + buffa::MessageName,
    {
        if !self.is_message::<T>() {
            return Ok(None);
        }
        T::decode(&mut self.value.as_ref()).map(Some)
    }

    /// Returns `true` if the `type_url` of this [`Any`] is exactly `type_url`,
    /// prefix included.
    ///
    /// Most callers want [`is_message`](Self::is_message), which accepts any
    /// prefix; use this method when the prefix itself carries meaning.
    pub fn is_type(&self, type_url: &str) -> bool {
        self.type_url == type_url
    }

    /// Returns the type name in the type URL of this [`Any`]: the text after
    /// the last `/`.
    ///
    /// For an [`Any`] packed by [`pack_message`](Self::pack_message), the
    /// type name is the message's
    /// [`MessageName::FULL_NAME`](buffa::MessageName::FULL_NAME). The text is
    /// not checked to be a valid message name. The JSON and text registries
    /// find a message by this name when its exact URL is not registered.
    ///
    /// Returns `None` when the URL has no `/`, or when the text after the
    /// last `/` is empty.
    ///
    /// To test for one generated type, use [`is_message`](Self::is_message).
    /// For a name that is known only at run time, compare the result:
    /// `any.type_name() == Some(name)`.
    ///
    /// # Examples
    ///
    /// Dispatch on the packed type:
    ///
    /// ```
    /// use buffa::MessageName;
    /// use buffa_types::google::protobuf::{Any, Duration, Timestamp};
    ///
    /// fn describe(any: &Any) -> &'static str {
    ///     match any.type_name() {
    ///         Some(Timestamp::FULL_NAME) => "a timestamp",
    ///         Some(Duration::FULL_NAME) => "a duration",
    ///         Some(_) => "another message",
    ///         None => "not a type URL",
    ///     }
    /// }
    ///
    /// let any = Any::pack(&Duration::default(), "example.com/v1/google.protobuf.Duration");
    /// assert_eq!(any.type_name(), Some("google.protobuf.Duration"));
    /// assert_eq!(describe(&any), "a duration");
    ///
    /// let no_slash = Any::pack(&Duration::default(), "google.protobuf.Duration");
    /// assert_eq!(no_slash.type_name(), None);
    /// assert_eq!(describe(&no_slash), "not a type URL");
    /// ```
    pub fn type_name(&self) -> Option<&str> {
        let (_, type_name) = self.type_url.rsplit_once('/')?;
        (!type_name.is_empty()).then_some(type_name)
    }

    /// Returns `true` if the type URL of this [`Any`] identifies `T` under
    /// any prefix: the name after the last `/` is the
    /// [`MessageName::FULL_NAME`](buffa::MessageName::FULL_NAME) of `T`.
    ///
    /// The prefix is not compared. Returns `false` when
    /// [`type_name`](Self::type_name) is `None`.
    pub fn is_message<T: buffa::MessageName>(&self) -> bool {
        self.type_name() == Some(<T as buffa::MessageName>::FULL_NAME)
    }

    /// Returns the type URL stored in this [`Any`].
    pub fn type_url(&self) -> &str {
        &self.type_url
    }
}

// ── WKT type registry ───────────────────────────────────────────────────────

/// Registers all well-known types with the given [`TypeRegistry`].
///
/// This registers Duration, Timestamp, FieldMask, Value, Struct, ListValue,
/// Empty, all wrapper types, Any itself, and the remaining official WKTs
/// (`Api`, `Method`, `Mixin`, `Type`, `Field`, `Enum`, `EnumValue`, `Option`,
/// `SourceContext`), enabling both proto3-compliant
/// JSON serialization (under the `json` feature) and textproto
/// `[type_url] { fields }` Any-expansion when these types appear inside
/// `google.protobuf.Any` fields.
///
/// Text entries are always registered (buffa-types unconditionally enables
/// `buffa/text`). JSON entries are registered under the `json` feature.
///
/// # Example
///
/// ```rust,no_run
/// use buffa::type_registry::{TypeRegistry, set_type_registry};
///
/// let mut reg = TypeRegistry::new();
/// buffa_types::register_wkt_types(&mut reg);
/// set_type_registry(reg);
/// ```
///
/// [`TypeRegistry`]: buffa::type_registry::TypeRegistry
pub fn register_wkt_types(reg: &mut buffa::type_registry::TypeRegistry) {
    use crate::google::protobuf::*;
    use buffa::type_registry::{any_encode_text, any_merge_text, TextAnyEntry};

    macro_rules! register_type {
        ($type:ty, $wkt:expr) => {
            #[cfg(feature = "json")]
            {
                use alloc::string::ToString;
                reg.register_json_any(buffa::type_registry::JsonAnyEntry {
                    type_url: <$type>::TYPE_URL,
                    to_json: |bytes| {
                        let msg = <$type as buffa::Message>::decode(&mut &*bytes)
                            .map_err(|e| e.to_string())?;
                        serde_json::to_value(&msg).map_err(|e| e.to_string())
                    },
                    from_json: |value| {
                        let msg: $type =
                            serde_json::from_value(value).map_err(|e| e.to_string())?;
                        buffa::Message::try_encode_to_vec(&msg).map_err(|e| e.to_string())
                    },
                    is_wkt: $wkt,
                });
            }
            // WKTs all implement TextFormat (generate_text is on for
            // buffa-types). Non-Option fn-ptrs — presence in the text map
            // means text-capable. `$wkt` is irrelevant here: textproto has
            // no `"value"` wrapping distinction.
            reg.register_text_any(TextAnyEntry {
                type_url: <$type>::TYPE_URL,
                text_encode: any_encode_text::<$type>,
                text_merge: any_merge_text::<$type>,
            });
        };
    }
    macro_rules! register_text_only {
        ($type:ty) => {
            reg.register_text_any(TextAnyEntry {
                type_url: <$type>::TYPE_URL,
                text_encode: any_encode_text::<$type>,
                text_merge: any_merge_text::<$type>,
            });
        };
    }

    // WKTs with special JSON mappings (use "value" wrapping in Any JSON).
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

    // Regular messages (fields inlined in Any JSON). Empty has a
    // hand-written JSON impl; Api/Type/SourceContext and their parts have
    // generated text impls and no serde impls, so they register for
    // textproto Any-expansion without a JSON entry.
    register_type!(Empty, false);

    register_text_only!(Api);
    register_text_only!(Method);
    register_text_only!(Mixin);
    register_text_only!(Type);
    register_text_only!(Field);
    register_text_only!(Enum);
    register_text_only!(EnumValue);
    register_text_only!(crate::google::protobuf::Option);
    register_text_only!(SourceContext);
}

// ── TextFormat impl ─────────────────────────────────────────────────────────
//
// Hand-written because textproto packs `Any` as `[type_url] { fields }` when
// the type is registered — a shape the generated field-by-field impl can't
// produce. Codegen's `impl_text.rs` skips `google.protobuf.Any` to avoid a
// conflicting impl.
//
// `try_write_any_expanded` and `read_any_expansion` consult the text-format
// Any map (installed via `set_type_registry`). When no registry is installed,
// this degrades to the vanilla `type_url: "..." value: "..."` form — still
// valid textproto, just not the expanded form.

impl buffa::text::TextFormat for Any {
    fn encode_text(&self, enc: &mut buffa::text::TextEncoder<'_>) -> core::fmt::Result {
        if !self.type_url.is_empty() && enc.try_write_any_expanded(&self.type_url, &self.value)? {
            return Ok(());
        }
        // Vanilla fallback: unregistered type, or no registry installed.
        if !self.type_url.is_empty() {
            enc.write_field_name("type_url")?;
            enc.write_string(&self.type_url)?;
        }
        if !self.value.is_empty() {
            enc.write_field_name("value")?;
            enc.write_bytes(&self.value)?;
        }
        Ok(())
    }

    fn merge_text(
        &mut self,
        dec: &mut buffa::text::TextDecoder<'_>,
    ) -> Result<(), buffa::text::ParseError> {
        while let Some(name) = dec.read_field_name()? {
            match name {
                "type_url" => self.type_url = dec.read_string()?.into_owned(),
                "value" => self.value = dec.read_bytes()?.into(),
                _ if name.starts_with('[') => {
                    let (url, bytes) = dec.read_any_expansion(name)?;
                    self.type_url = url.into_owned();
                    self.value = bytes.into();
                }
                _ => return Err(dec.unknown_field()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod text_tests {
    use super::Any;
    use buffa::text::{decode_from_str, encode_to_string, ParseErrorKind};

    #[test]
    fn vanilla_roundtrip_no_registry() {
        // Without a registry installed, Any uses the plain
        // `type_url: "..." value: "..."` form — exactly what the old
        // generated impl did.
        let orig = Any {
            type_url: "type.example.com/Foo".into(),
            value: alloc::vec![0x08, 0x2A].into(), // field 1 = varint 42
            ..Default::default()
        };
        let text = encode_to_string(&orig);
        assert_eq!(text, r#"type_url: "type.example.com/Foo" value: "\010*""#);
        let back: Any = decode_from_str(&text).unwrap();
        assert_eq!(back.type_url, orig.type_url);
        assert_eq!(back.value, orig.value);
    }

    #[test]
    fn unknown_fields_rejected_by_default() {
        let err = decode_from_str::<Any>(r#"type_urll: "type.example.com/Foo""#).unwrap_err();
        assert_eq!(err.kind, ParseErrorKind::UnknownField);
    }

    // Registry-manipulating tests live in `serde_tests` below — they share
    // the same global `AtomicPtr` as the JSON tests and must use the same
    // `REGISTRY_LOCK` to serialize.
}

// ── serde impls ──────────────────────────────────────────────────────────────
//
// Proto3 JSON for `Any` uses the global `AnyRegistry` to serialize the
// embedded message with its fields inline (regular messages) or wrapped in a
// `"value"` key (WKTs). When the registry is absent, or neither the type URL
// nor its message full name is registered, the payload is written as base64
// under `"value"`.

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
                // `to_json` decodes the payload and serializes it, so an `Any`
                // holding an `Any` re-enters this impl. See
                // `MAX_ANY_EXPANSION_DEPTH` for why the decoder's recursion
                // limit does not bound that.
                let Some(_depth_guard) = buffa::type_registry::enter_any_expansion() else {
                    return Err(serde::ser::Error::custom(alloc::format!(
                        "Any expansion nested deeper than {} levels",
                        buffa::type_registry::MAX_ANY_EXPANSION_DEPTH
                    )));
                };
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
        // `BufferedObject`, never `serde_json::Map`'s own `Deserialize`:
        // see `buffa::json_helpers::buffered`.
        let buffa::json_helpers::buffered::BufferedObject(mut obj) =
            serde::Deserialize::deserialize(d)?;

        let type_url = match obj.remove("@type") {
            Some(serde_json::Value::String(s)) => s,
            Some(_) => {
                return Err(serde::de::Error::custom("@type must be a string"));
            }
            None if obj.is_empty() => return Ok(Self::default()),
            None => {
                return Err(serde::de::Error::custom(
                    "Any object missing string \"@type\"",
                ));
            }
        };

        // The type URL must be non-empty, contain a '/', and have a non-empty
        // fully-qualified type name after the final slash (e.g.
        // "type.googleapis.com/google.protobuf.Duration").
        let type_name = type_url.rsplit('/').next().unwrap_or("");
        if type_url.is_empty() || !type_url.contains('/') || type_name.is_empty() {
            return Err(serde::de::Error::custom(
                "@type must be a valid type URL containing a '/' and a non-empty type name (e.g. type.googleapis.com/pkg.Type)",
            ));
        }

        let (registry_installed, lookup) = buffa::any_registry::with_any_registry(|reg| {
            let entry = reg.and_then(|r| r.lookup(&type_url));
            (reg.is_some(), entry.map(|e| (e.from_json, e.is_wkt)))
        });

        let value = match lookup {
            Some((from_json, true)) => {
                let json_val = obj.remove("value").ok_or_else(|| {
                    serde::de::Error::custom(alloc::format!(
                        "Any with WKT type {type_url:?} requires a \"value\" key"
                    ))
                })?;
                from_json(json_val).map_err(serde::de::Error::custom)?
            }
            Some((from_json, false)) => {
                let json_obj = serde_json::Value::Object(obj);
                from_json(json_obj).map_err(serde::de::Error::custom)?
            }
            None => {
                // The type has no JSON entry, so the message's own JSON cannot
                // be read. What parses is the encoded message as base64 under
                // "value", which is what `Serialize` writes for such a type; a
                // missing or null "value" is an empty payload. Any other key
                // is a field of the message and an error, whatever the
                // surrounding message does with unknown keys.
                let opaque = |problem: core::fmt::Arguments<'_>, alternative: &str| -> D::Error {
                    if registry_installed {
                        serde::de::Error::custom(format_args!(
                            "Any: type {type_url:?} has no JSON entry in the type registry, so its {problem}; register the message (generated `register_types` or `TypeRegistry::register_json_any`){alternative}"
                        ))
                    } else {
                        serde::de::Error::custom(format_args!(
                            "Any: no type registry is installed to resolve type {type_url:?}, so its {problem}; install one that registers the message with `set_type_registry`{alternative}"
                        ))
                    }
                };
                const OR_BASE64: &str = ", or send the message encoded as base64 under \"value\"";
                const FOR_JSON_FORM: &str = " to parse its JSON form";

                let payload = obj.remove("value");
                if let Some(key) = obj.keys().next() {
                    return Err(opaque(
                        format_args!("field {key:?} cannot be parsed"),
                        OR_BASE64,
                    ));
                }
                match payload {
                    Some(serde_json::Value::String(s)) => buffa::json_helpers::bytes::deserialize(
                        serde::de::value::StringDeserializer::<D::Error>::new(s),
                    )
                    .map_err(|e| {
                        // The decoder's message ends in a full stop.
                        let e = alloc::string::ToString::to_string(&e);
                        let e = e.trim_end_matches('.');
                        opaque(
                            format_args!("\"value\" must be a base64 string: {e}"),
                            FOR_JSON_FORM,
                        )
                    })?,
                    None | Some(serde_json::Value::Null) => alloc::vec::Vec::new(),
                    Some(_) => {
                        return Err(opaque(
                            format_args!("\"value\" must be a base64 string"),
                            FOR_JSON_FORM,
                        ));
                    }
                }
            }
        };

        Ok(Self {
            type_url,
            value: value.into(),
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
    fn any_view_to_owned_from_source_is_zero_copy() {
        use crate::google::protobuf::__buffa::view::AnyView;
        use buffa::view::{MessageView as _, OwnedView};

        let src = Any {
            type_url: "type.googleapis.com/x".into(),
            value: bytes::Bytes::from_static(&[1u8; 256]),
            ..Default::default()
        };
        let buf = bytes::Bytes::from(src.encode_to_vec());

        // Direct trait path: to_owned_from_source(Some(&buf)) → slice_ref.
        let view = AnyView::decode_view(&buf).unwrap();
        let owned = view.to_owned_from_source(Some(&buf)).unwrap();
        assert_eq!(owned.value, src.value);
        let value_ptr = owned.value.as_ptr() as usize;
        let buf_range = (buf.as_ptr() as usize)..(buf.as_ptr() as usize + buf.len());
        assert!(
            buf_range.contains(&value_ptr),
            "owned.value should point into buf (slice_ref), got {value_ptr:#x} outside {buf_range:#x?}"
        );

        // OwnedView path: the inherent OwnedView::to_owned_message routes
        // through to_owned_from_source(Some(&self.bytes)), so the bytes field
        // is a zero-copy slice_ref into the retained buffer.
        let ov = OwnedView::<AnyView<'static>>::decode(buf.clone()).unwrap();
        let owned2 = ov.to_owned_message();
        assert_eq!(owned2.value, src.value);
        assert!(buf_range.contains(&(owned2.value.as_ptr() as usize)));

        // No-source path still copies (correct, distinct allocation).
        let copied = view.to_owned_message().unwrap();
        assert_eq!(copied.value, src.value);
        assert!(!buf_range.contains(&(copied.value.as_ptr() as usize)));
    }

    #[cfg(feature = "arbitrary")]
    #[test]
    fn any_arbitrary_with_bytes_value() {
        use arbitrary::{Arbitrary, Unstructured};
        // Regression pin for https://github.com/anthropics/buffa/issues/88:
        // Any.value is bytes::Bytes (not Vec<u8>), so derive(Arbitrary) on Any
        // requires the ::buffa::__private::arbitrary_bytes shim.
        let raw = [0u8; 64];
        let mut u = Unstructured::new(&raw);
        let any = Any::arbitrary(&mut u).unwrap();
        let _ = any.value.slice(..);
    }

    /// Test double whose `compute_size` reports over the 2 GiB limit and
    /// whose `write_to` writes nothing — exercises `pack`'s guard without
    /// materializing gigabytes. Mirrors buffa's crate-internal
    /// `test_doubles::SizedMsg` (`#[cfg(test)]` items don't cross the crate
    /// boundary).
    #[derive(Clone, Default, PartialEq, Debug)]
    struct HugeMsg;

    impl buffa::MessageName for HugeMsg {
        const PACKAGE: &'static str = "test";
        const NAME: &'static str = "HugeMsg";
        const FULL_NAME: &'static str = "test.HugeMsg";
        const TYPE_URL: &'static str = "type.googleapis.com/test.HugeMsg";
    }

    impl buffa::DefaultInstance for HugeMsg {
        fn default_instance() -> &'static Self {
            static INST: buffa::__private::OnceBox<HugeMsg> = buffa::__private::OnceBox::new();
            INST.get_or_init(|| alloc::boxed::Box::new(HugeMsg))
        }
    }

    impl buffa::Message for HugeMsg {
        fn compute_size(&self, _cache: &mut buffa::SizeCache) -> u32 {
            buffa::MAX_MESSAGE_BYTES + 1
        }
        fn write_to(&self, _cache: &mut buffa::SizeCache, _buf: &mut impl buffa::EncodeSink) {}
        fn merge_field(
            &mut self,
            tag: buffa::encoding::Tag,
            buf: &mut impl bytes::Buf,
            _ctx: buffa::DecodeContext<'_>,
        ) -> Result<(), buffa::DecodeError> {
            buffa::encoding::skip_field(tag, buf)?;
            Ok(())
        }
        fn clear(&mut self) {}
    }

    #[test]
    fn try_pack_over_limit_errs() {
        assert_eq!(
            Any::try_pack(&HugeMsg, "type.googleapis.com/x"),
            Err(buffa::EncodeError::MessageTooLarge)
        );
    }

    #[test]
    #[should_panic(expected = "2 GiB protobuf limit")]
    fn pack_over_limit_panics() {
        let _ = Any::pack(&HugeMsg, "type.googleapis.com/x");
    }

    #[test]
    fn try_pack_matches_pack_for_normal_messages() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let url = "type.googleapis.com/google.protobuf.Timestamp";
        assert_eq!(Any::try_pack(&ts, url).unwrap(), Any::pack(&ts, url));
    }

    #[test]
    fn pack_message_uses_the_generated_type_url() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let any = Any::pack_message(&ts);

        assert_eq!(any.type_url(), <Timestamp as buffa::MessageName>::TYPE_URL);
        assert_eq!(any.unpack_message::<Timestamp>().unwrap(), Some(ts));
    }

    #[test]
    fn try_pack_message_matches_pack_message() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };

        assert_eq!(Any::try_pack_message(&ts).unwrap(), Any::pack_message(&ts));
    }

    #[test]
    fn try_pack_message_over_limit_errs() {
        assert_eq!(
            Any::try_pack_message(&HugeMsg),
            Err(buffa::EncodeError::MessageTooLarge)
        );
    }

    #[test]
    #[should_panic(expected = "2 GiB protobuf limit")]
    fn pack_message_over_limit_panics() {
        let _ = Any::pack_message(&HugeMsg);
    }

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
    fn type_name_uses_the_segment_after_the_last_slash() {
        let any = Any::pack(
            &Timestamp::default(),
            "custom.example/v1/google.protobuf.Timestamp",
        );
        assert_eq!(any.type_name(), Some("google.protobuf.Timestamp"));
    }

    #[test]
    fn type_name_is_none_without_a_non_empty_final_segment() {
        for type_url in ["", "/", "google.protobuf.Timestamp", "custom.example/v1/"] {
            let any = Any::pack(&Timestamp::default(), type_url);
            assert_eq!(any.type_name(), None, "{type_url:?}");
        }
    }

    #[test]
    fn type_name_accepts_an_empty_prefix() {
        let any = Any::pack(&Timestamp::default(), "/google.protobuf.Timestamp");
        assert_eq!(any.type_name(), Some("google.protobuf.Timestamp"));
        assert!(any.is_message::<Timestamp>());
    }

    #[test]
    fn is_message_compares_the_whole_name_not_a_suffix() {
        let any = Any::pack(&Timestamp::default(), "x/my.google.protobuf.Timestamp");
        assert_eq!(any.type_name(), Some("my.google.protobuf.Timestamp"));
        assert!(!any.is_message::<Timestamp>());
        assert_eq!(any.unpack_message::<Timestamp>().unwrap(), None);
    }

    #[test]
    fn is_message_accepts_a_view_type() {
        use crate::google::protobuf::TimestampView;
        use buffa::MessageView;

        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let any = Any::pack_message(&ts);

        assert!(any.is_message::<TimestampView<'_>>());
        let view = TimestampView::decode_view(&any.value).unwrap();
        assert_eq!(view.seconds, 42);
    }

    #[test]
    fn unpack_message_accepts_custom_prefix() {
        let ts = Timestamp {
            seconds: 42,
            ..Default::default()
        };
        let any = Any::pack(&ts, "custom.example/v1/google.protobuf.Timestamp");

        assert!(any.is_message::<Timestamp>());
        assert_eq!(any.unpack_message::<Timestamp>().unwrap(), Some(ts));
    }

    #[test]
    fn unpack_message_returns_none_for_other_types_and_malformed_urls() {
        use crate::google::protobuf::Duration;

        let any = Any::pack(
            &Timestamp::default(),
            "custom.example/v1/google.protobuf.Timestamp",
        );
        assert!(!any.is_message::<Duration>());
        assert_eq!(any.unpack_message::<Duration>().unwrap(), None);

        for type_url in ["google.protobuf.Timestamp", "custom.example/v1/"] {
            let malformed = Any::pack(&Timestamp::default(), type_url);
            assert!(!malformed.is_message::<Timestamp>(), "{type_url}");
            assert_eq!(
                malformed.unpack_message::<Timestamp>().unwrap(),
                None,
                "{type_url}"
            );
        }
    }

    #[test]
    fn unpack_message_reports_decode_errors_for_matching_names() {
        let any = Any {
            type_url: "custom.example/v1/google.protobuf.Timestamp".into(),
            value: bytes::Bytes::from_static(&[0x0f]),
            ..Default::default()
        };

        assert!(any.unpack_message::<Timestamp>().is_err());
    }

    #[test]
    fn clone_shares_payload_buffer() {
        let orig = Any {
            type_url: "type.googleapis.com/example.Msg".into(),
            value: alloc::vec![0xAB; 1024].into(),
            ..Default::default()
        };
        let dup = orig.clone();
        assert_eq!(orig.value.as_ptr(), dup.value.as_ptr());
        assert_eq!(orig.value.len(), dup.value.len());
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
        #[cfg(not(feature = "std"))]
        extern crate std;

        use super::*;
        use crate::google::protobuf::Duration;
        use alloc::{string::ToString, vec};
        use buffa::any_registry::clear_any_registry;
        use buffa::type_registry::{
            clear_text_registry, set_type_registry, TypeRegistry, MAX_ANY_EXPANSION_DEPTH,
        };

        /// Mutex to serialize tests that manipulate the global registries.
        /// Each test binary needs its own lock since #[cfg(test)] modules
        /// cannot be shared across crates.
        static REGISTRY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        fn with_registry<R>(f: impl FnOnce() -> R) -> R {
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut reg = TypeRegistry::new();
            register_wkt_types(&mut reg);
            set_type_registry(reg);
            let result = f();
            clear_any_registry();
            clear_text_registry();
            result
        }

        fn without_registry<R>(f: impl FnOnce() -> R) -> R {
            let _guard = REGISTRY_LOCK.lock().unwrap();
            clear_any_registry();
            clear_text_registry();
            f()
        }

        // ── TextFormat impl (Any expansion) ─────────────────────────────────
        //
        // Here rather than in `text_tests` because these manipulate the
        // same global `AtomicPtr` as the JSON tests above — both must
        // serialize on `REGISTRY_LOCK`.

        #[test]
        fn text_registry_roundtrip_wkt() {
            use crate::google::protobuf::Empty;
            use buffa::text::{decode_from_str, encode_to_string};
            with_registry(|| {
                // register_wkt_types installs Empty with text fn-ptrs.
                let any = Any::pack(&Empty::default(), Empty::TYPE_URL);
                let text = encode_to_string(&any);
                // Empty has no fields → `{}`.
                assert_eq!(text, "[type.googleapis.com/google.protobuf.Empty] {}");

                let back: Any = decode_from_str(&text).unwrap();
                assert_eq!(back.type_url, Empty::TYPE_URL);
                assert_eq!(back.value, alloc::vec::Vec::<u8>::new());
            });
        }

        #[test]
        fn text_registry_roundtrip_source_context() {
            use crate::google::protobuf::SourceContext;
            use buffa::text::{decode_from_str, encode_to_string};
            with_registry(|| {
                let sc = SourceContext {
                    file_name: "google/protobuf/api.proto".into(),
                    ..Default::default()
                };
                let any = Any::pack(&sc, SourceContext::TYPE_URL);
                let text = encode_to_string(&any);
                assert_eq!(
                    text,
                    r#"[type.googleapis.com/google.protobuf.SourceContext] {file_name: "google/protobuf/api.proto"}"#
                );

                let back: Any = decode_from_str(&text).unwrap();
                assert_eq!(back.type_url, SourceContext::TYPE_URL);
                let unpacked: SourceContext = back.unpack_unchecked().unwrap();
                assert_eq!(unpacked.file_name, sc.file_name);
            });
        }

        #[test]
        fn text_roundtrip_keeps_a_custom_type_url_prefix() {
            use crate::google::protobuf::SourceContext;
            use buffa::text::{decode_from_str, encode_to_string};
            with_registry(|| {
                // Registered as `type.googleapis.com/...`; found by its name.
                let type_url = "custom.example/v1/google.protobuf.SourceContext";
                let sc = SourceContext {
                    file_name: "a/b.proto".into(),
                    ..Default::default()
                };
                let any = Any::pack(&sc, type_url);
                let text = encode_to_string(&any);
                assert_eq!(
                    text,
                    r#"[custom.example/v1/google.protobuf.SourceContext] {file_name: "a/b.proto"}"#
                );

                let back: Any = decode_from_str(&text).unwrap();
                assert_eq!(back.type_url, type_url);
                assert_eq!(back.value, any.value);

                // Whitespace inside the brackets is not part of the URL.
                let spaced: Any = decode_from_str(
                    r#"[ custom.example / v1 / google.protobuf.SourceContext ] {file_name: "a/b.proto"}"#,
                )
                .unwrap();
                assert_eq!(spaced.type_url, type_url);
                assert_eq!(spaced.value, any.value);
            });
        }

        #[test]
        fn text_quotes_a_type_url_that_is_unsafe_between_brackets() {
            use crate::google::protobuf::Empty;
            use buffa::text::{decode_from_str, encode_to_string};
            with_registry(|| {
                // Each URL ends in a registered message name, so each resolves.
                // Written between brackets, the first would close the name
                // early and inject an `injected` field; the others would lose
                // a newline, a space or the text after `#` on the way back.
                for type_url in [
                    "a/b] {} injected: true [type.googleapis.com/google.protobuf.Empty",
                    "a\nb/google.protobuf.Empty",
                    "a b/google.protobuf.Empty",
                    "a#b/google.protobuf.Empty",
                ] {
                    let any = Any::pack(&Empty::default(), type_url);
                    let text = encode_to_string(&any);
                    assert!(text.starts_with("type_url: \""), "{text}");

                    let back: Any = decode_from_str(&text).unwrap();
                    assert_eq!(back.type_url, type_url);
                }
            });
        }

        #[test]
        fn text_registry_roundtrip_type_and_option() {
            // `Type` carries a repeated message and an enum; `Option` holds
            // an `Any`, so its expansion nests a second registered type.
            use crate::google::protobuf::{Field, SourceContext, Syntax, Type};
            use buffa::text::{decode_from_str, encode_to_string};
            with_registry(|| {
                let ty = Type {
                    name: "google.example.v1.Msg".into(),
                    fields: alloc::vec![Field {
                        name: "id".into(),
                        number: 1,
                        json_name: "id".into(),
                        ..Default::default()
                    }],
                    syntax: Syntax::SYNTAX_PROTO3.into(),
                    ..Default::default()
                };
                let any = Any::pack(&ty, Type::TYPE_URL);
                let text = encode_to_string(&any);
                assert!(
                    text.starts_with("[type.googleapis.com/google.protobuf.Type] {"),
                    "{text}"
                );
                let back: Any = decode_from_str(&text).unwrap();
                let unpacked: Type = back.unpack_unchecked().unwrap();
                assert_eq!(unpacked, ty);

                let sc = SourceContext {
                    file_name: "a/b.proto".into(),
                    ..Default::default()
                };
                let opt = crate::google::protobuf::Option {
                    name: "source".into(),
                    value: buffa::MessageField::some(Any::pack(&sc, SourceContext::TYPE_URL)),
                    ..Default::default()
                };
                let any = Any::pack(&opt, crate::google::protobuf::Option::TYPE_URL);
                let text = encode_to_string(&any);
                assert!(
                    text.contains("[type.googleapis.com/google.protobuf.SourceContext]"),
                    "nested Any must expand through the registry: {text}"
                );
                let back: Any = decode_from_str(&text).unwrap();
                let unpacked: crate::google::protobuf::Option = back.unpack_unchecked().unwrap();
                assert_eq!(unpacked, opt);
            });
        }

        #[test]
        fn text_unregistered_url_errors_on_decode() {
            use buffa::text::decode_from_str;
            // Registry installed but URL not in it — the
            // `AnyFieldWithInvalidType` conformance shape.
            with_registry(|| {
                let result: Result<Any, _> =
                    decode_from_str("[type.googleapis.com/unknown.Type] { x: 1 }");
                assert!(result.is_err(), "unknown URL should error, not skip");
            });
        }

        #[test]
        fn text_bracket_without_registry_errors() {
            use buffa::text::decode_from_str;
            // No registry at all → bracket name is a registry miss → error.
            without_registry(|| {
                let result: Result<Any, _> = decode_from_str("[type.example.com/Unknown] { x: 1 }");
                assert!(result.is_err());
            });
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
        fn serialize_value_wkt_requires_kind() {
            use crate::google::protobuf::Value;

            with_registry(|| {
                let unset = Any::pack_message(&Value::default());
                assert!(serde_json::to_string(&unset).is_err());

                let null = Any::pack_message(&Value::null());
                let json = serde_json::to_value(&null).unwrap();
                assert_eq!(
                    json,
                    serde_json::json!({ "@type": Value::TYPE_URL, "value": null })
                );
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

        const RAW_VALUE_KEY: &str = "$serde_json::private::RawValue";

        /// The tests of the private key also pass against
        /// `serde_json::Value`'s own `Deserialize` impl unless `serde_json`
        /// has `raw_value` on. It is a dev-dependency feature of this crate.
        #[test]
        fn the_test_build_has_serde_json_raw_value_enabled() {
            let text = serde_json::json!({ RAW_VALUE_KEY: "[1]" }).to_string();
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(value, serde_json::json!([1]));
        }

        #[test]
        fn a_payload_is_not_read_from_serde_jsons_private_key() {
            with_registry(|| {
                let plain = serde_json::json!({ "@type": Duration::TYPE_URL, "value": "1.5s" });
                serde_json::from_str::<Any>(&plain.to_string()).unwrap();

                let hidden = serde_json::json!({
                    "@type": Duration::TYPE_URL,
                    "value": { RAW_VALUE_KEY: "\"1.5s\"" },
                });
                serde_json::from_str::<Any>(&hidden.to_string())
                    .expect_err("an object is not a Duration");
            });
        }

        #[test]
        fn the_private_key_inside_a_payload_stays_data() {
            use crate::google::protobuf::{value::Kind, Value};
            with_registry(|| {
                // Read as `serde_json` reads it, the list would hold `[1]`.
                let json = serde_json::json!({
                    "@type": "type.googleapis.com/google.protobuf.Value",
                    "value": { "list": [{ RAW_VALUE_KEY: "[1]" }] },
                });
                let any: Any = serde_json::from_str(&json.to_string()).unwrap();
                let value: Value = any.unpack_unchecked().unwrap();
                let back = serde_json::to_value(&value).unwrap();
                assert_eq!(back, json["value"]);
                assert!(matches!(value.kind, Some(Kind::StructValue(_))));
            });
        }

        #[test]
        fn a_string_under_the_private_key_is_not_parsed() {
            with_registry(|| {
                // Parsing the string fails the recursion limit. As data it
                // is an object under `f`, which a type with no JSON entry
                // rejects by key, without reading the value.
                let deep = alloc::format!("{}0{}", "[".repeat(200), "]".repeat(200));
                let json = serde_json::json!({
                    "@type": "type.googleapis.com/no.such.Type",
                    "f": { RAW_VALUE_KEY: deep },
                });
                let err = serde_json::from_str::<Any>(&json.to_string()).unwrap_err();
                assert!(
                    err.to_string()
                        .contains("has no JSON entry in the type registry"),
                    "{err}"
                );
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

        const UNKNOWN_NO_REGISTRY: &str =
            r#"no type registry is installed to resolve type "type.googleapis.com/unknown.Type""#;
        const UNKNOWN_NO_ENTRY: &str =
            r#"type "type.googleapis.com/unknown.Type" has no JSON entry in the type registry"#;

        /// Runs `check` with no registry and with one that lacks
        /// `unknown.Type`, passing the cause the error names in each state.
        fn in_both_registry_states(check: impl Fn(&str)) {
            without_registry(|| check(UNKNOWN_NO_REGISTRY));
            with_registry(|| check(UNKNOWN_NO_ENTRY));
        }

        #[test]
        fn fallback_base64_rejects_non_string_payloads() {
            in_both_registry_states(|cause| {
                for value in ["123", "{}", "[]", "true", "false"] {
                    let json = alloc::format!(
                        r#"{{"@type":"type.googleapis.com/unknown.Type","value":{value}}}"#
                    );
                    let err = serde_json::from_str::<Any>(&json).unwrap_err().to_string();
                    assert!(
                        err.contains(cause) && err.contains("must be a base64 string;"),
                        "{json}: {err}"
                    );
                }
            });
        }

        #[test]
        fn fallback_base64_accepts_empty_and_valid_payloads() {
            in_both_registry_states(|cause| {
                for (json, expected) in [
                    (r#"{"@type":"type.googleapis.com/unknown.Type"}"#, &[][..]),
                    (
                        r#"{"@type":"type.googleapis.com/unknown.Type","value":null}"#,
                        &[][..],
                    ),
                    (
                        r#"{"@type":"type.googleapis.com/unknown.Type","value":""}"#,
                        &[][..],
                    ),
                    (
                        r#"{"@type":"type.googleapis.com/unknown.Type","value":"CJYB"}"#,
                        &[0x08, 0x96, 0x01][..],
                    ),
                    (
                        r#"{"value":"CJYB","@type":"type.googleapis.com/unknown.Type"}"#,
                        &[0x08, 0x96, 0x01][..],
                    ),
                    // A repeated key keeps its last value.
                    (
                        r#"{"@type":"type.googleapis.com/unknown.Type","value":123,"value":"CJYB"}"#,
                        &[0x08, 0x96, 0x01][..],
                    ),
                ] {
                    let any: Any = serde_json::from_str(json).unwrap();
                    assert_eq!(any.value.as_ref(), expected, "{json}");
                }
                // The base64 decoder's own error follows the cause.
                let json = r#"{"@type":"type.googleapis.com/unknown.Type","value":"!!!"}"#;
                let err = serde_json::from_str::<Any>(json).unwrap_err().to_string();
                assert!(
                    err.contains(cause) && err.contains("must be a base64 string: "),
                    "{err}"
                );
                assert!(!err.contains(".;"), "{err}");
            });
        }

        #[test]
        fn an_unregistered_type_rejects_every_key_but_value() {
            in_both_registry_states(|cause| {
                for json in [
                    // The expanded form of a message.
                    r#"{"@type":"type.googleapis.com/unknown.Type","name":"x"}"#,
                    r#"{"name":"x","@type":"type.googleapis.com/unknown.Type"}"#,
                    r#"{"@type":"type.googleapis.com/unknown.Type","name":null}"#,
                    // A field beside a payload that parses on its own.
                    r#"{"@type":"type.googleapis.com/unknown.Type","value":"CJYB","name":"x"}"#,
                    r#"{"@type":"type.googleapis.com/unknown.Type","value":null,"name":"x"}"#,
                ] {
                    let err = serde_json::from_str::<Any>(json).unwrap_err().to_string();
                    assert!(
                        err.contains(cause) && err.contains(r#"field "name" cannot be parsed"#),
                        "{json}: {err}"
                    );
                }
            });
        }

        #[test]
        fn a_wkt_in_json_form_without_a_registry_names_the_registry() {
            without_registry(|| {
                let json = alloc::format!(r#"{{"@type":"{}","value":"1.5s"}}"#, Duration::TYPE_URL);
                let err = serde_json::from_str::<Any>(&json).unwrap_err().to_string();
                assert!(
                    err.starts_with("Any: no type registry is installed")
                        && err.contains(r#""value" must be a base64 string: "#)
                        && err.contains("`set_type_registry`"),
                    "{err}"
                );
            });
        }

        #[test]
        fn a_type_with_only_a_text_entry_has_no_json_entry() {
            use crate::google::protobuf::SourceContext;
            with_registry(|| {
                let json = alloc::format!(
                    r#"{{"@type":"{}","fileName":"a.proto"}}"#,
                    SourceContext::TYPE_URL
                );
                let err = serde_json::from_str::<Any>(&json).unwrap_err().to_string();
                assert!(
                    err.contains("has no JSON entry in the type registry")
                        && err.contains("`register_types`"),
                    "{err}"
                );
            });
        }

        #[test]
        fn fallback_base64_without_registry() {
            without_registry(|| {
                let any = Any {
                    type_url: "type.googleapis.com/unknown.Type".into(),
                    value: vec![0x08, 0x96, 0x01].into(),
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
        fn deserialize_rejects_nonempty_object_without_type() {
            for json in [r#"{"value":""}"#, r#"{"unknown":1}"#] {
                let err = serde_json::from_str::<Any>(json).unwrap_err();
                assert!(
                    err.to_string()
                        .contains("Any object missing string \"@type\""),
                    "{json}: {err}"
                );
            }
        }

        #[test]
        fn fallback_base64_with_registry_but_unknown_type() {
            with_registry(|| {
                let any = Any {
                    type_url: "type.googleapis.com/unknown.Type".into(),
                    value: vec![0x08, 0x96, 0x01].into(),
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

        #[test]
        fn deserialize_accepts_arbitrary_type_url_prefix() {
            without_registry(|| {
                let json = r#"{"@type": "example.com/custom.Type", "value": "CAI="}"#;
                let any: Any = serde_json::from_str(json).unwrap();
                assert_eq!(any.type_url, "example.com/custom.Type");
                assert_eq!(any.value, vec![0x08, 0x02]);
            });
        }

        #[test]
        fn registered_any_accepts_arbitrary_type_url_prefix() {
            with_registry(|| {
                let type_url = "custom.example/v1/google.protobuf.Duration";
                let duration = Duration::from_secs_nanos(1, 500_000_000);
                let any = Any::pack(&duration, type_url);

                let json = serde_json::to_value(&any).unwrap();
                assert_eq!(json["@type"], type_url);
                assert_eq!(json["value"], "1.500s");

                let decoded: Any = serde_json::from_value(json).unwrap();
                assert_eq!(decoded.type_url, type_url);
                let decoded_duration: Duration = decoded.unpack_unchecked().unwrap();
                assert_eq!(decoded_duration, duration);
            });
        }

        #[test]
        fn deserialize_rejects_type_url_with_empty_type_name() {
            without_registry(|| {
                let json = r#"{"@type": "type.googleapis.com/", "value": ""}"#;
                let err = serde_json::from_str::<Any>(json).unwrap_err();
                assert!(err.to_string().contains("valid type URL"), "{err}");
            });
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
            use buffa::type_registry::JsonAnyEntry;
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut reg = TypeRegistry::new();
            // Register as NON-WKT (is_wkt=false) — fields inline at top level.
            reg.register_json_any(JsonAnyEntry {
                type_url: "type.example.com/user.Thing",
                to_json: user_type_to_json,
                from_json: user_type_from_json,
                is_wkt: false,
            });
            set_type_registry(reg);
            let result = f();
            clear_any_registry();
            clear_text_registry();
            result
        }

        #[test]
        fn is_message_agrees_with_the_registry_lookup_by_name() {
            use buffa::any_registry::AnyRegistry;
            use buffa::type_registry::JsonAnyEntry;

            // A local registry holding only the canonical URL: a lookup of
            // any other URL succeeds exactly when it falls back to the name.
            let mut reg = AnyRegistry::new();
            reg.register(JsonAnyEntry {
                type_url: "type.googleapis.com/google.protobuf.Timestamp",
                to_json: user_type_to_json,
                from_json: user_type_from_json,
                is_wkt: false,
            });

            for type_url in [
                "type.googleapis.com/google.protobuf.Timestamp",
                "custom.example/v1/google.protobuf.Timestamp",
                "/google.protobuf.Timestamp",
                "google.protobuf.Timestamp",
                "x/my.google.protobuf.Timestamp",
                "x/google.protobuf.Duration",
                "custom.example/v1/",
                "/",
                "",
            ] {
                let any = Any {
                    type_url: type_url.into(),
                    ..Default::default()
                };
                assert_eq!(
                    any.is_message::<Timestamp>(),
                    reg.lookup(type_url).is_some(),
                    "{type_url:?}"
                );
            }
        }

        #[test]
        fn serialize_non_wkt_inlines_fields() {
            with_user_type_registry(|| {
                // Encode {id: 42} as proto wire bytes.
                let any = Any {
                    type_url: "type.example.com/user.Thing".into(),
                    // field 1, varint 42: tag=0x08, value=0x2A
                    value: vec![0x08, 0x2A].into(),
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
                    value: vec![0x08, 0x07].into(), // id=7
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
            use buffa::type_registry::JsonAnyEntry;
            let _guard = REGISTRY_LOCK.lock().unwrap();
            let mut reg = TypeRegistry::new();
            reg.register_json_any(JsonAnyEntry {
                type_url: "type.example.com/user.BadType",
                to_json: |_bytes| Ok(serde_json::Value::Number(42.into())),
                from_json: |_v| Ok(alloc::vec::Vec::new()),
                is_wkt: false,
            });
            set_type_registry(reg);

            let any = Any {
                type_url: "type.example.com/user.BadType".into(),
                value: vec![].into(),
                ..Default::default()
            };
            let result = serde_json::to_string(&any);
            clear_any_registry();
            clear_text_registry();
            assert!(result.is_err(), "expected error for non-object to_json");
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("must return a JSON object"),
                "wrong error message"
            );
        }

        /// Corrupt `Any.value` bytes are ordinary untrusted input, not an
        /// invariant violation, so encoding them must not panic.
        ///
        /// `Any.value` is a raw `bytes` field that decode never validates.
        /// The release path already falls back to an empty body; a
        /// `debug_assert!` on the same condition makes a single malformed
        /// byte a panic wherever debug assertions are on — which this test
        /// suite has, so it fails here.
        #[test]
        fn corrupt_any_value_bytes_encode_to_text_without_panicking() {
            with_registry(|| {
                let a = Any {
                    type_url: Duration::TYPE_URL.to_string(),
                    // 0xFF is a varint continuation byte with nothing after
                    // it: a truncated field, not a valid Duration.
                    value: bytes::Bytes::from_static(&[0xFF]),
                    ..Default::default()
                };
                assert_eq!(
                    buffa::text::encode_to_string(&a),
                    "[type.googleapis.com/google.protobuf.Duration] {}",
                    "the expansion is still emitted, with an empty body"
                );
            });
        }

        #[test]
        fn deserialize_rejects_non_string_type() {
            // @type as a non-string value → error.
            let json = r#"{"@type": 123}"#;
            let err = serde_json::from_str::<Any>(json).unwrap_err();
            assert!(err.to_string().contains("@type must be a string"), "{err}");
        }

        /// An `Any` chain `depth` levels deep.
        fn any_chain(depth: usize) -> Any {
            let mut cur = Any::default();
            for _ in 0..depth {
                cur = Any {
                    type_url: Any::TYPE_URL.to_string(),
                    value: buffa::Message::encode_to_vec(&cur).into(),
                    ..Default::default()
                };
            }
            cur
        }

        #[test]
        fn a_shallow_any_chain_still_expands() {
            with_registry(|| {
                let json = serde_json::to_string(&any_chain(8)).expect("well within the cap");
                // Eight expansions, so eight nested "@type" keys.
                assert_eq!(json.matches("\"@type\"").count(), 8, "{json}");
            });
        }

        #[test]
        fn a_deep_any_chain_is_refused_rather_than_recursed() {
            with_registry(|| {
                // Deep enough to overflow a worker-sized stack if unbounded:
                // the decode below is cheap and shallow either way.
                let deep = any_chain(usize::try_from(MAX_ANY_EXPANSION_DEPTH).unwrap() + 5);
                let wire = buffa::Message::encode_to_vec(&deep);
                let decoded = <Any as buffa::Message>::decode_from_slice(&wire)
                    .expect("decoding the chain is one level deep and always succeeds");

                let err = serde_json::to_string(&decoded)
                    .expect_err("expansion past the cap must be an error, not a deeper stack");
                assert!(
                    err.to_string()
                        .contains(&alloc::format!("{MAX_ANY_EXPANSION_DEPTH} levels")),
                    "the error should state the limit it hit, as a number the \
                     reader can act on rather than a constant to go look up: {err}"
                );
            });
        }

        #[test]
        fn the_expansion_depth_is_restored_after_a_refusal() {
            with_registry(|| {
                let deep = any_chain(usize::try_from(MAX_ANY_EXPANSION_DEPTH).unwrap() + 5);
                assert!(serde_json::to_string(&deep).is_err());

                // The counter is ambient, so a refusal that failed to unwind
                // it would leave this thread unable to serialize any `Any`
                // again — a far worse outcome than the rejection itself.
                let json = serde_json::to_string(&any_chain(4))
                    .expect("a rejected serialization must not poison the thread");
                assert_eq!(json.matches("\"@type\"").count(), 4, "{json}");
            });
        }

        #[test]
        fn a_deep_any_chain_falls_back_to_the_vanilla_text_form() {
            with_registry(|| {
                let deep = any_chain(usize::try_from(MAX_ANY_EXPANSION_DEPTH).unwrap() + 5);
                // Textproto has no error channel here beyond a writer failure,
                // so past the cap the encoder emits the unexpanded
                // `type_url`/`value` form: still valid textproto, and finite.
                let text = buffa::text::encode_to_string(&deep);
                assert!(
                    text.contains("type_url:"),
                    "the innermost levels must fall back to the vanilla form: {text}"
                );
                // Count the expansion bracket itself, not bare `[` — a length
                // prefix of 0x5B inside the escaped `value` bytes renders as a
                // literal `[` and would inflate a looser count. Exact, so a
                // regression that expands one level too many also fails.
                assert_eq!(
                    text.matches("[type.googleapis.com/google.protobuf.Any]")
                        .count(),
                    usize::try_from(MAX_ANY_EXPANSION_DEPTH).unwrap(),
                    "expansion should stop at exactly the cap"
                );
            });
        }
    }
}
