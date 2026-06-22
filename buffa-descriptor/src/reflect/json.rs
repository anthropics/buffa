//! Proto3 canonical JSON for [`DynamicMessage`].
//!
//! Serialization is `impl serde::Serialize for DynamicMessage` — a mechanical
//! walk over the descriptor's fields, with per-[`SingularKind`] dispatch.
//! Deserialization needs the descriptor as input, so it's a `DeserializeSeed`
//! ([`DynamicMessageSeed`]) rather than a `Deserialize`; the ergonomic wrapper
//! is [`DynamicMessage::from_json`].
//!
//! Well-known types are special-cased by `MessageDescriptor::full_name`. The
//! WKT codecs are **reflective** — they read the WKT's fields by number
//! through the [`DynamicMessage`] surface and transform, rather than bridging
//! through `buffa-types`. This keeps `buffa-descriptor` free of a `buffa-types`
//! dependency edge at the cost of reimplementing the WKT JSON formatting
//! (Timestamp RFC3339, Duration `"3.5s"`, FieldMask camelCase, base64,
//! `Any`'s `@type` expansion).
//!
//! Known limitation: `google.protobuf.Any` requires the inner type to be
//! registered in the same pool — the spec permits failing on unregistered
//! types, and CEL evaluation requires the pool to carry the full schema
//! anyway.

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::dynamic::default_scalar_value;
use super::{DynamicMessage, MapKey, MapValue, ReflectMessage, ReflectMessageMut, Value};
use crate::{
    DescriptorPool, EnumIndex, FieldDescriptor, FieldKind, MessageDescriptor, MessageIndex,
    ScalarType, SingularKind,
};
use buffa::editions::EnumType;

// ── Serialize ───────────────────────────────────────────────────────────────

impl Serialize for DynamicMessage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let md = self.message_descriptor();
        if let Some(wkt) = WktKind::from_full_name(&md.full_name) {
            return wkt.serialize_message(self, s);
        }
        let mut map = s.serialize_map(None)?;
        for fd in &md.fields {
            if !self.has(fd) {
                continue;
            }
            let value = self
                .field_by_number(fd.number)
                .expect("has() ⇒ field is present");
            map.serialize_entry(&fd.json_name, &FieldRef::new(self.pool(), fd, value))?;
        }
        // Extensions present on this message serialize after the declared
        // fields as `"[full.name]": value`, per the proto2 JSON convention.
        for ext in self.pool().extensions_of(self.message_index()) {
            let fd = ext.field();
            if !self.has(fd) {
                continue;
            }
            let value = self
                .field_by_number(fd.number)
                .expect("has() ⇒ field is present");
            map.serialize_entry(ext.json_key(), &FieldRef::new(self.pool(), fd, value))?;
        }
        map.end()
    }
}

/// A field value paired with its descriptor and pool, for serde dispatch.
struct FieldRef<'a> {
    pool: &'a DescriptorPool,
    fd: &'a FieldDescriptor,
    value: &'a Value,
}

impl<'a> FieldRef<'a> {
    fn new(pool: &'a DescriptorPool, fd: &'a FieldDescriptor, value: &'a Value) -> Self {
        Self { pool, fd, value }
    }
}

impl Serialize for FieldRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (&self.fd.kind, self.value) {
            (FieldKind::Singular(sk), v) => SingularRef::new(self.pool, *sk, v).serialize(s),
            (FieldKind::List(sk), Value::List(items)) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&SingularRef::new(self.pool, *sk, item))?;
                }
                seq.end()
            }
            (FieldKind::Map { key, value: vk }, Value::Map(m)) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(
                        &MapKeyRef { key: *key, k },
                        &SingularRef::new(self.pool, *vk, v),
                    )?;
                }
                map.end()
            }
            // Stored value's shape doesn't match the descriptor — defensive.
            _ => s.serialize_none(),
        }
    }
}

/// A singular value paired with its kind, for serde dispatch.
struct SingularRef<'a> {
    pool: &'a DescriptorPool,
    kind: SingularKind,
    value: &'a Value,
}

impl<'a> SingularRef<'a> {
    fn new(pool: &'a DescriptorPool, kind: SingularKind, value: &'a Value) -> Self {
        Self { pool, kind, value }
    }
}

