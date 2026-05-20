// Reflective WKT JSON codecs.
//
// Each well-known type's JSON form is read off / written into the
// `DynamicMessage` via field-number access — there is no `buffa-types`
// dependency. Included from `json.rs` via `include!()` so the helpers can
// share the per-scalar serde dispatch above without `pub(crate)` plumbing.

/// Which well-known type a message descriptor names.
#[derive(Clone, Copy, Debug)]
enum WktKind {
    Timestamp,
    Duration,
    FieldMask,
    Empty,
    Struct,
    JsonValue,
    ListValue,
    Any,
    /// One of the nine wrapper types. Carries the inner field's scalar type.
    Wrapper(ScalarType),
}

impl WktKind {
    fn from_full_name(name: &str) -> Option<Self> {
        Some(match name {
            "google.protobuf.Timestamp" => Self::Timestamp,
            "google.protobuf.Duration" => Self::Duration,
            "google.protobuf.FieldMask" => Self::FieldMask,
            "google.protobuf.Empty" => Self::Empty,
            "google.protobuf.Struct" => Self::Struct,
            "google.protobuf.Value" => Self::JsonValue,
            "google.protobuf.ListValue" => Self::ListValue,
            "google.protobuf.Any" => Self::Any,
            "google.protobuf.DoubleValue" => Self::Wrapper(ScalarType::Double),
            "google.protobuf.FloatValue" => Self::Wrapper(ScalarType::Float),
            "google.protobuf.Int64Value" => Self::Wrapper(ScalarType::Int64),
            "google.protobuf.UInt64Value" => Self::Wrapper(ScalarType::Uint64),
            "google.protobuf.Int32Value" => Self::Wrapper(ScalarType::Int32),
            "google.protobuf.UInt32Value" => Self::Wrapper(ScalarType::Uint32),
            "google.protobuf.BoolValue" => Self::Wrapper(ScalarType::Bool),
            "google.protobuf.StringValue" => Self::Wrapper(ScalarType::String),
            "google.protobuf.BytesValue" => Self::Wrapper(ScalarType::Bytes),
            // `NullValue` is an enum; serialize/deserialize_enum handle it.
            _ => return None,
        })
    }

    /// Whether this WKT uses the `{"@type": ..., "value": ...}` form when
    /// nested inside `google.protobuf.Any`. Per the spec, message types with
    /// a custom JSON representation (everything except plain message and
    /// `Empty`) wrap their canonical form in a `"value"` key. Plain message
    /// types spread their fields alongside `@type`.
    fn uses_any_value_wrapping(self) -> bool {
        !matches!(self, Self::Empty)
    }

