//! Import management for generated code.
//!
//! Controls whether types are emitted as short names (e.g. `Option<T>`) or
//! fully-qualified paths (e.g. `::core::option::Option<T>`), by detecting
//! collisions with proto-defined type names in the current scope.
//!
//! `core` prelude types (`Option`, `Default`, etc.) are in scope in both `std`
//! and `no_std` contexts and can be emitted as bare names unless shadowed.
//! `alloc` types (`String`, `Vec`, `Box`) are always emitted as
//! `::buffa::alloc::*` paths because they are not in the `no_std` prelude,
//! consistent with the `HashMap` approach via `::buffa::__private::HashMap`.
//! Buffa runtime types are always emitted as absolute paths since generated
//! files may be combined via `include!`.

use std::collections::HashSet;

use crate::generated::descriptor::{DescriptorProto, FileDescriptorProto};
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

fn check_names_for_prelude_collisions<'a>(
    names: impl Iterator<Item = &'a str>,
) -> HashSet<String> {
    let prelude: HashSet<&str> = PRELUDE_NAMES.iter().copied().collect();
    let mut blocked = HashSet::new();
    for name in names {
        if prelude.contains(name) {
            blocked.insert(name.to_string());
        }
    }
    blocked
}

/// Tracks which short names are safe to use in a generated scope.
pub(crate) struct ImportResolver {
    /// Proto type names that collide with prelude names.
    blocked: HashSet<String>,
}

impl ImportResolver {
    /// Build a resolver for a single `.proto` file by checking top-level
    /// message and enum names against the set of short names we want to use.
    pub fn for_file(file: &FileDescriptorProto) -> Self {
        let names = file
            .message_type
            .iter()
            .filter_map(|m| m.name.as_deref())
            .chain(file.enum_type.iter().filter_map(|e| e.name.as_deref()));
        Self {
            blocked: check_names_for_prelude_collisions(names),
        }
    }

    /// Build a child resolver for a message's `pub mod` scope.
    ///
    /// Each message module contains `use super::*`, so parent-scope blocked
    /// names propagate. On top of those, the message's own nested types and
    /// nested enums introduce additional names that can shadow prelude types.
    pub fn child_for_message(&self, msg: &DescriptorProto) -> Self {
        let mut blocked = self.blocked.clone();
        let child_names = msg
            .nested_type
            .iter()
            .filter_map(|m| m.name.as_deref())
            .chain(msg.enum_type.iter().filter_map(|e| e.name.as_deref()));
        blocked.extend(check_names_for_prelude_collisions(child_names));
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