impl Serialize for SingularRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (self.kind, self.value) {
            (SingularKind::Scalar(sc), v) => serialize_scalar(sc, v, s),
            (SingularKind::Enum(eidx), Value::EnumNumber(n)) => {
                serialize_enum(self.pool, eidx, *n, s)
            }
            (SingularKind::Message(_), Value::Message(m)) => m.serialize(s),
            _ => s.serialize_none(),
        }
    }
}

fn serialize_scalar<S: Serializer>(sc: ScalarType, v: &Value, s: S) -> Result<S::Ok, S::Error> {
    match (sc, v) {
        (ScalarType::Bool, Value::Bool(b)) => s.serialize_bool(*b),
        (ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32, Value::I32(n)) => {
            s.serialize_i32(*n)
        }
        (ScalarType::Uint32 | ScalarType::Fixed32, Value::U32(n)) => s.serialize_u32(*n),
        // 64-bit integers serialize as quoted strings per the proto3 JSON spec
        // (JavaScript number precision is 2^53).
        (ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64, Value::I64(n)) => {
            s.serialize_str(&n.to_string())
        }
        (ScalarType::Uint64 | ScalarType::Fixed64, Value::U64(n)) => {
            s.serialize_str(&n.to_string())
        }
        (ScalarType::Float, Value::F32(f)) => serialize_float(f64::from(*f), s),
        (ScalarType::Double, Value::F64(f)) => serialize_float(*f, s),
        (ScalarType::String, Value::String(t)) => s.serialize_str(t),
        (ScalarType::Bytes, Value::Bytes(b)) => s.serialize_str(&base64_encode(b)),
        _ => s.serialize_none(),
    }
}

/// Float/double with the proto3 JSON special-value mapping: NaN, Infinity,
/// -Infinity serialize as strings.
fn serialize_float<S: Serializer>(f: f64, s: S) -> Result<S::Ok, S::Error> {
    if f.is_nan() {
        s.serialize_str("NaN")
    } else if f.is_infinite() {
        s.serialize_str(if f > 0.0 { "Infinity" } else { "-Infinity" })
    } else {
        s.serialize_f64(f)
    }
}

fn serialize_enum<S: Serializer>(
    pool: &DescriptorPool,
    eidx: EnumIndex,
    n: i32,
    s: S,
) -> Result<S::Ok, S::Error> {
    let ed = pool.enumeration(eidx);
    // NullValue serializes as JSON null, per the spec.
    if ed.full_name == "google.protobuf.NullValue" {
        return s.serialize_none();
    }
    match ed.value(n) {
        Some(ev) => s.serialize_str(&ev.name),
        // Unknown enum value: serialize as the raw number.
        None => s.serialize_i32(n),
    }
}

struct MapKeyRef<'a> {
    key: ScalarType,
    k: &'a MapKey,
}

impl Serialize for MapKeyRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // Map keys are always strings in proto3 JSON.
        let _ = self.key;
        match self.k {
            MapKey::Bool(b) => s.serialize_str(if *b { "true" } else { "false" }),
            MapKey::I32(n) => s.serialize_str(&n.to_string()),
            MapKey::I64(n) => s.serialize_str(&n.to_string()),
            MapKey::U32(n) => s.serialize_str(&n.to_string()),
            MapKey::U64(n) => s.serialize_str(&n.to_string()),
            MapKey::String(t) => s.serialize_str(t),
        }
    }
}

// ── Deserialize ─────────────────────────────────────────────────────────────

impl DynamicMessage {
    /// Parse proto3 canonical JSON into a `DynamicMessage`.
    ///
    /// Unknown fields are an error per the proto3 JSON spec. For lenient
    /// parsing (a transcoding gateway accepting input from a newer schema
    /// revision), use [`Self::from_json_ignoring_unknown`].
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input is not valid JSON or does
    /// not match the message descriptor.
    #[cfg(feature = "std")]
    pub fn from_json(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        json: &str,
    ) -> Result<Self, serde_json::Error> {
        let mut d = serde_json::Deserializer::from_str(json);
        let msg = DynamicMessageSeed::new(pool, msg_idx).deserialize(&mut d)?;
        d.end()?;
        Ok(msg)
    }

