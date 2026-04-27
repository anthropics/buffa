//! Unified type registry for `Any` expansion and extension brackets — JSON + text.
//!
//! `google.protobuf.Any` in both proto3 JSON and textproto needs to know how
//! to serialize the embedded message by its type URL. Extension fields in
//! both formats likewise need a lookup by `(extendee, number)` or
//! `[full_name]`. This module is the single install point for both formats:
//! populate a [`TypeRegistry`] with generated `register_types(&mut reg)` calls,
//! then [`set_type_registry`] once at startup.
//!
//! The per-format entry types are feature-split so that `json` and `text`
//! are independently enableable — no `#[cfg]` on struct-literal fields, no
//! `Option<fn>` placeholders, no cross-feature implication.
//!
//! # Usage
//!
//! ```rust,no_run
//! use buffa::type_registry::{TypeRegistry, set_type_registry};
//!
//! let mut reg = TypeRegistry::new();
//! // Per generated file — registers JSON entries (if generate_json was on)
//! // and text entries (if generate_text was on), as appropriate:
//! // my_pkg::register_types(&mut reg);
//! // Well-known types (hand-registered in buffa-types):
//! // buffa_types::register_wkt_types(&mut reg);
//! set_type_registry(reg);
//! ```
//!
//! # Codegen helpers
//!
//! [`any_to_json`] / [`any_from_json`] (under `json`) and [`any_encode_text`] /
//! [`any_merge_text`] (under `text`) are generic function pointers emitted by
//! codegen into each message's entry consts. They compose
//! `Message::decode_from_slice` / `encode_to_vec` with the message's own
//! `Serialize`/`Deserialize` or `TextFormat` impl.

use alloc::boxed::Box;

// ── JSON re-exports ────────────────────────────────────────────────────────

#[cfg(feature = "json")]
pub use crate::any_registry::JsonAnyEntry;
#[cfg(feature = "json")]
pub use crate::extension_registry::JsonExtEntry;

/// Deprecated alias for [`JsonAnyEntry`]. Retained for one release cycle.
#[cfg(feature = "json")]
#[deprecated(since = "0.3.0", note = "renamed to JsonAnyEntry")]
pub type AnyTypeEntry = JsonAnyEntry;

/// Deprecated alias for [`JsonExtEntry`]. Retained for one release cycle.
#[cfg(feature = "json")]
#[deprecated(since = "0.3.0", note = "renamed to JsonExtEntry")]
pub type ExtensionRegistryEntry = JsonExtEntry;

// ── TextAnyEntry ───────────────────────────────────────────────────────────

/// Registry entry for a single message type's textproto ↔ `Any` conversion.
///
/// Carries fn-ptrs for the `[type_url] { ... }` expanded form. Unlike the
/// pre-split unified entry, these are non-`Option` — presence in the map
/// means the type has text-format support.
#[cfg(feature = "text")]
pub struct TextAnyEntry {
    /// Full type URL (e.g. `"type.googleapis.com/google.protobuf.Duration"`).
    pub type_url: &'static str,

    /// `Any.value` bytes → textproto body: decode, write fields as `{ ... }`.
    /// Does not write the `[type_url]` name — the encoder does that.
    pub text_encode: fn(&[u8], &mut crate::text::TextEncoder<'_>) -> core::fmt::Result,

    /// Textproto `{ ... }` body → `Any.value` bytes: merge into a fresh
    /// instance, re-encode to wire bytes.
    pub text_merge: AnyTextMergeFn,
}

#[cfg(feature = "text")]
impl core::fmt::Debug for TextAnyEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextAnyEntry")
            .field("type_url", &self.type_url)
            .finish_non_exhaustive()
    }
}

