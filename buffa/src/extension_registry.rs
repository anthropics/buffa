//! Runtime registry for proto3 JSON serialization of extensions.
//!
//! ProtoJSON represents extension fields with bracketed fully-qualified keys:
//! `{"[my.pkg.ext_name]": <value>}`. buffa's JSON is serde-derive based and
//! `__buffa_unknown_fields` is `#[serde(skip)]`, so extension bytes are
//! silently dropped from JSON without a registry to look them up.
//!
//! **Prefer [`json_registry`](crate::json_registry).** [`set_extension_registry`]
//! is deprecated since 0.3.0 — [`set_json_registry`] installs both the `Any`
//! registry and the extension registry in one call, and generated
//! `register_json(&mut reg)` functions populate both maps. This module remains
//! for the registry types themselves (used internally by `json_registry`) and
//! the serde helper functions called from generated code.
//!
//! [`set_json_registry`]: crate::json_registry::set_json_registry
//!
//! Codegen emits:
//! - A `#[serde(flatten)]` newtype wrapper around `__buffa_unknown_fields`
//!   whose `Serialize` impl iterates the unknown fields and emits `"[...]"` keys
//!   for any registered extensions. Unregistered field numbers are silently
//!   dropped (matching protobuf-go/protobuf-es behavior).
//! - A `"[...]"`-key arm in the generated `Deserialize` impl that resolves the
//!   bracketed name against the registry and decodes into unknown-field records.
//! - A `pub const __EXT_JSON: ExtensionRegistryEntry` per `extend` declaration,
//!   plus a `register_extensions(&mut ExtensionRegistry)` convenience function.

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};
use hashbrown::HashMap;

use crate::unknown_fields::{UnknownField, UnknownFields};

/// Registry entry for a single extension field.
///
/// The `to_json` / `from_json` function pointers are monomorphized per proto
/// type by codegen — they compose an [`ExtensionCodec`](crate::ExtensionCodec)
/// decode/encode with the proto3 JSON rules for that type (int64 stringify,
/// bytes base64, message-as-object, etc.).
pub struct ExtensionRegistryEntry {
    /// Field number on the extendee.
    pub number: u32,
    /// Fully-qualified proto name. The JSON key is `"[<this>]"`.
    pub full_name: &'static str,
    /// Fully-qualified extendee message name (no leading dot). Used both for
    /// the serialize-side `(extendee, number)` lookup and for rejecting
    /// wrong-extendee parses on the deserialize side.
    pub extendee: &'static str,
    /// Extract this extension's value from the extendee's unknown fields and
    /// serialize it to a JSON value.
    pub to_json: fn(u32, &UnknownFields) -> Result<serde_json::Value, String>,
    /// Parse a JSON value into unknown-field records at the given number.
    pub from_json: fn(serde_json::Value, u32) -> Result<Vec<UnknownField>, String>,
}

/// A registry mapping extension identities to JSON encode/decode functions.
///
/// Two lookup axes: serialize looks up by `(extendee, number)` (iterate
/// unknown fields → is this number a known extension of this message?),
/// deserialize looks up by `full_name` (saw `"[pkg.ext]"` → what is this?).
pub struct ExtensionRegistry {
    by_number: HashMap<(String, u32), ExtensionRegistryEntry>,
    by_name: HashMap<String, (String, u32)>,
}

impl ExtensionRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            by_number: HashMap::new(),
            by_name: HashMap::new(),
        }
    }

    /// Registers an entry. Replaces any existing entry at the same
    /// `(extendee, number)` or `full_name`.
    pub fn register(&mut self, entry: ExtensionRegistryEntry) {
        let key = (entry.extendee.to_owned(), entry.number);
        self.by_name.insert(entry.full_name.to_owned(), key.clone());
        self.by_number.insert(key, entry);
    }

    /// Serialize-side lookup: is `number` a registered extension of `extendee`?
    pub fn by_number(&self, extendee: &str, number: u32) -> Option<&ExtensionRegistryEntry> {
        self.by_number.get(&(extendee.to_owned(), number))
    }

    /// Deserialize-side lookup: what extension is `"[full_name]"`?
    pub fn by_name(&self, full_name: &str) -> Option<&ExtensionRegistryEntry> {
        let key = self.by_name.get(full_name)?;
        self.by_number.get(key)
    }
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Global install (mirrors any_registry.rs) ───────────────────────────────

static REGISTRY: AtomicPtr<ExtensionRegistry> = AtomicPtr::new(core::ptr::null_mut());

/// Install the global extension registry.
///
/// Set once at startup, before any JSON serialization or deserialization that
/// involves extension fields. The registry is leaked (lives for the program
/// lifetime); subsequent calls leak the old registry.
#[deprecated(since = "0.3.0", note = "use buffa::json_registry::set_json_registry")]
pub fn set_extension_registry(reg: Box<ExtensionRegistry>) {
    let old = REGISTRY.swap(Box::into_raw(reg), Ordering::Release);
    if !old.is_null() {
        // Leak the old pointer rather than drop it — another thread may
        // still hold a reference obtained before the swap.
        let _ = old;
    }
}

/// Access the global extension registry, or `None` if not yet installed.
pub fn extension_registry() -> Option<&'static ExtensionRegistry> {
    let ptr = REGISTRY.load(Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        // SAFETY: the pointer came from Box::into_raw in set_extension_registry
        // and is never freed. Acquire synchronizes with the Release store.
        Some(unsafe { &*ptr })
    }
}