    /// Parse proto3 canonical JSON, silently discarding unknown fields.
    ///
    /// The proto3 JSON spec says parsers *should* reject unknown fields by
    /// default but *may* provide an option to ignore them. This is that
    /// option — use it when the JSON producer may be running a newer schema
    /// revision than this pool carries. Unknown fields are discarded, not
    /// preserved (there is no JSON equivalent of binary unknown-field
    /// round-tripping).
    ///
    /// Only the unknown-field check is relaxed. Other proto3 JSON spec
    /// violations — duplicate keys, multiple members of the same oneof,
    /// null elements in repeated fields, malformed values on *known*
    /// fields — remain errors.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input is not valid JSON or a
    /// *known* field does not match its descriptor.
    #[cfg(feature = "std")]
    pub fn from_json_ignoring_unknown(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        json: &str,
    ) -> Result<Self, serde_json::Error> {
        let mut d = serde_json::Deserializer::from_str(json);
        let msg = DynamicMessageSeed::new(pool, msg_idx)
            .ignore_unknown_fields(true)
            .deserialize(&mut d)?;
        d.end()?;
        Ok(msg)
    }

    /// Serialize this message as a proto3 canonical JSON string.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if serialization fails.
    #[cfg(feature = "std")]
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// A `DeserializeSeed` that carries the descriptor needed to interpret a
/// JSON object as a [`DynamicMessage`].
///
/// `serde::Deserialize` has no parameters — the implementation can't know
/// which message type it's parsing into. `DeserializeSeed` carries `self`
/// into `deserialize`, so the pool and message index travel with it.
///
/// This is also the long-form API for combining parse options: the
/// `from_json_*` constructors on [`DynamicMessage`] are conveniences over
/// `DynamicMessageSeed::new(..).<options>.deserialize(..)`. New parse
/// options are added here as builder-style setters rather than as new
/// `from_json_*` constructor permutations.
pub struct DynamicMessageSeed {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    ignore_unknown: bool,
}

impl DynamicMessageSeed {
    /// Create a seed for the given message type.
    #[must_use]
    pub fn new(pool: Arc<DescriptorPool>, msg_idx: MessageIndex) -> Self {
        Self {
            pool,
            msg_idx,
            ignore_unknown: false,
        }
    }

    /// Silently discard unknown fields instead of erroring (default: error).
    ///
    /// The setting propagates to nested messages, repeated elements, and map
    /// values. See [`DynamicMessage::from_json_ignoring_unknown`].
    #[must_use]
    pub fn ignore_unknown_fields(mut self, ignore: bool) -> Self {
        self.ignore_unknown = ignore;
        self
    }
}

impl<'de> DeserializeSeed<'de> for DynamicMessageSeed {
    type Value = DynamicMessage;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        let md = self.pool.message(self.msg_idx);
        if let Some(wkt) = WktKind::from_full_name(&md.full_name) {
            return wkt.deserialize_message(self.pool, self.msg_idx, d, self.ignore_unknown);
        }
        d.deserialize_map(MessageVisitor {
            pool: self.pool,
            msg_idx: self.msg_idx,
            ignore_unknown: self.ignore_unknown,
        })
    }
}

struct MessageVisitor {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    ignore_unknown: bool,
}