/// `TextAnyEntry::text_merge` fn-pointer type (factored out for clippy's
/// `type_complexity`).
#[cfg(feature = "text")]
pub type AnyTextMergeFn =
    fn(&mut crate::text::TextDecoder<'_>) -> Result<alloc::vec::Vec<u8>, crate::text::ParseError>;

// ── TextExtEntry ───────────────────────────────────────────────────────────

/// Registry entry for a single extension field's textproto conversion.
///
/// Carries fn-ptrs for the `[pkg.ext] { ... }` bracket syntax. Covers
/// message- and group-typed extensions (the conformance-exercised forms).
#[cfg(feature = "text")]
pub struct TextExtEntry {
    /// Field number on the extendee.
    pub number: u32,
    /// Fully-qualified proto name. Emitted as `[<this>]`.
    pub full_name: &'static str,
    /// Fully-qualified extendee message name (no leading dot). Checked on
    /// decode; a mismatch is a contract violation and fails the parse.
    pub extendee: &'static str,
    /// Extract this extension's value from unknown fields and write it.
    /// Does not write the `[full_name]` name — the encoder does that.
    pub text_encode: fn(
        u32,
        &crate::unknown_fields::UnknownFields,
        &mut crate::text::TextEncoder<'_>,
    ) -> core::fmt::Result,
    /// Consume a value and produce unknown-field records.
    pub text_merge: ExtTextMergeFn,
}

#[cfg(feature = "text")]
impl core::fmt::Debug for TextExtEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextExtEntry")
            .field("number", &self.number)
            .field("full_name", &self.full_name)
            .field("extendee", &self.extendee)
            .finish_non_exhaustive()
    }
}

/// `TextExtEntry::text_merge` fn-pointer type.
#[cfg(feature = "text")]
pub type ExtTextMergeFn =
    fn(
        &mut crate::text::TextDecoder<'_>,
        u32,
    )
        -> Result<alloc::vec::Vec<crate::unknown_fields::UnknownField>, crate::text::ParseError>;

// ── Text-side inner maps ───────────────────────────────────────────────────

#[cfg(feature = "text")]
#[derive(Default)]
struct TextAnyMap {
    entries: hashbrown::HashMap<alloc::string::String, TextAnyEntry>,
}

#[cfg(feature = "text")]
impl TextAnyMap {
    fn lookup(&self, type_url: &str) -> Option<&TextAnyEntry> {
        self.entries.get(type_url)
    }
}

#[cfg(feature = "text")]
#[derive(Default)]
struct TextExtMap {
    by_number: hashbrown::HashMap<(alloc::string::String, u32), TextExtEntry>,
    by_name: hashbrown::HashMap<alloc::string::String, (alloc::string::String, u32)>,
}

#[cfg(feature = "text")]
impl TextExtMap {
    fn by_number(&self, extendee: &str, number: u32) -> Option<&TextExtEntry> {
        use alloc::borrow::ToOwned;
        self.by_number.get(&(extendee.to_owned(), number))
    }
    fn by_name(&self, full_name: &str) -> Option<&TextExtEntry> {
        let key = self.by_name.get(full_name)?;
        self.by_number.get(key)
    }
}

// ── TypeRegistry ───────────────────────────────────────────────────────────

/// Unified registry: JSON `Any` + extension entries (under `json`) and
/// text `Any` + extension entries (under `text`).
///
/// Populate with generated `register_types(&mut reg)` functions, then
/// install once with [`set_type_registry`]. The JSON half delegates to the
/// existing [`AnyRegistry`](crate::any_registry::AnyRegistry) and
/// [`ExtensionRegistry`](crate::extension_registry::ExtensionRegistry) types;
/// the text half uses parallel maps. Both axes' lookup indirection
/// (`by_name` → `by_number`) is preserved.
#[derive(Default)]
pub struct TypeRegistry {
    #[cfg(feature = "json")]
    json_any: crate::any_registry::AnyRegistry,
    #[cfg(feature = "json")]
    json_ext: crate::extension_registry::ExtensionRegistry,
    #[cfg(feature = "text")]
    text_any: TextAnyMap,
    #[cfg(feature = "text")]
    text_ext: TextExtMap,
}

