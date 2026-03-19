//! Unified registry for proto3 JSON serialization — `Any` types + extensions.
//!
//! Replaces the separate [`AnyRegistry`] and [`ExtensionRegistry`] globals
//! with a single setup path. Internally both maps are kept; the unification
//! is at the API layer so users have one `register_json(&mut reg)` call per
//! generated file and one [`set_json_registry`] install.
//!
//! # Usage
//!
//! ```rust,no_run
//! use buffa::json_registry::{JsonRegistry, set_json_registry};
//!
//! let mut reg = JsonRegistry::new();
//! // Per generated file — registers both Any types and extensions:
//! // my_pkg::register_json(&mut reg);
//! // Well-known types (hand-registered, know their own `is_wkt` flag):
//! // buffa_types::register_wkt_types(reg.any_mut());
//! set_json_registry(reg);
//! ```
//!
//! # Codegen helpers
//!
//! [`any_to_json`] and [`any_from_json`] are generic function pointers emitted
//! by codegen into each message's `AnyTypeEntry` const. They compose
//! `Message::decode_from_slice` / `encode_to_vec` with the message's own
//! `Serialize` / `Deserialize` — the same mechanical closure users previously
//! wrote by hand.
//!
//! [`AnyRegistry`]: crate::any_registry::AnyRegistry
//! [`ExtensionRegistry`]: crate::extension_registry::ExtensionRegistry

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::any_registry::AnyRegistry;
use crate::extension_registry::ExtensionRegistry;

pub use crate::any_registry::AnyTypeEntry;
pub use crate::extension_registry::ExtensionRegistryEntry;

/// Unified JSON serialization registry: `Any` type entries + extension entries.
///
/// A thin aggregate over [`AnyRegistry`] and [`ExtensionRegistry`]. Populate
/// with generated `register_json(&mut reg)` functions, then install once with
/// [`set_json_registry`].
#[derive(Default)]
pub struct JsonRegistry {
    any: AnyRegistry,
    ext: ExtensionRegistry,
}

impl JsonRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an `Any` type entry. Replaces any existing entry for the
    /// same type URL.
    pub fn register_any(&mut self, entry: AnyTypeEntry) {
        self.any.register(entry);
    }

    /// Registers an extension entry. Replaces any existing entry at the same
    /// `(extendee, number)` or `full_name`.
    pub fn register_extension(&mut self, entry: ExtensionRegistryEntry) {
        self.ext.register(entry);
    }

    /// Mutable access to the inner [`AnyRegistry`], for APIs that still take
    /// it directly (notably `buffa_types::register_wkt_types`).
    pub fn any_mut(&mut self) -> &mut AnyRegistry {
        &mut self.any
    }

    /// Mutable access to the inner [`ExtensionRegistry`].
    pub fn ext_mut(&mut self) -> &mut ExtensionRegistry {
        &mut self.ext
    }
}

impl core::fmt::Debug for JsonRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsonRegistry")
            .field("any", &self.any)
            .finish_non_exhaustive()
    }
}

/// Install the global JSON registry.
///
/// Decomposes into separate [`set_any_registry`] and [`set_extension_registry`]
/// installs — generated code's serde impls and `Any`'s serde impl consult
/// those two `AtomicPtr` globals directly, so they must keep working.
///
/// Call once at startup, before any JSON serialization or deserialization
/// involving `Any` fields or extension fields. Both halves are leaked (live
/// for the program lifetime); subsequent calls leak the old registries.
///
/// [`set_any_registry`]: crate::any_registry::set_any_registry
/// [`set_extension_registry`]: crate::extension_registry::set_extension_registry
pub fn set_json_registry(reg: JsonRegistry) {
    let JsonRegistry { any, ext } = reg;
    #[allow(deprecated)]
    {
        crate::any_registry::set_any_registry(Box::new(any));
        crate::extension_registry::set_extension_registry(Box::new(ext));
    }
}

// ── Generic Any-entry converters (codegen points entry fields here) ────────
//
// Monomorphized per message type M; the resulting fn items coerce to the
// `fn(&[u8]) -> ...` / `fn(serde_json::Value) -> ...` pointer types on
// AnyTypeEntry. Same pattern as extension_registry::helpers::message_to_json.