impl<'de> Visitor<'de> for MessageVisitor {
    type Value = DynamicMessage;

    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(
            f,
            "a JSON object for message {}",
            self.pool.message(self.msg_idx).full_name
        )
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut msg = DynamicMessage::new(Arc::clone(&self.pool), self.msg_idx);
        // Per the proto3 JSON spec, supplying more than one member of the
        // same (non-synthetic) oneof is an error. Track which oneof indices
        // have been written. Synthetic oneofs (proto3 `optional`) are
        // single-member and can't trigger this.
        let mut seen_oneofs: Vec<u16> = Vec::new();
        // The spec also requires rejecting a field that appears more than
        // once in the same object. serde_json's streaming deserializer
        // yields every key in document order (it does not deduplicate), so
        // tracking seen field numbers catches both exact duplicates and the
        // proto-name/json-name casing variants (`foo_bar` then `fooBar`),
        // which resolve to the same descriptor.
        let mut seen_fields: Vec<u32> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            // The proto3 JSON spec says parsers must accept both the camelCase
            // json_name and the original proto field name. `field_by_name`
            // indexes both. A `"[pkg.ext]"` key names an extension of this
            // message, registered in the pool by its bracketed full name.
            let md = self.pool.message(self.msg_idx);
            let fd = if let Some(ext_name) = key.strip_prefix('[').and_then(|k| k.strip_suffix(']'))
            {
                match self.pool.extension_by_name(ext_name) {
                    Some(ext) if ext.extendee() == self.msg_idx => Some(ext.field()),
                    Some(ext) => {
                        return Err(de::Error::custom(format!(
                            "extension {:?} extends {}, not {}",
                            ext.full_name(),
                            self.pool.message(ext.extendee()).full_name,
                            md.full_name
                        )));
                    }
                    // Unregistered extension → same unknown-field handling
                    // as an unrecognized plain key.
                    None => None,
                }
            } else {
                md.field_by_name(&key)
            };
            let Some(fd) = fd else {
                // Unknown field — error per the spec, unless the caller
                // opted into lenient parsing. Note that this `continue`
                // bypasses the duplicate-key check below: a payload with
                // the same *unknown* key twice is silently collapsed in
                // lenient mode. There is no descriptor to deduplicate
                // against, and the spec's no-duplicates rule is in terms
                // of fields, not arbitrary keys.
                if self.ignore_unknown {
                    map.next_value::<de::IgnoredAny>()?;
                    continue;
                }
                return Err(de::Error::custom(format!(
                    "unknown field {key:?} on message {}",
                    md.full_name
                )));
            };
            // Extract the small Copy parts before mutating `msg`.
            let kind = fd.kind;
            let number = fd.number;
            let oneof_index = fd.oneof_index;
            let synthetic = oneof_index
                .and_then(|oi| md.oneofs.get(oi as usize))
                .is_some_and(|o| o.synthetic);
            if seen_fields.contains(&number) {
                return Err(de::Error::custom(format!(
                    "duplicate field {key:?} on message {}",
                    md.full_name
                )));
            }
            seen_fields.push(number);
            let v = map.next_value_seed(FieldSeed {
                pool: &self.pool,
                kind,
                ignore_unknown: self.ignore_unknown,
            })?;
            // null → leave the field unset (per spec, except NullValue which
            // FieldSeed handles).
            if let Some(v) = v {
                if let Some(oi) = oneof_index {
                    if !synthetic {
                        if seen_oneofs.contains(&oi) {
                            return Err(de::Error::custom(format!(
                                "more than one member of oneof set ({key:?})"
                            )));
                        }
                        seen_oneofs.push(oi);
                    }
                    // Clear sibling oneof members so a stale member doesn't
                    // survive in the field map.
                    let to_clear: Vec<u32> = self
                        .pool
                        .message(self.msg_idx)
                        .oneofs
                        .get(oi as usize)
                        .map(|o| {
                            o.field_indices
                                .iter()
                                .filter_map(|&fi| {
                                    self.pool.message(self.msg_idx).fields.get(fi as usize)
                                })
                                .map(|f| f.number)
                                .filter(|&n| n != number)
                                .collect()
                        })
                        .unwrap_or_default();
                    for n in to_clear {
                        if let Some(fd) = self.pool.message(self.msg_idx).field(n) {
                            msg.clear(fd);
                        }
                    }
                }
                // Re-resolve the descriptor by number (the `fd` borrow was
                // released before `next_value_seed` mutated `msg`). The
                // declared-field lookup misses extension numbers, so fall
                // back to the pool's extension index.
                if let Some(fd) = self.pool.message(self.msg_idx).field(number) {
                    msg.set(fd, v);
                } else if let Some(ext) = self.pool.extension_for(self.msg_idx, number) {
                    msg.set(ext.field(), v);
                }
            }
        }
        Ok(msg)
    }
}

struct FieldSeed<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: FieldKind,
    ignore_unknown: bool,
}

