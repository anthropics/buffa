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
//! dependency edge at the cost of reimplementing the WKT JSON structure here
//! (`Any`'s `@type` expansion, `Struct` / `Value` / `ListValue`, the
//! wrappers). The Timestamp, Duration and FieldMask text formats are shared
//! with `buffa-types` through `buffa::json_helpers::wkt`.
//!
//! `bytes` values encode and decode through `buffa::json_helpers::bytes`, the
//! codec generated messages use.
//!
//! Known limitation: `google.protobuf.Any` requires the inner type to be
//! registered in the same pool — the spec permits failing on unregistered
//! types, and CEL evaluation requires the pool to carry the full schema
//! anyway.
//!
//! `no_std` limitation: deserializing a `google.protobuf.Any` value from JSON
//! requires the `std` feature. Without it, any input containing an `Any`
//! fails with ``Any JSON deserialization requires the `std` feature``.
//! Serializing an `Any` has no such gate, so a `no_std` build can write JSON
//! that it cannot read back.

use core::cell::Cell;

use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::dynamic::default_scalar_value;
use super::{DynamicMessage, MapKey, MapValue, ReflectMessage, ReflectMessageMut, Value};
use crate::{
    DescriptorPool, EnumIndex, FieldDescriptor, FieldKind, MessageDescriptor, MessageIndex,
    ScalarType, SingularKind,
};
use buffa::editions::EnumType;
use buffa::json_helpers;
use buffa::RECURSION_LIMIT;

// ── Serialize ───────────────────────────────────────────────────────────────
//
// Nesting is bounded by a depth budget threaded through every recursion site
// below. A `DynamicMessage` built by `decode` is already at most
// `RECURSION_LIMIT` deep, but `google.protobuf.Any` carries its payload as
// opaque bytes that are only decoded here, at serialize time — so without a
// budget that spans `Any` boundaries, N nested `Any` layers cost N stack
// frames for a few bytes each and overflow the stack (an uncatchable abort)
// on untrusted input. The budget makes over-deep input a serde error instead.
//
// The budget is deliberately the binary decoder's `RECURSION_LIMIT`, not a
// JSON-specific constant: the contract is that anything `DynamicMessage::
// decode` accepts must serialize, and the inner `Any` decode continues the
// same count, so the two limits have to be one number.

/// The serde error for serialization nesting exhausting the
/// [`RECURSION_LIMIT`] budget — whether at a message boundary or inside an
/// `Any` payload's own decode, so callers see one message for one condition.
fn nesting_too_deep<E: serde::ser::Error>() -> E {
    E::custom(format_args!(
        "message nesting depth exceeds buffa::RECURSION_LIMIT ({RECURSION_LIMIT}) during JSON \
         serialization (google.protobuf.Any payloads count toward the limit)"
    ))
}

/// Consume one level of nesting budget, or fail with a serde error.
fn descend<E: serde::ser::Error>(depth: u32) -> Result<u32, E> {
    depth.checked_sub(1).ok_or_else(nesting_too_deep)
}

/// Proto3 canonical JSON via serde.
///
/// Fails with a serde error if message nesting exceeds
/// [`buffa::RECURSION_LIMIT`] levels below this message. `google.protobuf.Any`
/// payloads — decoded here, at serialize time — count toward the same budget
/// as ordinary sub-messages, so nesting hidden inside `Any.value` cannot
/// exhaust the stack. The count follows the binary decoder's, so a message
/// [`DynamicMessage::decode`] accepted serializes provided its nesting
/// *counted through* `Any` payloads stays within the limit and each `Any`
/// names a type in the pool that its bytes decode as; decode success alone
/// does not establish that. A message assembled deeper by other means (via
/// [`ReflectMessageMut`], a raised [`buffa::DecodeOptions`] recursion limit,
/// or `from_json`, whose only bound is the JSON parser's own) does not
/// serialize. The cap is not configurable in this release.
impl Serialize for DynamicMessage {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        // The one place the budget starts; every nested message goes through
        // `Nested` / `serialize_message` so nothing below resets it.
        serialize_message(self, RECURSION_LIMIT, s)
    }
}