impl TypeRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a JSON `Any` type entry. Replaces any existing entry for
    /// the same type URL.
    #[cfg(feature = "json")]
    pub fn register_json_any(&mut self, entry: JsonAnyEntry) {
        self.json_any.register(entry);
    }

    /// Registers a JSON extension entry. Replaces any existing entry at the
    /// same `(extendee, number)` or `full_name`.
    #[cfg(feature = "json")]
    pub fn register_json_ext(&mut self, entry: JsonExtEntry) {
        self.json_ext.register(entry);
    }

    /// Registers a text `Any` type entry.
    #[cfg(feature = "text")]
    pub fn register_text_any(&mut self, entry: TextAnyEntry) {
        use alloc::borrow::ToOwned;
        self.text_any
            .entries
            .insert(entry.type_url.to_owned(), entry);
    }

    /// Registers a text extension entry.
    #[cfg(feature = "text")]
    pub fn register_text_ext(&mut self, entry: TextExtEntry) {
        use alloc::borrow::ToOwned;
        let key = (entry.extendee.to_owned(), entry.number);
        self.text_ext
            .by_name
            .insert(entry.full_name.to_owned(), key.clone());
        self.text_ext.by_number.insert(key, entry);
    }

    /// Look up a JSON `Any` entry by type URL.
    #[cfg(feature = "json")]
    pub fn json_any_by_url(&self, type_url: &str) -> Option<&JsonAnyEntry> {
        self.json_any.lookup(type_url)
    }

    /// Look up a JSON extension entry by `(extendee, number)`.
    #[cfg(feature = "json")]
    pub fn json_ext_by_number(&self, extendee: &str, number: u32) -> Option<&JsonExtEntry> {
        self.json_ext.by_number(extendee, number)
    }

    /// Look up a JSON extension entry by full name.
    #[cfg(feature = "json")]
    pub fn json_ext_by_name(&self, full_name: &str) -> Option<&JsonExtEntry> {
        self.json_ext.by_name(full_name)
    }

    /// Look up a text `Any` entry by type URL.
    #[cfg(feature = "text")]
    pub fn text_any_by_url(&self, type_url: &str) -> Option<&TextAnyEntry> {
        self.text_any.lookup(type_url)
    }

    /// Look up a text extension entry by `(extendee, number)`.
    #[cfg(feature = "text")]
    pub fn text_ext_by_number(&self, extendee: &str, number: u32) -> Option<&TextExtEntry> {
        self.text_ext.by_number(extendee, number)
    }

    /// Look up a text extension entry by full name.
    #[cfg(feature = "text")]
    pub fn text_ext_by_name(&self, full_name: &str) -> Option<&TextExtEntry> {
        self.text_ext.by_name(full_name)
    }
}

/// Deprecated alias for [`TypeRegistry`]. The registry now covers both JSON
/// and text formats.
#[deprecated(since = "0.3.0", note = "renamed to TypeRegistry")]
pub type JsonRegistry = TypeRegistry;

impl core::fmt::Debug for TypeRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TypeRegistry").finish_non_exhaustive()
    }
}

// ── Global install ─────────────────────────────────────────────────────────

#[cfg(feature = "text")]
static TEXT_ANY: core::sync::atomic::AtomicPtr<TextAnyMap> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

#[cfg(feature = "text")]
static TEXT_EXT: core::sync::atomic::AtomicPtr<TextExtMap> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

/// Install the global type registry.
///
/// Decomposes into separate installs per format:
/// - Under `json`: delegates to the existing [`AnyRegistry`] and
///   [`ExtensionRegistry`] `AtomicPtr` globals, which generated serde impls
///   and `Any`'s serde impl consult directly.
/// - Under `text`: installs parallel text-only maps consulted by
///   [`TextEncoder::try_write_any_expanded`] / [`TextDecoder::read_any_expansion`]
///   and the extension bracket methods.
///
/// Call once at startup, before any serialization involving `Any` fields or
/// extension fields. All halves are leaked (live for the program lifetime);
/// subsequent calls leak the old allocations — same rationale as the
/// pre-existing `set_any_registry`: avoids use-after-free races with
/// concurrent readers.
///
/// [`AnyRegistry`]: crate::any_registry::AnyRegistry
/// [`ExtensionRegistry`]: crate::extension_registry::ExtensionRegistry
/// [`TextEncoder::try_write_any_expanded`]: crate::text::TextEncoder::try_write_any_expanded
/// [`TextDecoder::read_any_expansion`]: crate::text::TextDecoder::read_any_expansion
pub fn set_type_registry(reg: TypeRegistry) {
    // Destructure once, with each binding cfg'd to match its field. This is
    // the ergonomic form of per-feature field extraction — the binding only
    // exists when the field does.
    let TypeRegistry {
        #[cfg(feature = "json")]
        json_any,
        #[cfg(feature = "json")]
        json_ext,
        #[cfg(feature = "text")]
        text_any,
        #[cfg(feature = "text")]
        text_ext,
    } = reg;

    #[cfg(feature = "json")]
    #[allow(deprecated)]
    {
        crate::any_registry::set_any_registry(Box::new(json_any));
        crate::extension_registry::set_extension_registry(Box::new(json_ext));
    }

    #[cfg(feature = "text")]
    {
        use core::sync::atomic::Ordering;
        TEXT_ANY.swap(Box::into_raw(Box::new(text_any)), Ordering::Release);
        TEXT_EXT.swap(Box::into_raw(Box::new(text_ext)), Ordering::Release);
    }
}