    fn serialize_message<S: Serializer>(
        self,
        msg: &DynamicMessage,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            Self::Timestamp => {
                let secs = msg.read_i64(1).unwrap_or(0);
                let nanos = msg.read_i32(2).unwrap_or(0);
                s.serialize_str(&fmt_rfc3339(secs, nanos).map_err(serde::ser::Error::custom)?)
            }
            Self::Duration => {
                let secs = msg.read_i64(1).unwrap_or(0);
                let nanos = msg.read_i32(2).unwrap_or(0);
                s.serialize_str(&fmt_duration(secs, nanos).map_err(serde::ser::Error::custom)?)
            }
            Self::FieldMask => {
                let paths = msg.read_string_list(1);
                let camel: Result<Vec<String>, _> =
                    paths.iter().map(|p| field_mask_to_camel(p)).collect();
                s.serialize_str(&camel.map_err(serde::ser::Error::custom)?.join(","))
            }
            Self::Empty => s.serialize_map(Some(0))?.end(),
            Self::Wrapper(sc) => match msg.field_by_number(1) {
                Some(v) => serialize_scalar(sc, v, s),
                None => serialize_scalar(sc, &default_scalar_value(sc), s),
            },
            Self::Struct => {
                // fields: map<string, Value>
                let mut map = s.serialize_map(None)?;
                if let Some(Value::Map(m)) = msg.field_by_number(1) {
                    for (k, v) in m {
                        let MapKey::String(ks) = k else {
                            return Err(serde::ser::Error::custom("Struct map key must be string"));
                        };
                        let Value::Message(inner) = v else {
                            return Err(serde::ser::Error::custom("Struct value must be message"));
                        };
                        map.serialize_entry(ks, inner)?;
                    }
                }
                map.end()
            }
            Self::ListValue => {
                let mut seq = s.serialize_seq(None)?;
                if let Some(Value::List(items)) = msg.field_by_number(1) {
                    for v in items {
                        let Value::Message(inner) = v else {
                            return Err(serde::ser::Error::custom("ListValue elem must be message"));
                        };
                        seq.serialize_element(inner)?;
                    }
                }
                seq.end()
            }
            Self::JsonValue => serialize_json_value(msg, s),
            Self::Any => serialize_any(msg, s),
        }
    }

    fn deserialize_message<'de, D: Deserializer<'de>>(
        self,
        pool: Arc<DescriptorPool>,
        midx: MessageIndex,
        d: D,
    ) -> Result<DynamicMessage, D::Error> {
        match self {
            Self::Any => deserialize_any(pool, midx, d),
            Self::Timestamp => {
                let s: String = String::deserialize(d)?;
                let (secs, nanos) = parse_rfc3339(&s).map_err(de::Error::custom)?;
                Ok(make_two_field(pool, midx, secs, nanos))
            }
            Self::Duration => {
                let s: String = String::deserialize(d)?;
                let (secs, nanos) = parse_duration(&s).map_err(de::Error::custom)?;
                Ok(make_two_field(pool, midx, secs, nanos))
            }
            Self::FieldMask => {
                let s: String = String::deserialize(d)?;
                let paths: Result<Vec<Value>, _> = if s.is_empty() {
                    Ok(Vec::new())
                } else {
                    s.split(',')
                        .map(|p| field_mask_to_snake(p.trim()).map(Value::String))
                        .collect()
                };
                let mut m = DynamicMessage::new(pool, midx);
                m.set_by_number(1, Value::List(paths.map_err(de::Error::custom)?));
                Ok(m)
            }
            Self::Empty => {
                struct EmptyVisitor;
                impl<'de> Visitor<'de> for EmptyVisitor {
                    type Value = ();
                    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                        write!(f, "an empty object")
                    }
                    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
                        if map.next_key::<String>()?.is_some() {
                            return Err(de::Error::custom("unexpected field on Empty"));
                        }
                        Ok(())
                    }
                }
                d.deserialize_map(EmptyVisitor)?;
                Ok(DynamicMessage::new(pool, midx))
            }
            Self::Wrapper(sc) => {
                let v = deserialize_scalar(sc, d)?;
                let mut m = DynamicMessage::new(pool, midx);
                m.set_by_number(1, v);
                Ok(m)
            }
            Self::Struct => deserialize_struct(pool, midx, d),
            Self::ListValue => deserialize_list_value(pool, midx, d),
            Self::JsonValue => deserialize_json_value(pool, midx, d),
        }
    }
}

// ── DynamicMessage helpers for WKT field access ─────────────────────────────

impl DynamicMessage {
    fn read_i64(&self, number: u32) -> Option<i64> {
        match self.field_by_number(number) {
            Some(Value::I64(n)) => Some(*n),
            _ => None,
        }
    }
    fn read_i32(&self, number: u32) -> Option<i32> {
        match self.field_by_number(number) {
            Some(Value::I32(n)) => Some(*n),
            _ => None,
        }
    }
    fn read_string_list(&self, number: u32) -> Vec<&str> {
        match self.field_by_number(number) {
            Some(Value::List(items)) => items
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        }
    }
    /// Set a field by number without a `&FieldDescriptor` borrow. Used by the
    /// WKT codecs, which know the field layout statically and never need
    /// the oneof-clearing semantics that `ReflectMessageMut::set` provides
    /// (`google.protobuf.Value` is a oneof, but the JSON deserializer only
    /// ever sets one member, so there is nothing to clear).
    fn set_by_number(&mut self, number: u32, v: Value) {
        self.insert_value(number, v);
    }
}