// ── Serialize / Deserialize hooks called by generated code ─────────────────

/// Serialize an extendee's unknown fields as `"[full_name]": <value>` entries.
///
/// Called from the generated per-message `#[serde(flatten)]` wrapper's
/// `Serialize` impl. Iterates the unknown fields; for each field number with
/// a registered extension of `extendee`, emits a JSON key/value pair.
/// Unregistered numbers are silently dropped (they remain wire-only).
pub fn serialize_extensions<S: serde::Serializer>(
    extendee: &str,
    fields: &UnknownFields,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::{Error, SerializeMap};

    let mut map = serializer.serialize_map(None)?;

    // Fast path: no registry (user never called `set_extension_registry`) or
    // no unknown fields → nothing to emit. Without this, the dedup loop below
    // runs its O(k²) scan even when every iteration is a no-op.
    let Some(reg) = extension_registry() else {
        return map.end();
    };
    if fields.is_empty() {
        return map.end();
    }

    // Emit each extension once: the first record at a field number triggers
    // the lookup; the entry's to_json reads all records at that number (for
    // merge-semantics on messages, last-wins on scalars, collect on repeated).
    let mut seen: Vec<u32> = Vec::new();
    for uf in fields.iter() {
        if seen.contains(&uf.number) {
            continue;
        }
        seen.push(uf.number);
        if let Some(entry) = reg.by_number(extendee, uf.number) {
            let json = (entry.to_json)(uf.number, fields).map_err(S::Error::custom)?;
            let key = format!("[{}]", entry.full_name);
            map.serialize_entry(&key, &json)?;
        }
    }
    map.end()
}

/// Handle a single `"[...]"`-format JSON key during deserialization.
///
/// Called from generated `Deserialize` impls' unknown-key arms. Returns:
/// - `None` if `key` is not in `"[...]"` format — the caller should treat it
///   as a plain unknown key (ignore per current behavior).
/// - `Some(Ok(records))` if the bracketed name resolved to a registered
///   extension of `extendee` and the value decoded successfully. The caller
///   pushes these records into `__buffa_unknown_fields`.
/// - `Some(Err(..))` if the bracketed name extends a different message, the
///   value failed to decode, or [`JsonParseOptions::strict_extension_keys`]
///   is set and the name is not in the registry.
///
/// [`JsonParseOptions::strict_extension_keys`]: crate::json::JsonParseOptions::strict_extension_keys
pub fn deserialize_extension_key(
    extendee: &str,
    key: &str,
    value: serde_json::Value,
) -> Option<Result<Vec<UnknownField>, String>> {
    let name = key.strip_prefix('[')?.strip_suffix(']')?;
    // Unregistered `[...]` keys are treated as unknown (ignored) by default —
    // before the registry existed, ALL unknown JSON keys were silently dropped
    // by serde's derive, and failing here would regress that for users whose
    // upstream sends extensions they don't care about.
    //
    // `JsonParseOptions::strict_extension_keys` opts into erroring on miss
    // (protobuf-go/es behavior). Extendee mismatch and decode failures always
    // error regardless — those are contract violations, not mere misses.
    let Some(entry) = extension_registry().and_then(|r| r.by_name(name)) else {
        if crate::json::strict_extension_keys() {
            return Some(Err(format!("extension `{name}` not in registry")));
        }
        return None;
    };
    if entry.extendee != extendee {
        return Some(Err(format!(
            "extension `{name}` extends `{}`, not `{extendee}`",
            entry.extendee
        )));
    }
    Some((entry.from_json)(value, entry.number))
}