/// Deprecated: call [`set_type_registry`]. This alias exists for one release
/// cycle to ease migration; the registry now covers text entries too.
#[deprecated(since = "0.3.0", note = "renamed to set_type_registry")]
pub fn set_json_registry(reg: TypeRegistry) {
    set_type_registry(reg);
}

// ── Text global lookup (consulted by text encoder/decoder) ─────────────────

/// Look up a text `Any` entry in the global registry. Returns `None` if no
/// registry is installed or the URL is not registered.
#[cfg(feature = "text")]
pub(crate) fn global_text_any(type_url: &str) -> Option<&'static TextAnyEntry> {
    use core::sync::atomic::Ordering;
    let ptr = TEXT_ANY.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // SAFETY: ptr came from Box::into_raw in set_type_registry and is never
    // freed. Acquire synchronizes with the Release store.
    unsafe { &*ptr }.lookup(type_url)
}

/// Look up a text extension entry by `(extendee, number)` in the global registry.
#[cfg(feature = "text")]
pub(crate) fn global_text_ext_by_number(
    extendee: &str,
    number: u32,
) -> Option<&'static TextExtEntry> {
    use core::sync::atomic::Ordering;
    let ptr = TEXT_EXT.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // SAFETY: see global_text_any.
    unsafe { &*ptr }.by_number(extendee, number)
}

/// Look up a text extension entry by full name in the global registry.
#[cfg(feature = "text")]
pub(crate) fn global_text_ext_by_name(full_name: &str) -> Option<&'static TextExtEntry> {
    use core::sync::atomic::Ordering;
    let ptr = TEXT_EXT.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // SAFETY: see global_text_any.
    unsafe { &*ptr }.by_name(full_name)
}

/// Clear the global text maps. Test cleanup only; leaks old allocations.
#[cfg(feature = "text")]
#[doc(hidden)]
pub fn clear_text_registry() {
    use core::sync::atomic::Ordering;
    TEXT_ANY.swap(core::ptr::null_mut(), Ordering::Release);
    TEXT_EXT.swap(core::ptr::null_mut(), Ordering::Release);
}

// ── JSON Any-entry converters (codegen points JsonAnyEntry fields here) ────
//
// Monomorphized per message type M; the resulting fn items coerce to the
// `fn(&[u8]) -> ...` / `fn(serde_json::Value) -> ...` pointer types on
// JsonAnyEntry. Same pattern as extension_registry::helpers::message_to_json.

/// `Any.value` bytes → JSON: decode `M` from wire bytes, then serialize via
/// `M`'s own `Serialize` impl.
///
/// Codegen emits `to_json: ::buffa::type_registry::any_to_json::<Foo>` in each
/// message's `JsonAnyEntry` const. Not intended for direct use.
#[cfg(feature = "json")]
pub fn any_to_json<M>(bytes: &[u8]) -> Result<serde_json::Value, alloc::string::String>
where
    M: crate::Message + serde::Serialize,
{
    use alloc::format;
    let m = M::decode_from_slice(bytes).map_err(|e| format!("{e}"))?;
    serde_json::to_value(&m).map_err(|e| format!("{e}"))
}

/// JSON → `Any.value` bytes: deserialize `M` via its `Deserialize` impl, then
/// encode to wire bytes.
///
/// Codegen emits `from_json: ::buffa::type_registry::any_from_json::<Foo>` in
/// each message's `JsonAnyEntry` const. Not intended for direct use.
#[cfg(feature = "json")]
pub fn any_from_json<M>(v: serde_json::Value) -> Result<alloc::vec::Vec<u8>, alloc::string::String>
where
    M: crate::Message + for<'de> serde::Deserialize<'de>,
{
    use alloc::format;
    let m: M = serde_json::from_value(v).map_err(|e| format!("{e}"))?;
    Ok(m.encode_to_vec())
}