/// Serialize `msg` with `depth` levels of nesting budget remaining for its
/// sub-messages. `msg` itself is already paid for by the caller.
fn serialize_message<S: Serializer>(
    msg: &DynamicMessage,
    depth: u32,
    s: S,
) -> Result<S::Ok, S::Error> {
    let md = msg.message_descriptor();
    if let Some(wkt) = WktKind::from_full_name(&md.full_name) {
        return wkt.serialize_message(msg, depth, s);
    }
    let mut map = s.serialize_map(None)?;
    serialize_fields(&mut map, msg, depth)?;
    map.end()
}

/// Write `msg`'s set fields, extensions included, into an open JSON object.
fn serialize_fields<M: SerializeMap>(
    map: &mut M,
    msg: &DynamicMessage,
    depth: u32,
) -> Result<(), M::Error> {
    let md = msg.message_descriptor();
    let pool = msg.pool();
    for fd in &md.fields {
        if !msg.has(fd) {
            continue;
        }
        let value = msg
            .field_by_number(fd.number)
            .expect("has() ⇒ field is present");
        map.serialize_entry(&fd.json_name, &FieldRef::new(pool, fd, value, depth))?;
    }
    // Extensions present on this message serialize after the declared
    // fields as `"[full.name]": value`, per the proto2 JSON convention.
    for ext in pool.extensions_of(msg.message_index()) {
        let fd = ext.field();
        if !msg.has(fd) {
            continue;
        }
        let value = msg
            .field_by_number(fd.number)
            .expect("has() ⇒ field is present");
        map.serialize_entry(ext.json_key(), &FieldRef::new(pool, fd, value, depth))?;
    }
    Ok(())
}

/// A sub-message whose own level is already charged; `depth` is the budget
/// remaining for *its* sub-messages. Built via [`Nested::charge`], which is
/// where the one level is paid — the only other constructor site is
/// `serialize_any`, which charges explicitly because the inner decode needs
/// the figure before the message exists.
struct Nested<'a> {
    msg: &'a DynamicMessage,
    depth: u32,
}

impl<'a> Nested<'a> {
    /// Charge one level of the parent's remaining budget for `msg`.
    fn charge<E: serde::ser::Error>(msg: &'a DynamicMessage, parent_depth: u32) -> Result<Self, E> {
        Ok(Self {
            msg,
            depth: descend(parent_depth)?,
        })
    }
}

impl Serialize for Nested<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_message(self.msg, self.depth, s)
    }
}

/// A field value paired with its descriptor and pool, for serde dispatch.
struct FieldRef<'a> {
    pool: &'a DescriptorPool,
    fd: &'a FieldDescriptor,
    value: &'a Value,
    depth: u32,
}

impl<'a> FieldRef<'a> {
    fn new(
        pool: &'a DescriptorPool,
        fd: &'a FieldDescriptor,
        value: &'a Value,
        depth: u32,
    ) -> Self {
        Self {
            pool,
            fd,
            value,
            depth,
        }
    }
}

impl Serialize for FieldRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let singular = |kind, v| SingularRef::new(self.pool, kind, v, self.depth);
        match (&self.fd.kind, self.value) {
            (FieldKind::Singular(sk), v) => singular(*sk, v).serialize(s),
            (FieldKind::List(sk), Value::List(items)) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(&singular(*sk, item))?;
                }
                seq.end()
            }
            (FieldKind::Map { key, value: vk }, Value::Map(m)) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(&MapKeyRef { key: *key, k }, &singular(*vk, v))?;
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
    depth: u32,
}

impl<'a> SingularRef<'a> {
    fn new(pool: &'a DescriptorPool, kind: SingularKind, value: &'a Value, depth: u32) -> Self {
        Self {
            pool,
            kind,
            value,
            depth,
        }
    }
}