impl<'de> DeserializeSeed<'de> for FieldSeed<'_> {
    /// `None` means "unset the field" — the spec says JSON `null` for a
    /// singular field is equivalent to absent (except `NullValue` which is
    /// handled inside `SingularSeed`).
    type Value = Option<Value>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        match self.kind {
            FieldKind::Singular(sk) => SingularSeed {
                pool: self.pool,
                kind: sk,
                ignore_unknown: self.ignore_unknown,
            }
            .deserialize(d),
            FieldKind::List(sk) => d.deserialize_any(ListVisitor {
                pool: self.pool,
                kind: sk,
                ignore_unknown: self.ignore_unknown,
            }),
            FieldKind::Map { key, value } => d.deserialize_any(MapFieldVisitor {
                pool: self.pool,
                key,
                value,
                ignore_unknown: self.ignore_unknown,
            }),
        }
    }
}

struct SingularSeed<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: SingularKind,
    ignore_unknown: bool,
}

impl<'de> DeserializeSeed<'de> for SingularSeed<'_> {
    type Value = Option<Value>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        match self.kind {
            SingularKind::Scalar(sc) => deserialize_optional_scalar(sc, d),
            SingularKind::Enum(eidx) => deserialize_enum(self.pool, eidx, d),
            SingularKind::Message(midx) => {
                // `google.protobuf.Value` treats JSON `null` as a present
                // `null_value` member, not "unset" — dispatch straight to the
                // WKT seed so its visitor sees the unit token.
                if self.pool.message(midx).full_name == "google.protobuf.Value" {
                    return DynamicMessageSeed::new(Arc::clone(self.pool), midx)
                        .ignore_unknown_fields(self.ignore_unknown)
                        .deserialize(d)
                        .map(|m| Some(Value::Message(m)));
                }
                // Otherwise, JSON `null` maps to `None` ("unset").
                d.deserialize_option(NestedMessageVisitor {
                    pool: self.pool,
                    midx,
                    ignore_unknown: self.ignore_unknown,
                })
            }
        }
    }
}

/// Wrap [`deserialize_scalar`] so that JSON `null` maps to `None` ("unset").
fn deserialize_optional_scalar<'de, D: Deserializer<'de>>(
    sc: ScalarType,
    d: D,
) -> Result<Option<Value>, D::Error> {
    struct Opt(ScalarType);
    impl<'de> Visitor<'de> for Opt {
        type Value = Option<Value>;
        fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            write!(f, "a scalar JSON value or null")
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            deserialize_scalar(self.0, d).map(Some)
        }
    }
    d.deserialize_option(Opt(sc))
}

struct NestedMessageVisitor<'a> {
    pool: &'a Arc<DescriptorPool>,
    midx: MessageIndex,
    ignore_unknown: bool,
}

impl<'de> Visitor<'de> for NestedMessageVisitor<'_> {
    type Value = Option<Value>;

    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "a JSON object or null")
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        DynamicMessageSeed::new(Arc::clone(self.pool), self.midx)
            .ignore_unknown_fields(self.ignore_unknown)
            .deserialize(d)
            .map(|m| Some(Value::Message(m)))
    }
}

fn deserialize_scalar<'de, D: Deserializer<'de>>(sc: ScalarType, d: D) -> Result<Value, D::Error> {
    struct ScalarVisitor(ScalarType);
    impl<'de> Visitor<'de> for ScalarVisitor {
        type Value = Value;

        fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            write!(f, "a JSON value for {:?}", self.0)
        }

        fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
            match self.0 {
                ScalarType::Bool => Ok(Value::Bool(v)),
                _ => Err(de::Error::invalid_type(de::Unexpected::Bool(v), &self)),
            }
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            scalar_from_i64(self.0, v).ok_or_else(|| de::Error::custom("out of range"))
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            scalar_from_u64(self.0, v).ok_or_else(|| de::Error::custom("out of range"))
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            scalar_from_f64(self.0, v).ok_or_else(|| de::Error::custom("invalid number"))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            scalar_from_str(self.0, v).map_err(de::Error::custom)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            // `null` for a scalar means "unset" — handled at the caller.
            Err(de::Error::invalid_type(de::Unexpected::Unit, &self))
        }
    }
    d.deserialize_any(ScalarVisitor(sc))
}