// ── Text Any-entry converters (codegen points TextAnyEntry fields here) ────
//
// Mirror `any_to_json<M>`: monomorphized per message type M, the resulting
// fn items coerce to the `fn(...)` pointer types on TextAnyEntry.

/// `Any.value` bytes → textproto: decode `M`, write its fields as a
/// `{ ... }` message body.
///
/// Codegen emits `text_encode: ::buffa::type_registry::any_encode_text::<Foo>`
/// in each message's `TextAnyEntry` const when `generate_text` is on.
#[cfg(feature = "text")]
pub fn any_encode_text<M>(bytes: &[u8], enc: &mut crate::text::TextEncoder<'_>) -> core::fmt::Result
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    // If the stored bytes don't decode (corrupt Any), emit an empty `{}` —
    // best effort in the fmt::Result error model. Round-trip won't be perfect
    // but the output is still syntactically valid textproto. Signal in debug
    // so a corrupt Any isn't completely silent.
    let decoded = M::decode_from_slice(bytes);
    debug_assert!(
        decoded.is_ok(),
        "any_encode_text: corrupt Any.value bytes: {:?}",
        decoded.as_ref().err()
    );
    let m = decoded.unwrap_or_default();
    enc.write_map_entry(|enc| m.encode_text(enc))
}

/// Textproto `{ ... }` body → `Any.value` bytes: merge into a fresh `M`,
/// re-encode to wire bytes.
///
/// Codegen emits `text_merge: ::buffa::type_registry::any_merge_text::<Foo>`.
#[cfg(feature = "text")]
pub fn any_merge_text<M>(
    dec: &mut crate::text::TextDecoder<'_>,
) -> Result<alloc::vec::Vec<u8>, crate::text::ParseError>
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    let mut m = M::default();
    dec.merge_message(&mut m)?;
    Ok(m.encode_to_vec())
}

// ── Text extension converters (codegen points TextExtEntry fields here) ────
//
// Conformance only exercises `[pkg.ext] { ... }` for message/group-typed
// extensions. Scalar text helpers would mirror the JSON ones in
// `extension_registry::helpers`; deferred until a use case appears.

/// Textproto encode for a message-typed extension: decode `M` from the
/// unknown fields (merge semantics), write as a `{ ... }` message body.
#[cfg(feature = "text")]
pub fn message_encode_text<M>(
    n: u32,
    f: &crate::unknown_fields::UnknownFields,
    enc: &mut crate::text::TextEncoder<'_>,
) -> core::fmt::Result
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    use crate::extension::codecs::MessageCodec;
    use crate::extension::ExtensionCodec;
    let m = MessageCodec::<M>::decode(n, f).unwrap_or_default();
    enc.write_map_entry(|enc| m.encode_text(enc))
}

/// Textproto merge for a message-typed extension: consume `{ ... }`,
/// re-encode to a `LengthDelimited` unknown-field record.
#[cfg(feature = "text")]
pub fn message_merge_text<M>(
    dec: &mut crate::text::TextDecoder<'_>,
    n: u32,
) -> Result<alloc::vec::Vec<crate::unknown_fields::UnknownField>, crate::text::ParseError>
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    use crate::unknown_fields::{UnknownField, UnknownFieldData};
    let mut m = M::default();
    dec.merge_message(&mut m)?;
    Ok(alloc::vec![UnknownField {
        number: n,
        data: UnknownFieldData::LengthDelimited(m.encode_to_vec()),
    }])
}

/// Textproto encode for a group-encoded extension. Identical body to
/// [`message_encode_text`] on the encode side — only the wire-type of
/// the stored unknown field differs (`Group` vs `LengthDelimited`).
#[cfg(feature = "text")]
pub fn group_encode_text<M>(
    n: u32,
    f: &crate::unknown_fields::UnknownFields,
    enc: &mut crate::text::TextEncoder<'_>,
) -> core::fmt::Result
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    use crate::extension::codecs::GroupCodec;
    use crate::extension::ExtensionCodec;
    let m = GroupCodec::<M>::decode(n, f).unwrap_or_default();
    enc.write_map_entry(|enc| m.encode_text(enc))
}