impl Serialize for SingularRef<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match (self.kind, self.value) {
            (SingularKind::Scalar(sc), v) => serialize_scalar(sc, v, s),
            (SingularKind::Enum(eidx), Value::EnumNumber(n)) => {
                serialize_enum(self.pool, eidx, *n, s)
            }
            (SingularKind::Message(_), Value::Message(m)) => {
                Nested::charge(m, self.depth)?.serialize(s)
            }
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
        (ScalarType::Float, Value::F32(f)) => json_helpers::float::serialize(f, s),
        (ScalarType::Double, Value::F64(f)) => json_helpers::double::serialize(f, s),
        (ScalarType::String, Value::String(t)) => s.serialize_str(t),
        (ScalarType::Bytes, Value::Bytes(b)) => json_helpers::bytes::serialize(b, s),
        _ => s.serialize_none(),
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
//
// The repeated elements and map entries a parse builds are bounded by an
// element-memory budget, as `DecodeContext` bounds them for the binary
// codec. A `DecodeContext` never reaches this parser — `serde::Deserialize`
// has no context parameter — so the top-level seed creates the budget and
// the visitors borrow it.
//
// It is needed for the same reason the binary one is: an empty repeated
// message element is three JSON bytes (`{},`) and `size_of::<Value>()` in
// the `Vec` it lands in, so a payload well inside any input-size cap still
// expands about twentyfold. The charges match the reflective binary
// decoder's (`reflect/dynamic.rs`) element for element. A
// `google.protobuf.Any` payload has two costs with no counterpart there:
// the `serde_json::Value` tree it is buffered into, which is charged as a
// map entry for each object member and as a repeated element for each array
// element for as long as the buffer is held, and its elements, which are
// charged here while the binary decoder leaves `Any.value` undecoded.

/// Display text of the error for a parse that exceeds its element-memory
/// budget. `buffa::DecodeError::ElementMemoryLimitExceeded` displays the same
/// text.
const ELEMENT_MEMORY_LIMIT_EXCEEDED: &str = "element memory limit exceeded";

/// Charge `bytes` of element footprint against the parse's shared budget.
///
/// Leaves the budget unchanged when it fails, mirroring
/// [`buffa::DecodeContext::register_element_memory`].
///
/// Containers charge as they *admit* an element rather than before reading
/// it: `SeqAccess`/`MapAccess` have no peek, so whether another element
/// exists is only known once it has been read. The reflective binary
/// decoder's unpacked and map paths charge at the same point for the same
/// reason, and the overshoot
/// is one element — whatever that element materialized inside itself was
/// charged as it was built. `FieldMask`, whose paths are countable up
/// front, does charge before allocating.
fn charge<E: de::Error>(budget: &Cell<usize>, bytes: usize) -> Result<(), E> {
    let remaining = budget.get();
    if bytes > remaining {
        return Err(E::custom(ELEMENT_MEMORY_LIMIT_EXCEEDED));
    }
    budget.set(remaining - bytes);
    Ok(())
}

impl DynamicMessage {
    /// Parse proto3 canonical JSON into a `DynamicMessage`.
    ///
    /// Unknown fields are an error per the proto3 JSON spec. For lenient
    /// parsing (a transcoding gateway accepting input from a newer schema
    /// revision), use [`Self::from_json_ignoring_unknown`].
    ///
    /// The repeated elements and map entries the parse builds are bounded by
    /// [`buffa::DEFAULT_ELEMENT_MEMORY_LIMIT`] (32 MiB). To parse with
    /// another limit, call [`Self::from_json_with_element_memory_limit`]. For
    /// both a limit and lenient unknown fields, use a [`DynamicMessageSeed`].
    /// The limit also covers the buffer that a `google.protobuf.Any` payload
    /// is read into; the documentation of
    /// [`with_element_memory_limit`](DynamicMessageSeed::with_element_memory_limit)
    /// gives each charge.
    ///
    /// A `google.protobuf.Any` value in the input requires the `std`
    /// feature to deserialize; without it the parse fails with ``Any JSON
    /// deserialization requires the `std` feature``.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input is not valid JSON, does
    /// not match the message descriptor, or exceeds the element-memory limit.
    /// [`DynamicMessageSeed::is_element_memory_limit_error`] identifies the
    /// last case.
    ///
    /// # Panics
    ///
    /// Panics if `msg_idx` is out of range for `pool`, which happens only
    /// with an index issued by a different pool. See
    /// [`DescriptorPool::message`].
    pub fn from_json(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        json: &str,
    ) -> Result<Self, serde_json::Error> {
        DynamicMessageSeed::new(pool, msg_idx).parse_json(json)
    }