fn scalar_from_i64(sc: ScalarType, v: i64) -> Option<Value> {
    Some(match sc {
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => {
            Value::I32(i32::try_from(v).ok()?)
        }
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => Value::I64(v),
        ScalarType::Uint32 | ScalarType::Fixed32 => Value::U32(u32::try_from(v).ok()?),
        ScalarType::Uint64 | ScalarType::Fixed64 => Value::U64(u64::try_from(v).ok()?),
        ScalarType::Float => Value::F32(v as f32),
        ScalarType::Double => Value::F64(v as f64),
        ScalarType::Bool | ScalarType::String | ScalarType::Bytes => return None,
    })
}

fn scalar_from_u64(sc: ScalarType, v: u64) -> Option<Value> {
    Some(match sc {
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => {
            Value::I32(i32::try_from(v).ok()?)
        }
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => {
            Value::I64(i64::try_from(v).ok()?)
        }
        ScalarType::Uint32 | ScalarType::Fixed32 => Value::U32(u32::try_from(v).ok()?),
        ScalarType::Uint64 | ScalarType::Fixed64 => Value::U64(v),
        ScalarType::Float => Value::F32(v as f32),
        ScalarType::Double => Value::F64(v as f64),
        ScalarType::Bool | ScalarType::String | ScalarType::Bytes => return None,
    })
}

fn scalar_from_f64(sc: ScalarType, v: f64) -> Option<Value> {
    Some(match sc {
        ScalarType::Float => {
            // Reject values that overflow f32 — the spec requires erroring,
            // not saturating to ±Infinity. Allow exact ±Infinity through
            // (they came from "Infinity"/"-Infinity" string parse).
            if v.is_finite() && v.abs() > f64::from(f32::MAX) {
                return None;
            }
            Value::F32(v as f32)
        }
        ScalarType::Double => Value::F64(v),
        // Integers as JSON floats: accept exact integral values. The
        // `2^53` magnitude bound protects the `as` cast from saturating —
        // `f64` cannot exactly represent integers beyond that, and `as i64`
        // saturates rather than wrapping.
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32
            if v.fract() == 0.0 && integral_in_safe_range(v) =>
        {
            Value::I32(i32::try_from(v as i64).ok()?)
        }
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64
            if v.fract() == 0.0 && integral_in_safe_range(v) =>
        {
            Value::I64(v as i64)
        }
        ScalarType::Uint32 | ScalarType::Fixed32
            if v.fract() == 0.0 && v >= 0.0 && integral_in_safe_range(v) =>
        {
            Value::U32(u32::try_from(v as i64).ok()?)
        }
        ScalarType::Uint64 | ScalarType::Fixed64
            if v.fract() == 0.0 && v >= 0.0 && integral_in_safe_range(v) =>
        {
            Value::U64(v as u64)
        }
        _ => return None,
    })
}

/// Whether an integral `f64` is within the range where `f64` exactly
/// represents integers (`±2^53`). Beyond that the value is approximate and
/// `as i64` saturates rather than rounding to nearest, producing silent
/// corruption.
fn integral_in_safe_range(v: f64) -> bool {
    // MSRV: `f64::abs` is not const-stable until 1.85, and no caller needs
    // const evaluation here.
    v.abs() <= (1u64 << 53) as f64
}

