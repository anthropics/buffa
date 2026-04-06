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

use std::collections::{HashMap, HashSet};

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

/// Top-level message and enum names in `file` that shadow prelude types
/// (`Option`, etc.).
///
/// Nested types are omitted: they are emitted inside `pub mod` scopes and do
/// not occupy the module root when a `.proto` is generated to its own `.rs`
/// file.
fn top_level_prelude_blocked_names(file: &FileDescriptorProto) -> HashSet<String> {
    let names = file
        .message_type
        .iter()
        .filter_map(|m| m.name.as_deref())
        .chain(file.enum_type.iter().filter_map(|e| e.name.as_deref()));
    check_names_for_prelude_collisions(names)
}

/// Per-protobuf-package union of [`top_level_prelude_blocked_names`] over
/// every path in `files_to_generate` that belongs to that package.
///
/// Generated Rust for one package is typically `include!`d under one `pub mod`,
/// so a top-level `message Option` in file A is a sibling `pub struct Option`
/// at that module root and shadows `core::option::Option` for every other
/// generated file in the **same** package. Files in other packages live under
/// different modules and are not affected.
pub(crate) fn compilation_prelude_blocked_by_package(
    file_descriptors: &[FileDescriptorProto],
    files_to_generate: &[String],
) -> HashMap<String, HashSet<String>> {
    let mut by_package: HashMap<String, HashSet<String>> = HashMap::new();
    for file_name in files_to_generate {
        let Some(file) = file_descriptors
            .iter()
            .find(|f| f.name.as_deref() == Some(file_name.as_str()))
        else {
            continue;
        };
        let pkg = file.package.clone().unwrap_or_default();
        by_package
            .entry(pkg)
            .or_default()
            .extend(top_level_prelude_blocked_names(file));
    }
    by_package
}

/// Tracks which short names are safe to use in a generated scope.
pub(crate) struct ImportResolver {
    /// Proto type names that collide with prelude names.
    blocked: HashSet<String>,
}

impl ImportResolver {
    /// Resolver for one output file when generating a batch: `blocked` is the
    /// per-package union of prelude collisions for the file's protobuf package
    /// (see [`compilation_prelude_blocked_by_package`]).
    pub(crate) fn from_compilation_blocked(blocked: &HashSet<String>) -> Self {
        Self {
            blocked: blocked.clone(),
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