    /// Parse proto3 canonical JSON under a caller-supplied element-memory
    /// limit, in bytes.
    ///
    /// `element_memory_limit` is the memory the parse may spend on repeated
    /// elements and map entries. Each repeated element is charged
    /// `size_of::<Value>()` bytes, and each map entry the size of a
    /// [`MapKey`] plus a [`Value`]. It is a memory budget, so it differs
    /// from an element count and from a bound on the input length.
    /// [`DynamicMessageSeed::with_element_memory_limit`] lists every charge.
    ///
    /// [`Self::from_json`] applies [`buffa::DEFAULT_ELEMENT_MEMORY_LIMIT`]
    /// (32 MiB). Lower it for a tighter bound on untrusted input, or raise
    /// it for trusted input with very many elements. With `0`, the parse
    /// fails at the first repeated element or map entry, and input that
    /// holds only singular fields still parses. `usize::MAX` means no limit.
    ///
    /// Unknown fields are an error, as in `from_json`. To ignore them under
    /// a limit, call
    /// `DynamicMessageSeed::new(pool, msg_idx).ignore_unknown_fields(true).with_element_memory_limit(n).parse_json(json)`.
    ///
    /// The limit also covers the buffer that a `google.protobuf.Any` payload
    /// is read into, so a payload needs room for its buffer and its message
    /// together.
    ///
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use buffa_descriptor::{DescriptorPool, DynamicMessage};
    /// # fn parse(pool: Arc<DescriptorPool>, json: &str) -> Result<(), serde_json::Error> {
    /// let request = pool.message_index("my.pkg.Request").expect("message is in the pool");
    /// let msg = DynamicMessage::from_json_with_element_memory_limit(
    ///     pool,
    ///     request,
    ///     json,
    ///     8 * 1024 * 1024,
    /// )?;
    /// # drop(msg);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input:
    ///
    /// - is not valid JSON;
    /// - does not match the message descriptor;
    /// - builds repeated elements and map entries that cost more than
    ///   `element_memory_limit`.
    ///   [`DynamicMessageSeed::is_element_memory_limit_error`] identifies
    ///   this case.
    ///
    /// # Panics
    ///
    /// Panics if `msg_idx` is out of range for `pool`, which happens only
    /// with an index issued by a different pool. See
    /// [`DescriptorPool::message`].
    pub fn from_json_with_element_memory_limit(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        json: &str,
        element_memory_limit: usize,
    ) -> Result<Self, serde_json::Error> {
        DynamicMessageSeed::new(pool, msg_idx)
            .with_element_memory_limit(element_memory_limit)
            .parse_json(json)
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
    /// The element-memory limit is the [`Self::from_json`] default,
    /// [`buffa::DEFAULT_ELEMENT_MEMORY_LIMIT`]. To ignore unknown fields
    /// under another limit, call
    /// `DynamicMessageSeed::new(pool, msg_idx).ignore_unknown_fields(true).with_element_memory_limit(n).parse_json(json)`.
    ///
    /// A `google.protobuf.Any` value in the input requires the `std`
    /// feature to deserialize; without it the parse fails with ``Any JSON
    /// deserialization requires the `std` feature``.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input is not valid JSON, a
    /// *known* field does not match its descriptor, or the parse exceeds
    /// the element-memory limit.
    /// [`DynamicMessageSeed::is_element_memory_limit_error`] identifies the
    /// last case.
    ///
    /// # Panics
    ///
    /// Panics if `msg_idx` is out of range for `pool`, which happens only
    /// with an index issued by a different pool. See
    /// [`DescriptorPool::message`].
    pub fn from_json_ignoring_unknown(
        pool: Arc<DescriptorPool>,
        msg_idx: MessageIndex,
        json: &str,
    ) -> Result<Self, serde_json::Error> {
        DynamicMessageSeed::new(pool, msg_idx)
            .ignore_unknown_fields(true)
            .parse_json(json)
    }

    /// Serialize this message as a proto3 canonical JSON string.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if serialization fails — notably when
    /// message nesting, counting `google.protobuf.Any` payloads, exceeds
    /// [`buffa::RECURSION_LIMIT`] (see the [`Serialize`] impl), or when an
    /// `Any` names a type that is not in the pool.
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
/// `DynamicMessageSeed::new(..).<options>.parse_json(..)`. `from_json`
/// leaves every option at its default and each `from_json_*` constructor
/// sets exactly one; every option has a builder-style setter here, and a
/// parse that needs two uses the seed.
///
/// Prefer [`parse_json`](Self::parse_json) to driving the seed by hand. A
/// hand-driven [`DeserializeSeed::deserialize`] leaves the check for
/// trailing input to the caller, and each call starts with the full
/// element-memory limit. A seed is cheap to clone, so a server can configure
/// one and clone it for each request.
///
/// ```no_run
/// # use std::sync::Arc;
/// # use buffa_descriptor::{DescriptorPool, DynamicMessageSeed};
/// # fn parse(pool: Arc<DescriptorPool>, json: &str) -> Result<(), serde_json::Error> {
/// let request = pool.message_index("my.pkg.Request").expect("message is in the pool");
/// let msg = DynamicMessageSeed::new(pool, request)
///     .ignore_unknown_fields(true)
///     .with_element_memory_limit(8 * 1024 * 1024)
///     .parse_json(json)?;
/// # drop(msg);
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct DynamicMessageSeed {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    ignore_unknown: bool,
    element_memory_limit: usize,
}

impl DynamicMessageSeed {
    /// Create a seed for the given message type.
    #[must_use]
    pub fn new(pool: Arc<DescriptorPool>, msg_idx: MessageIndex) -> Self {
        Self {
            pool,
            msg_idx,
            ignore_unknown: false,
            element_memory_limit: buffa::DEFAULT_ELEMENT_MEMORY_LIMIT,
        }
    }

    /// Silently discard unknown fields instead of erroring (default: error).
    ///
    /// The setting propagates to nested messages, repeated elements, and map
    /// values. See [`DynamicMessage::from_json_ignoring_unknown`].
    ///
    /// An unknown field inside a `google.protobuf.Any` payload is buffered
    /// with the rest of the payload before it is discarded, and the buffer
    /// counts toward the
    /// [element-memory limit](Self::with_element_memory_limit).
    #[must_use]
    pub fn ignore_unknown_fields(mut self, ignore: bool) -> Self {
        self.ignore_unknown = ignore;
        self
    }

    /// Set the element-memory limit for this parse (default:
    /// [`buffa::DEFAULT_ELEMENT_MEMORY_LIMIT`], 32 MiB).
    ///
    /// The limit bounds the repeated elements and map entries the parse
    /// builds, not how much JSON it reads. Each repeated element (scalars
    /// included), `ListValue` element and `FieldMask` path is charged the
    /// size of a [`Value`], and each map entry and `Struct` member the size
    /// of a [`MapKey`] plus a [`Value`]. These are the charges the reflective
    /// binary decoder applies. With the default, that is about half a
    /// million repeated elements on a 64-bit target. One budget covers the
    /// whole parse: nested messages and `Any` payloads draw on it. Exceeding
    /// it is an error, which
    /// [`is_element_memory_limit_error`](Self::is_element_memory_limit_error)
    /// identifies. Pass `usize::MAX` for no limit.
    ///
    /// The binary codec's counterpart is
    /// [`DecodeOptions::with_element_memory_limit`](buffa::DecodeOptions::with_element_memory_limit).
    ///
    /// # `google.protobuf.Any` payloads
    ///
    /// `@type` can follow the fields it types, so the parser buffers an `Any`
    /// payload as a `serde_json::Value` tree and decodes the message from
    /// that tree. The buffer draws on the same budget. At every depth of the
    /// payload, each object member is charged as a map entry and each array
    /// element as a repeated element. The members include `@type`, singular
    /// fields and unknown fields. The parser returns the buffer's charge
    /// after it decodes the payload. The elements that the decode built stay
    /// charged until the parse ends.
    ///
    /// A payload needs room for its buffer and its message together, on top
    /// of what the parse has kept when it reaches the `Any`. So an `Any`
    /// that follows a large repeated field has less room than one that
    /// precedes it. When the order of members can vary, size the limit for
    /// the `Any` coming last. Each enclosing `Any` buffers a nested `Any`
    /// again, so a value at `Any` depth *d* is charged in *d* buffers while
    /// the innermost payload is decoded.
    ///
    /// A message costs more inside an `Any` than as a field of its own type.
    /// Where a [`Value`] is 64 bytes and a [`MapKey`] 24, as on a 64-bit
    /// target with a current compiler, the default lets a payload of
    /// singular fields buffer 381,300 members. A payload that is one
    /// repeated field of scalars can have about 262,000 elements, because
    /// each is charged in the buffer and again in the message.
    #[must_use]
    pub fn with_element_memory_limit(mut self, bytes: usize) -> Self {
        self.element_memory_limit = bytes;
        self
    }

    /// Parse one JSON document into a [`DynamicMessage`] with this seed's
    /// options.
    ///
    /// Input after the document, other than whitespace, is an error. Driving
    /// the seed through [`DeserializeSeed::deserialize`] leaves that check to
    /// the caller.
    ///
    /// A `google.protobuf.Any` value in the input requires the `std`
    /// feature to deserialize; without it the parse fails with ``Any JSON
    /// deserialization requires the `std` feature``.
    ///
    /// # Errors
    ///
    /// Returns a `serde_json::Error` if the input is not valid JSON, does
    /// not match the message descriptor, or exceeds the element-memory
    /// limit. See
    /// [`with_element_memory_limit`](Self::with_element_memory_limit) for
    /// what that limit charges.
    ///
    /// # Panics
    ///
    /// Panics if the seed's message index is out of range for its pool,
    /// which happens only with an index issued by a different pool. See
    /// [`DescriptorPool::message`].
    #[doc(alias = "from_json")]
    pub fn parse_json(self, json: &str) -> Result<DynamicMessage, serde_json::Error> {
        let mut d = serde_json::Deserializer::from_str(json);
        let msg = self.deserialize(&mut d)?;
        d.end()?;
        Ok(msg)
    }

    /// Returns `true` if `err` is the error a parse returns when it exceeds
    /// its element-memory limit.
    ///
    /// `serde_json::Error` has no variant for this error, and classifies it
    /// with every other mismatch between the input and the schema. A server
    /// uses this to answer "too large" for it and "malformed" for the rest:
    ///
    /// ```no_run
    /// # use buffa_descriptor::{DynamicMessage, DynamicMessageSeed};
    /// # fn parse(seed: DynamicMessageSeed, body: &str) -> Result<DynamicMessage, u16> {
    /// seed.parse_json(body).map_err(|err| {
    ///     if DynamicMessageSeed::is_element_memory_limit_error(&err) {
    ///         413
    ///     } else {
    ///         400
    ///     }
    /// })
    /// # }
    /// ```
    ///
    /// The check is on the error's text, which starts with
    /// `element memory limit exceeded`, the text
    /// [`buffa::DecodeError::ElementMemoryLimitExceeded`] displays. It holds
    /// for an error from [`parse_json`](Self::parse_json), any
    /// `DynamicMessage::from_json*` constructor, or a `serde_json`
    /// deserializer driving the seed directly. A seed driven by another format's `Deserializer`
    /// reports the same text through that format's error type.
    #[must_use]
    pub fn is_element_memory_limit_error(err: &serde_json::Error) -> bool {
        err.to_string().starts_with(ELEMENT_MEMORY_LIMIT_EXCEEDED)
    }
}

impl<'de> DeserializeSeed<'de> for DynamicMessageSeed {
    type Value = DynamicMessage;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        // Nested messages borrow this cell through `NestedSeed`.
        let budget = Cell::new(self.element_memory_limit);
        NestedSeed {
            pool: self.pool,
            msg_idx: self.msg_idx,
            ignore_unknown: self.ignore_unknown,
            budget: &budget,
        }
        .deserialize(d)
    }
}

/// The internal twin of [`DynamicMessageSeed`] that borrows the parse's
/// element-memory budget instead of owning a limit.
///
/// Every recursion site uses this, so one top-level parse draws on one
/// allowance. The budget stays a borrow — an `Rc`/`Arc<Cell<_>>` field on
/// the public seed would make it `!Send`.
struct NestedSeed<'a> {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
}

impl<'de> DeserializeSeed<'de> for NestedSeed<'_> {
    type Value = DynamicMessage;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        let md = self.pool.message(self.msg_idx);
        if let Some(wkt) = WktKind::from_full_name(&md.full_name) {
            return wkt.deserialize_message(
                self.pool,
                self.msg_idx,
                d,
                self.ignore_unknown,
                self.budget,
            );
        }
        d.deserialize_map(MessageVisitor {
            pool: self.pool,
            msg_idx: self.msg_idx,
            ignore_unknown: self.ignore_unknown,
            budget: self.budget,
        })
    }
}