/// `Deserialize` visitor for the generated `#[serde(flatten)]` wrapper.
///
/// When a message uses `#[derive(Deserialize)]` (no oneofs), serde's flatten
/// collects all keys the outer struct didn't claim and passes them here. We
/// handle `"[...]"` keys via the registry and silently ignore the rest
/// (matching buffa's existing lenient unknown-key behavior).
pub fn deserialize_extensions<'de, D: serde::Deserializer<'de>>(
    extendee: &'static str,
    deserializer: D,
) -> Result<UnknownFields, D::Error> {
    use serde::de::{Error, MapAccess, Visitor};

    struct V {
        extendee: &'static str,
    }
    impl<'de> Visitor<'de> for V {
        type Value = UnknownFields;
        fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            f.write_str("extension map")
        }
        fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<UnknownFields, M::Error> {
            let mut out = UnknownFields::new();
            while let Some(key) = map.next_key::<String>()? {
                let value: serde_json::Value = map.next_value()?;
                match deserialize_extension_key(self.extendee, &key, value) {
                    Some(Ok(records)) => {
                        for r in records {
                            out.push(r);
                        }
                    }
                    Some(Err(e)) => return Err(M::Error::custom(e)),
                    None => {} // non-bracket key — ignore
                }
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(V { extendee })
}

// ── Per-type JSON converters (codegen points entry.to_json / from_json here)
//
// Each proto scalar type has distinct proto3 JSON rules. These helpers
// compose the existing ExtensionCodec decode/encode with those rules so
// codegen just picks a function pointer — no closures emitted.

/// Per-type JSON converters for [`ExtensionRegistryEntry`] function pointers.
pub mod helpers {
    use super::*;
    use crate::extension::codecs::*;
    use crate::extension::ExtensionCodec;
    use crate::unknown_fields::{UnknownField, UnknownFieldData};

    fn missing(n: u32) -> String {
        format!("extension field {n}: no value present")
    }

    /// JSON value → single-varint `UnknownField`. Handles both number and
    /// string forms (proto3 JSON accepts either for all integer types).
    fn json_int<T>(
        v: serde_json::Value,
        n: u32,
        encode: fn(T) -> u64,
    ) -> Result<Vec<UnknownField>, String>
    where
        T: TryFrom<i64> + core::str::FromStr,
        T::Error: core::fmt::Display,
        <T as core::str::FromStr>::Err: core::fmt::Display,
    {
        let i: T = match v {
            serde_json::Value::Number(num) => {
                let x = num
                    .as_i64()
                    .ok_or_else(|| format!("field {n}: not an integer"))?;
                T::try_from(x).map_err(|e| format!("field {n}: {e}"))?
            }
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Varint(encode(i)),
        }])
    }

    fn json_uint<T>(
        v: serde_json::Value,
        n: u32,
        encode: fn(T) -> u64,
    ) -> Result<Vec<UnknownField>, String>
    where
        T: TryFrom<u64> + core::str::FromStr,
        T::Error: core::fmt::Display,
        <T as core::str::FromStr>::Err: core::fmt::Display,
    {
        let i: T = match v {
            serde_json::Value::Number(num) => {
                let x = num
                    .as_u64()
                    .ok_or_else(|| format!("field {n}: not an unsigned integer"))?;
                T::try_from(x).map_err(|e| format!("field {n}: {e}"))?
            }
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Varint(encode(i)),
        }])
    }

    // ── 32-bit integers: JSON number ────────────────────────────────────────

    pub fn int32_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Int32::decode(n, f)
            .map(|v| serde_json::json!(v))
            .ok_or_else(|| missing(n))
    }
    pub fn int32_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_int::<i32>(v, n, |v| v as i64 as u64)
    }

    pub fn sint32_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Sint32::decode(n, f)
            .map(|v| serde_json::json!(v))
            .ok_or_else(|| missing(n))
    }
    pub fn sint32_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_int::<i32>(v, n, |v| crate::types::zigzag_encode_i32(v) as u64)
    }

    pub fn uint32_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Uint32::decode(n, f)
            .map(|v| serde_json::json!(v))
            .ok_or_else(|| missing(n))
    }
    pub fn uint32_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_uint::<u32>(v, n, |v| v as u64)
    }

    pub fn sfixed32_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Sfixed32::decode(n, f)
            .map(|v| serde_json::json!(v))
            .ok_or_else(|| missing(n))
    }
    pub fn sfixed32_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let i: i32 = match v {
            serde_json::Value::Number(num) => num
                .as_i64()
                .and_then(|x| i32::try_from(x).ok())
                .ok_or_else(|| format!("field {n}: not an i32"))?,
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed32(i as u32),
        }])
    }

    pub fn fixed32_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Fixed32::decode(n, f)
            .map(|v| serde_json::json!(v))
            .ok_or_else(|| missing(n))
    }
    pub fn fixed32_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let i: u32 = match v {
            serde_json::Value::Number(num) => num
                .as_u64()
                .and_then(|x| u32::try_from(x).ok())
                .ok_or_else(|| format!("field {n}: not a u32"))?,
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed32(i),
        }])
    }

    // ── 64-bit integers: JSON *string* (proto3 JSON spec) ───────────────────

    pub fn int64_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Int64::decode(n, f)
            .map(|v| serde_json::Value::String(alloc::format!("{v}")))
            .ok_or_else(|| missing(n))
    }
    pub fn int64_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_int::<i64>(v, n, |v| v as u64)
    }

    pub fn sint64_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Sint64::decode(n, f)
            .map(|v| serde_json::Value::String(alloc::format!("{v}")))
            .ok_or_else(|| missing(n))
    }
    pub fn sint64_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_int::<i64>(v, n, crate::types::zigzag_encode_i64)
    }

    pub fn uint64_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Uint64::decode(n, f)
            .map(|v| serde_json::Value::String(alloc::format!("{v}")))
            .ok_or_else(|| missing(n))
    }
    pub fn uint64_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        json_uint::<u64>(v, n, |v| v)
    }

    pub fn sfixed64_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Sfixed64::decode(n, f)
            .map(|v| serde_json::Value::String(alloc::format!("{v}")))
            .ok_or_else(|| missing(n))
    }
    pub fn sfixed64_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let i: i64 = match v {
            serde_json::Value::Number(num) => num
                .as_i64()
                .ok_or_else(|| format!("field {n}: not an i64"))?,
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed64(i as u64),
        }])
    }

    pub fn fixed64_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Fixed64::decode(n, f)
            .map(|v| serde_json::Value::String(alloc::format!("{v}")))
            .ok_or_else(|| missing(n))
    }
    pub fn fixed64_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let i: u64 = match v {
            serde_json::Value::Number(num) => num
                .as_u64()
                .ok_or_else(|| format!("field {n}: not a u64"))?,
            serde_json::Value::String(s) => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed64(i),
        }])
    }

    // ── bool, string, bytes ─────────────────────────────────────────────────

    pub fn bool_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Bool::decode(n, f)
            .map(serde_json::Value::Bool)
            .ok_or_else(|| missing(n))
    }
    pub fn bool_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let b = v
            .as_bool()
            .ok_or_else(|| format!("field {n}: expected bool"))?;
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Varint(b as u64),
        }])
    }

    pub fn string_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        StringCodec::decode(n, f)
            .map(serde_json::Value::String)
            .ok_or_else(|| missing(n))
    }
    pub fn string_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let serde_json::Value::String(s) = v else {
            return Err(format!("field {n}: expected string"));
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::LengthDelimited(s.into_bytes()),
        }])
    }

    pub fn bytes_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        use base64::Engine;
        BytesCodec::decode(n, f)
            .map(|b| serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b)))
            .ok_or_else(|| missing(n))
    }
    pub fn bytes_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        use base64::Engine;
        let serde_json::Value::String(s) = v else {
            return Err(format!("field {n}: expected base64 string"));
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|e| format!("field {n}: base64: {e}"))?;
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::LengthDelimited(bytes),
        }])
    }

    // ── float / double: JSON number OR "Infinity"/"-Infinity"/"NaN" ─────────

    pub fn float_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Float::decode(n, f)
            .map(|v| {
                if v.is_nan() {
                    serde_json::Value::String("NaN".into())
                } else if v == f32::INFINITY {
                    serde_json::Value::String("Infinity".into())
                } else if v == f32::NEG_INFINITY {
                    serde_json::Value::String("-Infinity".into())
                } else {
                    serde_json::json!(v)
                }
            })
            .ok_or_else(|| missing(n))
    }
    pub fn float_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let f: f32 = match v {
            serde_json::Value::Number(num) => {
                num.as_f64()
                    .ok_or_else(|| format!("field {n}: not a number"))? as f32
            }
            serde_json::Value::String(s) => match s.as_str() {
                "NaN" => f32::NAN,
                "Infinity" => f32::INFINITY,
                "-Infinity" => f32::NEG_INFINITY,
                _ => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            },
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed32(f.to_bits()),
        }])
    }

    pub fn double_to_json(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
        Double::decode(n, f)
            .map(|v| {
                if v.is_nan() {
                    serde_json::Value::String("NaN".into())
                } else if v == f64::INFINITY {
                    serde_json::Value::String("Infinity".into())
                } else if v == f64::NEG_INFINITY {
                    serde_json::Value::String("-Infinity".into())
                } else {
                    serde_json::json!(v)
                }
            })
            .ok_or_else(|| missing(n))
    }
    pub fn double_from_json(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
        let f: f64 = match v {
            serde_json::Value::Number(num) => num
                .as_f64()
                .ok_or_else(|| format!("field {n}: not a number"))?,
            serde_json::Value::String(s) => match s.as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                _ => s.parse().map_err(|e| format!("field {n}: {e}"))?,
            },
            _ => return Err(format!("field {n}: expected number or string")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Fixed64(f.to_bits()),
        }])
    }

    // ── Enum: proto3 JSON uses the variant name (open-enum falls back to i32)

    /// JSON encode for an enum-typed extension. Known values emit the proto
    /// variant name (e.g. `"GREEN"`); unknown `i32` values fall back to a
    /// numeric JSON value per the proto3 open-enum JSON rule.
    pub fn enum_to_json<E: crate::Enumeration>(
        n: u32,
        f: &UnknownFields,
    ) -> Result<serde_json::Value, String> {
        let i = Int32::decode(n, f).ok_or_else(|| missing(n))?;
        Ok(match E::from_i32(i) {
            Some(e) => serde_json::Value::String(e.proto_name().into()),
            None => serde_json::json!(i),
        })
    }

    /// JSON decode for an enum-typed extension. Accepts either the proto
    /// variant name string or a numeric value (proto3 JSON allows both).
    pub fn enum_from_json<E: crate::Enumeration>(
        v: serde_json::Value,
        n: u32,
    ) -> Result<Vec<UnknownField>, String> {
        let i = match v {
            serde_json::Value::String(s) => E::from_proto_name(&s)
                .map(|e| e.to_i32())
                .ok_or_else(|| format!("field {n}: unknown enum variant `{s}`"))?,
            serde_json::Value::Number(num) => num
                .as_i64()
                .and_then(|x| i32::try_from(x).ok())
                .ok_or_else(|| format!("field {n}: not an i32"))?,
            _ => return Err(format!("field {n}: expected string or number")),
        };
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::Varint(i as i64 as u64),
        }])
    }

    // ── Message-typed: uses the target type's own Serialize / Deserialize ───

    /// JSON encode for a message-typed extension.
    ///
    /// Decodes `M` from the unknown fields (merging all records per proto
    /// spec), then `serde_json::to_value` runs `M`'s own `Serialize` impl.
    pub fn message_to_json<M>(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String>
    where
        M: crate::Message + Default + serde::Serialize,
    {
        let m = MessageCodec::<M>::decode(n, f).ok_or_else(|| missing(n))?;
        serde_json::to_value(&m).map_err(|e| alloc::format!("field {n}: {e}"))
    }

    /// JSON decode for a message-typed extension.
    pub fn message_from_json<M>(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String>
    where
        M: crate::Message + Default + for<'de> serde::Deserialize<'de>,
    {
        let m: M = serde_json::from_value(v).map_err(|e| alloc::format!("field {n}: {e}"))?;
        Ok(alloc::vec![UnknownField {
            number: n,
            data: UnknownFieldData::LengthDelimited(m.encode_to_vec()),
        }])
    }

    // ── Repeated: JSON array, per-element conversion respects proto type ────
    //
    // Per-type helpers are required because each proto type has distinct JSON
    // rules (int64 stringifies, bytes base64-encodes, float has NaN/Infinity).
    // Decode side reuses the singular `*_from_json` per element; encode side
    // reuses the element-value-to-JSON closure from the singular `*_to_json`.

    /// Shared array-decode driver: apply `from_one` to each element,
    /// concatenating the resulting unknown-field records.
    fn repeated_from_array(
        v: serde_json::Value,
        n: u32,
        from_one: fn(serde_json::Value, u32) -> Result<Vec<UnknownField>, String>,
    ) -> Result<Vec<UnknownField>, String> {
        let serde_json::Value::Array(arr) = v else {
            return Err(format!("field {n}: expected array"));
        };
        let mut out = Vec::with_capacity(arr.len());
        for elem in arr {
            out.extend(from_one(elem, n)?);
        }
        Ok(out)
    }

    /// Emit `repeated_<name>_{to,from}_json` for a scalar codec. `$to_elem`
    /// maps one decoded value to a `serde_json::Value` (capturing the
    /// type-specific JSON rule); `$from_one` is the singular `*_from_json`
    /// helper, reused per element.
    macro_rules! repeated_scalar {
        ($to_name:ident, $from_name:ident, $codec:ty, $to_elem:expr, $from_one:path) => {
            pub fn $to_name(n: u32, f: &UnknownFields) -> Result<serde_json::Value, String> {
                let vs = Repeated::<$codec>::decode(n, f);
                Ok(serde_json::Value::Array(
                    vs.into_iter().map($to_elem).collect(),
                ))
            }
            pub fn $from_name(v: serde_json::Value, n: u32) -> Result<Vec<UnknownField>, String> {
                repeated_from_array(v, n, $from_one)
            }
        };
    }

    // Shared element-value-to-JSON closures for reuse between singular and
    // repeated float/double encode (the NaN/Infinity branching is bulky).
    fn f32_to_value(v: f32) -> serde_json::Value {
        if v.is_nan() {
            serde_json::Value::String("NaN".into())
        } else if v == f32::INFINITY {
            serde_json::Value::String("Infinity".into())
        } else if v == f32::NEG_INFINITY {
            serde_json::Value::String("-Infinity".into())
        } else {
            serde_json::json!(v)
        }
    }
    fn f64_to_value(v: f64) -> serde_json::Value {
        if v.is_nan() {
            serde_json::Value::String("NaN".into())
        } else if v == f64::INFINITY {
            serde_json::Value::String("Infinity".into())
        } else if v == f64::NEG_INFINITY {
            serde_json::Value::String("-Infinity".into())
        } else {
            serde_json::json!(v)
        }
    }

    // 32-bit integers → JSON number.
    repeated_scalar!(
        repeated_int32_to_json,
        repeated_int32_from_json,
        Int32,
        |v| serde_json::json!(v),
        int32_from_json
    );
    repeated_scalar!(
        repeated_sint32_to_json,
        repeated_sint32_from_json,
        Sint32,
        |v| serde_json::json!(v),
        sint32_from_json
    );
    repeated_scalar!(
        repeated_uint32_to_json,
        repeated_uint32_from_json,
        Uint32,
        |v| serde_json::json!(v),
        uint32_from_json
    );
    repeated_scalar!(
        repeated_sfixed32_to_json,
        repeated_sfixed32_from_json,
        Sfixed32,
        |v| serde_json::json!(v),
        sfixed32_from_json
    );
    repeated_scalar!(
        repeated_fixed32_to_json,
        repeated_fixed32_from_json,
        Fixed32,
        |v| serde_json::json!(v),
        fixed32_from_json
    );
    // 64-bit integers → JSON string (proto3 JSON spec).
    repeated_scalar!(
        repeated_int64_to_json,
        repeated_int64_from_json,
        Int64,
        |v| serde_json::Value::String(alloc::format!("{v}")),
        int64_from_json
    );
    repeated_scalar!(
        repeated_sint64_to_json,
        repeated_sint64_from_json,
        Sint64,
        |v| serde_json::Value::String(alloc::format!("{v}")),
        sint64_from_json
    );
    repeated_scalar!(
        repeated_uint64_to_json,
        repeated_uint64_from_json,
        Uint64,
        |v| serde_json::Value::String(alloc::format!("{v}")),
        uint64_from_json
    );
    repeated_scalar!(
        repeated_sfixed64_to_json,
        repeated_sfixed64_from_json,
        Sfixed64,
        |v| serde_json::Value::String(alloc::format!("{v}")),
        sfixed64_from_json
    );
    repeated_scalar!(
        repeated_fixed64_to_json,
        repeated_fixed64_from_json,
        Fixed64,
        |v| serde_json::Value::String(alloc::format!("{v}")),
        fixed64_from_json
    );
    // bool / string / bytes.
    repeated_scalar!(
        repeated_bool_to_json,
        repeated_bool_from_json,
        Bool,
        serde_json::Value::Bool,
        bool_from_json
    );
    repeated_scalar!(
        repeated_string_to_json,
        repeated_string_from_json,
        StringCodec,
        serde_json::Value::String,
        string_from_json
    );
    repeated_scalar!(
        repeated_bytes_to_json,
        repeated_bytes_from_json,
        BytesCodec,
        |b| {
            use base64::Engine;
            serde_json::Value::String(base64::engine::general_purpose::STANDARD.encode(b))
        },
        bytes_from_json
    );
    // float / double (NaN / Infinity special-cased).
    repeated_scalar!(
        repeated_float_to_json,
        repeated_float_from_json,
        Float,
        f32_to_value,
        float_from_json
    );
    repeated_scalar!(
        repeated_double_to_json,
        repeated_double_from_json,
        Double,
        f64_to_value,
        double_from_json
    );

    /// JSON encode for a repeated enum-typed extension: each known element
    /// becomes its variant-name string; unknown elements fall back to numeric.
    pub fn repeated_enum_to_json<E: crate::Enumeration>(
        n: u32,
        f: &UnknownFields,
    ) -> Result<serde_json::Value, String> {
        let vs = Repeated::<EnumI32>::decode(n, f);
        Ok(serde_json::Value::Array(
            vs.into_iter()
                .map(|i| match E::from_i32(i) {
                    Some(e) => serde_json::Value::String(e.proto_name().into()),
                    None => serde_json::json!(i),
                })
                .collect(),
        ))
    }

    /// JSON decode for a repeated enum-typed extension.
    pub fn repeated_enum_from_json<E: crate::Enumeration>(
        v: serde_json::Value,
        n: u32,
    ) -> Result<Vec<UnknownField>, String> {
        repeated_from_array(v, n, enum_from_json::<E>)
    }

    /// JSON encode for a repeated message-typed extension: each element runs
    /// `M`'s own `Serialize` impl.
    pub fn repeated_message_to_json<M>(
        n: u32,
        f: &UnknownFields,
    ) -> Result<serde_json::Value, String>
    where
        M: crate::Message + Default + serde::Serialize,
    {
        let ms = Repeated::<MessageCodec<M>>::decode(n, f);
        let mut arr = Vec::with_capacity(ms.len());
        for m in ms {
            arr.push(serde_json::to_value(&m).map_err(|e| alloc::format!("field {n}: {e}"))?);
        }
        Ok(serde_json::Value::Array(arr))
    }

    /// JSON decode for a repeated message-typed extension.
    pub fn repeated_message_from_json<M>(
        v: serde_json::Value,
        n: u32,
    ) -> Result<Vec<UnknownField>, String>
    where
        M: crate::Message + Default + for<'de> serde::Deserialize<'de>,
    {
        repeated_from_array(v, n, message_from_json::<M>)
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(deprecated)]

    use super::helpers::*;
    use super::*;
    use crate::unknown_fields::{UnknownField, UnknownFieldData};
    use alloc::vec;

    fn fields_with(uf: UnknownField) -> UnknownFields {
        let mut f = UnknownFields::new();
        f.push(uf);
        f
    }

    #[test]
    fn int32_roundtrip() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(42),
        });
        let json = int32_to_json(1, &f).unwrap();
        assert_eq!(json, serde_json::json!(42));
        let back = int32_from_json(json, 1).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].data, UnknownFieldData::Varint(42));
    }

    #[test]
    fn int32_from_string() {
        // Proto3 JSON accepts string form for all integer types.
        let back = int32_from_json(serde_json::json!("42"), 1).unwrap();
        assert_eq!(back[0].data, UnknownFieldData::Varint(42));
    }

    #[test]
    fn int32_negative_sign_extends() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint((-7_i64) as u64),
        });
        let json = int32_to_json(1, &f).unwrap();
        assert_eq!(json, serde_json::json!(-7));
        let back = int32_from_json(json, 1).unwrap();
        assert_eq!(back[0].data, UnknownFieldData::Varint((-7_i64) as u64));
    }

    #[test]
    fn int64_stringifies() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(9_999_999_999),
        });
        let json = int64_to_json(1, &f).unwrap();
        assert_eq!(json, serde_json::json!("9999999999"));
        let back = int64_from_json(json, 1).unwrap();
        assert_eq!(back[0].data, UnknownFieldData::Varint(9_999_999_999));
    }

    #[test]
    fn sint32_zigzag_roundtrip() {
        // zigzag(-1) = 1
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(1),
        });
        let json = sint32_to_json(1, &f).unwrap();
        assert_eq!(json, serde_json::json!(-1));
        let back = sint32_from_json(json, 1).unwrap();
        assert_eq!(back[0].data, UnknownFieldData::Varint(1));
    }

    #[test]
    fn bool_roundtrip() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(1),
        });
        assert_eq!(bool_to_json(1, &f).unwrap(), serde_json::json!(true));
        let back = bool_from_json(serde_json::json!(false), 1).unwrap();
        assert_eq!(back[0].data, UnknownFieldData::Varint(0));
    }

    #[test]
    fn string_roundtrip() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::LengthDelimited(b"hello".to_vec()),
        });
        assert_eq!(string_to_json(1, &f).unwrap(), serde_json::json!("hello"));
        let back = string_from_json(serde_json::json!("world"), 1).unwrap();
        assert_eq!(
            back[0].data,
            UnknownFieldData::LengthDelimited(b"world".to_vec())
        );
    }

    #[test]
    fn bytes_base64_roundtrip() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::LengthDelimited(vec![0xDE, 0xAD, 0xBE, 0xEF]),
        });
        let json = bytes_to_json(1, &f).unwrap();
        assert_eq!(json, serde_json::json!("3q2+7w=="));
        let back = bytes_from_json(json, 1).unwrap();
        assert_eq!(
            back[0].data,
            UnknownFieldData::LengthDelimited(vec![0xDE, 0xAD, 0xBE, 0xEF])
        );
    }

    #[test]
    fn float_special_values() {
        for (bits, expected) in [
            (f32::NAN.to_bits(), "NaN"),
            (f32::INFINITY.to_bits(), "Infinity"),
            (f32::NEG_INFINITY.to_bits(), "-Infinity"),
        ] {
            let f = fields_with(UnknownField {
                number: 1,
                data: UnknownFieldData::Fixed32(bits),
            });
            assert_eq!(float_to_json(1, &f).unwrap(), serde_json::json!(expected));
        }
        let back = float_from_json(serde_json::json!("NaN"), 1).unwrap();
        let UnknownFieldData::Fixed32(b) = back[0].data else {
            panic!()
        };
        assert!(f32::from_bits(b).is_nan());
    }

    #[test]
    fn registry_lookup_both_axes() {
        let mut reg = ExtensionRegistry::new();
        reg.register(ExtensionRegistryEntry {
            number: 120,
            full_name: "pkg.ext",
            extendee: "pkg.Msg",
            to_json: int32_to_json,
            from_json: int32_from_json,
        });
        assert!(reg.by_number("pkg.Msg", 120).is_some());
        assert!(reg.by_number("pkg.Msg", 999).is_none());
        assert!(reg.by_number("other.Msg", 120).is_none());
        assert_eq!(reg.by_name("pkg.ext").unwrap().number, 120);
        assert!(reg.by_name("pkg.nonexistent").is_none());
    }

    #[test]
    fn deserialize_extension_key_shapes() {
        let mut reg = ExtensionRegistry::new();
        reg.register(ExtensionRegistryEntry {
            number: 120,
            full_name: "pkg.ext",
            extendee: "pkg.Msg",
            to_json: int32_to_json,
            from_json: int32_from_json,
        });
        set_extension_registry(Box::new(reg));

        // Non-bracket key → None (caller ignores).
        assert!(
            deserialize_extension_key("pkg.Msg", "regular_key", serde_json::json!(1)).is_none()
        );
        // Registered + correct extendee → Ok.
        let result =
            deserialize_extension_key("pkg.Msg", "[pkg.ext]", serde_json::json!(42)).unwrap();
        assert_eq!(result.unwrap()[0].data, UnknownFieldData::Varint(42));
        // Wrong extendee → error.
        let result =
            deserialize_extension_key("other.Msg", "[pkg.ext]", serde_json::json!(1)).unwrap();
        assert!(result.is_err());
        // Unregistered → None (treated as unknown key, not an error —
        // preserves pre-registry behavior where unknown JSON keys were
        // silently ignored). The default is lenient.
        assert!(
            deserialize_extension_key("pkg.Msg", "[pkg.missing]", serde_json::json!(1)).is_none()
        );
    }

    #[test]
    #[cfg(feature = "std")]
    fn deserialize_extension_key_strict_mode() {
        use crate::json::{with_json_parse_options, JsonParseOptions};

        // Install a fresh registry with `pkg.ext` but not `pkg.missing`. This
        // test previously leaned on leaked state from a sibling test, which
        // raced against other global-registry installers under `cargo test`.
        let mut reg = ExtensionRegistry::new();
        reg.register(ExtensionRegistryEntry {
            number: 120,
            full_name: "pkg.ext",
            extendee: "pkg.Msg",
            to_json: int32_to_json,
            from_json: int32_from_json,
        });
        set_extension_registry(Box::new(reg));

        let strict = JsonParseOptions::new().strict_extension_keys(true);

        with_json_parse_options(&strict, || {
            // Unregistered → error (go/es behavior).
            let result =
                deserialize_extension_key("pkg.Msg", "[pkg.missing]", serde_json::json!(1));
            let err = result.expect("Some in strict mode").unwrap_err();
            assert!(err.contains("not in registry"), "{err}");

            // Non-bracket keys still ignored — strict mode is about
            // `[...]`-format keys specifically.
            assert!(
                deserialize_extension_key("pkg.Msg", "plain_key", serde_json::json!(1)).is_none()
            );

            // Registered + matching → still Ok.
            let result = deserialize_extension_key("pkg.Msg", "[pkg.ext]", serde_json::json!(42));
            assert_eq!(
                result.unwrap().unwrap()[0].data,
                UnknownFieldData::Varint(42)
            );
        });

        // Scope restored — lenient again outside the closure.
        assert!(
            deserialize_extension_key("pkg.Msg", "[pkg.missing]", serde_json::json!(1)).is_none()
        );
    }

    #[test]
    fn serialize_extensions_via_registry() {
        // A global is already set by the previous test — add a fresh entry.
        let mut reg = ExtensionRegistry::new();
        reg.register(ExtensionRegistryEntry {
            number: 50,
            full_name: "pkg.weight",
            extendee: "pkg.Carrier",
            to_json: int32_to_json,
            from_json: int32_from_json,
        });
        set_extension_registry(Box::new(reg));

        let mut fields = UnknownFields::new();
        fields.push(UnknownField {
            number: 50,
            data: UnknownFieldData::Varint(7),
        });
        // Unregistered number — should be silently dropped.
        fields.push(UnknownField {
            number: 99,
            data: UnknownFieldData::Varint(0),
        });

        let json = serde_json::to_value(SerWrap {
            extendee: "pkg.Carrier",
            fields: &fields,
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({"[pkg.weight]": 7}));
    }

    // Helper: a Serialize wrapper that calls serialize_extensions.
    struct SerWrap<'a> {
        extendee: &'static str,
        fields: &'a UnknownFields,
    }
    impl serde::Serialize for SerWrap<'_> {
        fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            serialize_extensions(self.extendee, self.fields, s)
        }
    }

    // ── Enum helpers ────────────────────────────────────────────────────────

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    enum Color {
        Red = 0,
        Green = 1,
        Blue = 2,
    }
    impl crate::Enumeration for Color {
        fn from_i32(v: i32) -> Option<Self> {
            match v {
                0 => Some(Color::Red),
                1 => Some(Color::Green),
                2 => Some(Color::Blue),
                _ => None,
            }
        }
        fn to_i32(&self) -> i32 {
            *self as i32
        }
        fn proto_name(&self) -> &'static str {
            match self {
                Color::Red => "RED",
                Color::Green => "GREEN",
                Color::Blue => "BLUE",
            }
        }
        fn from_proto_name(name: &str) -> Option<Self> {
            match name {
                "RED" => Some(Color::Red),
                "GREEN" => Some(Color::Green),
                "BLUE" => Some(Color::Blue),
                _ => None,
            }
        }
    }

    #[test]
    fn enum_to_json_known_variant_emits_name() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(1), // GREEN
        });
        assert_eq!(
            enum_to_json::<Color>(1, &f).unwrap(),
            serde_json::json!("GREEN")
        );
    }

    #[test]
    fn enum_to_json_unknown_falls_back_to_numeric() {
        let f = fields_with(UnknownField {
            number: 1,
            data: UnknownFieldData::Varint(99),
        });
        assert_eq!(enum_to_json::<Color>(1, &f).unwrap(), serde_json::json!(99));
    }

    #[test]
    fn enum_from_json_accepts_name_and_number() {
        let by_name = enum_from_json::<Color>(serde_json::json!("BLUE"), 1).unwrap();
        assert_eq!(by_name[0].data, UnknownFieldData::Varint(2));
        let by_num = enum_from_json::<Color>(serde_json::json!(1), 1).unwrap();
        assert_eq!(by_num[0].data, UnknownFieldData::Varint(1));
    }

    #[test]
    fn enum_from_json_unknown_name_errors() {
        let err = enum_from_json::<Color>(serde_json::json!("PURPLE"), 1).unwrap_err();
        assert!(err.contains("unknown enum variant"), "{err}");
    }

    // ── Repeated helpers ────────────────────────────────────────────────────

    fn fields_from(records: Vec<UnknownField>) -> UnknownFields {
        let mut f = UnknownFields::new();
        for r in records {
            f.push(r);
        }
        f
    }

    #[test]
    fn repeated_int32_roundtrip() {
        let json = serde_json::json!([1, -2, 3]);
        let records = repeated_int32_from_json(json.clone(), 5).unwrap();
        assert_eq!(records.len(), 3);
        let f = fields_from(records);
        assert_eq!(repeated_int32_to_json(5, &f).unwrap(), json);
    }

    #[test]
    fn repeated_int64_stringifies_elements() {
        let records = repeated_int64_from_json(serde_json::json!(["7", 8]), 5).unwrap();
        let f = fields_from(records);
        // Encode always stringifies; decode accepted both string and number.
        assert_eq!(
            repeated_int64_to_json(5, &f).unwrap(),
            serde_json::json!(["7", "8"])
        );
    }

    #[test]
    fn repeated_string_roundtrip() {
        let json = serde_json::json!(["a", "b", "c"]);
        let records = repeated_string_from_json(json.clone(), 5).unwrap();
        let f = fields_from(records);
        assert_eq!(repeated_string_to_json(5, &f).unwrap(), json);
    }

    #[test]
    fn repeated_from_json_rejects_non_array() {
        let err = repeated_int32_from_json(serde_json::json!(42), 5).unwrap_err();
        assert!(err.contains("expected array"), "{err}");
    }

    #[test]
    fn repeated_enum_roundtrip() {
        let json = serde_json::json!(["RED", "BLUE"]);
        let records = repeated_enum_from_json::<Color>(json.clone(), 5).unwrap();
        assert_eq!(records.len(), 2);
        let f = fields_from(records);
        assert_eq!(repeated_enum_to_json::<Color>(5, &f).unwrap(), json);
    }

    #[test]
    fn repeated_to_json_empty_when_no_records() {
        // No records at field 5 → empty JSON array, not an error.
        let f = UnknownFields::new();
        assert_eq!(
            repeated_int32_to_json(5, &f).unwrap(),
            serde_json::json!([])
        );
    }

    #[test]
    fn enum_helpers_satisfy_fn_pointer_signature() {
        // Monomorphized generic helpers coerce to the registry's fn-pointer
        // type — this is what codegen relies on when emitting const entries.
        let _: fn(u32, &UnknownFields) -> Result<serde_json::Value, String> = enum_to_json::<Color>;
        let _: fn(serde_json::Value, u32) -> Result<Vec<UnknownField>, String> =
            enum_from_json::<Color>;
        let _: fn(u32, &UnknownFields) -> Result<serde_json::Value, String> =
            repeated_enum_to_json::<Color>;
    }
}
