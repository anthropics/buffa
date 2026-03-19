//! Type registry for `google.protobuf.Any` proto3 JSON serialization.
//!
//! Proto3 JSON encoding of `Any` requires knowing how to serialize and
//! deserialize the embedded message type. The [`AnyRegistry`] maps type URLs
//! to conversion functions that translate between wire-format bytes and
//! [`serde_json::Value`].
//!
//! # Usage
//!
//! ```rust,no_run
//! use buffa::json_registry::{JsonRegistry, set_json_registry};
//!
//! let mut reg = JsonRegistry::new();
//! // ... register types via generated register_json(&mut reg) ...
//! set_json_registry(reg);
//! // Now serde_json operations on messages containing Any fields will use
//! // the registry for proper proto3 JSON encoding.
//! ```

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicPtr, Ordering};
use hashbrown::HashMap;

/// Registry entry for a single message type, holding function pointers
/// for proto3 JSON ↔ Any conversion.
pub struct AnyTypeEntry {
    /// The full type URL (e.g. `"type.googleapis.com/google.protobuf.Duration"`).
    pub type_url: &'static str,

    /// Serialize: binary `Any.value` bytes → JSON representation of the
    /// embedded message (without `@type`).
    pub to_json: fn(&[u8]) -> Result<serde_json::Value, String>,

    /// Deserialize: JSON representation (without `@type`) → binary wire
    /// bytes for `Any.value`. Takes ownership to avoid cloning large values.
    pub from_json: fn(serde_json::Value) -> Result<Vec<u8>, String>,

    /// Whether this type uses `"value"` wrapping in Any JSON.
    ///
    /// WKTs like Duration, Timestamp, FieldMask, Value, Struct, ListValue,
    /// and wrapper types serialize to non-object JSON (strings, numbers,
    /// arrays). In `Any`, they must be wrapped:
    /// `{"@type": "...", "value": <wkt_json>}`.
    ///
    /// Regular messages serialize to JSON objects and have their fields
    /// inlined: `{"@type": "...", "field1": v1, "field2": v2}`.
    pub is_wkt: bool,
}

/// A registry mapping type URLs to their JSON encode/decode functions.
///
/// Populate with [`register`](AnyRegistry::register), then install globally
/// with [`set_any_registry`] before using `serde_json` to serialize or
/// deserialize messages containing `Any` fields.
pub struct AnyRegistry {
    entries: HashMap<String, AnyTypeEntry>,
}

impl AnyRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        AnyRegistry {
            entries: HashMap::new(),
        }
    }

    /// Registers a type entry. Replaces any existing entry for the same type URL.
    pub fn register(&mut self, entry: AnyTypeEntry) {
        self.entries.insert(entry.type_url.to_owned(), entry);
    }

    /// Looks up a type entry by its full type URL.
    pub fn lookup(&self, type_url: &str) -> Option<&AnyTypeEntry> {
        self.entries.get(type_url)
    }
}

