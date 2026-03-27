//! Import management for generated code.
//!
//! Controls whether types are emitted as short names (e.g. `Option<T>`) or
//! fully-qualified paths (e.g. `::core::option::Option<T>`), by detecting
//! collisions with proto-defined type names in the current file.
//!
//! `core` prelude types (`Option`, `Default`, etc.) are in scope in both `std`
//! and `no_std` contexts and can be emitted as bare names unless shadowed.
//! `alloc` types (`String`, `Vec`, `Box`) are always emitted as
//! `::buffa::alloc::*` paths because they are not in the `no_std` prelude,
//! consistent with the `HashMap` approach via `::buffa::__private::HashMap`.
//! Buffa runtime types are always emitted as absolute paths since generated
//! files may be combined via `include!`.

use std::collections::HashSet;

use crate::generated::descriptor::FileDescriptorProto;
use proc_macro2::TokenStream;
use quote::quote;

/// Names from the `core` prelude that are in scope in both `std` and `no_std`
/// contexts. These can be emitted as bare names unless a proto type in the
/// same file shadows them.
///
/// `String`, `Vec`, and `Box` are intentionally excluded — they are only in
/// the `std` prelude, not the `no_std` prelude (even with `extern crate alloc`).
/// Those types are always emitted via `::buffa::alloc::*` re-exports.
const PRELUDE_NAMES: &[&str] = &["Option"];

/// Tracks which short names are safe to use in a generated file.
pub(crate) struct ImportResolver {
    /// Proto type names that collide with prelude names.
    blocked: HashSet<String>,
}

impl ImportResolver {
    /// Build a resolver for a single `.proto` file by checking top-level
    /// message and enum names against the set of short names we want to use.
    pub fn for_file(file: &FileDescriptorProto) -> Self {
        let mut proto_names = HashSet::new();
        for msg in &file.message_type {
            if let Some(name) = &msg.name {
                proto_names.insert(name.as_str());
            }
        }
        for e in &file.enum_type {
            if let Some(name) = &e.name {
                proto_names.insert(name.as_str());
            }
        }

        let mut blocked = HashSet::new();
        for &name in PRELUDE_NAMES {
            if proto_names.contains(name) {
                blocked.insert(name.to_string());
            }
        }
        Self { blocked }
    }

    /// Emit the `use` block for the top of a generated file.
    ///
    /// Currently empty — prelude types need no imports and buffa runtime
    /// types use absolute paths to be `include!`-safe. This method exists
    /// as a hook for future import additions.
    pub fn generate_use_block(&self) -> TokenStream {
        TokenStream::new()
    }

    /// Whether `name` is safe to use unqualified (not shadowed by a proto type).
    fn is_available(&self, name: &str) -> bool {
        !self.blocked.contains(name)
    }

    // ── Prelude type tokens ─────────────────────────────────────────────

    pub fn option(&self) -> TokenStream {
        if self.is_available("Option") {
            quote! { Option }
        } else {
            quote! { ::core::option::Option }
        }
    }

    // ── Alloc types (always absolute, no_std-safe via ::buffa::alloc) ───

    pub fn string(&self) -> TokenStream {
        quote! { ::buffa::alloc::string::String }
    }

    pub fn vec(&self) -> TokenStream {
        quote! { ::buffa::alloc::vec::Vec }
    }

    // ── Buffa runtime types (always absolute, include!-safe) ────────────

    pub fn message_field(&self) -> TokenStream {
        quote! { ::buffa::MessageField }
    }

    pub fn enum_value(&self) -> TokenStream {
        quote! { ::buffa::EnumValue }
    }

    pub fn hashmap(&self) -> TokenStream {
        quote! { ::buffa::__private::HashMap }
    }
}