fn make_two_field(
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
    secs: i64,
    nanos: i32,
) -> DynamicMessage {
    let mut m = DynamicMessage::new(pool, midx);
    if secs != 0 {
        m.set_by_number(1, Value::I64(secs));
    }
    if nanos != 0 {
        m.set_by_number(2, Value::I32(nanos));
    }
    m
}

// ── google.protobuf.Value (recursive JSON) ──────────────────────────────────

fn serialize_json_value<S: Serializer>(msg: &DynamicMessage, s: S) -> Result<S::Ok, S::Error> {
    // Value is a oneof: null_value(1), number_value(2), string_value(3),
    // bool_value(4), struct_value(5), list_value(6).
    if msg.field_by_number(1).is_some() {
        return s.serialize_none();
    }
    if let Some(Value::F64(n)) = msg.field_by_number(2) {
        // `google.protobuf.Value.number_value` cannot represent NaN/Inf in
        // JSON — unlike a `double` field, which serializes them as the
        // strings `"NaN"`/`"Infinity"`. The spec requires rejection.
        if n.is_nan() || n.is_infinite() {
            return Err(serde::ser::Error::custom(
                "google.protobuf.Value.number_value cannot represent NaN or Infinity in JSON",
            ));
        }
        return s.serialize_f64(*n);
    }
    if let Some(Value::String(t)) = msg.field_by_number(3) {
        return s.serialize_str(t);
    }
    if let Some(Value::Bool(b)) = msg.field_by_number(4) {
        return s.serialize_bool(*b);
    }
    if let Some(Value::Message(inner)) = msg.field_by_number(5) {
        return inner.serialize(s);
    }
    if let Some(Value::Message(inner)) = msg.field_by_number(6) {
        return inner.serialize(s);
    }
    // Unset Value: spec is ambiguous; serialize as null.
    s.serialize_none()
}

fn deserialize_json_value<'de, D: Deserializer<'de>>(
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
    d: D,
) -> Result<DynamicMessage, D::Error> {
    struct ValueVisitor {
        pool: Arc<DescriptorPool>,
        midx: MessageIndex,
    }
    impl<'de> Visitor<'de> for ValueVisitor {
        type Value = DynamicMessage;
        fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            write!(f, "any JSON value")
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(1, Value::EnumNumber(0)); // null_value = NULL_VALUE
            Ok(m)
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            self.visit_unit()
        }
        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(4, Value::Bool(v));
            Ok(m)
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(2, Value::F64(v));
            Ok(m)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            self.visit_f64(v as f64)
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            self.visit_f64(v as f64)
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(3, Value::String(v.to_owned()));
            Ok(m)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
            let lv_idx = self
                .pool
                .message_index("google.protobuf.ListValue")
                .ok_or_else(|| de::Error::custom("ListValue not in pool"))?;
            let lv = ListValueVisitor {
                pool: Arc::clone(&self.pool),
                midx: lv_idx,
            }
            .visit_seq(seq)?;
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(6, Value::Message(lv));
            Ok(m)
        }
        fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
            let s_idx = self
                .pool
                .message_index("google.protobuf.Struct")
                .ok_or_else(|| de::Error::custom("Struct not in pool"))?;
            let s = StructVisitor {
                pool: Arc::clone(&self.pool),
                midx: s_idx,
            }
            .visit_map(map)?;
            let mut m = DynamicMessage::new(self.pool, self.midx);
            m.set_by_number(5, Value::Message(s));
            Ok(m)
        }
    }
    d.deserialize_any(ValueVisitor { pool, midx })
}

struct StructVisitor {
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
}