impl core::fmt::Debug for AnyRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AnyRegistry")
            .field("type_urls", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Default for AnyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

static ANY_REGISTRY: AtomicPtr<AnyRegistry> = AtomicPtr::new(core::ptr::null_mut());

/// Sets the global `Any` type registry.
///
/// Call this before using `serde_json::from_str` or `serde_json::to_string`
/// on messages containing `Any` fields. The registry is visible to all
/// threads and persists for the lifetime of the process.
///
/// If a registry was previously installed, the old allocation is
/// intentionally leaked. This avoids a use-after-free race where a
/// concurrent reader in [`with_any_registry`] could hold a reference to the
/// old registry while it is dropped. In practice `set_any_registry` is
/// called once at startup, so the leak is negligible.
#[deprecated(since = "0.3.0", note = "use buffa::json_registry::set_json_registry")]
pub fn set_any_registry(registry: Box<AnyRegistry>) {
    let new_ptr = Box::into_raw(registry);
    ANY_REGISTRY.swap(new_ptr, Ordering::AcqRel);
}

/// Clears the global `Any` type registry.
///
/// Intended for test cleanup. The old allocation is intentionally leaked
/// (see [`set_any_registry`] for rationale).
#[doc(hidden)]
pub fn clear_any_registry() {
    ANY_REGISTRY.swap(core::ptr::null_mut(), Ordering::AcqRel);
}

/// Runs a closure with access to the global `Any` type registry.
///
/// Passes `None` if no registry has been set via [`set_any_registry`].
pub fn with_any_registry<R>(f: impl FnOnce(Option<&AnyRegistry>) -> R) -> R {
    let ptr = ANY_REGISTRY.load(Ordering::Acquire);
    if ptr.is_null() {
        f(None)
    } else {
        // SAFETY: a non-null pointer was installed by `set_any_registry` via
        // `Box::into_raw`. The pointee is never freed — `set_any_registry`
        // and `clear_any_registry` intentionally leak old registries — so the
        // pointer remains valid for the lifetime of the process. The
        // `Acquire` ordering ensures we see the fully initialized data.
        f(Some(unsafe { &*ptr }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(deprecated)]

    use super::*;

    fn dummy_to_json(_bytes: &[u8]) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    fn dummy_from_json(_value: serde_json::Value) -> Result<Vec<u8>, String> {
        Ok(Vec::new())
    }

    #[test]
    fn register_and_lookup() {
        let mut registry = AnyRegistry::new();
        registry.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Message",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });

        assert!(registry
            .lookup("type.googleapis.com/test.Message")
            .is_some());
        assert!(registry.lookup("type.googleapis.com/test.Other").is_none());
    }

    #[test]
    fn lookup_returns_correct_entry() {
        let mut registry = AnyRegistry::new();
        registry.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Wkt",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: true,
        });
        registry.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Regular",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });

        let wkt = registry.lookup("type.googleapis.com/test.Wkt").unwrap();
        assert!(wkt.is_wkt);

        let regular = registry.lookup("type.googleapis.com/test.Regular").unwrap();
        assert!(!regular.is_wkt);
    }

    /// Serializes tests that manipulate the global `ANY_REGISTRY`.
    static REGISTRY_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn global_registry() {
        let _guard = REGISTRY_TEST_LOCK.lock().unwrap();
        let mut registry = AnyRegistry::new();
        registry.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Global",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });
        set_any_registry(Box::new(registry));

        with_any_registry(|reg| {
            let reg = reg.expect("registry should be set");
            assert!(reg.lookup("type.googleapis.com/test.Global").is_some());
        });

        clear_any_registry();

        with_any_registry(|reg| {
            assert!(reg.is_none());
        });
    }

    #[test]
    fn set_registry_twice_supersedes_first() {
        let _guard = REGISTRY_TEST_LOCK.lock().unwrap();
        let mut first = AnyRegistry::new();
        first.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.First",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });
        set_any_registry(Box::new(first));

        let mut second = AnyRegistry::new();
        second.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Second",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: true,
        });
        set_any_registry(Box::new(second));

        with_any_registry(|reg| {
            let reg = reg.expect("registry should be set");
            assert!(
                reg.lookup("type.googleapis.com/test.First").is_none(),
                "first registry should be superseded"
            );
            assert!(
                reg.lookup("type.googleapis.com/test.Second").is_some(),
                "second registry should be active"
            );
        });

        clear_any_registry();
    }

    #[test]
    fn default_registry_is_empty() {
        let registry = AnyRegistry::default();
        assert!(registry.lookup("anything").is_none());
    }

    #[test]
    fn debug_shows_type_urls() {
        let mut registry = AnyRegistry::new();
        registry.register(AnyTypeEntry {
            type_url: "type.googleapis.com/test.Debug",
            to_json: dummy_to_json,
            from_json: dummy_from_json,
            is_wkt: false,
        });
        let debug = format!("{:?}", registry);
        assert!(
            debug.contains("test.Debug"),
            "Debug output should contain the type URL: {debug}"
        );
        assert!(
            debug.contains("AnyRegistry"),
            "Debug output should contain struct name: {debug}"
        );
    }
}