fn scalar_from_str(sc: ScalarType, v: &str) -> Result<Value, String> {
    match sc {
        ScalarType::String => Ok(Value::String(v.to_owned())),
        ScalarType::Bytes => base64_decode(v)
            .map(Value::Bytes)
            .ok_or_else(|| "invalid base64".to_owned()),
        // 64-bit integers are quoted strings. Spec also accepts decimal and
        // exponential notation as long as the value is integral.
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => {
            parse_int_str(v).map(Value::I64)
        }
        ScalarType::Uint64 | ScalarType::Fixed64 => {
            // Try the direct parse first to preserve full u64 range; fall
            // back to the integral-float path for exponential notation.
            if let Ok(n) = v.parse::<u64>() {
                Ok(Value::U64(n))
            } else {
                parse_int_str(v)
                    .and_then(|n| u64::try_from(n).map_err(|_| "negative uint64".to_owned()))
                    .map(Value::U64)
            }
        }
        // 32-bit integers may also appear as strings.
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => parse_int_str(v)
            .and_then(|n| i32::try_from(n).map_err(|_| "out of range int32".to_owned()))
            .map(Value::I32),
        ScalarType::Uint32 | ScalarType::Fixed32 => parse_int_str(v)
            .and_then(|n| u32::try_from(n).map_err(|_| "out of range uint32".to_owned()))
            .map(Value::U32),
        // Float/double special values.
        ScalarType::Float => parse_float_str(v).map(|f| Value::F32(f as f32)),
        ScalarType::Double => parse_float_str(v).map(Value::F64),
        ScalarType::Bool => Err("string is not a bool".to_owned()),
    }
}

/// Parse a quoted-string integer, accepting integral decimal/exponential
/// forms (`"1.5e3"` → `1500`) per the proto3 JSON spec.
fn parse_int_str(v: &str) -> Result<i64, String> {
    if let Ok(n) = v.parse::<i64>() {
        return Ok(n);
    }
    // Only fall back to the float path when the string visibly carries a
    // decimal point or exponent — a pure-integer string that failed
    // `i64::parse` is out of range, not a float.
    if !v.contains(['.', 'e', 'E']) {
        return Err("integer out of range".to_owned());
    }
    let f: f64 = v.parse().map_err(|_| "invalid integer string".to_owned())?;
    if f.fract() != 0.0 || f.is_nan() || f.is_infinite() {
        return Err("non-integral string for integer field".to_owned());
    }
    // f64 has 53 bits of mantissa; values above 2^53 cannot be exactly
    // represented and the cast to i64 silently saturates. Reject to be safe.
    if f.abs() >= (1u64 << 53) as f64 {
        return Err("out of exact integer range".to_owned());
    }
    Ok(f as i64)
}

fn parse_float_str(v: &str) -> Result<f64, String> {
    match v {
        "NaN" => Ok(f64::NAN),
        "Infinity" => Ok(f64::INFINITY),
        "-Infinity" => Ok(f64::NEG_INFINITY),
        _ => v.parse().map_err(|_| "invalid float".to_owned()),
    }
}

fn deserialize_enum<'de, D: Deserializer<'de>>(
    pool: &Arc<DescriptorPool>,
    eidx: EnumIndex,
    d: D,
) -> Result<Option<Value>, D::Error> {
    struct EnumVisitor<'a> {
        pool: &'a Arc<DescriptorPool>,
        eidx: EnumIndex,
    }
    impl<'de> Visitor<'de> for EnumVisitor<'_> {
        type Value = Option<Value>;
        fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            write!(f, "an enum string, number, or null")
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let ed = self.pool.enumeration(self.eidx);
            ed.value_by_name(v)
                .map(|ev| Some(Value::EnumNumber(ev.number)))
                .ok_or_else(|| de::Error::custom(format!("unknown enum value {v:?}")))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            let n = i32::try_from(v).map_err(de::Error::custom)?;
            // Closed enums reject unknown values; open enums accept any i32.
            let ed = self.pool.enumeration(self.eidx);
            if ed.enum_type == EnumType::Closed && ed.value(n).is_none() {
                return Err(de::Error::custom("unknown closed enum value"));
            }
            Ok(Some(Value::EnumNumber(n)))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            self.visit_i64(i64::try_from(v).map_err(de::Error::custom)?)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            // null for an enum: NullValue gets `None` from a unit; other
            // enums treat null as unset.
            let ed = self.pool.enumeration(self.eidx);
            if ed.full_name == "google.protobuf.NullValue" {
                Ok(Some(Value::EnumNumber(0)))
            } else {
                Ok(None)
            }
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            self.visit_unit()
        }
        fn visit_some<D2: Deserializer<'de>>(self, d: D2) -> Result<Self::Value, D2::Error> {
            d.deserialize_any(self)
        }
    }
    // deserialize_option lets us distinguish null from absent.
    d.deserialize_option(EnumVisitor { pool, eidx })
}