impl<'de> Visitor<'de> for StructVisitor {
    type Value = DynamicMessage;
    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "a JSON object")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let value_idx = self
            .pool
            .message_index("google.protobuf.Value")
            .ok_or_else(|| de::Error::custom("Value not in pool"))?;
        let mut fields: Vec<(MapKey, Value)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let v = map.next_value_seed(DynamicMessageSeed::new(Arc::clone(&self.pool), value_idx))?;
            fields.push((MapKey::String(key), Value::Message(v)));
        }
        let mut m = DynamicMessage::new(self.pool, self.midx);
        m.set_by_number(1, Value::Map(MapValue::from_entries(fields)));
        Ok(m)
    }
}

fn deserialize_struct<'de, D: Deserializer<'de>>(
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
    d: D,
) -> Result<DynamicMessage, D::Error> {
    d.deserialize_map(StructVisitor { pool, midx })
}

struct ListValueVisitor {
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
}

impl<'de> Visitor<'de> for ListValueVisitor {
    type Value = DynamicMessage;
    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "a JSON array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let value_idx = self
            .pool
            .message_index("google.protobuf.Value")
            .ok_or_else(|| de::Error::custom("Value not in pool"))?;
        let mut items = Vec::new();
        while let Some(v) =
            seq.next_element_seed(DynamicMessageSeed::new(Arc::clone(&self.pool), value_idx))?
        {
            items.push(Value::Message(v));
        }
        let mut m = DynamicMessage::new(self.pool, self.midx);
        m.set_by_number(1, Value::List(items));
        Ok(m)
    }
}

fn deserialize_list_value<'de, D: Deserializer<'de>>(
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
    d: D,
) -> Result<DynamicMessage, D::Error> {
    d.deserialize_seq(ListValueVisitor { pool, midx })
}

// ── google.protobuf.Any ─────────────────────────────────────────────────────

/// Serialize an `Any` as `{"@type": ..., ...inner fields}` (for plain message
/// types) or `{"@type": ..., "value": <wkt JSON>}` (for inner types with a
/// custom JSON form).
///
/// Requires the inner type to be registered in the same pool — the spec
/// permits failing on unregistered types, and CEL evaluation requires the
/// pool to carry the full schema anyway.
fn serialize_any<S: Serializer>(msg: &DynamicMessage, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::Error as _;
    let type_url = match msg.field_by_number(1) {
        Some(Value::String(u)) => u.as_str(),
        _ => "",
    };
    let value_bytes: &[u8] = match msg.field_by_number(2) {
        Some(Value::Bytes(b)) => b,
        _ => &[],
    };
    let pool = msg.pool();
    if type_url.is_empty() && value_bytes.is_empty() {
        // Empty Any → empty object.
        return s.serialize_map(Some(0))?.end();
    }
    let Some(inner_idx) = resolve_any_type(pool, type_url) else {
        return Err(S::Error::custom(format!(
            "Any type_url {type_url:?} not registered in the descriptor pool"
        )));
    };
    let inner = DynamicMessage::decode(Arc::clone(pool), inner_idx, value_bytes)
        .map_err(|e| S::Error::custom(format!("Any inner decode failed: {e}")))?;
    let inner_md = pool.message(inner_idx);
    let inner_wkt = WktKind::from_full_name(&inner_md.full_name);

    let mut map = s.serialize_map(None)?;
    map.serialize_entry("@type", type_url)?;
    if let Some(wkt) = inner_wkt {
        if wkt.uses_any_value_wrapping() {
            map.serialize_entry("value", &inner)?;
            return map.end();
        }
    }
    // Spread the inner fields. We can't use `inner.serialize` because that
    // opens a new object; instead, replay the field walk.
    for fd in &inner_md.fields {
        if !inner.has(fd) {
            continue;
        }
        let value = inner
            .field_by_number(fd.number)
            .expect("has() implies present");
        map.serialize_entry(&fd.json_name, &FieldRef::new(pool, fd, value))?;
    }
    map.end()
}