struct MessageVisitor<'a> {
    pool: Arc<DescriptorPool>,
    msg_idx: MessageIndex,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
}

impl<'de> Visitor<'de> for MessageVisitor<'_> {
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
            let enum_type = fd.enum_type;
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
                enum_type,
                ignore_unknown: self.ignore_unknown,
                budget: self.budget,
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
                // Re-resolve the field by number (the `fd` borrow was
                // released before `next_value_seed` mutated `msg`). The
                // declared-field lookup misses extension numbers, so fall
                // back to the pool's extension index. Membership is proven
                // by construction here — the field was just resolved from
                // this message's own pool — and oneof siblings were cleared
                // above, so insert directly instead of paying `set`'s
                // re-validation lookup and duplicate oneof sweep per field.
                if self.pool.message(self.msg_idx).field(number).is_some()
                    || self.pool.extension_for(self.msg_idx, number).is_some()
                {
                    msg.insert_value(number, v);
                }
            }
        }
        Ok(msg)
    }
}

struct FieldSeed<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: FieldKind,
    enum_type: Option<EnumType>,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
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
                enum_type: self.enum_type,
                ignore_unknown: self.ignore_unknown,
                budget: self.budget,
            }
            .deserialize(d),
            FieldKind::List(sk) => d.deserialize_any(ListVisitor {
                pool: self.pool,
                kind: sk,
                enum_type: self.enum_type,
                ignore_unknown: self.ignore_unknown,
                budget: self.budget,
            }),
            FieldKind::Map { key, value } => d.deserialize_any(MapFieldVisitor {
                pool: self.pool,
                key,
                value,
                enum_type: self.enum_type,
                ignore_unknown: self.ignore_unknown,
                budget: self.budget,
            }),
        }
    }
}