struct ListVisitor<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: SingularKind,
    ignore_unknown: bool,
}

impl<'de> Visitor<'de> for ListVisitor<'_> {
    type Value = Option<Value>;
    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "a JSON array or null")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = seq.next_element_seed(SingularSeed {
            pool: self.pool,
            kind: self.kind,
            ignore_unknown: self.ignore_unknown,
        })? {
            // Per the spec, repeated fields cannot contain null elements.
            let v = v.ok_or_else(|| de::Error::custom("null element in repeated field"))?;
            out.push(v);
        }
        Ok(Some(Value::List(out)))
    }
}

struct MapFieldVisitor<'a> {
    pool: &'a Arc<DescriptorPool>,
    key: ScalarType,
    value: SingularKind,
    ignore_unknown: bool,
}

impl<'de> Visitor<'de> for MapFieldVisitor<'_> {
    type Value = Option<Value>;
    fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "a JSON object or null")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        // Collect into a Vec then sort+dedup once at the end, rather than
        // sorted-insert per entry (which is `O(n)` per insert into a `Vec`).
        let mut out: Vec<(MapKey, Value)> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            let k = parse_map_key(self.key, &key).map_err(de::Error::custom)?;
            let v = map.next_value_seed(SingularSeed {
                pool: self.pool,
                kind: self.value,
                ignore_unknown: self.ignore_unknown,
            })?;
            let v = v.ok_or_else(|| de::Error::custom("null value in map field"))?;
            out.push((k, v));
        }
        Ok(Some(Value::Map(MapValue::from_entries(out))))
    }
}

fn parse_map_key(sc: ScalarType, s: &str) -> Result<MapKey, String> {
    Ok(match sc {
        ScalarType::Bool => match s {
            "true" => MapKey::Bool(true),
            "false" => MapKey::Bool(false),
            _ => return Err("invalid bool map key".to_owned()),
        },
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => {
            MapKey::I32(s.parse().map_err(|_| "invalid int32 key".to_owned())?)
        }
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => {
            MapKey::I64(s.parse().map_err(|_| "invalid int64 key".to_owned())?)
        }
        ScalarType::Uint32 | ScalarType::Fixed32 => {
            MapKey::U32(s.parse().map_err(|_| "invalid uint32 key".to_owned())?)
        }
        ScalarType::Uint64 | ScalarType::Fixed64 => {
            MapKey::U64(s.parse().map_err(|_| "invalid uint64 key".to_owned())?)
        }
        ScalarType::String => MapKey::String(s.to_owned()),
        ScalarType::Double | ScalarType::Float | ScalarType::Bytes => {
            return Err("invalid map key type".to_owned())
        }
    })
}

// ── Base64 ──────────────────────────────────────────────────────────────────

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_STD[(n >> 18 & 0x3F) as usize] as char);
        out.push(B64_STD[(n >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_STD[(n >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_STD[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Decode standard or URL-safe base64, with or without padding (the proto3
/// JSON spec accepts both forms on parse).
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        })
    }
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4 + 1);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let n = (val(bytes[i])? << 18)
            | (val(bytes[i + 1])? << 12)
            | (val(bytes[i + 2])? << 6)
            | val(bytes[i + 3])?;
        out.push((n >> 16) as u8);
        out.push((n >> 8) as u8);
        out.push(n as u8);
        i += 4;
    }
    let rem = bytes.len() - i;
    match rem {
        0 => {}
        2 => {
            let n = (val(bytes[i])? << 18) | (val(bytes[i + 1])? << 12);
            out.push((n >> 16) as u8);
        }
        3 => {
            let n = (val(bytes[i])? << 18) | (val(bytes[i + 1])? << 12) | (val(bytes[i + 2])? << 6);
            out.push((n >> 16) as u8);
            out.push((n >> 8) as u8);
        }
        _ => return None,
    }
    Some(out)
}

// ── Well-known types ────────────────────────────────────────────────────────

include!("json_wkt.rs");

// Suppress unused warnings for the items that the WKT codec keeps.
#[allow(unused)]
const _: fn(&MessageDescriptor) = |_| {};