/// Deserialize an `Any` from `{"@type": ..., ...}`. Buffers the object as a
/// `serde_json::Map` because `@type` may come before or after the inner
/// fields (`AnyUnorderedTypeTag`).
#[cfg(feature = "std")]
fn deserialize_any<'de, D: Deserializer<'de>>(
    pool: Arc<DescriptorPool>,
    midx: MessageIndex,
    d: D,
) -> Result<DynamicMessage, D::Error> {
    use serde::de::Error as _;
    let mut obj: serde_json::Map<String, serde_json::Value> =
        serde_json::Map::deserialize(d)?;
    let mut any = DynamicMessage::new(Arc::clone(&pool), midx);
    if obj.is_empty() {
        return Ok(any);
    }
    let Some(serde_json::Value::String(type_url)) = obj.remove("@type") else {
        return Err(D::Error::custom("Any object missing string \"@type\""));
    };
    let Some(inner_idx) = resolve_any_type(&pool, &type_url) else {
        return Err(D::Error::custom(format!(
            "Any type_url {type_url:?} not registered in the descriptor pool"
        )));
    };
    let inner_md = pool.message(inner_idx);
    let inner_wkt = WktKind::from_full_name(&inner_md.full_name);
    // The inner object to deserialize from: either the unwrapped `"value"`
    // (for WKTs) or the remaining fields.
    let inner_json = if let Some(wkt) = inner_wkt {
        if wkt.uses_any_value_wrapping() {
            obj.remove("value").ok_or_else(|| {
                D::Error::custom(format!(
                    "Any with WKT type {type_url:?} requires a \"value\" key"
                ))
            })?
        } else {
            serde_json::Value::Object(obj)
        }
    } else {
        serde_json::Value::Object(obj)
    };
    // Re-deserialize the inner JSON value into the inner message type.
    let inner = DynamicMessageSeed::new(Arc::clone(&pool), inner_idx)
        .deserialize(inner_json)
        .map_err(|e| D::Error::custom(format!("Any inner deserialize failed: {e}")))?;
    any.set_by_number(1, Value::String(type_url));
    any.set_by_number(2, Value::Bytes(inner.encode_to_vec()));
    Ok(any)
}

#[cfg(not(feature = "std"))]
fn deserialize_any<'de, D: Deserializer<'de>>(
    _pool: Arc<DescriptorPool>,
    _midx: MessageIndex,
    _d: D,
) -> Result<DynamicMessage, D::Error> {
    Err(de::Error::custom(
        "Any JSON deserialization requires the `std` feature",
    ))
}

/// Resolve a `type_url` to a [`MessageIndex`]. Accepts `type.googleapis.com/`
/// and any other prefix; the type name is the segment after the last `/`.
fn resolve_any_type(pool: &DescriptorPool, type_url: &str) -> Option<MessageIndex> {
    let name = type_url.rsplit('/').next()?;
    pool.message_index(name)
}


// ── Timestamp / Duration / FieldMask formatting ─────────────────────────────
//
// The shared formatting and parsing primitives live in
// `buffa::json_helpers::wkt`. Both this reflective codec and `buffa-types`'s
// typed serde impls call into the same code, so the two paths can't drift on
// edge cases the conformance suite exercises.

use buffa::json_helpers::wkt::{
    camel_to_snake, field_mask_path_round_trips, fmt_duration, fmt_timestamp as fmt_rfc3339,
    parse_duration, parse_timestamp as parse_rfc3339, snake_to_camel,
};

/// Convert a snake_case field-mask path to lowerCamelCase, rejecting paths
/// that don't round-trip per the proto3 JSON spec (double underscores,
/// underscore-digit, uppercase).
fn field_mask_to_camel(p: &str) -> Result<String, &'static str> {
    if !field_mask_path_round_trips(p) {
        return Err("FieldMask path does not round-trip through camelCase");
    }
    Ok(snake_to_camel(p))
}

/// Convert a lowerCamelCase field-mask path to snake_case, rejecting paths
/// that don't round-trip.
fn field_mask_to_snake(p: &str) -> Result<String, &'static str> {
    let snake = camel_to_snake(p);
    if snake_to_camel(&snake) != p {
        return Err("FieldMask JSON path is not canonical lowerCamelCase");
    }
    Ok(snake)
}