struct SingularSeed<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: SingularKind,
    enum_type: Option<EnumType>,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
}

impl<'de> DeserializeSeed<'de> for SingularSeed<'_> {
    type Value = Option<Value>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        match self.kind {
            SingularKind::Scalar(sc) => deserialize_optional_scalar(sc, d),
            SingularKind::Enum(eidx) => deserialize_enum(self.pool, eidx, self.enum_type, d),
            SingularKind::Message(midx) => {
                // `google.protobuf.Value` treats JSON `null` as a present
                // `null_value` member, not "unset" — dispatch straight to the
                // WKT seed so its visitor sees the unit token.
                if self.pool.message(midx).full_name == "google.protobuf.Value" {
                    return NestedSeed {
                        pool: Arc::clone(self.pool),
                        msg_idx: midx,
                        ignore_unknown: self.ignore_unknown,
                        budget: self.budget,
                    }
                    .deserialize(d)
                    .map(|m| Some(Value::Message(m)));
                }
                // Otherwise, JSON `null` maps to `None` ("unset").
                d.deserialize_option(NestedMessageVisitor {
                    pool: self.pool,
                    midx,
                    ignore_unknown: self.ignore_unknown,
                    budget: self.budget,
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
    budget: &'a Cell<usize>,
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
        NestedSeed {
            pool: Arc::clone(self.pool),
            msg_idx: self.midx,
            ignore_unknown: self.ignore_unknown,
            budget: self.budget,
        }
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
            scalar_from_f64(self.0, v)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            scalar_from_str(self.0, v)
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

/// Integer and `float` scalars go through the same serde with-modules
/// generated messages use (`json_helpers::{int32, uint32, int64, uint64,
/// float}`), so the reflective decoder accepts and rejects exactly what the
/// generated one does.
///
/// For the integer types, quoted decimal and exponent forms parse exactly
/// across the full range, and unquoted floats are rejected above the
/// magnitude where serde_json's float parsing can no longer identify the
/// token uniquely.
fn scalar_from_f64<E: de::Error>(sc: ScalarType, v: f64) -> Result<Value, E> {
    let d = v.into_deserializer();
    Ok(match sc {
        ScalarType::Float => Value::F32(json_helpers::float::deserialize(d)?),
        ScalarType::Double => Value::F64(v),
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => {
            Value::I32(json_helpers::int32::deserialize(d)?)
        }
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => {
            Value::I64(json_helpers::int64::deserialize(d)?)
        }
        ScalarType::Uint32 | ScalarType::Fixed32 => {
            Value::U32(json_helpers::uint32::deserialize(d)?)
        }
        ScalarType::Uint64 | ScalarType::Fixed64 => {
            Value::U64(json_helpers::uint64::deserialize(d)?)
        }
        ScalarType::Bool | ScalarType::String | ScalarType::Bytes => {
            return Err(E::invalid_type(
                de::Unexpected::Float(v),
                &"a JSON value for this field",
            ));
        }
    })
}

fn scalar_from_str<E: de::Error>(sc: ScalarType, v: &str) -> Result<Value, E> {
    let d = v.into_deserializer();
    Ok(match sc {
        ScalarType::String => Value::String(v.to_owned()),
        ScalarType::Bytes => Value::Bytes(json_helpers::bytes::deserialize(d)?),
        // 64-bit integers are quoted strings; 32-bit ones may be. Spec also
        // accepts decimal and exponential notation as long as the value is
        // integral — see `scalar_from_f64` for why the shared modules do it.
        ScalarType::Int64 | ScalarType::Sint64 | ScalarType::Sfixed64 => {
            Value::I64(json_helpers::int64::deserialize(d)?)
        }
        ScalarType::Uint64 | ScalarType::Fixed64 => {
            Value::U64(json_helpers::uint64::deserialize(d)?)
        }
        ScalarType::Int32 | ScalarType::Sint32 | ScalarType::Sfixed32 => {
            Value::I32(json_helpers::int32::deserialize(d)?)
        }
        ScalarType::Uint32 | ScalarType::Fixed32 => {
            Value::U32(json_helpers::uint32::deserialize(d)?)
        }
        // The special tokens, plus the shared range check: a quoted value
        // outside the type's range is an error, not infinity.
        ScalarType::Float => Value::F32(json_helpers::float::deserialize(d)?),
        ScalarType::Double => Value::F64(json_helpers::double::deserialize(d)?),
        ScalarType::Bool => return Err(E::custom("string is not a bool")),
    })
}

fn deserialize_enum<'de, D: Deserializer<'de>>(
    pool: &Arc<DescriptorPool>,
    eidx: EnumIndex,
    enum_type: Option<EnumType>,
    d: D,
) -> Result<Option<Value>, D::Error> {
    struct EnumVisitor<'a> {
        pool: &'a Arc<DescriptorPool>,
        eidx: EnumIndex,
        enum_type: Option<EnumType>,
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
            if self.enum_type.unwrap_or(ed.enum_type) == EnumType::Closed && ed.value(n).is_none() {
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
    d.deserialize_option(EnumVisitor {
        pool,
        eidx,
        enum_type,
    })
}

struct ListVisitor<'a> {
    pool: &'a Arc<DescriptorPool>,
    kind: SingularKind,
    enum_type: Option<EnumType>,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
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
            enum_type: self.enum_type,
            ignore_unknown: self.ignore_unknown,
            budget: self.budget,
        })? {
            // Per the spec, repeated fields cannot contain null elements.
            let v = v.ok_or_else(|| de::Error::custom("null element in repeated field"))?;
            // Scalars are charged too: each lands in a `Vec<Value>` slot, so
            // the generated decoder's packed-scalar exemption, sized for a
            // `Vec<i32>`, does not apply.
            charge(self.budget, core::mem::size_of::<Value>())?;
            out.push(v);
        }
        Ok(Some(Value::List(out)))
    }
}

struct MapFieldVisitor<'a> {
    pool: &'a Arc<DescriptorPool>,
    key: ScalarType,
    value: SingularKind,
    enum_type: Option<EnumType>,
    ignore_unknown: bool,
    budget: &'a Cell<usize>,
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
                enum_type: self.enum_type,
                ignore_unknown: self.ignore_unknown,
                budget: self.budget,
            })?;
            let v = v.ok_or_else(|| de::Error::custom("null value in map field"))?;
            charge(
                self.budget,
                core::mem::size_of::<MapKey>() + core::mem::size_of::<Value>(),
            )?;
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

// ── Well-known types ────────────────────────────────────────────────────────

include!("json_wkt.rs");

// Suppress unused warnings for the items that the WKT codec keeps.
#[allow(unused)]
const _: fn(&MessageDescriptor) = |_| {};

#[cfg(test)]
mod tests {
    use super::{field_mask_to_camel, field_mask_to_snake};

    #[test]
    fn field_mask_leading_underscore_roundtrip() {
        for (snake, camel) in [
            ("_foo", "Foo"),
            ("foo._bar", "foo.Bar"),
            ("foo._b_bar", "foo.BBar"),
        ] {
            assert_eq!(field_mask_to_camel(snake).unwrap(), camel);
            assert_eq!(field_mask_to_snake(camel).unwrap(), snake);
        }
    }
}