/// Textproto merge for a group-encoded extension: consume `{ ... }`,
/// re-encode the message's fields into a `Group(UnknownFields)` record.
///
/// Same round-trip-through-bytes as [`GroupCodec::encode`]: encode `M`
/// to wire bytes, re-parse as `UnknownFields`, wrap in `Group`. The
/// inner `UnknownFields` then serialize as `SGROUP … EGROUP` on binary
/// output.
///
/// [`GroupCodec::encode`]: crate::extension::codecs::GroupCodec
#[cfg(feature = "text")]
pub fn group_merge_text<M>(
    dec: &mut crate::text::TextDecoder<'_>,
    n: u32,
) -> Result<alloc::vec::Vec<crate::unknown_fields::UnknownField>, crate::text::ParseError>
where
    M: crate::Message + crate::text::TextFormat + Default,
{
    use crate::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};
    let mut m = M::default();
    dec.merge_message(&mut m)?;
    let bytes = m.encode_to_vec();
    // Freshly-encoded → re-decode cannot fail short of an encoder bug.
    // Surface as Internal rather than panicking so a library caller
    // isn't taken down by it.
    let inner = UnknownFields::decode_from_slice(&bytes).map_err(|_| {
        crate::text::ParseError::new(
            0,
            0,
            crate::text::ParseErrorKind::Internal(
                "re-decoding freshly-encoded group message bytes failed",
            ),
        )
    })?;
    Ok(alloc::vec![UnknownField {
        number: n,
        data: UnknownFieldData::Group(inner),
    }])
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_empty() {
        let reg = TypeRegistry::default();
        #[cfg(feature = "json")]
        assert!(reg.json_any_by_url("anything").is_none());
        #[cfg(feature = "text")]
        assert!(reg.text_any_by_url("anything").is_none());
        #[cfg(not(any(feature = "json", feature = "text")))]
        let _ = reg;
    }

    #[test]
    fn debug_impl() {
        let reg = TypeRegistry::new();
        let s = alloc::format!("{reg:?}");
        assert!(s.contains("TypeRegistry"), "{s}");
    }

    // ── JSON half ───────────────────────────────────────────────────────────

    #[cfg(feature = "json")]
    mod json {
        use super::*;
        use crate::any_registry::with_any_registry;
        use crate::extension_registry::{extension_registry, helpers};

        fn dummy_to_json(_: &[u8]) -> Result<serde_json::Value, alloc::string::String> {
            Ok(serde_json::json!({"ok": true}))
        }
        fn dummy_from_json(
            _: serde_json::Value,
        ) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
            Ok(alloc::vec![1, 2, 3])
        }

        fn any_entry(url: &'static str, wkt: bool) -> JsonAnyEntry {
            JsonAnyEntry {
                type_url: url,
                to_json: dummy_to_json,
                from_json: dummy_from_json,
                is_wkt: wkt,
            }
        }

        fn ext_entry(num: u32, name: &'static str, ext: &'static str) -> JsonExtEntry {
            JsonExtEntry {
                number: num,
                full_name: name,
                extendee: ext,
                to_json: helpers::int32_to_json,
                from_json: helpers::int32_from_json,
            }
        }

        #[test]
        fn register_json_any_and_ext_independently() {
            let mut reg = TypeRegistry::new();
            reg.register_json_any(any_entry("type.googleapis.com/test.Foo", false));
            reg.register_json_ext(ext_entry(100, "test.ext", "test.Foo"));

            assert!(reg
                .json_any_by_url("type.googleapis.com/test.Foo")
                .is_some());
            assert!(reg.json_ext_by_number("test.Foo", 100).is_some());
            assert!(reg.json_ext_by_name("test.ext").is_some());
        }

        /// Serializes this test with the global-registry tests in `any_registry`
        /// and `extension_registry` — all three modules share the same two
        /// `AtomicPtr` globals.
        static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        #[test]
        fn set_type_registry_installs_json_halves() {
            let _g = GLOBAL_LOCK.lock().unwrap();

            let mut reg = TypeRegistry::new();
            reg.register_json_any(any_entry("type.googleapis.com/test.Unified", false));
            reg.register_json_ext(ext_entry(200, "test.unified_ext", "test.Unified"));
            set_type_registry(reg);

            with_any_registry(|r| {
                let r = r.expect("any registry installed");
                assert!(r.lookup("type.googleapis.com/test.Unified").is_some());
            });
            let ext = extension_registry().expect("extension registry installed");
            assert_eq!(ext.by_name("test.unified_ext").map(|e| e.number), Some(200));

            crate::any_registry::clear_any_registry();
        }

        #[test]
        #[allow(deprecated)]
        fn deprecated_aliases_compile() {
            // Type aliases resolve to the new types.
            let _: AnyTypeEntry = any_entry("x", false);
            let _: ExtensionRegistryEntry = ext_entry(1, "a", "B");
            let _: JsonRegistry = TypeRegistry::new();
            // Deprecated fn redirects.
            let _g = GLOBAL_LOCK.lock().unwrap();
            set_json_registry(TypeRegistry::new());
            crate::any_registry::clear_any_registry();
        }
    }

    // ── Text half ───────────────────────────────────────────────────────────

    #[cfg(feature = "text")]
    mod text {
        use super::*;
        use crate::unknown_fields::{UnknownField, UnknownFieldData, UnknownFields};

        // Hand-implemented Message + TextFormat to avoid depending on codegen
        // (mirrors the TestMsg pattern in text/decoder.rs tests).

        #[derive(Default, Clone, PartialEq, Debug)]
        struct Inner {
            n: i32,
        }

        impl crate::DefaultInstance for Inner {
            fn default_instance() -> &'static Self {
                static I: crate::__private::OnceBox<Inner> = crate::__private::OnceBox::new();
                I.get_or_init(|| alloc::boxed::Box::new(Inner::default()))
            }
        }

        impl crate::Message for Inner {
            fn compute_size(&self) -> u32 {
                if self.n != 0 {
                    1 + crate::encoding::varint_len(self.n as i64 as u64) as u32
                } else {
                    0
                }
            }
            fn write_to(&self, buf: &mut impl bytes::BufMut) {
                if self.n != 0 {
                    crate::encoding::Tag::new(1, crate::encoding::WireType::Varint).encode(buf);
                    crate::encoding::encode_varint(self.n as i64 as u64, buf);
                }
            }
            fn merge_field(
                &mut self,
                tag: crate::encoding::Tag,
                buf: &mut impl bytes::Buf,
                _: u32,
            ) -> Result<(), crate::DecodeError> {
                if tag.field_number() == 1 && tag.wire_type() == crate::encoding::WireType::Varint {
                    self.n = crate::encoding::decode_varint(buf)? as i32;
                    Ok(())
                } else {
                    crate::encoding::skip_field(tag, buf)
                }
            }
            fn cached_size(&self) -> u32 {
                self.compute_size()
            }
            fn clear(&mut self) {
                self.n = 0;
            }
        }

        impl crate::text::TextFormat for Inner {
            fn encode_text(&self, enc: &mut crate::text::TextEncoder<'_>) -> core::fmt::Result {
                if self.n != 0 {
                    enc.write_field_name("n")?;
                    enc.write_i32(self.n)?;
                }
                Ok(())
            }
            fn merge_text(
                &mut self,
                dec: &mut crate::text::TextDecoder<'_>,
            ) -> Result<(), crate::text::ParseError> {
                while let Some(name) = dec.read_field_name()? {
                    match name {
                        "n" => self.n = dec.read_i32()?,
                        _ => dec.skip_value()?,
                    }
                }
                Ok(())
            }
        }

        fn fields_from(records: alloc::vec::Vec<UnknownField>) -> UnknownFields {
            let mut f = UnknownFields::new();
            for r in records {
                f.push(r);
            }
            f
        }

        #[test]
        fn message_text_roundtrip() {
            let mut dec = crate::text::TextDecoder::new("f { n: 7 }");
            dec.read_field_name().unwrap();
            let records = message_merge_text::<Inner>(&mut dec, 50).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].number, 50);
            let UnknownFieldData::LengthDelimited(ref bytes) = records[0].data else {
                panic!("expected LengthDelimited, got {:?}", records[0].data);
            };
            assert_eq!(*bytes, alloc::vec![0x08, 0x07]);

            let fields = fields_from(records);
            let mut s = alloc::string::String::new();
            let mut enc = crate::text::TextEncoder::new(&mut s);
            enc.write_extension_name("pkg.ext").unwrap();
            message_encode_text::<Inner>(50, &fields, &mut enc).unwrap();
            assert_eq!(s, "[pkg.ext] {n: 7}");
        }

        #[test]
        fn group_text_roundtrip() {
            let mut dec = crate::text::TextDecoder::new("f { n: 7 }");
            dec.read_field_name().unwrap();
            let records = group_merge_text::<Inner>(&mut dec, 121).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].number, 121);
            let UnknownFieldData::Group(ref inner) = records[0].data else {
                panic!("expected Group, got {:?}", records[0].data);
            };
            let inner_vec: alloc::vec::Vec<_> = inner.iter().collect();
            assert_eq!(inner_vec.len(), 1);
            assert_eq!(inner_vec[0].number, 1);
            assert_eq!(inner_vec[0].data, UnknownFieldData::Varint(7));

            let fields = fields_from(records);
            let mut s = alloc::string::String::new();
            let mut enc = crate::text::TextEncoder::new(&mut s);
            enc.write_extension_name("pkg.groupfield").unwrap();
            group_encode_text::<Inner>(121, &fields, &mut enc).unwrap();
            assert_eq!(s, "[pkg.groupfield] {n: 7}");
        }

        #[test]
        fn helpers_satisfy_fn_pointer_signature() {
            // Codegen relies on the monomorphized helpers coercing to the
            // registry's fn-pointer types.
            let _: fn(&[u8], &mut crate::text::TextEncoder<'_>) -> core::fmt::Result =
                any_encode_text::<Inner>;
            let _: AnyTextMergeFn = any_merge_text::<Inner>;
            let _: fn(u32, &UnknownFields, &mut crate::text::TextEncoder<'_>) -> core::fmt::Result =
                message_encode_text::<Inner>;
            let _: ExtTextMergeFn = message_merge_text::<Inner>;
            let _: fn(u32, &UnknownFields, &mut crate::text::TextEncoder<'_>) -> core::fmt::Result =
                group_encode_text::<Inner>;
            let _: ExtTextMergeFn = group_merge_text::<Inner>;
        }

        #[test]
        fn register_and_lookup_text_any() {
            let mut reg = TypeRegistry::new();
            reg.register_text_any(TextAnyEntry {
                type_url: "type.example.com/Inner",
                text_encode: any_encode_text::<Inner>,
                text_merge: any_merge_text::<Inner>,
            });
            assert!(reg.text_any_by_url("type.example.com/Inner").is_some());
            assert!(reg.text_any_by_url("type.example.com/Missing").is_none());
        }

        #[test]
        fn register_and_lookup_text_ext_both_axes() {
            let mut reg = TypeRegistry::new();
            reg.register_text_ext(TextExtEntry {
                number: 50,
                full_name: "pkg.inner_ext",
                extendee: "pkg.Carrier",
                text_encode: message_encode_text::<Inner>,
                text_merge: message_merge_text::<Inner>,
            });
            // by_number axis.
            let e = reg.text_ext_by_number("pkg.Carrier", 50).unwrap();
            assert_eq!(e.full_name, "pkg.inner_ext");
            assert!(reg.text_ext_by_number("pkg.Carrier", 99).is_none());
            assert!(reg.text_ext_by_number("other.Msg", 50).is_none());
            // by_name axis → same entry via indirection.
            assert_eq!(reg.text_ext_by_name("pkg.inner_ext").unwrap().number, 50);
            assert!(reg.text_ext_by_name("pkg.missing").is_none());
        }

        /// Serializes with other tests touching the global TEXT_ANY/TEXT_EXT.
        static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

        #[test]
        fn set_type_registry_installs_text_halves() {
            let _g = GLOBAL_LOCK.lock().unwrap();

            let mut reg = TypeRegistry::new();
            reg.register_text_any(TextAnyEntry {
                type_url: "type.example.com/Global",
                text_encode: any_encode_text::<Inner>,
                text_merge: any_merge_text::<Inner>,
            });
            reg.register_text_ext(TextExtEntry {
                number: 77,
                full_name: "pkg.global_ext",
                extendee: "pkg.Msg",
                text_encode: message_encode_text::<Inner>,
                text_merge: message_merge_text::<Inner>,
            });
            set_type_registry(reg);

            assert!(global_text_any("type.example.com/Global").is_some());
            assert!(global_text_any("type.example.com/Absent").is_none());
            assert_eq!(
                global_text_ext_by_name("pkg.global_ext").map(|e| e.number),
                Some(77)
            );
            assert!(global_text_ext_by_number("pkg.Msg", 77).is_some());

            clear_text_registry();
            assert!(global_text_any("type.example.com/Global").is_none());
        }
    }
}