/// `Any.value` bytes → JSON: decode `M` from wire bytes, then serialize via
/// `M`'s own `Serialize` impl.
///
/// Codegen emits `to_json: ::buffa::json_registry::any_to_json::<Foo>` in each
/// message's `AnyTypeEntry` const. Not intended for direct use.
pub fn any_to_json<M>(bytes: &[u8]) -> Result<serde_json::Value, String>
where
    M: crate::Message + serde::Serialize,
{
    let m = M::decode_from_slice(bytes).map_err(|e| format!("{e}"))?;
    serde_json::to_value(&m).map_err(|e| format!("{e}"))
}

/// JSON → `Any.value` bytes: deserialize `M` via its `Deserialize` impl, then
/// encode to wire bytes.
///
/// Codegen emits `from_json: ::buffa::json_registry::any_from_json::<Foo>` in
/// each message's `AnyTypeEntry` const. Not intended for direct use.
pub fn any_from_json<M>(v: serde_json::Value) -> Result<Vec<u8>, String>
where
    M: crate::Message + for<'de> serde::Deserialize<'de>,
{
    let m: M = serde_json::from_value(v).map_err(|e| format!("{e}"))?;
    Ok(m.encode_to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::any_registry::with_any_registry;
    use crate::extension_registry::{extension_registry, helpers};

    fn dummy_to_json(_bytes: &[u8]) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({"ok": true}))
    }
    fn dummy_from_json(_v: serde_json::Value) -> Result<Vec<u8>, String> {
        Ok(alloc::vec![1, 2, 3])
    }

    #[test]
    fn register_any_and_extension_independently() {
        let mut reg = JsonRegistry::new();
        reg.register_any(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Foo",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });
        reg.register_extension(ExtensionRegistryEntry {
            number: 100,
            full_name: "test.ext",
            extendee: "test.Foo",
            to_json: helpers::int32_to_json,
            from_json: helpers::int32_from_json,
        });

        assert!(reg.any.lookup("type.googleapis.com/test.Foo").is_some());
        assert!(reg.ext.by_number("test.Foo", 100).is_some());
        assert!(reg.ext.by_name("test.ext").is_some());
    }

    #[test]
    fn any_mut_and_ext_mut_give_inner_access() {
        let mut reg = JsonRegistry::new();
        // Simulates buffa_types::register_wkt_types(reg.any_mut()).
        reg.any_mut().register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Wkt",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: true,
        });
        assert!(
            reg.any_mut()
                .lookup("type.googleapis.com/test.Wkt")
                .unwrap()
                .is_wkt
        );
        // ext_mut exercised for symmetry.
        reg.ext_mut().register(ExtensionRegistryEntry {
            number: 1,
            full_name: "x",
            extendee: "Y",
            to_json: helpers::bool_to_json,
            from_json: helpers::bool_from_json,
        });
        assert!(reg.ext_mut().by_name("x").is_some());
    }

    /// Serializes this test with the global-registry tests in `any_registry`
    /// and `extension_registry` — all three modules share the same two
    /// `AtomicPtr` globals. This lock is module-local but the globals aren't,
    /// so we also immediately verify-and-clear rather than asserting absence.
    static GLOBAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn set_json_registry_installs_both_halves() {
        let _g = GLOBAL_LOCK.lock().unwrap();

        let mut reg = JsonRegistry::new();
        reg.register_any(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Unified",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });
        reg.register_extension(ExtensionRegistryEntry {
            number: 200,
            full_name: "test.unified_ext",
            extendee: "test.Unified",
            to_json: helpers::int32_to_json,
            from_json: helpers::int32_from_json,
        });
        set_json_registry(reg);

        // Any half reaches the with_any_registry global.
        with_any_registry(|r| {
            let r = r.expect("any registry installed");
            assert!(r.lookup("type.googleapis.com/test.Unified").is_some());
        });
        // Extension half reaches extension_registry().
        let ext = extension_registry().expect("extension registry installed");
        assert_eq!(ext.by_name("test.unified_ext").map(|e| e.number), Some(200));

        // Clear the Any half for any_registry's own global tests; the
        // extension half has no clear fn (its tests tolerate leaked state).
        crate::any_registry::clear_any_registry();
    }

    #[test]
    fn default_is_empty() {
        let reg = JsonRegistry::default();
        assert!(reg.any.lookup("anything").is_none());
        assert!(reg.ext.by_name("anything").is_none());
    }

    #[test]
    fn debug_impl_mentions_any() {
        let reg = JsonRegistry::new();
        let s = format!("{reg:?}");
        assert!(s.contains("JsonRegistry"), "{s}");
        assert!(s.contains("any"), "{s}");
    }
}
